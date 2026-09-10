//! ポート共通の HAL 実装。
//!
//! import の解決（生成された表との完全一致リンク、abi-spec §6.4）、
//! ハンドル表（§5.3）、ステータスの組み立て（§4.4）、トレースの整形（§9）は
//! 全ポートで同じでなければならない。特にトレースは、書式が少しでも違うと
//! §2-10 の「両ボードでトレースが一致」が成立しなくなる。
//!
//! ボード固有の操作だけを `Board` トレイトに切り出し、ここは `no_std` で書く。

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod fmt;

use wasmicon_core::error::{Error, Result, Trap};
use wasmicon_core::generated::gpio::{Level, PinMode};
use wasmicon_core::generated::i2c::Speed;
use wasmicon_core::generated::log::Level as LogLevel;
use wasmicon_core::generated::spi::Mode as SpiMode;
use wasmicon_core::generated::{self, ErrorCode, HostFn};
use wasmicon_core::instance::{Extern, ExternType, Resolver};

use fmt::Buf;

/// abi-spec §5.3 が保証する同時ハンドル数。
pub const MAX_PINS: usize = 16;
pub const MAX_I2C: usize = 2;
pub const MAX_SPI: usize = 2;

/// AssemblyScript が import する `env.abort` に割り当てる識別子。
/// `world app` には無い例外的な import（HANDOFF §6）。
const HOST_ENV_ABORT: u32 = 0xffff;

/// 入出力が重なりうる転送で使う一時領域の大きさ。
///
/// `spi.transfer` と `i2c.write-read` は送信元と受信先がどちらもゲストの
/// 線形メモリにあり、範囲が重なりうる。借用を分けられないので送信側を
/// 一度ここへ写す。v0.1 の用途（SHT31 の 6 バイト、ILI9341 の ID 読み）には
/// 十分で、これを超える転送は `unsupported` を返す。
pub const SCRATCH: usize = 128;

/// トレース 1 行の最大長。
const LINE: usize = 160;

/// ボード操作の結果。ボード側でトラップは起きないので `ErrorCode` を直接返す。
pub type BoardResult<T> = core::result::Result<T, ErrorCode>;

/// ボード固有の操作。ポートはこれだけを実装する。
///
/// 番号はいずれもボードの GPIO 番号・ペリフェラル番号で、ハンドルではない。
/// ハンドルの管理は `Hal` 側が行う。
pub trait Board {
    /// 役割名から GPIO 番号を引く（abi-spec §8）。
    fn pin_by_role(&self, role: &str) -> Option<u32>;

    /// GPIO の本数。範囲検査に使う。
    fn gpio_count(&self) -> u32;

    fn gpio_configure(&mut self, index: u32, mode: PinMode) -> BoardResult<()>;
    fn gpio_write(&mut self, index: u32, level: Level) -> BoardResult<()>;
    fn gpio_read(&mut self, index: u32) -> BoardResult<Level>;
    fn gpio_release(&mut self, index: u32);

    fn i2c_open(&mut self, index: u32, speed: Speed) -> BoardResult<()>;
    fn i2c_write(&mut self, index: u32, address: u16, data: &[u8]) -> BoardResult<()>;
    fn i2c_read(&mut self, index: u32, address: u16, buf: &mut [u8]) -> BoardResult<usize>;
    fn i2c_write_read(
        &mut self,
        index: u32,
        address: u16,
        data: &[u8],
        buf: &mut [u8],
    ) -> BoardResult<usize>;
    fn i2c_close(&mut self, index: u32);

    fn spi_open(&mut self, index: u32, frequency_hz: u32, mode: SpiMode) -> BoardResult<()>;
    fn spi_write(&mut self, index: u32, data: &[u8]) -> BoardResult<()>;
    fn spi_transfer(&mut self, index: u32, data: &[u8], buf: &mut [u8]) -> BoardResult<usize>;
    fn spi_close(&mut self, index: u32);

    fn now_us(&mut self) -> u64;
    fn sleep_ms(&mut self, ms: u32);
    fn sleep_us(&mut self, us: u32);

    /// ゲストの `log` 出力。UTF-8 の検証はしない（abi-spec §4.2）。
    fn log(&mut self, level: LogLevel, message: &[u8]);

    /// トレース 1 行（改行を含まない）。ポートがシリアル等へ出す。
    fn trace(&mut self, line: &[u8]);
}

#[derive(Clone, Copy, Default)]
struct PinSlot {
    open: bool,
    index: u32,
}

#[derive(Clone, Copy, Default)]
struct BusSlot {
    open: bool,
    index: u32,
}

/// 生成された import 表に沿って HAL を提供する。
pub struct Hal<B: Board> {
    board: B,
    trace_on: bool,
    pins: [PinSlot; MAX_PINS],
    i2c: [BusSlot; MAX_I2C],
    spi: [BusSlot; MAX_SPI],
    /// `pin-by-role` で配った番号と役割名（abi-spec §9）。
    roles: [(u32, &'static str); 8],
    nroles: usize,
    scratch: [u8; SCRATCH],
}

impl<B: Board> Hal<B> {
    #[must_use]
    pub fn new(board: B, trace_on: bool) -> Self {
        Hal {
            board,
            trace_on,
            pins: [PinSlot::default(); MAX_PINS],
            i2c: [BusSlot::default(); MAX_I2C],
            spi: [BusSlot::default(); MAX_SPI],
            roles: [(0, ""); 8],
            nroles: 0,
            scratch: [0; SCRATCH],
        }
    }

    /// ボードへの参照。ポートの後始末に使う。
    pub fn board_mut(&mut self) -> &mut B {
        &mut self.board
    }

    /// abi-spec §9: 役割名で配った GPIO 番号は数値ではなく役割名で出す。
    ///
    /// 判断材料は数値だけなので、ゲストがハードコードした番号が役割割り当てと
    /// 一致していると、それも役割名になる。§9 がその前提を明記している。
    fn write_pin(&self, out: &mut Buf<'_>, index: u32) {
        for &(i, role) in &self.roles[..self.nroles] {
            if i == index {
                out.str("role:");
                out.str(role);
                return;
            }
        }
        out.u32(index);
    }

    fn remember_role(&mut self, index: u32, role: &'static str) {
        if self.roles[..self.nroles].iter().any(|(i, _)| *i == index) {
            return;
        }
        if self.nroles < self.roles.len() {
            self.roles[self.nroles] = (index, role);
            self.nroles += 1;
        }
    }
}

/// ハンドル（1 始まり）を配列の添字に。0 は無効（abi-spec §5.1）。
fn slot_of(handle: u64, len: usize) -> core::result::Result<usize, u32> {
    let h = handle as u32;
    if h == 0 || h as usize > len {
        return Err(ErrorCode::InvalidHandle.status());
    }
    Ok(h as usize - 1)
}

fn guest_slice(mem: &[u8], ptr: u64, len: u64) -> Result<&[u8]> {
    let a = ptr as u32 as usize;
    let b = len as u32 as usize;
    mem.get(
        a..a.checked_add(b)
            .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?,
    )
    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))
}

fn put_u32(mem: &mut [u8], ptr: u64, v: u32) -> Result<()> {
    let a = ptr as u32 as usize;
    let s = mem
        .get_mut(
            a..a.checked_add(4)
                .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?,
        )
        .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
    s.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

/// CRC-32 (IEEE)。長い `list<u8>` のトレース用（abi-spec §9）。
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        let mut i = 0;
        while i < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            i += 1;
        }
    }
    !crc
}

/// abi-spec §9 の `list<u8>` 表記。32 バイトを超えたら先頭 16 バイト + CRC-32。
fn write_bytes(out: &mut Buf<'_>, data: &[u8]) {
    out.str("0x");
    if data.len() <= 32 {
        for &b in data {
            out.hex(u32::from(b), 2);
        }
    } else {
        for &b in &data[..16] {
            out.hex(u32::from(b), 2);
        }
        out.str("..len=");
        out.u32(data.len() as u32);
        out.str(" crc32=");
        out.hex(crc32(data), 8);
    }
}

impl<B: Board> Resolver for Hal<B> {
    fn resolve(&mut self, module: &str, name: &str, ty: &ExternType<'_>) -> Option<Extern> {
        // AssemblyScript の env.abort は world app に無い例外的な import（HANDOFF §6）。
        if module == "env" && name == "abort" {
            return Some(Extern::Func(HOST_ENV_ABORT));
        }
        let ExternType::Func(ft) = ty else {
            return None;
        };
        // 生成された表と module + name + sig で完全一致（abi-spec §6.4）。
        let desc = generated::resolve(module, name)?;
        if !ft.sig_matches(desc.sig) {
            return None;
        }
        Some(Extern::Func(u32::from(desc.host_fn.index())))
    }

    #[allow(clippy::too_many_lines)]
    fn call(&mut self, host: u32, args: &[u64], results: &mut [u64], mem: &mut [u8]) -> Result<()> {
        if host == HOST_ENV_ABORT {
            return Err(Error::Trap(Trap::UnreachableHostCall));
        }
        let f =
            HostFn::from_index(host as u16).ok_or(Error::Unlinkable("unknown host function"))?;
        let desc = &generated::IMPORTS[host as usize];

        let mut argbuf = [0u8; LINE];
        let mut outbuf = [0u8; LINE];
        let mut a = Buf::new(&mut argbuf);
        let mut o = Buf::new(&mut outbuf);
        let mut status: u32 = 0;
        let mut result64: u64 = 0;

        match f {
            HostFn::BoardPinByRole => {
                let role = guest_slice(mem, args[0], args[1])?;
                let role = core::str::from_utf8(role).unwrap_or("");
                a.byte(b'"');
                a.str(role);
                a.byte(b'"');
                match self.board.pin_by_role(role) {
                    Some(index) => {
                        // 静的な役割名を覚えるため、ボードの表から名前を引き直す。
                        let name = ROLE_NAMES.iter().find(|r| **r == role).copied();
                        put_u32(mem, args[2], index)?;
                        if let Some(name) = name {
                            self.remember_role(index, name);
                        }
                        self.write_pin(&mut o, index);
                    }
                    None => status = ErrorCode::Unsupported.status(),
                }
            }

            HostFn::GpioPinOpen => {
                let index = args[0] as u32;
                let mode = args[1] as u32;
                self.write_pin(&mut a, index);
                a.str(", ");
                a.u32(mode);
                let slot = self.pins.iter().position(|p| !p.open);
                let taken = self.pins.iter().any(|p| p.open && p.index == index);
                status = if index >= self.board.gpio_count() || mode >= PinMode::COUNT {
                    ErrorCode::InvalidArgument.status()
                } else if taken {
                    ErrorCode::Busy.status()
                } else if let Some(slot) = slot {
                    let m = PinMode::from_u32(mode).ok_or(Error::Invalid("bad mode"))?;
                    match self.board.gpio_configure(index, m) {
                        Ok(()) => {
                            self.pins[slot] = PinSlot { open: true, index };
                            let handle = slot as u32 + 1;
                            put_u32(mem, args[2], handle)?;
                            o.u32(handle);
                            0
                        }
                        Err(e) => e.status(),
                    }
                } else {
                    ErrorCode::OutOfMemory.status()
                };
            }
            HostFn::GpioPinSetMode => {
                let mode = args[1] as u32;
                a.u64(args[0]);
                a.str(", ");
                a.u32(mode);
                status = match slot_of(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if mode >= PinMode::COUNT => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        let index = self.pins[i].index;
                        let m = PinMode::from_u32(mode).ok_or(Error::Invalid("bad mode"))?;
                        match self.board.gpio_configure(index, m) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::GpioPinRead => {
                a.u64(args[0]);
                status = match slot_of(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(i) => {
                        let index = self.pins[i].index;
                        match self.board.gpio_read(index) {
                            Ok(level) => {
                                put_u32(mem, args[1], level as u32)?;
                                o.u32(level as u32);
                                0
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::GpioPinWrite => {
                let level = args[1] as u32;
                a.u64(args[0]);
                a.str(", ");
                a.u32(level);
                status = match slot_of(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if level >= Level::COUNT => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        let index = self.pins[i].index;
                        let l = Level::from_u32(level).ok_or(Error::Invalid("bad level"))?;
                        match self.board.gpio_write(index, l) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::GpioPinToggle => {
                a.u64(args[0]);
                status = match slot_of(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(i) => {
                        let index = self.pins[i].index;
                        match self.board.gpio_read(index) {
                            Ok(level) => {
                                let next = if level == Level::Low {
                                    Level::High
                                } else {
                                    Level::Low
                                };
                                match self.board.gpio_write(index, next) {
                                    Ok(()) => 0,
                                    Err(e) => e.status(),
                                }
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::GpioPinDrop => {
                a.u64(args[0]);
                if let Ok(i) = slot_of(args[0], MAX_PINS)
                    && self.pins[i].open
                {
                    let index = self.pins[i].index;
                    self.board.gpio_release(index);
                    self.pins[i] = PinSlot::default();
                }
            }

            HostFn::I2cBusOpen => {
                let index = args[0] as u32;
                let speed = args[1] as u32;
                a.u32(index);
                a.str(", ");
                a.u32(speed);
                status = if index as usize >= MAX_I2C || speed >= Speed::COUNT {
                    ErrorCode::InvalidArgument.status()
                } else if self.i2c[index as usize].open {
                    ErrorCode::Busy.status()
                } else {
                    let s = Speed::from_u32(speed).ok_or(Error::Invalid("bad speed"))?;
                    match self.board.i2c_open(index, s) {
                        Ok(()) => {
                            self.i2c[index as usize] = BusSlot { open: true, index };
                            let handle = index + 1;
                            put_u32(mem, args[2], handle)?;
                            o.u32(handle);
                            0
                        }
                        Err(e) => e.status(),
                    }
                };
            }
            HostFn::I2cBusWrite => {
                let data = guest_slice(mem, args[2], args[3])?;
                a.u64(args[0]);
                a.str(", ");
                a.u64(args[1]);
                a.str(", ");
                write_bytes(&mut a, data);
                let n = data.len();
                status = match slot_of(args[0], MAX_I2C) {
                    Err(e) => e,
                    Ok(i) if !self.i2c[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if n == 0 => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        let index = self.i2c[i].index;
                        let addr = args[1] as u16;
                        // data はゲストメモリを直接読むだけなので写さない。
                        let data = guest_slice(mem, args[2], args[3])?;
                        match self.board.i2c_write(index, addr, data) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::I2cBusRead => {
                let want = args[2] as u32 as usize;
                let cap = args[4] as u32 as usize;
                a.u64(args[0]);
                a.str(", ");
                a.u64(args[1]);
                a.str(", ");
                a.u32(want as u32);
                status = match slot_of(args[0], MAX_I2C) {
                    Err(e) => e,
                    Ok(i) if !self.i2c[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if want == 0 || want > cap => ErrorCode::InvalidArgument.status(),
                    Ok(_) if want > SCRATCH => ErrorCode::Unsupported.status(),
                    Ok(i) => {
                        let index = self.i2c[i].index;
                        let addr = args[1] as u16;
                        match self.board.i2c_read(index, addr, &mut self.scratch[..want]) {
                            Ok(got) => {
                                let src = &self.scratch[..got];
                                let dst = args[3] as u32 as usize;
                                let end = dst
                                    .checked_add(got)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
                                mem.get_mut(dst..end)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?
                                    .copy_from_slice(src);
                                put_u32(mem, args[5], got as u32)?;
                                o.str("len=");
                                o.u32(got as u32);
                                0
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::I2cBusWriteRead => {
                let want = args[4] as u32 as usize;
                let cap = args[6] as u32 as usize;
                let data_len = args[3] as u32 as usize;
                {
                    let data = guest_slice(mem, args[2], args[3])?;
                    a.u64(args[0]);
                    a.str(", ");
                    a.u64(args[1]);
                    a.str(", ");
                    write_bytes(&mut a, data);
                    a.str(", ");
                    a.u32(want as u32);
                }
                status = match slot_of(args[0], MAX_I2C) {
                    Err(e) => e,
                    Ok(i) if !self.i2c[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if want == 0 || want > cap || data_len == 0 => {
                        ErrorCode::InvalidArgument.status()
                    }
                    Ok(_) if want > SCRATCH || data_len > SCRATCH => {
                        ErrorCode::Unsupported.status()
                    }
                    Ok(i) => {
                        let index = self.i2c[i].index;
                        let addr = args[1] as u16;
                        // 送信元と受信先が重なりうるので、送信側を一度写す。
                        let mut tx = [0u8; SCRATCH];
                        tx[..data_len].copy_from_slice(guest_slice(mem, args[2], args[3])?);
                        match self.board.i2c_write_read(
                            index,
                            addr,
                            &tx[..data_len],
                            &mut self.scratch[..want],
                        ) {
                            Ok(got) => {
                                let dst = args[5] as u32 as usize;
                                let end = dst
                                    .checked_add(got)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
                                mem.get_mut(dst..end)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?
                                    .copy_from_slice(&self.scratch[..got]);
                                put_u32(mem, args[7], got as u32)?;
                                o.str("len=");
                                o.u32(got as u32);
                                0
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::I2cBusDrop => {
                a.u64(args[0]);
                if let Ok(i) = slot_of(args[0], MAX_I2C)
                    && self.i2c[i].open
                {
                    let index = self.i2c[i].index;
                    self.board.i2c_close(index);
                    self.i2c[i] = BusSlot::default();
                }
            }

            HostFn::SpiBusOpen => {
                let index = args[0] as u32;
                let freq = args[1] as u32;
                let mode = args[2] as u32;
                a.u32(index);
                a.str(", ");
                a.u32(freq);
                a.str(", ");
                a.u32(mode);
                status = if index as usize >= MAX_SPI || mode >= SpiMode::COUNT {
                    ErrorCode::InvalidArgument.status()
                } else if self.spi[index as usize].open {
                    ErrorCode::Busy.status()
                } else {
                    let m = SpiMode::from_u32(mode).ok_or(Error::Invalid("bad mode"))?;
                    match self.board.spi_open(index, freq, m) {
                        Ok(()) => {
                            self.spi[index as usize] = BusSlot { open: true, index };
                            let handle = index + 1;
                            put_u32(mem, args[3], handle)?;
                            o.u32(handle);
                            0
                        }
                        Err(e) => e.status(),
                    }
                };
            }
            HostFn::SpiBusWrite => {
                let data = guest_slice(mem, args[1], args[2])?;
                // abi-spec §9: spi.write の data は CRC-32 のみ。
                a.u64(args[0]);
                a.str(", crc32=");
                a.hex(crc32(data), 8);
                let n = data.len();
                status = match slot_of(args[0], MAX_SPI) {
                    Err(e) => e,
                    Ok(i) if !self.spi[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if n == 0 => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        let index = self.spi[i].index;
                        let data = guest_slice(mem, args[1], args[2])?;
                        match self.board.spi_write(index, data) {
                            Ok(()) => 0,
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::SpiBusTransfer => {
                let data_len = args[2] as u32 as usize;
                let cap = args[4] as u32 as usize;
                {
                    let data = guest_slice(mem, args[1], args[2])?;
                    a.u64(args[0]);
                    a.str(", crc32=");
                    a.hex(crc32(data), 8);
                }
                status = match slot_of(args[0], MAX_SPI) {
                    Err(e) => e,
                    Ok(i) if !self.spi[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if data_len == 0 || cap < data_len => ErrorCode::InvalidArgument.status(),
                    Ok(_) if data_len > SCRATCH => ErrorCode::Unsupported.status(),
                    Ok(i) => {
                        let index = self.spi[i].index;
                        // 送信元と受信先が重なりうるので、送信側を一度写す。
                        let mut tx = [0u8; SCRATCH];
                        tx[..data_len].copy_from_slice(guest_slice(mem, args[1], args[2])?);
                        match self.board.spi_transfer(
                            index,
                            &tx[..data_len],
                            &mut self.scratch[..data_len],
                        ) {
                            Ok(got) => {
                                let dst = args[3] as u32 as usize;
                                let end = dst
                                    .checked_add(got)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
                                mem.get_mut(dst..end)
                                    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?
                                    .copy_from_slice(&self.scratch[..got]);
                                put_u32(mem, args[5], got as u32)?;
                                o.str("len=");
                                o.u32(got as u32);
                                0
                            }
                            Err(e) => e.status(),
                        }
                    }
                };
            }
            HostFn::SpiBusDrop => {
                a.u64(args[0]);
                if let Ok(i) = slot_of(args[0], MAX_SPI)
                    && self.spi[i].open
                {
                    let index = self.spi[i].index;
                    self.board.spi_close(index);
                    self.spi[i] = BusSlot::default();
                }
            }

            HostFn::TimeNowUs => result64 = self.board.now_us(),
            HostFn::TimeSleepMs => self.board.sleep_ms(args[0] as u32),
            HostFn::TimeSleepUs => self.board.sleep_us(args[0] as u32),

            HostFn::LogLog => {
                let level = args[0] as u32;
                let msg = guest_slice(mem, args[1], args[2])?;
                a.u32(level);
                a.str(", ");
                a.byte(b'"');
                a.bytes(msg);
                a.byte(b'"');
                let l = LogLevel::from_u32(level).unwrap_or(LogLevel::Info);
                let msg = guest_slice(mem, args[1], args[2])?;
                // 借用の都合で写してから渡す。長いメッセージは切り詰める。
                let n = msg.len().min(SCRATCH);
                self.scratch[..n].copy_from_slice(&msg[..n]);
                let (scratch, board) = (&self.scratch, &mut self.board);
                board.log(l, &scratch[..n]);
            }
        }

        // 戻り値（abi-spec §4.4: result を返す関数は単一の i32 ステータス）。
        if !desc.sig.ends_with(':') && !results.is_empty() {
            results[0] = if matches!(f, HostFn::TimeNowUs) {
                result64
            } else {
                u64::from(status)
            };
        }

        // トレース（time は含めない。abi-spec §9）。
        if self.trace_on
            && !matches!(
                f,
                HostFn::TimeNowUs | HostFn::TimeSleepMs | HostFn::TimeSleepUs
            )
        {
            let mut line = [0u8; LINE * 2];
            let mut l = Buf::new(&mut line);
            l.str("> ");
            l.str(desc.module);
            l.byte(b'/');
            l.str(desc.name);
            l.byte(b'(');
            l.bytes(a.as_bytes());
            l.byte(b')');
            self.board.trace(l.as_bytes());

            l.clear();
            l.byte(b'<');
            if !desc.sig.ends_with(':') {
                l.byte(b' ');
                l.u32(status);
            }
            if !o.as_bytes().is_empty() {
                l.str(" [");
                l.bytes(o.as_bytes());
                l.byte(b']');
            }
            self.board.trace(l.as_bytes());
        }

        Ok(())
    }
}

/// abi-spec §8 が定める役割名。`'static` にするためここに置く。
pub const ROLE_NAMES: &[&str] = &["led", "lcd-cs", "lcd-dc", "lcd-rst"];
