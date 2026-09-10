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

    match wasmicon_host::run_wasm(&wasm, trace) {
        Ok(out) => {
            if trace {
                print!("{}", out.trace);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            // docs/handoff.md §3 #4: トラップしたらログを出して停止する。
            eprintln!("wasmicon: {} [{}]", e.reason(), e.kind().name());
            ExitCode::FAILURE
        }
    }
}
