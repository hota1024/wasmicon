//! PC 上で `.wasm` を実行し、mock HAL のトレース（abi-spec §9 形式）を出すポート。

use std::process::ExitCode;

const USAGE: &str = "\
wasmicon-host — PC 上で Wasmicon のゲストを実行する

使い方:
    wasmicon-host [--trace] <file.wasm>

オプション:
    --trace      全 host call を abi-spec §9 の形式で標準出力に出す
    -h, --help   このヘルプ

環境変数 WASMICON_TRACE=1 でも --trace と同じ。
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
                eprintln!("未知のオプション: {other}\n\n{USAGE}");
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
            eprintln!("{path} を読めない: {e}");
            return ExitCode::FAILURE;
        }
    };

    // **失敗してもトレースを出す。** このバイナリは
    // docs/verification-report.md §6 / §7 の参照トレースを取るのに使う。
    // 取り込み中にトラップしたときこそ、そこまでの行が要る。
    let replay = match std::env::var("WASMICON_I2C_REPLAY") {
        Err(_) => Vec::new(),
        Ok(path) => match wasmicon_host::load_i2c_replay(std::path::Path::new(&path)) {
            Ok(r) => r,
            Err(e) => {
                // 設定ミスを黙って「センサー無し」に落とさない。
                eprintln!("wasmicon: WASMICON_I2C_REPLAY={path} を読めない: {e}");
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
        },
    );
    if trace {
        print!("{}", out.trace);
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // docs/handoff.md §3 #4: トラップしたらログを出して停止する。
            eprintln!("wasmicon: {} [{}]", e.reason(), e.kind().name());
            ExitCode::FAILURE
        }
    }
}
