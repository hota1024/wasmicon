//! 自作インタプリタと wasmtime で同じ `.wasm` を走らせ、トレースを突き合わせる。
//!
//! docs/handoff.md §5 Phase 6 の (3)。実機とは無関係に**インタプリタの正しさ**を
//! 外部の参照実装で検証する。
//!
//! 肝は **HAL を共有する**こと。`wasmicon-port` の `Hal` は
//! `wasmicon_core::Resolver` を実装しているが、その `call` はエンジンに依存しない
//! （引数のスロット列とゲストメモリのスライスしか触らない）。同じ `Hal` を
//! wasmtime からも駆動するので、トレースに差が出たらそれは**エンジンの差**、
//! つまり自作インタプリタのバグということになる。

use anyhow::{Result, anyhow, bail};
use wasmicon_core::instance::Resolver;
use wasmicon_host::hal::HostBoard;
use wasmicon_port::Hal;
use wasmtime::{Caller, Engine, Extern, FuncType, Linker, Module, Store, Val, ValType};

/// AssemblyScript が import する `env.abort`（docs/handoff.md §6）。
const HOST_ENV_ABORT: u32 = 0xffff;

/// ホスト関数の引数・戻り値の最大個数。
///
/// `wasmicon_core::interp` の `MAX_HOST_ARITY` と同じ値にしてある。
/// 現状の最大は `i2c.write-read` の 8 なので余裕はあるが、超えたときに
/// wasmtime のコールバックの中で添字外れの panic になるのは読みにくいので
/// 明示的に検査する。
const MAX_ARITY: usize = 16;

/// wasmtime の `Store` に載せる状態。
struct State {
    hal: Hal<HostBoard>,
}

/// シグネチャ文字列（abi-spec §7）から wasmtime の関数型へ。
fn func_type(engine: &Engine, sig: &str) -> Result<FuncType> {
    let Some((params, results)) = sig.split_once(':') else {
        bail!("シグネチャが壊れている: {sig}");
    };
    let map = |s: &str| -> Result<Vec<ValType>> {
        s.chars()
            .map(|c| match c {
                'i' => Ok(ValType::I32),
                'I' => Ok(ValType::I64),
                'f' => Ok(ValType::F32),
                'F' => Ok(ValType::F64),
                _ => bail!("知らないシグネチャ文字: {c}"),
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

/// ゲストの線形メモリと状態を同時に借りる。
fn memory_and_state<'a>(
    caller: &'a mut Caller<'_, State>,
) -> wasmtime::Result<(&'a mut [u8], &'a mut State)> {
    let Some(Extern::Memory(mem)) = caller.get_export("memory") else {
        return Err(wasmtime::Error::msg(
            "ゲストが memory を export していない（abi-spec §3.3）",
        ));
    };
    Ok(mem.data_and_store_mut(caller))
}

/// 1 つのホスト関数を wasmtime に登録する。`Hal::call` に委ねる。
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
                    "{module_name}/{fn_name} の引数か戻り値が {MAX_ARITY} を超えている"
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

/// 同じ `.wasm` を 2 つのエンジンで走らせた結果。
pub struct Comparison {
    /// 自作インタプリタのトレース。
    pub wasmicon: String,
    /// wasmtime のトレース。
    pub wasmtime: String,
}

impl Comparison {
    /// 一致しているか。
    #[must_use]
    pub fn agrees(&self) -> bool {
        self.wasmicon == self.wasmtime
    }

    /// 最初に食い違う行（1 始まり）と、その両側。
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
                a.get(i).map_or("(終わり)", |s| s).to_string(),
                b.get(i).map_or("(終わり)", |s| s).to_string(),
            ));
        }
        None
    }
}

/// wasmtime で走らせてトレースを返す。
///
/// # Errors
/// モジュールが読めない、リンクできない、実行が失敗したとき。
pub fn run_on_wasmtime(wasm: &[u8], i2c_replay: Vec<Vec<u8>>) -> Result<String> {
    let engine = Engine::default();
    let module =
        Module::new(&engine, wasm).map_err(|e| anyhow!("wasmtime がモジュールを読めない: {e}"))?;

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
    // world app には無い例外的な import（docs/handoff.md §6）。
    define(
        &mut linker,
        &engine,
        "env",
        "abort",
        "iiii:",
        HOST_ENV_ABORT,
    )?;

    let board = HostBoard::new().with_i2c_replay(i2c_replay);
    let mut store = Store::new(
        &engine,
        State {
            hal: Hal::new(board, true),
        },
    );
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| anyhow!("wasmtime がインスタンス化できない: {e}"))?;

    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| anyhow!("export run が無い（abi-spec §3.3）: {e}"))?;
    run.call(&mut store, ())
        .map_err(|e| anyhow!("wasmtime で run が失敗した: {e}"))?;

    Ok(store.data_mut().hal.board_mut().trace_output().to_string())
}

/// 同じ `.wasm` を自作インタプリタと wasmtime で走らせて比べる。
///
/// # Errors
/// どちらかのエンジンで実行が失敗したとき。
pub fn compare(wasm: &[u8], i2c_replay: Vec<Vec<u8>>) -> Result<Comparison> {
    let ours = wasmicon_host::run_wasm_with(wasm, true, i2c_replay.clone())
        .map_err(|e| anyhow!("wasmicon: {} [{}]", e.reason(), e.kind().name()))?;
    let theirs = run_on_wasmtime(wasm, i2c_replay)?;
    Ok(Comparison {
        wasmicon: ours.trace,
        wasmtime: theirs,
    })
}
