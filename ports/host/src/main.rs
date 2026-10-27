//! PC port binary: runs a `.wasm` and prints the mock HAL trace (docs/abi-spec.md §9 format).

use std::process::ExitCode;

const USAGE: &str = "\
wasmicon-host - runs a Wasmicon guest on the PC

Usage:
    wasmicon-host [--trace] <file.wasm>

Options:
    --trace      print every host call to stdout in the docs/abi-spec.md §9 format
    -h, --help   this help

WASMICON_TRACE=1 in the environment is the same as --trace.
";

fn main() -> ExitCode {
    let mut path: Option<String> = None;
    let mut trace = std::env::var("WASMICON_TRACE").is_ok_and(|v| v != "0");

    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--trace" => trace = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => {
                eprintln!("unknown option: {other}\n\n{USAGE}");
                return ExitCode::FAILURE;
            }
            other => path = Some(other.to_string()),
        }
    }

    let Some(path) = path else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };

    let wasm = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Print the trace even on failure. This binary produces the reference
    // traces in docs/verification-report.md §6, §7, and a trap during capture
    // is exactly when the lines so far are needed.
    let replay = match std::env::var("WASMICON_I2C_REPLAY") {
        Err(_) => Vec::new(),
        Ok(path) => match wasmicon_host::load_i2c_replay(std::path::Path::new(&path)) {
            Ok(r) => r,
            Err(e) => {
                // Do not silently turn a configuration mistake into "no sensor".
                eprintln!("wasmicon: cannot read WASMICON_I2C_REPLAY={path}: {e}");
                Vec::new()
            }
        },
    };
    let (out, result) = wasmicon_host::run_wasm_capture(
        &wasm,
        wasmicon_host::Options {
            trace,
            i2c_replay: replay,
            spi_unsupported: false,
            assets: Vec::new(),
            no_display: false,
            display_size: None,
            display: None,
            bus_log: false,
        },
    );
    if trace {
        print!("{}", out.trace);
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // docs/decisions.md A-04: on a trap, log the reason and stop.
            eprintln!("wasmicon: {} [{}]", e.reason(), e.kind().name());
            ExitCode::FAILURE
        }
    }
}
