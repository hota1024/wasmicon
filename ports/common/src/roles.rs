//! Role map (role name -> GPIO number) and the config slot format that holds
//! it (docs/app-workflow.md §3.9), together with the board's display entry
//! (`Wiring`; docs/device-layer.md §5.2).
//!
//! Role names are not firmware vocabulary. They are a contract between the
//! app (`[requirements] pin-roles` in `wasmicon.toml`) and the wiring
//! (`[board.<name>.roles]`); the firmware only answers `pin-by-role` from the
//! table `deploy` wrote to the config slot. The firmware has no default table:
//! with an empty or corrupt slot it hands out no role and drives no pin. The
//! layout is on `Header`.
//!
//! The body is text so that no new compatibility axis (role IDs) is needed
//! and a flash dump stays human-readable (docs/app-workflow.md §3.9).
//!
//! The writer (CLI) and the reader (firmware) share this module so the checks
//! are written once. Without `alloc`, the name length and the number of roles
//! are bounded (`MAX_NAME` / `MAX_ROLES`).

use crate::crc32;
use crate::fmt::Buf;
use crate::profile::{Profile, Slot};

/// Identifier at the start of the slot. Distinct from the app slot's `"WMCA"`.
pub const MAGIC: [u8; 4] = *b"WMCR";

/// Newest format version this firmware reads. Version 1 holds roles only;
/// version 2 may add one `@display=` line. The writer uses version 1 when there
/// is no display entry, so a firmware that reads only version 1 keeps reading
/// every other app's roles (docs/TODO.md §6-5).
pub const FORMAT_VERSION: u16 = 2;

/// Header size. The body starts right after it.
pub const HEADER_LEN: usize = 16;

/// Maximum number of roles.
pub const MAX_ROLES: usize = 8;

/// Maximum role name length in bytes.
pub const MAX_NAME: usize = 16;

/// Maximum body length. Holds `MAX_ROLES` lines of (16-byte name + `=` + 3-digit
/// number + newline) plus the longest display line (a test locks this in).
pub const MAX_BODY: usize = 256;

/// Prefix of the display line. `@` cannot start a role name (`valid_name`).
const DISPLAY_KEY: &[u8] = b"@display=";

/// Size reserved for the config slot (one flash erase unit).
pub const SLOT_LEN: u32 = 4096;

/// Why the role map cannot be read or built.
///
/// The text is a `&'static str` (no `core::fmt`). Which entry failed is
/// reported by the writer (CLI), which calls `insert` one entry at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RolesError {
    /// Erased (`0xff`) or never written. Not a failure but "empty".
    Empty,
    /// Wrong magic.
    BadMagic,
    /// A version this firmware cannot read.
    UnsupportedVersion,
    /// The body is too long (over `MAX_BODY`) or points past the slot.
    TooLong,
    /// CRC mismatch.
    BadCrc,
    /// A line is not of the form `name=gpio`.
    BadLine,
    /// A role name contains a disallowed character or has a bad length.
    BadName,
    /// A GPIO number is not a decimal number.
    BadNumber,
    /// More than `MAX_ROLES` roles.
    TooMany,
    /// The same role name appears twice.
    DuplicateName,
    /// Two roles are assigned to the same GPIO number.
    DuplicatePin,
    /// A GPIO number exceeds the board's GPIO count.
    OutOfRange,
    /// A reserved pin (trace UART, flash, radio chip and so on).
    Reserved,
    /// A pin the port uses for SPI / I2C. Assigning it breaks the bus silently.
    BusPin,
    /// The firmware's end reaches into the config slot. Must not be read.
    Overlap,
    /// The display line is malformed, repeated, or has a value out of range.
    BadDisplay,
}

impl RolesError {
    /// Text for the log.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            RolesError::Empty => "roles empty",
            RolesError::BadMagic => "roles magic mismatch",
            RolesError::UnsupportedVersion => "roles format version not supported",
            RolesError::TooLong => "roles too long",
            RolesError::BadCrc => "roles crc mismatch",
            RolesError::BadLine => "roles line is not name=gpio",
            RolesError::BadName => "role name must be 1..16 of a-z 0-9 - starting with a-z",
            RolesError::BadNumber => "role gpio is not a decimal number",
            RolesError::TooMany => "too many roles",
            RolesError::DuplicateName => "role name appears twice",
            RolesError::DuplicatePin => "two roles share one gpio",
            RolesError::OutOfRange => "role gpio out of range",
            RolesError::Reserved => "role gpio is reserved",
            RolesError::BusPin => "role gpio is used by spi / i2c",
            RolesError::Overlap => "firmware overlaps the roles slot",
            RolesError::BadDisplay => "display entry is malformed",
        }
    }

    /// Whether the slot is empty. The one case that is not an error.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == RolesError::Empty
    }
}

/// Constraints on which GPIOs may be assigned on the board.
#[derive(Clone, Copy)]
pub struct Limits<'a> {
    /// Number of GPIOs.
    pub gpio_count: u32,
    /// Reserved pins.
    pub reserved: &'a [u32],
    /// Pins the port uses for SPI / I2C.
    pub bus_pins: &'a [u32],
}

impl Limits<'static> {
    /// Builds the limits from a profile.
    #[must_use]
    pub const fn of(p: &'static Profile) -> Self {
        Limits {
            gpio_count: p.gpio_count,
            reserved: p.reserved,
            bus_pins: p.bus_pins,
        }
    }
}

/// Whether `name` is a valid role name: 1..=16 characters of `a-z` / `0-9` /
/// `-`, starting with a lowercase letter.
///
/// The alphabet is restricted so that names cannot collide with the body's
/// `=` and newline or with `role:<name>` in traces.
#[must_use]
pub fn valid_name(name: &[u8]) -> bool {
    if name.is_empty() || name.len() > MAX_NAME || !name[0].is_ascii_lowercase() {
        return false;
    }
    name.iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// Role name -> GPIO number table. Fixed size, no `alloc`.
#[derive(Clone, Copy)]
pub struct RoleMap {
    names: [[u8; MAX_NAME]; MAX_ROLES],
    lens: [u8; MAX_ROLES],
    pins: [u32; MAX_ROLES],
    n: usize,
}

impl RoleMap {
    /// A table with no roles. The firmware runs with this when the config slot
    /// is empty or corrupt.
    pub const EMPTY: RoleMap = RoleMap {
        names: [[0; MAX_NAME]; MAX_ROLES],
        lens: [0; MAX_ROLES],
        pins: [0; MAX_ROLES],
        n: 0,
    };

    /// Number of roles.
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Name of the `i`-th role.
    #[must_use]
    pub fn name(&self, i: usize) -> &[u8] {
        &self.names[i][..self.lens[i] as usize]
    }

    /// GPIO number of the `i`-th role.
    #[must_use]
    pub fn pin(&self, i: usize) -> u32 {
        self.pins[i]
    }

    /// Looks up a role name. Returns its position in the table when found.
    #[must_use]
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        (0..self.n).find(|&i| self.name(i) == name)
    }

    /// Looks up the GPIO number of a role name.
    #[must_use]
    pub fn get(&self, name: &[u8]) -> Option<u32> {
        self.find(name).map(|i| self.pins[i])
    }

    /// Adds one entry. The board constraints are checked here too.
    ///
    /// # Errors
    /// Invalid name, duplicate, over the limit, or a number the board cannot assign.
    pub fn insert(&mut self, name: &[u8], pin: u32, limits: &Limits<'_>) -> Result<(), RolesError> {
        if !valid_name(name) {
            return Err(RolesError::BadName);
        }
        if self.find(name).is_some() {
            return Err(RolesError::DuplicateName);
        }
        if pin >= limits.gpio_count {
            return Err(RolesError::OutOfRange);
        }
        if limits.reserved.contains(&pin) {
            return Err(RolesError::Reserved);
        }
        if limits.bus_pins.contains(&pin) {
            return Err(RolesError::BusPin);
        }
        if self.pins[..self.n].contains(&pin) {
            return Err(RolesError::DuplicatePin);
        }
        if self.n == MAX_ROLES {
            return Err(RolesError::TooMany);
        }
        self.names[self.n][..name.len()].copy_from_slice(name);
        self.lens[self.n] = name.len() as u8;
        self.pins[self.n] = pin;
        self.n += 1;
        Ok(())
    }
}

/// The display backend a board uses and its wiring (docs/device-layer.md §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayConfig {
    /// An ILI9341 on SPI bus `spi`, driven by the host
    /// (`@display=ili9341 spi=0 cs=10 dc=14 rst=15 rotation=90`). `rotation`
    /// is 0, 90, 180 or 270; 90 is the 320x240 landscape of the apps.
    Ili9341 {
        spi: u32,
        cs: u32,
        dc: u32,
        rst: u32,
        rotation: u32,
    },
    /// The M5Stack Tab5's own MIPI-DSI panel (`@display=tab5`).
    Tab5,
}

impl DisplayConfig {
    /// The driver's name, as `@display=` and `[board.<name>.display]` spell it.
    #[must_use]
    pub fn driver(&self) -> &'static str {
        match *self {
            DisplayConfig::Ili9341 { .. } => "ili9341",
            DisplayConfig::Tab5 => "tab5",
        }
    }

    /// GPIOs the backend drives. The guest may not open them and no role may
    /// name them (docs/TODO.md §6-6 for the Tab5).
    #[must_use]
    pub fn pins(&self) -> [u32; 3] {
        match *self {
            DisplayConfig::Ili9341 { cs, dc, rst, .. } => [cs, dc, rst],
            // Backlight, and the internal I2C that powers the panel.
            DisplayConfig::Tab5 => [22, 31, 32],
        }
    }

    /// The SPI bus the backend holds, if any.
    #[must_use]
    pub fn spi(&self) -> Option<u32> {
        match *self {
            DisplayConfig::Ili9341 { spi, .. } => Some(spi),
            DisplayConfig::Tab5 => None,
        }
    }

    /// Checks the entry against the board and against the roles.
    ///
    /// # Errors
    /// A pin out of range, reserved, a bus pin, used twice, or shared with a
    /// role; an SPI index or rotation out of range.
    pub fn check(&self, limits: &Limits<'_>, roles: &RoleMap) -> Result<(), RolesError> {
        if let DisplayConfig::Ili9341 { spi, rotation, .. } = *self
            && (spi as usize >= crate::MAX_SPI || !matches!(rotation, 0 | 90 | 180 | 270))
        {
            return Err(RolesError::BadDisplay);
        }
        let pins = self.pins();
        for (i, &pin) in pins.iter().enumerate() {
            if pin >= limits.gpio_count {
                return Err(RolesError::OutOfRange);
            }
            if limits.reserved.contains(&pin) {
                return Err(RolesError::Reserved);
            }
            if limits.bus_pins.contains(&pin) {
                return Err(RolesError::BusPin);
            }
            if pins[..i].contains(&pin) || (0..roles.len()).any(|r| roles.pin(r) == pin) {
                return Err(RolesError::DuplicatePin);
            }
        }
        Ok(())
    }

    /// Parses what follows `@display=`.
    fn parse(text: &[u8]) -> Result<DisplayConfig, RolesError> {
        let mut words = text.split(|b| *b == b' ').filter(|w| !w.is_empty());
        match words.next() {
            Some(b"tab5") => match words.next() {
                None => Ok(DisplayConfig::Tab5),
                Some(_) => Err(RolesError::BadDisplay),
            },
            Some(b"ili9341") => {
                // spi, cs, dc, rst, rotation; each exactly once.
                const KEYS: [&[u8]; 5] = [b"spi", b"cs", b"dc", b"rst", b"rotation"];
                let mut v: [Option<u32>; 5] = [None; 5];
                for w in words {
                    let eq = w
                        .iter()
                        .position(|b| *b == b'=')
                        .ok_or(RolesError::BadDisplay)?;
                    let k = KEYS
                        .iter()
                        .position(|k| *k == &w[..eq])
                        .ok_or(RolesError::BadDisplay)?;
                    if v[k].is_some() {
                        return Err(RolesError::BadDisplay);
                    }
                    v[k] = Some(parse_u32(&w[eq + 1..])?);
                }
                let [Some(spi), Some(cs), Some(dc), Some(rst), Some(rotation)] = v else {
                    return Err(RolesError::BadDisplay);
                };
                Ok(DisplayConfig::Ili9341 {
                    spi,
                    cs,
                    dc,
                    rst,
                    rotation,
                })
            }
            _ => Err(RolesError::BadDisplay),
        }
    }

    /// Writes what follows `@display=`.
    fn encode(&self, out: &mut Buf<'_>) {
        match *self {
            DisplayConfig::Tab5 => out.str("tab5"),
            DisplayConfig::Ili9341 {
                spi,
                cs,
                dc,
                rst,
                rotation,
            } => {
                out.str("ili9341");
                for (k, v) in [
                    (" spi=", spi),
                    (" cs=", cs),
                    (" dc=", dc),
                    (" rst=", rst),
                    (" rotation=", rotation),
                ] {
                    out.str(k);
                    out.u32(v);
                }
            }
        }
    }
}

/// What the config slot holds: the role table and the display entry.
#[derive(Clone, Copy)]
pub struct Wiring {
    pub roles: RoleMap,
    pub display: Option<DisplayConfig>,
}

impl Wiring {
    /// No roles and no display. The firmware runs with this when the config
    /// slot is empty or corrupt.
    pub const EMPTY: Wiring = Wiring {
        roles: RoleMap::EMPTY,
        display: None,
    };

    /// The format version the writer uses: 2 only with a display entry.
    #[must_use]
    pub fn version(&self) -> u16 {
        if self.display.is_some() { 2 } else { 1 }
    }
}

/// Parses the body (lines of `name=gpio`, and in version 2 one
/// `@display=` line).
///
/// # Errors
/// A malformed line, or whatever `insert` / `DisplayConfig::check` reject.
/// One bad entry discards the whole table (the caller runs with
/// `Wiring::EMPTY`).
pub fn parse_body(body: &[u8], version: u16, limits: &Limits<'_>) -> Result<Wiring, RolesError> {
    let mut w = Wiring::EMPTY;
    for line in body.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(DISPLAY_KEY) {
            if version < 2 || w.display.is_some() {
                return Err(RolesError::BadDisplay);
            }
            w.display = Some(DisplayConfig::parse(rest)?);
            continue;
        }
        let Some(eq) = line.iter().position(|b| *b == b'=') else {
            return Err(RolesError::BadLine);
        };
        let (name, digits) = (&line[..eq], &line[eq + 1..]);
        w.roles.insert(name, parse_u32(digits)?, limits)?;
    }
    // Checked once every role is known, so the order of the lines does not matter.
    if let Some(d) = &w.display {
        d.check(limits, &w.roles)?;
    }
    Ok(w)
}

/// Non-negative decimal integer. Rejects empty input, non-decimal characters
/// and u32 overflow.
fn parse_u32(digits: &[u8]) -> Result<u32, RolesError> {
    if digits.is_empty() {
        return Err(RolesError::BadNumber);
    }
    let mut v: u32 = 0;
    for &d in digits {
        if !d.is_ascii_digit() {
            return Err(RolesError::BadNumber);
        }
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add(u32::from(d - b'0')))
            .ok_or(RolesError::BadNumber)?;
    }
    Ok(v)
}

/// What the header says.
///
/// Config slot layout:
///
/// ```text
/// offset  size  contents
/// 0       4     magic "WMCR" (Wasmicon Roles)
/// 4       2     format version (u16 LE, = 1)
/// 6       2     reserved (0)
/// 8       4     body length (u32 LE)
/// 12      4     CRC-32 of the body (u32 LE)
/// 16      n     body: one entry per line, `name=gpio\n` (decimal)
/// ```
#[derive(Clone, Copy)]
pub struct Header {
    /// Format version (1 or 2).
    pub version: u16,
    /// Body length (already checked to be at most `MAX_BODY`).
    pub len: usize,
    /// CRC-32 of the body.
    pub crc: u32,
}

/// Reads only the header (boards without XIP read this first, then just the body).
///
/// # Errors
/// Empty, wrong magic, wrong version, or a body longer than `MAX_BODY`.
pub fn parse_header(head: &[u8]) -> Result<Header, RolesError> {
    let Some(head) = head.get(..HEADER_LEN) else {
        return Err(RolesError::Empty);
    };
    let magic = &head[..4];
    if magic != MAGIC {
        if magic.iter().all(|b| *b == 0xff) || magic.iter().all(|b| *b == 0) {
            return Err(RolesError::Empty);
        }
        return Err(RolesError::BadMagic);
    }
    let version = u16::from_le_bytes([head[4], head[5]]);
    if !(1..=FORMAT_VERSION).contains(&version) {
        return Err(RolesError::UnsupportedVersion);
    }
    let len = u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as usize;
    if len > MAX_BODY {
        return Err(RolesError::TooLong);
    }
    Ok(Header {
        version,
        len,
        crc: u32::from_le_bytes([head[12], head[13], head[14], head[15]]),
    })
}

/// Checks the body's CRC, then parses it.
///
/// # Errors
/// Wrong length or CRC, or whatever `parse_body` reports.
pub fn parse_checked(body: &[u8], h: &Header, limits: &Limits<'_>) -> Result<Wiring, RolesError> {
    if body.len() != h.len {
        return Err(RolesError::TooLong);
    }
    if crc32(body) != h.crc {
        return Err(RolesError::BadCrc);
    }
    parse_body(body, h.version, limits)
}

/// Parses a whole slot (header + body; the rest is ignored).
///
/// # Errors
/// Whatever `parse_header` / `parse_checked` report.
pub fn parse(bytes: &[u8], limits: &Limits<'_>) -> Result<Wiring, RolesError> {
    let h = parse_header(bytes)?;
    let Some(body) = bytes.get(HEADER_LEN..HEADER_LEN + h.len) else {
        return Err(RolesError::TooLong);
    };
    parse_checked(body, &h, limits)
}

/// Encodes the wiring as a body and returns the length written (for the
/// writer). The display line, if any, comes last.
///
/// `out` of `MAX_BODY` bytes or more is always enough.
#[must_use]
pub fn encode_body(w: &Wiring, out: &mut [u8]) -> usize {
    let n = encode_roles(&w.roles, out);
    let Some(d) = &w.display else { return n };
    let mut line = Buf::new(&mut out[n..]);
    line.bytes(DISPLAY_KEY);
    d.encode(&mut line);
    line.byte(b'\n');
    n + line.as_bytes().len()
}

fn encode_roles(map: &RoleMap, out: &mut [u8]) -> usize {
    let mut n = 0;
    let mut put = |b: u8, out: &mut [u8]| {
        if n < out.len() {
            out[n] = b;
            n += 1;
        }
    };
    for i in 0..map.len() {
        for &b in map.name(i) {
            put(b, out);
        }
        put(b'=', out);
        let mut digits = [0u8; 10];
        let mut d = 0;
        let mut v = map.pin(i);
        loop {
            digits[d] = b'0' + (v % 10) as u8;
            d += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while d > 0 {
            d -= 1;
            put(digits[d], out);
        }
        put(b'\n', out);
    }
    n
}

/// Builds the header (for the writer). `version` is `Wiring::version`.
#[must_use]
pub fn header(body: &[u8], version: u16) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(&MAGIC);
    h[4..6].copy_from_slice(&version.to_le_bytes());
    let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
    h[8..12].copy_from_slice(&len.to_le_bytes());
    h[12..16].copy_from_slice(&crc32(body).to_le_bytes());
    h
}

/// Reads the config slot from memory-mapped (XIP) flash (RP2040 / RP2350).
///
/// # Errors
/// Overlap with the firmware, or whatever `parse` reports.
///
/// # Safety
/// `slot.len` bytes starting at `xip_base + slot.offset` must be readable
/// memory-mapped flash that stays mapped for the rest of the program (the
/// contract of `crate::slot::xip_region`).
pub unsafe fn read_xip(
    xip_base: usize,
    slot: Slot,
    fw_end: usize,
    limits: &Limits<'_>,
) -> Result<Wiring, RolesError> {
    // SAFETY: this function's own contract guarantees that `slot.len` bytes
    // from `xip_base + slot.offset` are readable memory-mapped flash that stays
    // mapped for the rest of the program, which is exactly what `xip_region`
    // requires.
    let bytes = unsafe { crate::slot::xip_region(xip_base, slot, fw_end) }
        .map_err(|_| RolesError::Overlap)?;
    parse(bytes, limits)
}

/// Builds the one line printed to the serial port at boot.
///
/// `wasmicon: roles lcd-cs=17 lcd-dc=20` when the table was read (`roles none`
/// when empty), followed by ` display=ili9341 spi=0 ...` when there is a
/// display entry; `wasmicon: <reason>, no roles` otherwise. A banner line,
/// not a trace line (`>` / `<`), so `trace diff` does not compare it.
///
/// `out` of 256 bytes holds even `MAX_ROLES` entries and a display entry.
pub fn describe(result: &Result<Wiring, RolesError>, out: &mut Buf<'_>) {
    out.str("wasmicon: ");
    match result {
        Ok(w) => {
            let map = &w.roles;
            if map.is_empty() {
                out.str("roles none");
            } else {
                out.str("roles");
            }
            for i in 0..map.len() {
                out.byte(b' ');
                out.bytes(map.name(i));
                out.byte(b'=');
                out.u32(map.pin(i));
            }
            if let Some(d) = &w.display {
                out.str(" display=");
                d.encode(out);
            }
        }
        Err(e) => {
            out.str(e.reason());
            out.str(", no roles");
        }
    }
}

/// Builds a map from a `(name, number)` table (used by the host mock).
///
/// # Errors
/// Whatever `RoleMap::insert` rejects.
pub fn from_table(table: &[(&str, u32)], limits: &Limits<'_>) -> Result<RoleMap, RolesError> {
    let mut map = RoleMap::EMPTY;
    for &(name, pin) in table {
        map.insert(name.as_bytes(), pin, limits)?;
    }
    Ok(map)
}
