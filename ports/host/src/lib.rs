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
    // 設定ミスを黙って「センサー無し」に落とさない。読めなければ理由を出す。
    let replay = match std::env::var("WASMICON_I2C_REPLAY") {
        Err(_) => Vec::new(),
        Ok(path) => match load_i2c_replay(std::path::Path::new(&path)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("wasmicon: WASMICON_I2C_REPLAY={path} を読めない: {e}");
                Vec::new()
            }
        },
    };
    run_wasm_with(wasm, trace, replay)
}

/// 実行時の設定。
#[derive(Default)]
pub struct Options {
    /// トレースを溜めるか。
    pub trace: bool,
    /// I2C の読み出しに順に返す応答。空なら `Nack`。
    pub i2c_replay: Vec<Vec<u8>>,
    /// SPI を `unsupported` にする。実機ポートの現状を模した失敗経路のテスト用。
    pub spi_unsupported: bool,
}

/// 記録済みの I2C 応答を明示して実行する。
///
/// # Errors
/// デコード・検証・インスタンス化・実行のいずれかが失敗したとき。
pub fn run_wasm_with(wasm: &[u8], trace: bool, i2c_replay: Vec<Vec<u8>>) -> Result<Outcome, Error> {
    run_wasm_opts(
        wasm,
        Options {
            trace,
            i2c_replay,
            spi_unsupported: false,
        },
    )
}

/// 設定を明示して実行する。
///
/// # Errors
/// デコード・検証・インスタンス化・実行のいずれかが失敗したとき。
pub fn run_wasm_opts(wasm: &[u8], opts: Options) -> Result<Outcome, Error> {
    let (trace, i2c_replay, spi_unsupported) = (opts.trace, opts.i2c_replay, opts.spi_unsupported);
    let mut buf = vec![0u8; ARENA];
    let mut scratch_buf = vec![0u8; SCRATCH];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let cfg = Config::default();

    let m = decode::decode(wasm, &mut arena)?;
    let v = validate::validate(&m, &cfg, &mut arena, &mut scratch)?;
    // Exec は線形メモリ（arena の残り全部）より先に確保する。
    let mut exec = Exec::new(&cfg, &mut arena)?;
    let mut hal = Hal::new(
        hal::HostBoard::new()
            .with_i2c_replay(i2c_replay)
            .with_spi_unsupported(spi_unsupported),
        trace,
    );
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
