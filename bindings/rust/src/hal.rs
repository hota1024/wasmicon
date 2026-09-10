//! 生成された extern 宣言に薄い安全ラッパをかぶせる。
//!
//! - ステータス i32 を `Result<_, ErrorCode>` にする（abi-spec §4.4）
//! - ハンドルは `Drop` で `[resource-drop]` を呼ぶ（abi-spec §5.2）
//! - `list<u8>` の戻り値は呼び出し側のバッファを渡す（abi-spec §4.4）
//!
//! ここには数値のハードコードを書かない。エラーコードもピン番号も
//! 生成物と `board` インターフェース経由で得る。

use crate::generated::types::ErrorCode;

/// HAL の結果。
pub type Result<T> = core::result::Result<T, ErrorCode>;

/// ステータスを `Result` に変換する。
#[inline]
fn check(status: u32) -> Result<()> {
    match ErrorCode::from_status(status) {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

/// 汎用デジタル入出力。
pub mod gpio {
    use super::{Result, check};
    use crate::generated::gpio as raw;

    pub use crate::generated::gpio::{Level, PinMode};

    /// 確保済みの GPIO ピン。`Drop` で解放する。
    pub struct Pin(u32);

    impl Pin {
        /// ピンを確保しモードを設定する。
        ///
        /// # Errors
        /// 既に確保済みなら `Busy`、範囲外なら `InvalidArgument`。
        pub fn open(index: u32, mode: PinMode) -> Result<Pin> {
            let mut h: u32 = 0;
            // SAFETY: h はスタック上の有効な u32。ホストは成功時のみ書き込む。
            check(unsafe { raw::pin_open(index, mode as u32, &raw mut h) })?;
            Ok(Pin(h))
        }

        /// モードを変更する。
        ///
        /// # Errors
        /// ハンドルが無効なら `InvalidHandle`。
        pub fn set_mode(&self, mode: PinMode) -> Result<()> {
            // SAFETY: self.0 は open が返した有効なハンドル。
            check(unsafe { raw::pin_set_mode(self.0, mode as u32) })
        }

        /// 現在の入力レベルを読む。
        ///
        /// # Errors
        /// ハンドルが無効なら `InvalidHandle`。
        pub fn read(&self) -> Result<Level> {
            let mut v: u32 = 0;
            // SAFETY: v はスタック上の有効な u32。
            check(unsafe { raw::pin_read(self.0, &raw mut v) })?;
            // ホストが範囲外の値を返すことは仕様上ないが、握りつぶさない。
            Level::from_u32(v).ok_or(super::ErrorCode::Io)
        }

        /// 出力レベルを設定する。
        ///
        /// # Errors
        /// 入力モードなら `InvalidArgument`。
        pub fn write(&self, level: Level) -> Result<()> {
            // SAFETY: self.0 は有効なハンドル。
            check(unsafe { raw::pin_write(self.0, level as u32) })
        }

        /// 出力レベルを反転する。
        ///
        /// # Errors
        /// 入力モードなら `InvalidArgument`。
        pub fn toggle(&self) -> Result<()> {
            // SAFETY: self.0 は有効なハンドル。
            check(unsafe { raw::pin_toggle(self.0) })
        }
    }

    impl Drop for Pin {
        fn drop(&mut self) {
            // SAFETY: 無効なハンドルの drop は無視される（abi-spec §5.2）。
            unsafe { raw::pin_drop(self.0) }
        }
    }
}

/// I2C マスター。
pub mod i2c {
    use super::{Result, check};
    use crate::generated::i2c as raw;

    pub use crate::generated::i2c::Speed;

    /// 確保済みの I2C バス。
    pub struct Bus(u32);

    impl Bus {
        /// バスを初期化して確保する。
        ///
        /// # Errors
        /// 既に確保済みなら `Busy`。
        pub fn open(index: u32, speed: Speed) -> Result<Bus> {
            let mut h: u32 = 0;
            // SAFETY: h はスタック上の有効な u32。
            check(unsafe { raw::bus_open(index, speed as u32, &raw mut h) })?;
            Ok(Bus(h))
        }

        /// `data` を書く。
        ///
        /// # Errors
        /// NACK なら `Nack`、長さ 0 なら `InvalidArgument`。
        pub fn write(&self, address: u16, data: &[u8]) -> Result<()> {
            // SAFETY: data のポインタと長さは対になっている。ホストは読むだけ。
            check(unsafe {
                raw::bus_write(self.0, u32::from(address), data.as_ptr(), data.len() as u32)
            })
        }

        /// `buf` の長さだけ読む。読めたバイト数を返す。
        ///
        /// # Errors
        /// NACK なら `Nack`。
        pub fn read(&self, address: u16, buf: &mut [u8]) -> Result<usize> {
            let mut len: u32 = 0;
            let cap = buf.len() as u32;
            // SAFETY: buf は cap バイトの可変スライス。len はスタック上の u32。
            check(unsafe {
                raw::bus_read(
                    self.0,
                    u32::from(address),
                    cap,
                    buf.as_mut_ptr(),
                    cap,
                    &raw mut len,
                )
            })?;
            Ok(len as usize)
        }

        /// `data` を書いてから `buf` の長さだけ読む（Repeated START）。
        ///
        /// # Errors
        /// NACK なら `Nack`。
        pub fn write_read(&self, address: u16, data: &[u8], buf: &mut [u8]) -> Result<usize> {
            let mut len: u32 = 0;
            let cap = buf.len() as u32;
            // SAFETY: data / buf ともポインタと長さが対になっている。
            check(unsafe {
                raw::bus_write_read(
                    self.0,
                    u32::from(address),
                    data.as_ptr(),
                    data.len() as u32,
                    cap,
                    buf.as_mut_ptr(),
                    cap,
                    &raw mut len,
                )
            })?;
            Ok(len as usize)
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            // SAFETY: 無効なハンドルの drop は無視される。
            unsafe { raw::bus_drop(self.0) }
        }
    }
}

/// SPI マスター。CS / DC はゲストが `gpio` で制御する（abi-spec §2 の設計）。
pub mod spi {
    use super::{Result, check};
    use crate::generated::spi as raw;

    pub use crate::generated::spi::Mode;

    /// 確保済みの SPI バス。
    pub struct Bus(u32);

    impl Bus {
        /// バスを初期化して確保する。
        ///
        /// # Errors
        /// 既に確保済みなら `Busy`。
        pub fn open(index: u32, frequency_hz: u32, mode: Mode) -> Result<Bus> {
            let mut h: u32 = 0;
            // SAFETY: h はスタック上の有効な u32。
            check(unsafe { raw::bus_open(index, frequency_hz, mode as u32, &raw mut h) })?;
            Ok(Bus(h))
        }

        /// `data` を送信する。受信は捨てる。描画のホットパス。
        ///
        /// # Errors
        /// 長さ 0 なら `InvalidArgument`。
        pub fn write(&self, data: &[u8]) -> Result<()> {
            // SAFETY: data のポインタと長さは対になっている。
            check(unsafe { raw::bus_write(self.0, data.as_ptr(), data.len() as u32) })
        }

        /// 全二重転送。`buf` は `data` 以上の長さが要る。
        ///
        /// # Errors
        /// `buf` が短ければ `InvalidArgument`。
        pub fn transfer(&self, data: &[u8], buf: &mut [u8]) -> Result<usize> {
            let mut len: u32 = 0;
            // SAFETY: data / buf ともポインタと長さが対になっている。
            check(unsafe {
                raw::bus_transfer(
                    self.0,
                    data.as_ptr(),
                    data.len() as u32,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &raw mut len,
                )
            })?;
            Ok(len as usize)
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            // SAFETY: 無効なハンドルの drop は無視される。
            unsafe { raw::bus_drop(self.0) }
        }
    }
}

/// 時間。
pub mod time {
    use crate::generated::time as raw;

    /// 起動からの単調増加マイクロ秒。
    #[must_use]
    pub fn now_us() -> u64 {
        // SAFETY: 引数なし・副作用なしの読み出し。
        unsafe { raw::now_us() }
    }

    /// ミリ秒待つ。
    pub fn sleep_ms(ms: u32) {
        // SAFETY: 引数はスカラーのみ。
        unsafe { raw::sleep_ms(ms) }
    }

    /// マイクロ秒待つ。
    pub fn sleep_us(us: u32) {
        // SAFETY: 引数はスカラーのみ。
        unsafe { raw::sleep_us(us) }
    }
}

/// デバッグ出力。
pub mod log {
    use crate::generated::log as raw;

    pub use crate::generated::log::Level;

    /// メッセージを出す。改行はホストが付ける。
    pub fn write(level: Level, message: &str) {
        // SAFETY: message のポインタと長さは対になっている。ホストは読むだけ。
        unsafe { raw::log(level as u32, message.as_ptr(), message.len() as u32) }
    }

    /// `info` で出す。
    pub fn info(message: &str) {
        write(Level::Info, message);
    }

    /// `error` で出す。
    pub fn error(message: &str) {
        write(Level::Error, message);
    }
}

/// ボード固有のピン割り当て。
pub mod board {
    use super::{Result, check};
    use crate::generated::board as raw;

    /// 役割名から GPIO 番号を引く。
    ///
    /// これを使うと同一バイナリが複数のボードで動く（abi-spec §8）。
    ///
    /// # Errors
    /// そのボードに割り当てが無ければ `Unsupported`。
    pub fn pin_by_role(role: &str) -> Result<u32> {
        let mut index: u32 = 0;
        // SAFETY: role のポインタと長さは対になっている。index はスタック上の u32。
        check(unsafe { raw::pin_by_role(role.as_ptr(), role.len() as u32, &raw mut index) })?;
        Ok(index)
    }
}
