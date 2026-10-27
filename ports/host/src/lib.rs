//! PC port. Loads a `.wasm` and runs it on the mock HAL.
//!
//! Runs a guest on the host and collects its trace (docs/abi-spec.md §9). The
//! host tests and the `wasmicon` CLI build on this.

pub mod assets;
pub mod hal;

use wasmicon_core::validate::Validated;
use wasmicon_core::{Arena, Config, Error, Exec, decode, instantiate, invoke, validate};
use wasmicon_port::Hal;

/// Arena size. The remainder becomes linear memory. `profile::HOST` is the source of truth.
pub const ARENA: usize = wasmicon_port::profile::HOST.arena;
/// Working memory used only during validation. `profile::HOST` is the source of truth.
pub const SCRATCH: usize = wasmicon_port::profile::HOST.scratch;

/// Result of a run.
pub struct Outcome {
    /// Trace in the docs/abi-spec.md §9 format.
    pub trace: String,
    /// What the guest needed of each `Config` limit. `None` when decode or
    /// validate failed. Includes the calls up to a trap.
    pub usage: Option<Usage>,
    /// The mock display's visible picture after the run (`None` with
    /// `Options::no_display`).
    pub frame: Option<hal::Frame>,
}

/// The most of each stack-like `Config` limit a module or run needed. Fields
/// are named after the `Config` field they size, so a board fits when every
/// field is at most that board's value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Deepest block nesting in any function (validate).
    pub max_control_depth: u32,
    /// Highest operand stack in any function (validate).
    pub max_value_stack: u32,
    /// Most parameters plus locals in any function (validate).
    pub max_locals: u32,
    /// Deepest call chain (run time; 0 until something is invoked).
    pub max_call_depth: u32,
    /// Most operand slots reserved at once (run time).
    pub operand_stack_slots: u32,
}

impl Usage {
    /// The validate-time fields; the run-time ones stay 0.
    #[must_use]
    pub fn of_module(v: &Validated<'_>) -> Self {
        Usage {
            max_control_depth: v.max_control,
            max_value_stack: v.funcs.iter().map(|f| f.max_stack).max().unwrap_or(0),
            max_locals: v.funcs.iter().map(|f| f.frame_slots).max().unwrap_or(0),
            ..Usage::default()
        }
    }

    /// The limits of `cfg` this usage is over, as `(field, needed, limit)`.
    #[must_use]
    pub fn over(&self, cfg: &Config) -> Vec<(&'static str, u32, usize)> {
        [
            (
                "max_control_depth",
                self.max_control_depth,
                cfg.max_control_depth,
            ),
            ("max_value_stack", self.max_value_stack, cfg.max_value_stack),
            ("max_locals", self.max_locals, cfg.max_locals),
            ("max_call_depth", self.max_call_depth, cfg.max_call_depth),
            (
                "operand_stack_slots",
                self.operand_stack_slots,
                cfg.operand_stack_slots,
            ),
        ]
        .into_iter()
        .filter(|&(_, need, limit)| need as usize > limit)
        .collect()
    }
}

/// Loads recorded I2C responses.
///
/// One response per line, as hex bytes. Lines starting with `#` and empty
/// lines are ignored.
///
/// # Errors
/// The file cannot be read, or a line is not hex.
pub fn load_i2c_replay(path: &std::path::Path) -> std::io::Result<Vec<Vec<u8>>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut bytes = Vec::new();
        for tok in line.split_whitespace() {
            let b = u8::from_str_radix(tok, 16).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "a token is not a hex byte")
            })?;
            bytes.push(b);
        }
        out.push(bytes);
    }
    Ok(out)
}

/// Loads a `.wasm` and calls `run`.
///
/// I2C responses come from the file named by the `WASMICON_I2C_REPLAY`
/// environment variable.
///
/// # Errors
/// Decoding, validation, instantiation or execution failed.
pub fn run_wasm(wasm: &[u8], trace: bool) -> Result<Outcome, Error> {
    // Do not silently turn a configuration mistake into "no sensor": print the reason.
    let replay = match std::env::var("WASMICON_I2C_REPLAY") {
        Err(_) => Vec::new(),
        Ok(path) => match load_i2c_replay(std::path::Path::new(&path)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("wasmicon: cannot read WASMICON_I2C_REPLAY={path}: {e}");
                Vec::new()
            }
        },
    };
    run_wasm_with(wasm, trace, replay)
}

/// Run options.
#[derive(Clone, Default)]
pub struct Options {
    /// Whether to collect the trace.
    pub trace: bool,
    /// Responses returned to I2C reads, in order. Empty means `Nack`.
    pub i2c_replay: Vec<Vec<u8>>,
    /// Makes SPI return `unsupported`, as a port without SPI does, to test the
    /// failure path.
    pub spi_unsupported: bool,
    /// Assets the guest may open, as `(name, body)`: the files behind the
    /// `[assets]` table of `wasmicon.toml` (docs/app-workflow.md §3.10).
    pub assets: Vec<(String, Vec<u8>)>,
    /// Removes the mock display, as on a firmware without one, so the display
    /// imports do not link (docs/device-layer.md §6).
    pub no_display: bool,
    /// Size of the mock display; `None` is `hal::MOCK_DISPLAY` (320x240).
    pub display_size: Option<(u32, u32)>,
}

/// Runs with explicit recorded I2C responses.
///
/// # Errors
/// Decoding, validation, instantiation or execution failed.
pub fn run_wasm_with(wasm: &[u8], trace: bool, i2c_replay: Vec<Vec<u8>>) -> Result<Outcome, Error> {
    run_wasm_opts(
        wasm,
        Options {
            trace,
            i2c_replay,
            spi_unsupported: false,
            assets: Vec::new(),
            no_display: false,
            display_size: None,
        },
    )
}

/// Runs with explicit options.
///
/// # Errors
/// Decoding, validation, instantiation or execution failed.
pub fn run_wasm_opts(wasm: &[u8], opts: Options) -> Result<Outcome, Error> {
    let (outcome, result) = run_wasm_capture(wasm, opts);
    result?;
    Ok(outcome)
}

/// Runs and returns the collected trace even on failure.
///
/// `run_wasm_opts` returns failures as `Err`, which discards the trace
/// collected up to that point. A trap is exactly when the trace matters (only
/// it shows at which host call the runs diverged), so this entry point returns
/// both. `wasmicon run --trace` uses it.
///
/// When the second element is `Err`, the first still holds every line up to
/// the failure.
///
/// # Panics
/// When `opts.assets` has a name outside the grammar, a duplicate, or more
/// than eight entries. The CLI checks the manifest before it gets here.
pub fn run_wasm_capture(wasm: &[u8], opts: Options) -> (Outcome, Result<(), Error>) {
    let (trace, i2c_replay, spi_unsupported) = (opts.trace, opts.i2c_replay, opts.spi_unsupported);
    // The same image the CLI writes to a board (docs/app-workflow.md §3.10),
    // so the host and the boards read one layout.
    let image = match assets::image(&opts.assets) {
        Ok(image) => image,
        Err(e) => panic!("invalid assets list: {}", e.reason()),
    };
    let dir = match assets::directory(&image) {
        Ok(dir) => dir,
        Err(e) => panic!("the assets image does not parse: {}", e.reason()),
    };
    let mut buf = vec![0u8; ARENA];
    let mut scratch_buf = vec![0u8; SCRATCH];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    // The host is looser than every board (max_memory_pages = 65536). To check
    // against a board's limits use profile::<board>.config
    // (docs/app-workflow.md §4.3).
    let cfg = wasmicon_port::profile::HOST.config;

    let mut hal = Hal::new(
        hal::HostBoard::new()
            .with_i2c_replay(i2c_replay)
            .with_spi_unsupported(spi_unsupported)
            .with_display(!opts.no_display)
            .with_display_size(opts.display_size.unwrap_or(hal::MOCK_DISPLAY))
            .with_assets(image),
        trace,
    )
    .with_roles(hal::host_roles())
    .with_assets(dir);

    // A failure before the guest starts (decode / validate / Exec / instantiate)
    // leaves the trace empty; a failure afterwards leaves every line so far in
    // hal. One block folds both into the same shape.
    let mut usage = None;
    let result: Result<(), Error> = 'guest: {
        let m = match decode::decode(wasm, &mut arena) {
            Ok(m) => m,
            Err(e) => break 'guest Err(e),
        };
        let v = match validate::validate(&m, &cfg, &mut arena, &mut scratch) {
            Ok(v) => v,
            Err(e) => break 'guest Err(e),
        };
        usage = Some(Usage::of_module(&v));
        // Exec must be allocated before linear memory (the rest of the arena).
        let mut exec = match Exec::new(&cfg, &mut arena) {
            Ok(e) => e,
            Err(e) => break 'guest Err(e),
        };
        let mut inst = match instantiate(m, v, &cfg, &mut arena, &mut hal) {
            Ok(i) => i,
            Err(e) => break 'guest Err(e),
        };

        let ran = 'run: {
            if let Some(start) = inst.module.start
                && let Err(e) = invoke(&mut inst, &mut exec, &mut hal, start, &[], &mut [])
            {
                break 'run Err(e);
            }
            let Some(run) = inst.export_func("run") else {
                break 'run Err(Error::Unlinkable(
                    "missing export `run` (docs/abi-spec.md §3.3)",
                ));
            };
            invoke(&mut inst, &mut exec, &mut hal, run, &[], &mut [])
        };

        // docs/abi-spec.md §5.2, §6.6: once `run` returns (even by a trap), drop
        // every remaining handle. Called at the same point as on the board ports.
        // No trace line is added (`Hal::release_all`).
        hal.release_all();
        if let Some(u) = usage.as_mut() {
            let peak = exec.peak();
            u.max_call_depth = peak.call_depth;
            u.operand_stack_slots = peak.operand_slots;
        }
        ran
    };

    let outcome = Outcome {
        trace: hal.board_mut().trace_output().to_string(),
        usage,
        frame: hal.board_mut().frame().cloned(),
    };
    (outcome, result)
}
