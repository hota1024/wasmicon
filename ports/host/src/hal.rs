//! PC 用の mock HAL。
//!
//! import の解決は生成された表（`wasmicon_core::generated::IMPORTS`）を
//! module 名 + name + sig の完全一致で引く（abi-spec §6.4）。
//! HAL の意味づけはここ（ポート層）の責務で、コアには入れない。
//!
//! `WASMICON_TRACE` を立てると全 host call を abi-spec §9 の形式で出す。

use std::fmt::Write as _;
use std::time::Instant;

use wasmicon_core::error::{Error, Result, Trap};
use wasmicon_core::generated::{self, ErrorCode, HostFn};
use wasmicon_core::instance::{Extern, ExternType, Resolver};

/// abi-spec §5.3 が保証する同時ハンドル数。
const MAX_PINS: usize = 16;
const MAX_I2C: usize = 2;
const MAX_SPI: usize = 2;

/// ボードの GPIO 本数（mock）。
const NUM_GPIO: usize = 48;

/// mock ボードの役割名 → GPIO 番号（abi-spec §8）。
/// 実機のポートはこの表を自分のボードのものに差し替える。
const ROLES: &[(&str, u32)] = &[("led", 2), ("lcd-cs", 10), ("lcd-dc", 11), ("lcd-rst", 12)];

/// AssemblyScript が import する `env.abort` に割り当てる識別子。
/// `world app` には無い例外的な import（HANDOFF §6）。
const HOST_ENV_ABORT: u32 = 0xffff;

#[derive(Clone, Copy, Default)]
struct Pin {
    open: bool,
    index: u32,
    mode: u32,
    level: u32,
}

#[derive(Clone, Copy, Default)]
struct Bus {
    open: bool,
}

/// mock HAL の状態とトレース。
pub struct MockHal {
    trace: bool,
    out: String,
    pins: [Pin; MAX_PINS],
    i2c: [Bus; MAX_I2C],
    spi: [Bus; MAX_SPI],
    gpio_used: [bool; NUM_GPIO],
    /// `pin-by-role` で配った番号と役割名（abi-spec §9 の正規化に使う）。
    roles: Vec<(u32, &'static str)>,
    start: Instant,
}

impl MockHal {
    #[must_use]
    pub fn new(trace: bool) -> Self {
        MockHal {
            trace,
            out: String::new(),
            pins: [Pin::default(); MAX_PINS],
            i2c: [Bus::default(); MAX_I2C],
            spi: [Bus::default(); MAX_SPI],
            gpio_used: [false; NUM_GPIO],
            roles: Vec::new(),
            start: Instant::now(),
        }
    }

    /// 溜めたトレース。
    #[must_use]
    pub fn trace_output(&self) -> &str {
        &self.out
    }

    /// abi-spec §9: 役割名で配った GPIO 番号は番号ではなく役割名で出す。
    ///
    /// 判断材料は数値だけなので、ゲストがハードコードした番号が役割割り当てと
    /// 一致していると、それも役割名になる。abi-spec §9 がその前提を明記している。
    fn fmt_pin(&self, index: u32) -> String {
        match self.roles.iter().find(|(i, _)| *i == index) {
            Some((_, role)) => format!("role:{role}"),
            None => index.to_string(),
        }
    }

    fn emit(&mut self, line: &str) {
        if self.trace {
            self.out.push_str(line);
            self.out.push('\n');
        }
    }
}

/// CRC-32 (IEEE)。長い `list<u8>` のトレース用。
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// abi-spec §9 の `list<u8>` 表記。
fn fmt_bytes(data: &[u8]) -> String {
    if data.len() <= 32 {
        let mut s = String::from("0x");
        for b in data {
            let _ = write!(s, "{b:02x}");
        }
        s
    } else {
        let mut s = String::from("0x");
        for b in &data[..16] {
            let _ = write!(s, "{b:02x}");
        }
        let _ = write!(s, "..len={} crc32={:08x}", data.len(), crc32(data));
        s
    }
}

fn guest_slice(mem: &[u8], ptr: u32, len: u32) -> Result<&[u8]> {
    let a = ptr as usize;
    let b = len as usize;
    mem.get(
        a..a.checked_add(b)
            .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?,
    )
    .ok_or(Error::Trap(Trap::MemoryOutOfBounds))
}

fn put_u32(mem: &mut [u8], ptr: u32, v: u32) -> Result<()> {
    let a = ptr as usize;
    let s = mem
        .get_mut(a..a + 4)
        .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
    s.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn put_bytes(mem: &mut [u8], ptr: u32, src: &[u8]) -> Result<()> {
    let a = ptr as usize;
    let s = mem
        .get_mut(a..a + src.len())
        .ok_or(Error::Trap(Trap::MemoryOutOfBounds))?;
    s.copy_from_slice(src);
    Ok(())
}

/// ハンドル（1 始まり）を配列の添字に。0 は無効（abi-spec §5.1）。
fn handle_index(h: u64, len: usize) -> std::result::Result<usize, u32> {
    let h = h as u32;
    if h == 0 || h as usize > len {
        return Err(ErrorCode::InvalidHandle.status());
    }
    Ok(h as usize - 1)
}

impl Resolver for MockHal {
    fn resolve(&mut self, module: &str, name: &str, ty: &ExternType<'_>) -> Option<Extern> {
        // AssemblyScript の env.abort は world app に無い例外的な import。
        if module == "env" && name == "abort" {
            return Some(Extern::Func(HOST_ENV_ABORT));
        }
        let ExternType::Func(ft) = ty else {
            return None;
        };
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

        // 引数のトレース（time は含めない。abi-spec §9）。
        let traced = !matches!(
            f,
            HostFn::TimeNowUs | HostFn::TimeSleepMs | HostFn::TimeSleepUs
        );

        let mut argtext = String::new();
        let mut status: u32 = 0;
        let mut outs: Vec<String> = Vec::new();

        match f {
            HostFn::GpioPinOpen => {
                let (index, mode, out) = (args[0] as u32, args[1] as u32, args[2] as u32);
                let _ = write!(argtext, "{}, {mode}", self.fmt_pin(index));
                let slot = self.pins.iter().position(|p| !p.open);
                status = if index as usize >= NUM_GPIO || mode >= generated::gpio::PinMode::COUNT {
                    ErrorCode::InvalidArgument.status()
                } else if self.gpio_used[index as usize] {
                    ErrorCode::Busy.status()
                } else if let Some(slot) = slot {
                    self.pins[slot] = Pin {
                        open: true,
                        index,
                        mode,
                        level: 0,
                    };
                    self.gpio_used[index as usize] = true;
                    let handle = slot as u32 + 1;
                    put_u32(mem, out, handle)?;
                    outs.push(handle.to_string());
                    0
                } else {
                    ErrorCode::OutOfMemory.status()
                };
            }
            HostFn::GpioPinSetMode => {
                let mode = args[1] as u32;
                let _ = write!(argtext, "{}, {mode}", args[0]);
                status = match handle_index(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if mode >= generated::gpio::PinMode::COUNT => {
                        ErrorCode::InvalidArgument.status()
                    }
                    Ok(i) => {
                        self.pins[i].mode = mode;
                        0
                    }
                };
            }
            HostFn::GpioPinRead => {
                let out = args[1] as u32;
                let _ = write!(argtext, "{}", args[0]);
                status = match handle_index(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(i) => {
                        let level = self.pins[i].level;
                        put_u32(mem, out, level)?;
                        outs.push(level.to_string());
                        0
                    }
                };
            }
            HostFn::GpioPinWrite => {
                let level = args[1] as u32;
                let _ = write!(argtext, "{}, {level}", args[0]);
                status = match handle_index(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if level >= generated::gpio::Level::COUNT => {
                        ErrorCode::InvalidArgument.status()
                    }
                    Ok(i) if self.pins[i].mode < 3 => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        self.pins[i].level = level;
                        0
                    }
                };
            }
            HostFn::GpioPinToggle => {
                let _ = write!(argtext, "{}", args[0]);
                status = match handle_index(args[0], MAX_PINS) {
                    Err(e) => e,
                    Ok(i) if !self.pins[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(i) if self.pins[i].mode < 3 => ErrorCode::InvalidArgument.status(),
                    Ok(i) => {
                        self.pins[i].level ^= 1;
                        0
                    }
                };
            }
            HostFn::GpioPinDrop => {
                let _ = write!(argtext, "{}", args[0]);
                if let Ok(i) = handle_index(args[0], MAX_PINS)
                    && self.pins[i].open
                {
                    self.gpio_used[self.pins[i].index as usize] = false;
                    self.pins[i] = Pin::default();
                }
            }

            HostFn::I2cBusOpen => {
                let (index, speed, out) = (args[0] as u32, args[1] as u32, args[2] as u32);
                let _ = write!(argtext, "{index}, {speed}");
                status = if index as usize >= MAX_I2C || speed >= generated::i2c::Speed::COUNT {
                    ErrorCode::InvalidArgument.status()
                } else if self.i2c[index as usize].open {
                    ErrorCode::Busy.status()
                } else {
                    self.i2c[index as usize] = Bus { open: true };
                    0
                };
                if status == 0 {
                    let handle = index + 1;
                    put_u32(mem, out, handle)?;
                    outs.push(handle.to_string());
                }
            }
            HostFn::I2cBusWrite => {
                let data = guest_slice(mem, args[2] as u32, args[3] as u32)?;
                let _ = write!(argtext, "{}, {}, {}", args[0], args[1], fmt_bytes(data));
                status = match handle_index(args[0], MAX_I2C) {
                    Err(e) => e,
                    Ok(i) if !self.i2c[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if data.is_empty() => ErrorCode::InvalidArgument.status(),
                    // mock にはデバイスがいないので NACK。Phase 6 で記録応答を差す。
                    Ok(_) => ErrorCode::Nack.status(),
                };
            }
            HostFn::I2cBusRead => {
                let _ = write!(argtext, "{}, {}, {}", args[0], args[1], args[2]);
                status = ErrorCode::Nack.status();
            }
            HostFn::I2cBusWriteRead => {
                let data = guest_slice(mem, args[2] as u32, args[3] as u32)?;
                let _ = write!(
                    argtext,
                    "{}, {}, {}, {}",
                    args[0],
                    args[1],
                    fmt_bytes(data),
                    args[4]
                );
                status = ErrorCode::Nack.status();
            }
            HostFn::I2cBusDrop => {
                let _ = write!(argtext, "{}", args[0]);
                if let Ok(i) = handle_index(args[0], MAX_I2C) {
                    self.i2c[i] = Bus::default();
                }
            }

            HostFn::SpiBusOpen => {
                let (index, freq, mode, out) = (
                    args[0] as u32,
                    args[1] as u32,
                    args[2] as u32,
                    args[3] as u32,
                );
                let _ = write!(argtext, "{index}, {freq}, {mode}");
                status = if index as usize >= MAX_SPI || mode >= generated::spi::Mode::COUNT {
                    ErrorCode::InvalidArgument.status()
                } else if self.spi[index as usize].open {
                    ErrorCode::Busy.status()
                } else {
                    self.spi[index as usize] = Bus { open: true };
                    0
                };
                if status == 0 {
                    let handle = index + 1;
                    put_u32(mem, out, handle)?;
                    outs.push(handle.to_string());
                }
            }
            HostFn::SpiBusWrite => {
                let data = guest_slice(mem, args[1] as u32, args[2] as u32)?;
                // abi-spec §9: spi.write の data は CRC-32 のみ。
                let _ = write!(argtext, "{}, crc32={:08x}", args[0], crc32(data));
                status = match handle_index(args[0], MAX_SPI) {
                    Err(e) => e,
                    Ok(i) if !self.spi[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if data.is_empty() => ErrorCode::InvalidArgument.status(),
                    Ok(_) => 0,
                };
            }
            HostFn::SpiBusTransfer => {
                let (ptr, len, buf, cap, len_out) = (
                    args[1] as u32,
                    args[2] as u32,
                    args[3] as u32,
                    args[4] as u32,
                    args[5] as u32,
                );
                let data = guest_slice(mem, ptr, len)?.to_vec();
                let _ = write!(argtext, "{}, crc32={:08x}", args[0], crc32(&data));
                status = match handle_index(args[0], MAX_SPI) {
                    Err(e) => e,
                    Ok(i) if !self.spi[i].open => ErrorCode::InvalidHandle.status(),
                    Ok(_) if data.is_empty() || cap < len => ErrorCode::InvalidArgument.status(),
                    Ok(_) => {
                        // mock は MISO を 0 で返す。
                        let zeros = vec![0u8; data.len()];
                        put_bytes(mem, buf, &zeros)?;
                        put_u32(mem, len_out, len)?;
                        outs.push(format!("len={len}"));
                        0
                    }
                };
            }
            HostFn::SpiBusDrop => {
                let _ = write!(argtext, "{}", args[0]);
                if let Ok(i) = handle_index(args[0], MAX_SPI) {
                    self.spi[i] = Bus::default();
                }
            }

            HostFn::TimeNowUs => {
                results[0] = self.start.elapsed().as_micros() as u64;
            }
            HostFn::TimeSleepMs => {
                std::thread::sleep(std::time::Duration::from_millis(args[0]));
            }
            HostFn::TimeSleepUs => {
                std::thread::sleep(std::time::Duration::from_micros(args[0]));
            }

            HostFn::BoardPinByRole => {
                let role = guest_slice(mem, args[0] as u32, args[1] as u32)?;
                let text = String::from_utf8_lossy(role).into_owned();
                let _ = write!(argtext, "{text:?}");
                match ROLES.iter().find(|(r, _)| *r == text) {
                    Some(&(role_name, index)) => {
                        put_u32(mem, args[2] as u32, index)?;
                        if !self.roles.iter().any(|(i, _)| *i == index) {
                            self.roles.push((index, role_name));
                        }
                        // 番号そのものはボード依存なので役割名で出す（abi-spec §9）。
                        outs.push(format!("role:{role_name}"));
                    }
                    None => status = ErrorCode::Unsupported.status(),
                }
            }

            HostFn::LogLog => {
                let level = args[0] as u32;
                let msg = guest_slice(mem, args[1] as u32, args[2] as u32)?;
                // abi-spec §4.2: UTF-8 の検証はしない。
                let text = String::from_utf8_lossy(msg);
                let _ = write!(argtext, "{level}, {text:?}");
                println!("[wasm] {text}");
            }
        }

        if traced {
            let outtext = if outs.is_empty() {
                String::new()
            } else {
                format!(" [{}]", outs.join(", "))
            };
            let line = format!("> {}/{}({argtext})", desc.module, desc.name);
            self.emit(&line);
            let has_status = !desc.sig.ends_with(':');
            if has_status {
                self.emit(&format!("< {status}{outtext}"));
            } else {
                self.emit("<");
            }
        }

        if has_result(desc.sig) && results.is_empty() {
            return Err(Error::Invalid("host result arity mismatch"));
        }
        if !matches!(f, HostFn::TimeNowUs) && has_result(desc.sig) {
            results[0] = u64::from(status);
        }
        Ok(())
    }
}

fn has_result(sig: &str) -> bool {
    !sig.ends_with(':')
}
