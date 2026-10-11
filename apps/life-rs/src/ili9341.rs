//! ILI9341 の最小ドライバ（初期化と矩形塗りだけ）。
//!
//! 初期化の手順は `apps/README.md` §2 と同じ。他の 3 つの写し
//! （`apps/README.md` §0）と違い、**矩形のピクセルを行ごとではなく、
//! 1 回の SPI 転送にまとめて送る**（640 バイトずつ）。ライフゲームは毎世代
//! 何十ものセルを描き直すので、行ごとに `dc` と `spi.write` を呼ぶと
//! host call が 7 倍になり、トレースを出すビルドでは UART が追いつかない。

use wasmicon_hal::gpio::{Level, Pin};
use wasmicon_hal::spi::Bus;
use wasmicon_hal::{ErrorCode, time};

use crate::font;

pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 240;

/// 1 回に送るピクセルのバッファ（320 px ぶん）。
const CHUNK: usize = WIDTH as usize * 2;

type Result<T> = core::result::Result<T, ErrorCode>;

/// 繋がった ILI9341。
pub struct Display<'a> {
    spi: &'a Bus,
    cs: &'a Pin,
    dc: &'a Pin,
}

impl<'a> Display<'a> {
    #[must_use]
    pub fn new(spi: &'a Bus, cs: &'a Pin, dc: &'a Pin) -> Self {
        Display { spi, cs, dc }
    }

    /// コマンドと引数を 1 回の CS で送る。
    fn send(&self, c: u8, args: &[u8]) -> Result<()> {
        self.cs.write(Level::Low)?;
        self.dc.write(Level::Low)?;
        self.spi.write(&[c])?;
        if !args.is_empty() {
            self.dc.write(Level::High)?;
            self.spi.write(args)?;
        }
        self.cs.write(Level::High)
    }

    /// リセットと初期化列（`apps/README.md` §2）。
    ///
    /// # Errors
    /// GPIO か SPI が失敗したとき。
    pub fn init(&self, rst: &Pin) -> Result<()> {
        self.cs.write(Level::High)?;
        rst.write(Level::Low)?;
        time::sleep_ms(10);
        rst.write(Level::High)?;
        time::sleep_ms(120);
        self.send(0x01, &[])?; // SWRESET
        time::sleep_ms(120);
        self.send(0x11, &[])?; // SLPOUT
        time::sleep_ms(120);
        self.send(0x3A, &[0x55])?; // PIXFMT = RGB565
        self.send(0x36, &[0x28])?; // MADCTL = 横向き、BGR
        self.send(0x29, &[]) // DISPON
    }

    /// 矩形を単色で塗る。ピクセルは 640 バイトずつまとめて送る。
    ///
    /// # Errors
    /// 画面からはみ出すとき（`invalid-argument`）、GPIO か SPI が失敗したとき。
    pub fn fill_rect(&self, x: u16, y: u16, w: u16, h: u16, color: u16) -> Result<()> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        // u32 に広げてから比べる（u16 の和は折り返す。apps/README.md §0）。
        if u32::from(x) + u32::from(w) > u32::from(WIDTH)
            || u32::from(y) + u32::from(h) > u32::from(HEIGHT)
        {
            return Err(ErrorCode::InvalidArgument);
        }
        let (x1, y1) = (x + w - 1, y + h - 1);
        self.send(0x2A, &[(x >> 8) as u8, x as u8, (x1 >> 8) as u8, x1 as u8])?;
        self.send(0x2B, &[(y >> 8) as u8, y as u8, (y1 >> 8) as u8, y1 as u8])?;

        // 要る分だけ埋める。7×7 のセルなら 98 バイトで、640 バイト全部を
        // 毎回埋めると、それだけで 1 セルに数千命令かかっていた。
        let total = usize::from(w) * usize::from(h) * 2;
        let fill = if total < CHUNK { total } else { CHUNK };
        let mut buf = [0u8; CHUNK];
        let mut i = 0;
        while i < fill {
            buf[i] = (color >> 8) as u8;
            buf[i + 1] = color as u8;
            i += 2;
        }
        self.cs.write(Level::Low)?;
        self.dc.write(Level::Low)?;
        self.spi.write(&[0x2C])?; // RAMWR
        self.dc.write(Level::High)?;
        let mut left = total;
        while left > 0 {
            let n = if left < CHUNK { left } else { CHUNK };
            self.spi.write(&buf[..n])?;
            left -= n;
        }
        self.cs.write(Level::High)
    }

    /// 等幅 8×8 で文字列を描く（1 行ずつ送る）。画面 1 行に収まる 40 文字まで。
    ///
    /// # Errors
    /// 画面からはみ出すとき（`invalid-argument`）、GPIO か SPI が失敗したとき。
    pub fn draw_text(&self, x: u16, y: u16, text: &[u8], fg: u16, bg: u16) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let w = text.len() * 8;
        if u32::from(x) + w as u32 > u32::from(WIDTH) || u32::from(y) + 8 > u32::from(HEIGHT) {
            return Err(ErrorCode::InvalidArgument);
        }
        let (x1, y1) = (x + w as u16 - 1, y + 7);
        self.send(0x2A, &[(x >> 8) as u8, x as u8, (x1 >> 8) as u8, x1 as u8])?;
        self.send(0x2B, &[(y >> 8) as u8, y as u8, (y1 >> 8) as u8, y1 as u8])?;
        self.cs.write(Level::Low)?;
        self.dc.write(Level::Low)?;
        self.spi.write(&[0x2C])?; // RAMWR
        self.dc.write(Level::High)?;
        let mut buf = [0u8; CHUNK];
        for row in 0..8 {
            let mut p = 0;
            for &c in text {
                let bits = font::glyph(c)[row];
                for bit in 0..8 {
                    let color = if bits & (0x80 >> bit) != 0 { fg } else { bg };
                    buf[p] = (color >> 8) as u8;
                    buf[p + 1] = color as u8;
                    p += 2;
                }
            }
            self.spi.write(&buf[..p])?;
        }
        self.cs.write(Level::High)
    }

    /// 用意済みのピクセル（RGB565、ビッグエンディアン）を矩形に流し込む。
    ///
    /// **窓の指定と書き込みを 1 回の CS の中で続けて送る**（host call 14 回）。
    /// ライフゲームのセルのように、同じ絵を何度も描くときに使う。画素を
    /// 毎回組み立てないので、インタプリタの命令数も減る。
    ///
    /// # Errors
    /// 画面からはみ出すとき、`pixels` の長さが `w * h * 2` でないとき
    /// （`invalid-argument`）、GPIO か SPI が失敗したとき。
    pub fn blit(&self, x: u16, y: u16, w: u16, h: u16, pixels: &[u8]) -> Result<()> {
        if w == 0
            || h == 0
            || u32::from(x) + u32::from(w) > u32::from(WIDTH)
            || u32::from(y) + u32::from(h) > u32::from(HEIGHT)
            || pixels.len() != usize::from(w) * usize::from(h) * 2
        {
            return Err(ErrorCode::InvalidArgument);
        }
        let (x1, y1) = (x + w - 1, y + h - 1);
        self.cs.write(Level::Low)?;
        for (cmd, a, b) in [(0x2A, x, x1), (0x2B, y, y1)] {
            self.dc.write(Level::Low)?;
            self.spi.write(&[cmd])?;
            self.dc.write(Level::High)?;
            self.spi
                .write(&[(a >> 8) as u8, a as u8, (b >> 8) as u8, b as u8])?;
        }
        self.dc.write(Level::Low)?;
        self.spi.write(&[0x2C])?; // RAMWR
        self.dc.write(Level::High)?;
        self.spi.write(pixels)?;
        self.cs.write(Level::High)
    }
}
