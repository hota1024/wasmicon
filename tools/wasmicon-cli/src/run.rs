//! `wasmicon run` — host ポートで実行する（`docs/app-workflow.md` §4.2）。
//!
//! 中身は `wasmicon-host` の `run_wasm_opts` を呼ぶだけ。**crate は残して
//! lib として使う**（`verify/differential` も依存している）。
//!
//! **host は全ボードより緩い**（`Config::DEFAULT` = 65536 ページ）。ここで
//! 通っても実機の validate で落ちるアプリは書けるので、実機の判定は
//! `wasmicon check --board <board>` が出す（§4.3）。

use anyhow::{Context, Result};
use std::path::PathBuf;

pub struct Options {
    pub path: PathBuf,
    /// 全 host call を abi-spec §9 の形式で標準出力に出す。
    pub trace: bool,
    /// 記録済みの I2C 応答。host には実物のセンサーが無いので、
    /// 読み出しに返すものをファイルから与える（`verify/sht4x-replay.txt`）。
    pub i2c_replay: Option<PathBuf>,
}

/// 実行する。トラップせずに終われば `true`。
///
/// # Errors
/// ファイルが読めないとき。ゲストのトラップは `Ok(false)` として扱う
/// （使い方の誤りではないので）。
pub fn run(opts: &Options) -> Result<bool> {
    let wasm =
        std::fs::read(&opts.path).with_context(|| format!("{} を読めない", opts.path.display()))?;

    let i2c_replay = match &opts.i2c_replay {
        None => Vec::new(),
        Some(p) => wasmicon_host::load_i2c_replay(p)
            .with_context(|| format!("{} を読めない", p.display()))?,
    };

    let host_opts = wasmicon_host::Options {
        trace: opts.trace,
        i2c_replay,
        spi_unsupported: false,
    };

    // **失敗してもトレースを受け取る口を使う。** トラップしたときこそ
    // トレースが欲しい（どの host call で分岐したかはそこにしか無い）。
    let (out, result) = wasmicon_host::run_wasm_capture(&wasm, host_opts);
    if opts.trace {
        print!("{}", out.trace);
    }
    match result {
        Ok(()) => Ok(true),
        Err(e) => {
            // docs/handoff.md §3 #4: トラップしたらログを出して停止する。
            eprintln!("wasmicon: {} [{}]", e.reason(), e.kind().name());
            if opts.trace {
                let n = out.trace.lines().count();
                eprintln!("  ここまでのトレース {n} 行は標準出力に出した");
            }
            Ok(false)
        }
    }
}
