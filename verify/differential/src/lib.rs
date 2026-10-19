//! Runs the same `.wasm` on wasmicon-core and on wasmtime and cross-checks the traces.
//!
//! This verifies the interpreter against an external reference implementation,
//! independently of any hardware (docs/verification-report.md §2).
//!
//! The key is that the two engines **share the HAL**. `Hal` from `wasmicon-port`
//! implements `wasmicon_core::Resolver`, but its `call` does not depend on the engine
//! (it touches only the argument slots and a slice of guest memory). The same `Hal` is
//! driven from wasmtime too, so a difference between the traces is a difference between
//! the engines, that is, a bug in our interpreter.

use anyhow::{Result, anyhow, bail};
use wasmicon_core::instance::Resolver;
use wasmicon_host::hal::HostBoard;
use wasmicon_port::Hal;
use wasmtime::{Caller, Engine, Extern, FuncType, Linker, Module, Store, Val, ValType};

/// The `env.abort` that AssemblyScript imports (docs/decisions.md P-02).
const HOST_ENV_ABORT: u32 = 0xffff;

/// Maximum number of arguments or results of a host function.
///
/// Kept equal to `MAX_HOST_ARITY` in `wasmicon_core::interp`. The current maximum is 8
/// (`i2c.write-read`), so there is slack, but an index-out-of-bounds panic inside a
/// wasmtime callback would be hard to read, so the limit is checked explicitly.
const MAX_ARITY: usize = 16;

/// The state carried by the wasmtime `Store`.
struct State {
    hal: Hal<HostBoard>,
}

/// Converts a signature string (docs/abi-spec.md §7) into a wasmtime function type.
fn func_type(engine: &Engine, sig: &str) -> Result<FuncType> {
    let Some((params, results)) = sig.split_once(':') else {
        bail!("malformed signature `{sig}`");
    };
    let map = |s: &str| -> Result<Vec<ValType>> {
        s.chars()
            .map(|c| match c {
                'i' => Ok(ValType::I32),
                'I' => Ok(ValType::I64),
                'f' => Ok(ValType::F32),
                'F' => Ok(ValType::F64),
                _ => bail!("unknown signature character `{c}`"),
            })
            .collect()
    };
    Ok(FuncType::new(engine, map(params)?, map(results)?))
}

fn to_slot(v: &Val) -> u64 {
    match v {
        Val::I32(x) => u64::from(*x as u32),
        Val::I64(x) => *x as u64,
        Val::F32(bits) => u64::from(*bits),
        Val::F64(bits) => *bits,
        _ => 0,
    }
}

fn from_slot(ty: &ValType, slot: u64) -> Val {
    match ty {
        ValType::I32 => Val::I32(slot as u32 as i32),
        ValType::I64 => Val::I64(slot as i64),
        ValType::F32 => Val::F32(slot as u32),
        ValType::F64 => Val::F64(slot),
        _ => Val::I32(0),
    }
}

/// Borrows the guest's linear memory and the state at the same time.
fn memory_and_state<'a>(
    caller: &'a mut Caller<'_, State>,
) -> wasmtime::Result<(&'a mut [u8], &'a mut State)> {
    let Some(Extern::Memory(mem)) = caller.get_export("memory") else {
        return Err(wasmtime::Error::msg(
            "missing export `memory` (docs/abi-spec.md §3.3)",
        ));
    };
    Ok(mem.data_and_store_mut(caller))
}

/// Registers one host function with wasmtime. The body delegates to `Hal::call`.
fn define(
    linker: &mut Linker<State>,
    engine: &Engine,
    module: &str,
    name: &str,
    sig: &str,
    host: u32,
) -> Result<()> {
    let ty = func_type(engine, sig)?;
    let result_types: Vec<ValType> = ty.results().collect();
    let module_name = module.to_string();
    let fn_name = name.to_string();
    linker
        .func_new(module, name, ty, move |mut caller, params, results| {
            let n = params.len();
            let m = result_types.len();
            if n > MAX_ARITY || m > MAX_ARITY {
                return Err(wasmtime::Error::msg(format!(
                    "{module_name}/{fn_name} has more than {MAX_ARITY} arguments or results"
                )));
            }
            let mut args = [0u64; MAX_ARITY];
            for (i, v) in params.iter().enumerate() {
                args[i] = to_slot(v);
            }
            let mut out = [0u64; MAX_ARITY];
            let (mem, state) = memory_and_state(&mut caller)?;
            state
                .hal
                .call(host, &args[..n], &mut out[..m], mem)
                .map_err(|e| {
                    wasmtime::Error::msg(format!("{} [{}]", e.reason(), e.kind().name()))
                })?;
            for (i, ty) in result_types.iter().enumerate() {
                results[i] = from_slot(ty, out[i]);
            }
            Ok(())
        })
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

/// The result of running the same `.wasm` on the two engines.
pub struct Comparison {
    /// The trace from our interpreter.
    pub wasmicon: String,
    /// The trace from wasmtime.
    pub wasmtime: String,
}

impl Comparison {
    /// Returns whether the two traces are identical.
    #[must_use]
    pub fn agrees(&self) -> bool {
        self.wasmicon == self.wasmtime
    }

    /// Returns the first differing line (1-based) together with both sides.
    #[must_use]
    pub fn first_difference(&self) -> Option<(usize, String, String)> {
        let a: Vec<&str> = self.wasmicon.lines().collect();
        let b: Vec<&str> = self.wasmtime.lines().collect();
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            if x != y {
                return Some((i + 1, (*x).to_string(), (*y).to_string()));
            }
        }
        if a.len() != b.len() {
            let i = a.len().min(b.len());
            return Some((
                i + 1,
                a.get(i).map_or("(end)", |s| s).to_string(),
                b.get(i).map_or("(end)", |s| s).to_string(),
            ));
        }
        None
    }
}

/// Runs the module on wasmtime and returns its trace.
///
/// # Errors
/// When the module cannot be loaded, linked or run.
pub fn run_on_wasmtime(wasm: &[u8], i2c_replay: Vec<Vec<u8>>) -> Result<String> {
    let engine = Engine::default();
    let module =
        Module::new(&engine, wasm).map_err(|e| anyhow!("wasmtime cannot load the module: {e}"))?;

    let mut linker: Linker<State> = Linker::new(&engine);
    for desc in &wasmicon_core::generated::IMPORTS {
        define(
            &mut linker,
            &engine,
            desc.module,
            desc.name,
            desc.sig,
            u32::from(desc.host_fn.index()),
        )?;
    }
    // The one import outside world app (docs/decisions.md P-02).
    // ports/common's AS_ABORT is the source of truth for the spelling (it decides the link).
    define(
        &mut linker,
        &engine,
        wasmicon_port::AS_ABORT.0,
        wasmicon_port::AS_ABORT.1,
        "iiii:",
        HOST_ENV_ABORT,
    )?;

    let board = HostBoard::new().with_i2c_replay(i2c_replay);
    let mut store = Store::new(
        &engine,
        State {
            hal: Hal::new(board, true).with_roles(wasmicon_host::hal::host_roles()),
        },
    );
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| anyhow!("wasmtime cannot instantiate the module: {e}"))?;

    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| anyhow!("missing export `run` (docs/abi-spec.md §3.3): {e}"))?;
    run.call(&mut store, ())
        .map_err(|e| anyhow!("run failed on wasmtime: {e}"))?;

    Ok(store.data_mut().hal.board_mut().trace_output().to_string())
}

/// Runs the same `.wasm` on our interpreter and on wasmtime and compares the traces.
///
/// # Errors
/// When either engine fails to run the module.
pub fn compare(wasm: &[u8], i2c_replay: Vec<Vec<u8>>) -> Result<Comparison> {
    let ours = wasmicon_host::run_wasm_with(wasm, true, i2c_replay.clone())
        .map_err(|e| anyhow!("wasmicon: {} [{}]", e.reason(), e.kind().name()))?;
    let theirs = run_on_wasmtime(wasm, i2c_replay)?;
    Ok(Comparison {
        wasmicon: ours.trace,
        wasmtime: theirs,
    })
}
