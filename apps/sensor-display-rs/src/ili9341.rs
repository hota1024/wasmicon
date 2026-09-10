//! ILI9341 の最小ドライバ。`apps/README.md` §2 が正。
//!
//! 全画面フレームバッファは持たない（RP2040 に載らない）。矩形を指定して
//! 行単位でピクセルを流し込む。

use wasmicon_hal::gpio::{Level, Pin};
use wasmicon_hal::spi::Bus;
use wasmicon_hal::{ErrorCode, time};

use crate::font;

pub const WIDTH: u16 = 320;
pub const HEIGHT: u16 = 240;

/// 1 行分のピクセルバッファ（320 px × 2 バイト）。
const ROW_BYTES: usize = WIDTH as usize * 2;

/// 文字列描画で一度に送れる最大文字数。
pub const MAX_TEXT: usize = 12;

/// 文字列描画のバッファ（幅 MAX_TEXT×8 px × 8 行 × 2 バイト）。
const TEXT_BYTES: usize = MAX_TEXT * 8 * 8 * 2;

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

    fn begin(&self) -> Result<()> {
        self.cs.write(Level::Low)
    }

    fn end(&self) -> Result<()> {
        self.cs.write(Level::High)
    }

    fn cmd(&self, c: u8) -> Result<()> {
        self.dc.write(Level::Low)?;
        self.spi.write(&[c])
    }

    fn data(&self, d: &[u8]) -> Result<()> {
        self.dc.write(Level::High)?;
        self.spi.write(d)
    }

    /// コマンドと引数を 1 回の CS で送る。
    fn send(&self, c: u8, args: &[u8]) -> Result<()> {
        self.begin()?;
        self.cmd(c)?;
        if !args.is_empty() {
            self.data(args)?;
        }
        self.end()
    }

    /// リセットと初期化列。
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
        self.send(0x36, &[0x28])?; // MADCTL = 横向き
        self.send(0x29, &[]) // DISPON
    }

    /// 書き込み先の矩形を決める。
    fn window(&self, x: u16, y: u16, w: u16, h: u16) -> Result<()> {
        let x1 = x + w - 1;
        let y1 = y + h - 1;
        self.send(0x2A, &[(x >> 8) as u8, x as u8, (x1 >> 8) as u8, x1 as u8])?;
        self.send(0x2B, &[(y >> 8) as u8, y as u8, (y1 >> 8) as u8, y1 as u8])
    }

    /// 矩形を単色で塗る。行単位で送る。
    ///
    /// # Errors
    /// GPIO か SPI が失敗したとき。
    pub fn fill_rect(&self, x: u16, y: u16, w: u16, h: u16, color: u16) -> Result<()> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        self.window(x, y, w, h)?;

        let mut row = [0u8; ROW_BYTES];
        let n = w as usize * 2;
        let mut i = 0;
        while i < n {
            row[i] = (color >> 8) as u8;
            row[i + 1] = color as u8;
            i += 2;
        }

        self.begin()?;
        self.cmd(0x2C)?; // RAMWR
        let mut r = 0;
        while r < h {
            self.data(&row[..n])?;
            r += 1;
        }
        self.end()
    }

    /// 等幅 8×8 で文字列を描く。`MAX_TEXT` 文字まで。
    ///
    /// # Errors
    /// GPIO か SPI が失敗したとき。
    pub fn draw_text(&self, x: u16, y: u16, text: &[u8], fg: u16, bg: u16) -> Result<()> {
        let len = if text.len() > MAX_TEXT {
            MAX_TEXT
        } else {
            text.len()
        };
        if len == 0 {
            return Ok(());
        }
        let w = (len * 8) as u16;
        self.window(x, y, w, 8)?;

        // 8 行ぶんをまとめて組み立てる。行の中は文字ごとに 8 px。
        let mut buf = [0u8; TEXT_BYTES];
        let stride = len * 8 * 2;
        let mut row = 0;
        while row < 8 {
            let mut col = 0;
            while col < len {
                let bits = font::GLYPHS[font::index_of(text[col])][row];
                let mut bit = 0;
                while bit < 8 {
                    let on = bits & (0x80 >> bit) != 0;
                    let c = if on { fg } else { bg };
                    let p = row * stride + (col * 8 + bit) * 2;
                    buf[p] = (c >> 8) as u8;
                    buf[p + 1] = c as u8;
                    bit += 1;
                }
                col += 1;
            }
            row += 1;
        }

        self.begin()?;
        self.cmd(0x2C)?; // RAMWR
        self.data(&buf[..stride * 8])?;
        self.end()
    }
}
