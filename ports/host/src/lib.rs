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

/// `.wasm` を読み込んで `run` を呼ぶ。
///
/// # Errors
/// デコード・検証・インスタンス化・実行のいずれかが失敗したとき。
pub fn run_wasm(wasm: &[u8], trace: bool) -> Result<Outcome, Error> {
    let mut buf = vec![0u8; ARENA];
    let mut scratch_buf = vec![0u8; SCRATCH];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let cfg = Config::default();

    let m = decode::decode(wasm, &mut arena)?;
    let v = validate::validate(&m, &cfg, &mut arena, &mut scratch)?;
    // Exec は線形メモリ（arena の残り全部）より先に確保する。
    let mut exec = Exec::new(&cfg, &mut arena)?;
    let mut hal = Hal::new(hal::HostBoard::new(), trace);
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
