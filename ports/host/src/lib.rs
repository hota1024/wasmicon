//! PC 用ポート。`.wasm` を読み込み、mock HAL で実行する。
//!
//! HANDOFF §5 Phase 2 の完了条件（host で動かしてトレースが出る）はここで検証する。

pub mod hal;

use wasmicon_core::{Arena, Config, Error, Exec, decode, instantiate, invoke, validate};
use wasmicon_port::Hal;

/// arena の大きさ。残りが線形メモリになる。
pub const ARENA: usize = 16 << 20;
/// 検証中だけ使う作業領域。
pub const SCRATCH: usize = 4 << 20;

/// 実行結果。
pub struct Outcome {
    /// abi-spec §9 形式のトレース。
    pub trace: String,
}

/// 記録済みの I2C 応答を読む。
///
/// 形式は 1 行 1 応答の 16 進。`#` で始まる行と空行は無視する。
///
/// # Errors
/// ファイルが読めないか、16 進として解釈できない行があるとき。
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
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "16 進として読めないトークンがある",
                )
            })?;
            bytes.push(b);
        }
        out.push(bytes);
    }
    Ok(out)
}

/// `.wasm` を読み込んで `run` を呼ぶ。
///
/// I2C の応答は環境変数 `WASMICON_I2C_REPLAY` が指すファイルから読む。
///
/// # Errors
/// デコード・検証・インスタンス化・実行のいずれかが失敗したとき。
pub fn run_wasm(wasm: &[u8], trace: bool) -> Result<Outcome, Error> {
    let replay = std::env::var("WASMICON_I2C_REPLAY")
        .ok()
        .and_then(|p| load_i2c_replay(std::path::Path::new(&p)).ok())
        .unwrap_or_default();
    run_wasm_with(wasm, trace, replay)
}

/// 記録済みの I2C 応答を明示して実行する。
///
/// # Errors
/// デコード・検証・インスタンス化・実行のいずれかが失敗したとき。
pub fn run_wasm_with(wasm: &[u8], trace: bool, i2c_replay: Vec<Vec<u8>>) -> Result<Outcome, Error> {
    let mut buf = vec![0u8; ARENA];
    let mut scratch_buf = vec![0u8; SCRATCH];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let cfg = Config::default();

    let m = decode::decode(wasm, &mut arena)?;
    let v = validate::validate(&m, &cfg, &mut arena, &mut scratch)?;
    // Exec は線形メモリ（arena の残り全部）より先に確保する。
    let mut exec = Exec::new(&cfg, &mut arena)?;
    let mut hal = Hal::new(hal::HostBoard::new().with_i2c_replay(i2c_replay), trace);
    let mut inst = instantiate(m, v, &cfg, &mut arena, &mut hal)?;

    if let Some(start) = inst.module.start {
        invoke(&mut inst, &mut exec, &mut hal, start, &[], &mut [])?;
    }
    let run = inst
        .export_func("run")
        .ok_or(Error::Unlinkable("export run が無い（abi-spec §3.3）"))?;
    invoke(&mut inst, &mut exec, &mut hal, run, &[], &mut [])?;

    Ok(Outcome {
        trace: hal.board_mut().trace_output().to_string(),
    })
}
