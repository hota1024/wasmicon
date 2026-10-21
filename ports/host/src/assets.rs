//! Builds an assets slot image from named byte strings (docs/app-workflow.md §3.10).
//!
//! The host mock loads the files named in `wasmicon.toml` into an image of
//! the same layout the firmware reads from flash, and `wasmicon assets pack`
//! writes that image to a file. One builder keeps the two in step: what
//! `wasmicon run` reads on the host is byte for byte what the board gets.

use wasmicon_port::assets::{self, AssetsError, BODY_START, DIR_LEN, Directory};
use wasmicon_port::crc32;

/// Builds the slot image: the directory, then each body on a 4 KiB boundary,
/// in the order given. Padding is `0xff`, as erased flash reads.
///
/// # Errors
/// Whatever `Directory::insert` rejects (name, duplicate, count), or a body
/// longer than `u32::MAX`.
pub fn image(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, AssetsError> {
    let mut dir = Directory::EMPTY;
    for (name, body) in entries {
        let len = u32::try_from(body.len()).map_err(|_| AssetsError::BadLayout)?;
        dir.insert(name.as_bytes(), len, crc32(body))?;
    }
    let mut out = vec![0xff; dir.image_len() as usize];
    let mut head = [0u8; DIR_LEN];
    assets::encode(&dir, &mut head);
    out[..DIR_LEN].copy_from_slice(&head);
    debug_assert!(DIR_LEN as u32 <= BODY_START);
    for (i, (_, body)) in entries.iter().enumerate() {
        let at = dir.entry(i).offset as usize;
        out[at..at + body.len()].copy_from_slice(body);
    }
    Ok(out)
}

/// Parses the directory of an image built by `image` (or read back from a
/// board), treating the image's length as the slot length.
///
/// # Errors
/// Whatever `assets::parse` reports.
pub fn directory(image: &[u8]) -> Result<Directory, AssetsError> {
    let len = u32::try_from(image.len()).map_err(|_| AssetsError::BadLayout)?;
    assets::parse(image, len)
}
