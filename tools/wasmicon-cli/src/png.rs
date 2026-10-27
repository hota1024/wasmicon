//! Writes the mock display's picture as a PNG (`wasmicon run --display-out`).
//!
//! - 8-bit RGB, no interlace. Each RGB565 channel is widened by repeating its
//!   high bits, so 0x1f and 0x3f become 0xff.
//! - The zlib stream uses stored (uncompressed) deflate blocks: a 320x240
//!   picture is about 230 KB, and no compression crate is needed. The chunk
//!   CRC is the CRC-32 of `wasmicon_port`, the one PNG uses.

use wasmicon_host::hal::Frame;

/// Largest payload of one stored deflate block.
const STORED_MAX: usize = 65535;

/// Encodes `frame` as a PNG file.
#[must_use]
pub fn encode(frame: &Frame) -> Vec<u8> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    // Each row starts with filter type 0 (none).
    let mut raw = Vec::with_capacity(h * (1 + 3 * w));
    for row in frame.pixels.chunks(w.max(1)).take(h) {
        raw.push(0);
        for &p in row {
            raw.extend_from_slice(&rgb888(p));
        }
    }

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&frame.width.to_be_bytes());
    ihdr.extend_from_slice(&frame.height.to_be_bytes());
    // Bit depth 8, color type 2 (RGB), deflate, adaptive filter, no interlace.
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// RGB565 to 8-bit channels.
fn rgb888(p: u16) -> [u8; 3] {
    let r = ((p >> 11) & 0x1f) as u8;
    let g = ((p >> 5) & 0x3f) as u8;
    let b = (p & 0x1f) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = wasmicon_port::crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A zlib stream (RFC 1950) of stored deflate blocks (RFC 1951 §3.2.4).
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks = data.chunks(STORED_MAX).collect::<Vec<_>>();
    if blocks.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    for (i, block) in blocks.iter().enumerate() {
        out.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_widen_to_full_scale() {
        assert_eq!(rgb888(0xffff), [0xff, 0xff, 0xff]);
        assert_eq!(rgb888(0xf800), [0xff, 0, 0]);
        assert_eq!(rgb888(0x07e0), [0, 0xff, 0]);
        assert_eq!(rgb888(0x001f), [0, 0, 0xff]);
        assert_eq!(rgb888(0), [0, 0, 0]);
    }

    #[test]
    fn adler32_matches_rfc_1950_example() {
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn a_frame_larger_than_one_stored_block_encodes() {
        let frame = Frame {
            width: 320,
            height: 240,
            pixels: vec![0xf800; 320 * 240],
        };
        let png = encode(&frame);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.ends_with(&[0xae, 0x42, 0x60, 0x82]), "IEND CRC");
        // Header + 4 stored blocks of 5 bytes each + Adler-32, around the raw rows.
        let raw = 240 * (1 + 3 * 320);
        assert!(png.len() > raw && png.len() < raw + 200);
    }
}
