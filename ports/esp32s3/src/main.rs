//! Wasmicon の ESP32-S3 (DevKitC-1) ポート。
//!
//! ゲストの `.wasm` はアプリスロット（フラッシュ）から `esp-storage` で読み、
//! arena に写してランタイムに渡す。ファームにアプリは入っていない
//! （`docs/app-workflow.md` §3.3）。スロットが空なら理由を出して `led` を
//! 点滅させて待つ（`wasmicon_port::idle`）。
//! トレースは UART0 (GPIO43=TX, GPIO44=RX) 115200 8N1 に出す。
//! DevKitC-1 では UART0 が USB シリアル変換に繋がっている。
//!
//! SPI2 と I2C0 は実装済み（`esp-hal` のドライバ）。
//!
//! **DevKitC-1 実機で確認済み**（2026-09-26）。`ports/rp2350` と同じゲスト
//! （`lcd_demo_rs.wasm`、SHA-256 `fc470947…`）を走らせ、トレースが host ポートと
//! 14,352 行完全一致し、ILI9341 に絵が出た（docs/verification-report.md §7）。
//! I2C（SHT40）も 2026-10-05 に確認した（同 §11）。`led` の配線は未確認のまま
//! （docs/TODO.md §1.1）。
//!
//! ビルドには espup が入れる Xtensa の GCC が要る:
//! `. ~/export-esp.sh && cargo build --release`

#![no_std]
#![no_main]

mod board;
#[cfg(feature = "hw-probe")]
mod probe;

use esp_hal::clock::CpuClock;
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_storage::FlashStorage;
use wasmicon_core::{decode, instantiate, invoke, validate, Arena, Config, Exec};
use wasmicon_port::fmt::Buf;
use wasmicon_port::identity::{self, Identity};
use wasmicon_port::roles::{self, Limits, RoleMap};
use wasmicon_port::{idle, slot, Hal};

use board::{EspBoard, Serial};

// ESP-IDF の 2 段目ブートローダが読む app descriptor をフラッシュの先頭付近に置く。
// 中身はバージョンとビルド日時で、ランタイムからは使わない。espflash 4.5 以降は
// これが無い ELF を受け付けない（RP2350 の `IMAGE_DEF` に当たるもの）。
esp_bootloader_esp_idf::esp_app_desc!();

/// 設定スロット（配線表。`docs/app-workflow.md` §3.9）。アプリスロットの直後。
const ROLE_SLOT: wasmicon_port::profile::Slot = match wasmicon_port::profile::ESP32S3.role_slot {
    Some(s) => s,
    None => panic!("ESP32-S3 のプロファイルに設定スロットが無い"),
};

/// 配線表で割り当ててよい GPIO の制約。
const LIMITS: Limits<'static> = Limits::of(&wasmicon_port::profile::ESP32S3);

/// アプリスロット（`ports/common` の `profile::ESP32S3`）。
const SLOT: wasmicon_port::profile::Slot = match wasmicon_port::profile::ESP32S3.slot {
    Some(s) => s,
    None => panic!("ESP32-S3 のプロファイルにスロットが無い"),
};

/// ランタイムの arena。残りが線形メモリになる（`Arena::alloc_rest`）。
/// ESP32-S3 は PSRAM 無しで SRAM 512 KB。線形メモリ 4 ページ（256 KB）が
/// 収まる大きさにしてある（docs/handoff.md §5 Phase 4）。
static mut ARENA: [u8; wasmicon_port::profile::ESP32S3.arena] =
    [0; wasmicon_port::profile::ESP32S3.arena];

/// 検証中だけ使う作業領域。`MCU_CONFIG` の上限に合わせてある。
static mut SCRATCH: [u8; wasmicon_port::profile::ESP32S3.scratch] =
    [0; wasmicon_port::profile::ESP32S3.scratch];

/// マイコン向けの上限。**正は `ports/common` の `profile::ESP32S3.config`**
/// （CLI が同じ表を読む。docs/app-workflow.md §4.3）。
/// ホストの既定値のままだと scratch が足りない。
const MCU_CONFIG: Config = wasmicon_port::profile::ESP32S3.config;

/// UART0 への出力。
struct SerialPort<'a>(Uart<'a, esp_hal::Blocking>);

impl Serial for SerialPort<'_> {
    /// **全部書き切る。** `Uart::write` は送信 FIFO（128 バイト）に入る分だけ書いて
    /// 書けたバイト数を返すので、戻り値を捨てると 128 バイトを超える行の後ろが
    /// 黙って落ちる（2026-10-09 に名乗りの行で踏んだ。トレース行は最大 160 バイト）。
    fn write(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            match self.0.write(rest) {
                Ok(n) if n > 0 => rest = &rest[n..],
                // 書けない（エラーか 0）なら諦める。シリアルの失敗を出す先は無い。
                _ => break,
            }
        }
        let _ = self.0.flush();
    }
}

#[esp_hal::main]
fn main() -> ! {
    // CPU は 240 MHz（2026-10-09 オーナー決定。既定は 80 MHz）。インタプリタが
    // そのまま 3 倍速くなる。SPI / I2C / UART は APB（80 MHz のまま）から作るので
    // 変わらず、`time` はトレースに出ないので記録済みのトレースとの一致も保たれる。
    // プリセットは PLL を 480 MHz のまま分周だけ変える（USB-Serial-JTAG を壊さない）。
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let uart = Uart::new(p.UART0, UartConfig::default())
        .unwrap()
        .with_tx(p.GPIO43)
        .with_rx(p.GPIO44);
    let mut serial = SerialPort(uart);
    serial.write(b"wasmicon esp32s3\r\n");
    // 自分が何者かを 1 行で名乗る（docs/app-workflow.md §3.8）。版・git・ABI・
    // 構成をトレースの取り込みに残し、どのビルドで取ったかを後から追えるようにする。
    {
        let mut line = [0u8; 192];
        let mut out = Buf::new(&mut line);
        identity::describe(
            &Identity {
                profile: &wasmicon_port::profile::ESP32S3,
                fw: env!("CARGO_PKG_VERSION"),
                git: env!("WASMICON_GIT"),
                trace: cfg!(feature = "trace"),
            },
            &mut out,
        );
        serial.write(out.as_bytes());
        serial.write(b"\r\n");
    }

    // SAFETY: EspBoard がこれ以降 GPIO / IO_MUX / SPI2 / I2C0 と、SPI2 と I2C0 に
    // 割り当てたピンを排他的に使う。UART は GPIO43/44 を占有するが、役割名に
    // 割り当てた GPIO とも SPI2 のピン (GPIO11/12/13) とも I2C0 のピン
    // (GPIO8/9) とも重ねていない。
    // hw-probe のときだけ可変で借りる。
    #[cfg_attr(not(feature = "hw-probe"), allow(unused_mut))]
    let mut board = unsafe { EspBoard::new(serial) };

    // ポート層がパッドまで届いているかの切り分け（src/probe.rs）。
    // ゲストはこの後そのまま走るので、済んだら feature を外すだけでよい。
    #[cfg(feature = "hw-probe")]
    probe::run(&mut board);

    // 設定スロットとアプリスロットの両方を読むので、ここで 1 回だけ作る。
    let mut storage = FlashStorage::new(p.FLASH);

    let mut hal = Hal::new(board, cfg!(feature = "trace"));
    // 役割は設定スロットの表だけで決まる。ファームは既定の表を持たない
    // （空・壊れていれば役割は 1 つも配らない。docs/app-workflow.md §3.9）。
    let map = pick_roles(&mut storage, hal.board_mut().serial());
    let mut hal = hal.with_roles(map);

    // SAFETY: 単一のタスクからしか触らないので、可変静的への参照はここでしか作らない。
    let arena_buf = unsafe { &mut *core::ptr::addr_of_mut!(ARENA) };
    let scratch_buf = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };

    let outcome = run(&mut hal, &mut storage, arena_buf, scratch_buf);

    // ファームにアプリは入っていない（docs/app-workflow.md §3.3）。
    // スロットが空・壊れているなら、理由は pick_guest が出している。
    if let Ok(false) = outcome {
        idle::park(hal.board_mut());
    }

    // **理由を先に出す。** 下の release_all は無制限に待ちうる
    // （`spi_close` の `BSY` 待ちなど。docs/TODO.md §2.1）ので、掃除を
    // 先に回すとペリフェラルが固まったときに理由が出ないまま無言で止まる
    // ——  docs/handoff.md §3 #4「ログを出して停止する」が守れない。
    // abi-spec §6.6 は drop → ログの順で書いてあるが、順序を入れ替えても
    // 観測できるのは「理由が出る」ことだけ増える（docs/TODO.md §2 に記録）。
    if let Err(e) = outcome {
        let s = hal.board_mut().serial();
        s.write(b"wasmicon: ");
        s.write(e.reason().as_bytes());
        s.write(b" [");
        s.write(e.kind().name().as_bytes());
        s.write(b"]\r\n");
    }

    // abi-spec §5.2 / §6.6: `run` から戻ったら（トラップでも）残っている
    // ハンドルを全部 drop する。ゲストの Drop は走らないので、ここだけが
    // 片付ける。トレース行は出さない（`Hal::release_all` のコメント）。
    hal.release_all();
    loop {
        core::hint::spin_loop();
    }
}

/// スロットから走らせるアプリを選ぶ。
///
/// 走らせるものが無ければ `None`（ファームにアプリは入っていない。
/// `docs/app-workflow.md` §3.3）。ESP32-S3 は任意オフセットが XIP に
/// マップされている前提を置けないので、Pico 系（`slot::read_xip`）と違って
/// **RAM に写す**。写すのは**ヘッダが持つ長さの分だけ** —— スロットは
/// 64 KiB だがアプリは数 KB で、DRAM はほぼ使い切っている
/// （`docs/TODO.md` §1.4）。arena から取るので静的な領域を増やさない。
fn pick_guest<'a>(
    storage: &mut FlashStorage<'_>,
    arena: &mut Arena<'a>,
    serial: &mut SerialPort<'static>,
) -> Option<&'a [u8]> {
    let mut line = [0u8; 96];
    let mut out = Buf::new(&mut line);

    let mut head = [0u8; slot::HEADER_LEN];
    if storage.read(SLOT.offset, &mut head).is_err() {
        serial.write(b"wasmicon: slot read failed, idle\r\n");
        return None;
    }
    let header = match slot::parse_header(&head) {
        Ok(h) => h,
        Err(e) => {
            out.str("wasmicon: ");
            out.str(e.reason());
            out.str(", idle");
            serial.write(out.as_bytes());
            serial.write(b"\r\n");
            return None;
        }
    };
    if header.len > (SLOT.len as usize - slot::HEADER_LEN) {
        serial.write(b"wasmicon: slot truncated, idle\r\n");
        return None;
    }

    // **arena を取る前に CRC を確かめる。** `Arena` は bump で、返す手段が
    // 無い（`runtime/src/arena.rs` に reset は無い）。今は壊れていれば
    // idle に入るだけなので実害は無いが、スーパーバイザのループ
    // （`docs/TODO.md` §5）で同じ arena から次のアプリを走らせるようになると、
    // **壊れたスロットの分だけ arena が減ったまま**になり、`wasmicon check` が
    // 見積もる余裕と実機がずれる。だから一度フラッシュから流して CRC だけ見る。
    //
    // 読みは小さなバッファで分割する。`esp-storage` の `read` は**呼ぶたびに
    // 4 KiB のセクタバッファをスタックに作る**（あちらの実装）ので、
    // ここを大きくしてもスタックの山は変わらない。順に呼ぶので山は 1 つ分
    // （`docs/TODO.md` §1.4 に記録した）。
    let mut chunk = [0u8; 512];
    let mut crc = wasmicon_port::CRC32_INIT;
    let mut left = header.len;
    let mut at = SLOT.offset + slot::HEADER_LEN as u32;
    while left > 0 {
        let n = if left < chunk.len() {
            left
        } else {
            chunk.len()
        };
        if storage.read(at, &mut chunk[..n]).is_err() {
            serial.write(b"wasmicon: slot read failed, idle\r\n");
            return None;
        }
        crc = wasmicon_port::crc32_update(crc, &chunk[..n]);
        at += n as u32;
        left -= n;
    }
    if wasmicon_port::crc32_end(crc) != header.crc {
        serial.write(b"wasmicon: slot crc mismatch, idle\r\n");
        return None;
    }

    // CRC が合ったので、ここで初めて arena を取る。失敗する枝はもう
    // 「arena が足りない」だけ。
    let Ok(buf) = arena.alloc_bytes(header.len) else {
        serial.write(b"wasmicon: arena too small for the slot app, idle\r\n");
        return None;
    };
    if storage
        .read(SLOT.offset + slot::HEADER_LEN as u32, buf)
        .is_err()
    {
        serial.write(b"wasmicon: slot read failed, idle\r\n");
        return None;
    }
    // **写したものをもう一度検査する。** 上で確かめたのは流し読みした
    // バイト列で、decode に渡すのはこの `buf`。2 度目の読みが壊れていたら
    // ここで捕まえる（arena は既に取ってあるが、走らせはしない）。
    if let Err(e) = slot::verify(buf, &header) {
        out.str("wasmicon: ");
        out.str(e.reason());
        out.str(", idle");
        serial.write(out.as_bytes());
        serial.write(b"\r\n");
        return None;
    }

    out.str("wasmicon: slot ");
    out.u32(header.len as u32);
    out.str(" B crc32=");
    out.hex(header.crc, 8);
    serial.write(out.as_bytes());
    serial.write(b"\r\n");
    Some(buf)
}

/// 設定スロットから配線表を読む。読めた表（または読めない理由）をシリアルに出す。
///
/// 本文は `roles::MAX_BODY`（256 バイト）までなので、スタックに読む。
fn pick_roles(storage: &mut FlashStorage<'_>, serial: &mut SerialPort<'static>) -> RoleMap {
    let result = read_roles(storage);
    let mut line = [0u8; 192];
    let mut out = Buf::new(&mut line);
    roles::describe(&result, &mut out);
    serial.write(out.as_bytes());
    serial.write(b"\r\n");
    result.unwrap_or(RoleMap::EMPTY)
}

fn read_roles(storage: &mut FlashStorage<'_>) -> Result<RoleMap, roles::RolesError> {
    let mut head = [0u8; roles::HEADER_LEN];
    if storage.read(ROLE_SLOT.offset, &mut head).is_err() {
        // 読めないのは空と同じ扱いにする（役割を配らない側に倒す）。
        return Err(roles::RolesError::Empty);
    }
    let header = roles::parse_header(&head)?;
    let mut body = [0u8; roles::MAX_BODY];
    let body = &mut body[..header.len];
    if storage
        .read(ROLE_SLOT.offset + roles::HEADER_LEN as u32, body)
        .is_err()
    {
        return Err(roles::RolesError::Empty);
    }
    roles::parse_checked(body, &header, &LIMITS)
}

/// スロットの読み出しから `run` の呼び出しまで。
///
/// 走らせるアプリが無ければ `Ok(false)`、走らせて戻ったら `Ok(true)`。
fn run(
    hal: &mut Hal<EspBoard<SerialPort<'static>>>,
    storage: &mut FlashStorage<'_>,
    arena_buf: &'static mut [u8],
    scratch_buf: &'static mut [u8],
) -> Result<bool, wasmicon_core::Error> {
    let mut arena = Arena::new(arena_buf);
    let mut scratch = Arena::new(scratch_buf);

    let Some(guest) = pick_guest(storage, &mut arena, hal.board_mut().serial()) else {
        return Ok(false);
    };
    let m = decode::decode(guest, &mut arena)?;
    let v = validate::validate(&m, &MCU_CONFIG, &mut arena, &mut scratch)?;
    // Exec は線形メモリ（arena の残り全部）より先に確保する。
    let mut exec = Exec::new(&MCU_CONFIG, &mut arena)?;
    let mut inst = instantiate(m, v, &MCU_CONFIG, &mut arena, hal)?;

    if let Some(start) = inst.module.start {
        invoke(&mut inst, &mut exec, hal, start, &[], &mut [])?;
    }
    let entry = inst
        .export_func("run")
        .ok_or(wasmicon_core::Error::Unlinkable("export run が無い"))?;
    invoke(&mut inst, &mut exec, hal, entry, &[], &mut [])?;
    Ok(true)
}

/// docs/handoff.md §3 #4 は「ログを出して停止、再起動しない」だが、**理由は出せない**。
///
/// シリアルはボードが持っていて panic handler からは届かないため、今は
/// 停止するだけ。理由を出すには、シリアルのハンドルを panic handler から
/// 触れる場所（critical-section 付きのグローバル）に置く必要がある。
/// ランタイム由来の失敗は `main` が捕まえて UART に出すので、ここに来るのは
/// ポート自身のバグに限られる。
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
