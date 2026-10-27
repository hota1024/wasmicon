//! `wasmicon.toml`: the one configuration file per project (docs/app-workflow.md §4.7).
//!
//! It holds what the app needs and how each board is wired. `deploy` writes
//! the wiring table (`[board.<name>.roles]`) to the config slot, and the
//! firmware answers `pin-by-role` from that table alone (it has no default
//! table; docs/app-workflow.md §3.9).
//!
//! ```toml
//! version = 1
//!
//! [requirements]
//! # Role names this app looks up through `board.pin-by-role`. No numbers here
//! pin-roles = ["lcd-cs", "lcd-dc", "lcd-rst"]
//!
//! [defaults]
//! board = "rp2350"
//! i2c-replay = "fixtures/sht4x.txt"
//!
//! [board.rp2350.roles]   # the wiring on the desk; deploy writes it to the config slot
//! lcd-cs = 17
//! lcd-dc = 20
//! lcd-rst = 21
//!
//! [board.esp32s3.display] # the board's display backend (docs/device-layer.md §5.2)
//! driver = "ili9341"
//! cs = 10
//! dc = 14
//! rst = 15
//!
//! [assets]               # read-only data the app opens; assets write flashes it
//! "font.psf" = "assets/terminus-12.psf"
//! ```
//!
//! Schema rules (docs/app-workflow.md §4.7):
//!
//! - Nothing derivable is written. The app name and language come from
//!   `Cargo.toml` / `asconfig.json` (a second copy always drifts).
//! - One file per app, even inside a workspace.
//! - Requirements only. "Works without `led`" degradation is not expressed
//!   in v0.1.
//! - Role names are free-form; the firmware has no vocabulary. Up to 16
//!   characters of `a-z` / `0-9` / `-`, starting with a lowercase letter
//!   (`wasmicon_port::roles::valid_name`).
//! - "This board has no such role" is `"none"` (same as omitting the key:
//!   nothing is handed out).
//! - Asset names follow `wasmicon_port::assets::valid_name` (1 to 32 of
//!   `a-z` / `0-9` / `-` / `_` / `.`, starting with a letter or a digit); a
//!   name with a `.` is a quoted key. At most 8 assets. What the app can
//!   open on a device is exactly this table (`assets write` replaces the
//!   slot with it), so there is no separate declaration.
//! - Paths are relative to the directory of `wasmicon.toml`. Relative to the
//!   CLI's cwd they would break depending on where the CLI is run from.
//! - Unknown keys are errors. Ignoring them silently hides "I set it but it
//!   has no effect".
//! - A `version` newer than the CLI supports is an error; an older one is
//!   accepted.
//! - The file is optional. `check` / `run` work without it.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use wasmicon_port::assets;
use wasmicon_port::profile::{self, Profile};
use wasmicon_port::roles::{self, DisplayConfig, Limits, RoleMap, Wiring};

/// The `version` this CLI can read.
pub const SUPPORTED_VERSION: u32 = 1;

/// File name. Searched upward, like `cargo` does.
pub const FILE_NAME: &str = "wasmicon.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Raw {
    version: u32,
    #[serde(default)]
    requirements: RawRequirements,
    #[serde(default)]
    defaults: RawDefaults,
    /// `[board.<name>.roles]`.
    #[serde(default)]
    board: BTreeMap<String, RawBoard>,
    /// `[assets]`: asset name -> file.
    #[serde(default)]
    assets: BTreeMap<String, PathBuf>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawRequirements {
    /// Role names this app looks up through `pin-by-role`.
    #[serde(default)]
    pin_roles: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawDefaults {
    board: Option<String>,
    i2c_replay: Option<PathBuf>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawBoard {
    /// Role name -> GPIO number, or `"none"`.
    #[serde(default)]
    roles: BTreeMap<String, toml::Value>,
    /// `[board.<name>.display]`.
    display: Option<RawDisplay>,
}

/// `[board.<name>.display]` (docs/device-layer.md §5.2).
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawDisplay {
    /// `"ili9341"` or `"tab5"`.
    driver: String,
    /// SPI bus index; default 0.
    spi: Option<u32>,
    cs: Option<u32>,
    dc: Option<u32>,
    rst: Option<u32>,
    /// 0, 90, 180 or 270; default 90 (320x240 landscape, as the apps draw).
    rotation: Option<u32>,
}

/// A loaded `wasmicon.toml`.
#[derive(Debug)]
pub struct Manifest {
    /// Directory of the file. Relative paths resolve against it.
    pub dir: PathBuf,
    /// Role names this app looks up (a declaration; check / deploy cross-check
    /// them against the wiring table).
    pub pin_roles: Vec<String>,
    /// Board used when `--board` is absent.
    pub default_board: Option<String>,
    /// File used when `--i2c-replay` is absent (already resolved against
    /// `dir`).
    pub default_i2c_replay: Option<PathBuf>,
    /// Per-board wiring tables (`deploy` writes them to the config slot;
    /// docs/app-workflow.md §3.9). Checked against the board's constraints
    /// (range, reserved pins, bus pins, duplicates) at load time.
    pub board_roles: BTreeMap<String, BTreeMap<String, Option<u32>>>,
    /// Per-board display entries, written to the config slot with the roles
    /// (docs/device-layer.md §5.2). Checked against the board and its roles at
    /// load time.
    pub board_display: BTreeMap<String, DisplayConfig>,
    /// Asset name -> file (already resolved against `dir`): what `run` serves
    /// to the guest and `assets write` puts in the board's assets slot
    /// (docs/app-workflow.md §3.10). Names are checked at load time; the
    /// files are checked by the commands that read them. The map's order
    /// (by name) is the order of the slot's directory.
    pub assets: BTreeMap<String, PathBuf>,
}

impl Manifest {
    /// The board's wiring table in the form written to the config slot.
    ///
    /// A firmware variant reads its board's table (`Profile::wiring`;
    /// `esp32s3-psram` reads `[board.esp32s3.roles]`). An empty table (no
    /// roles handed out) when the board has none.
    ///
    /// # Errors
    /// When the table violates the board's constraints (already checked by
    /// `load`, so in practice only after a profile change).
    pub fn role_map(&self, board: &'static Profile) -> Result<RoleMap> {
        match self.board_roles.get(board.wiring) {
            Some(table) => build_map(table, board),
            None => Ok(RoleMap::EMPTY),
        }
    }

    /// The board's display entry, if `[board.<wiring>.display]` exists.
    #[must_use]
    pub fn display(&self, board: &'static Profile) -> Option<DisplayConfig> {
        self.board_display.get(board.wiring).copied()
    }

    /// What `deploy` writes to the board's config slot: the roles and the
    /// display entry.
    ///
    /// # Errors
    /// As `role_map`.
    pub fn wiring(&self, board: &'static Profile) -> Result<Wiring> {
        Ok(Wiring {
            roles: self.role_map(board)?,
            display: self.display(board),
        })
    }
}

/// Turns `[board.<name>.display]` into a `DisplayConfig` and checks it against
/// the board and its roles, as the firmware does at boot.
fn build_display(
    raw: &RawDisplay,
    board: &'static Profile,
    roles: &RoleMap,
) -> Result<DisplayConfig> {
    let at = format!("[board.{}.display]", board.wiring);
    let d = match raw.driver.as_str() {
        "ili9341" => {
            let pin = |v: Option<u32>, key: &str| {
                v.with_context(|| format!("{at}: driver = \"ili9341\" needs `{key}`"))
            };
            DisplayConfig::Ili9341 {
                spi: raw.spi.unwrap_or(0),
                cs: pin(raw.cs, "cs")?,
                dc: pin(raw.dc, "dc")?,
                rst: pin(raw.rst, "rst")?,
                rotation: raw.rotation.unwrap_or(90),
            }
        }
        "tab5" => {
            // The Tab5 panel is the ESP32-P4 board's own; its pins are fixed.
            if board.name != profile::ESP32P4.name {
                bail!("{at}: driver = \"tab5\" is the M5Stack Tab5's panel (board esp32p4)");
            }
            if raw.spi.is_some()
                || raw.cs.is_some()
                || raw.dc.is_some()
                || raw.rst.is_some()
                || raw.rotation.is_some()
            {
                bail!("{at}: driver = \"tab5\" takes no other keys");
            }
            DisplayConfig::Tab5
        }
        other => bail!("{at}: unknown driver `{other}` (known: ili9341, tab5)"),
    };
    d.check(&Limits::of(board), roles)
        .map_err(|e| anyhow::anyhow!("{at}: {}", e.reason()))?;
    Ok(d)
}

/// Turns one `[board.<name>.roles]` table into a `RoleMap`. `"none"` entries
/// are not handed out.
fn build_map(table: &BTreeMap<String, Option<u32>>, board: &'static Profile) -> Result<RoleMap> {
    let limits = Limits::of(board);
    let mut map = RoleMap::EMPTY;
    for (role, pin) in table {
        if let Some(pin) = *pin {
            map.insert(role.as_bytes(), pin, &limits).map_err(|e| {
                anyhow::anyhow!(
                    "[board.{}.roles] {role} = {pin}: {}",
                    board.wiring,
                    e.reason()
                )
            })?;
        }
    }
    Ok(map)
}

/// Searches upward from `dir` for `wasmicon.toml`.
///
/// # Errors
/// When the file found cannot be read or is invalid. Not finding one is not
/// an error (the file is optional).
pub fn find(from: &Path) -> Result<Option<Manifest>> {
    let mut dir = Some(from);
    while let Some(d) = dir {
        let candidate = d.join(FILE_NAME);
        if candidate.is_file() {
            return Ok(Some(load(&candidate)?));
        }
        dir = d.parent();
    }
    Ok(None)
}

/// Loads one file.
///
/// # Errors
/// When the file cannot be read, is not valid TOML, has unknown keys, has a
/// newer `version`, has a malformed role name, or has a wiring table that
/// violates the board's constraints.
pub fn load(path: &Path) -> Result<Manifest> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read `{}`", path.display()))?;
    let raw: Raw = toml::from_str(&text).with_context(|| {
        format!(
            "cannot read `{}` (unknown key or wrong type)",
            path.display()
        )
    })?;

    if raw.version > SUPPORTED_VERSION {
        bail!(
            "`{}` has version = {}, newer than this wasmicon reads (up to {})",
            path.display(),
            raw.version,
            SUPPORTED_VERSION
        );
    }

    let dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    // Role names are not matched against a vocabulary (the firmware has
    // none); only the format is checked. A typo surfaces as "declaration
    // (pin-roles) and wiring table disagree", which check / deploy stop
    // without a device attached.
    let mut pin_roles = Vec::new();
    for name in &raw.requirements.pin_roles {
        valid_role(name)
            .with_context(|| format!("`{}`: invalid [requirements] pin-roles", path.display()))?;
        pin_roles.push(name.clone());
    }

    let mut board_roles = BTreeMap::new();
    let mut board_display = BTreeMap::new();
    for (board, b) in raw.board {
        let Some(p) = profile::by_name(&board) else {
            bail!(
                "`{}`: [board.{board}] names an unknown board",
                path.display()
            );
        };
        // A variant has no table of its own. Accepting one would leave two
        // tables for one circuit board, and only one of them would be used.
        if p.wiring != p.name {
            bail!(
                "`{}`: [board.{board}] is a firmware variant of {}; write its wiring in \
                 [board.{}.roles]",
                path.display(),
                p.wiring,
                p.wiring
            );
        }
        let mut map = BTreeMap::new();
        for (role, value) in b.roles {
            valid_role(&role)
                .with_context(|| format!("`{}`: invalid [board.{board}.roles]", path.display()))?;
            let pin = role_number(&value, &role, board.as_str())?;
            map.insert(role, pin);
        }
        // The board's constraints (range, reserved pins, bus pins,
        // duplicates) are checked here. The firmware repeats the check at
        // boot, but stopping before flashing is faster.
        let role_map = build_map(&map, p)
            .with_context(|| format!("`{}`: invalid wiring table", path.display()))?;
        if let Some(raw) = &b.display {
            let d = build_display(raw, p, &role_map)
                .with_context(|| format!("`{}`: invalid display entry", path.display()))?;
            board_display.insert(board.clone(), d);
        }
        board_roles.insert(board, map);
    }

    // Names are checked here, files when a command reads them: `check`
    // reports a missing file, `run` and `assets pack` stop on it.
    if raw.assets.len() > assets::MAX_ENTRIES {
        bail!(
            "`{}`: [assets] has {} entries, more than the {} the slot holds",
            path.display(),
            raw.assets.len(),
            assets::MAX_ENTRIES
        );
    }
    let mut assets_table = BTreeMap::new();
    for (name, file) in raw.assets {
        if !assets::valid_name(name.as_bytes()) {
            bail!(
                "`{}`: `{name}` is not a valid asset name (1 to {} of a-z / 0-9 / - / _ / ., \
                 starting with a letter or a digit)",
                path.display(),
                assets::MAX_NAME
            );
        }
        assets_table.insert(name, dir.join(file));
    }

    Ok(Manifest {
        dir: dir.clone(),
        pin_roles,
        default_board: raw.defaults.board,
        default_i2c_replay: raw.defaults.i2c_replay.map(|p| dir.join(p)),
        board_roles,
        board_display,
        assets: assets_table,
    })
}

/// Role name format (`wasmicon_port::roles::valid_name`).
fn valid_role(name: &str) -> Result<()> {
    if roles::valid_name(name.as_bytes()) {
        Ok(())
    } else {
        bail!(
            "`{name}` is not a valid role name (a lowercase letter first, then up to {} of \
             a-z / 0-9 / -)",
            roles::MAX_NAME
        )
    }
}

/// A GPIO number, or `"none"` (this board has no such role).
fn role_number(value: &toml::Value, role: &str, board: &str) -> Result<Option<u32>> {
    match value {
        toml::Value::Integer(n) => u32::try_from(*n).map(Some).map_err(|_| {
            anyhow::anyhow!("[board.{board}.roles] {role} = {n} is not a GPIO number")
        }),
        toml::Value::String(s) if s == "none" => Ok(None),
        other => {
            bail!("[board.{board}.roles] {role} must be a GPIO number or \"none\" (got {other})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a `wasmicon.toml` and loads it.
    ///
    /// One directory per test. Tests run in parallel in one process, so a
    /// shared directory would have them overwrite each other's files. The
    /// process ID is included too (a fixed name collides with another
    /// process when several worktrees run tests side by side).
    fn parse(name: &str, text: &str) -> Result<Manifest> {
        let dir = std::env::temp_dir()
            .join(format!("wasmicon-manifest-{}", std::process::id()))
            .join(name);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        let p = dir.join(FILE_NAME);
        std::fs::write(&p, text).expect("cannot write");
        load(&p)
    }

    #[test]
    fn reads_the_declared_roles() {
        let m = parse(
            "roles",
            r#"
version = 1
[requirements]
pin-roles = ["lcd-cs", "lcd-dc"]
"#,
        )
        .expect("loads");
        assert_eq!(m.pin_roles, ["lcd-cs", "lcd-dc"]);
        assert!(m.default_board.is_none());
    }

    #[test]
    fn a_malformed_role_name_is_rejected_offline() {
        // There is no vocabulary, but the format is checked (so a name cannot
        // collide with `=` or a newline in the body, or `role:` in traces).
        let e = parse(
            "bad-role",
            r#"
version = 1
[requirements]
pin-roles = ["LCD_CS"]
"#,
        )
        .expect_err("rejected");
        assert!(format!("{e:#}").contains("LCD_CS"), "{e:#}");
    }

    #[test]
    fn any_well_formed_role_name_is_accepted() {
        // The firmware has no vocabulary, so a name like `status-led` works.
        let m = parse(
            "free-role",
            r#"
version = 1
[requirements]
pin-roles = ["status-led"]
[board.esp32s3.roles]
status-led = 2
"#,
        )
        .expect("loads");
        let map = m.role_map(&profile::ESP32S3).expect("builds");
        assert_eq!(map.get(b"status-led"), Some(2));
    }

    #[test]
    fn the_wiring_is_checked_against_the_board_offline() {
        // The trace UART (GP0), an I2C pin (GP4), the same number assigned
        // twice, a board that does not exist. All stop before flashing.
        for (name, text, needle) in [
            ("reserved", "[board.rp2350.roles]\nx = 0\n", "reserved"),
            ("bus", "[board.rp2350.roles]\nx = 4\n", "spi / i2c"),
            ("dup", "[board.rp2350.roles]\na = 15\nb = 15\n", "share"),
            ("range", "[board.rp2350.roles]\nx = 30\n", "out of range"),
            ("board", "[board.nope.roles]\nx = 2\n", "unknown board"),
        ] {
            let e = parse(name, &format!("version = 1\n{text}")).expect_err(name);
            assert!(format!("{e:#}").contains(needle), "{name}: {e:#}");
        }
    }

    #[test]
    fn unknown_keys_are_rejected() {
        // Ignoring them silently hides "I set it but it has no effect".
        let e = parse(
            "unknown-key",
            r#"
version = 1
[requirements]
pin_roles = ["led"]
"#,
        )
        .expect_err("keys are kebab-case only");
        assert!(format!("{e:#}").contains("cannot read"), "{e:#}");
    }

    #[test]
    fn a_newer_version_is_rejected() {
        let e = parse("version", "version = 2\n").expect_err("unreadable version");
        assert!(format!("{e:#}").contains("newer"), "{e:#}");
    }

    #[test]
    fn paths_are_relative_to_the_manifest() {
        let m = parse(
            "paths",
            r#"
version = 1
[defaults]
board = "rp2350"
i2c-replay = "fixtures/sht4x.txt"
"#,
        )
        .expect("loads");
        assert_eq!(m.default_board.as_deref(), Some("rp2350"));
        let p = m.default_i2c_replay.expect("present");
        assert!(p.is_absolute() || p.starts_with(&m.dir), "{p:?}");
        assert!(p.ends_with("fixtures/sht4x.txt"));
    }

    #[test]
    fn a_role_can_be_declared_absent() {
        let m = parse(
            "absent",
            r#"
version = 1
[board.rp2350.roles]
lcd-cs = 22
lcd-rst = "none"
"#,
        )
        .expect("loads");
        let b = &m.board_roles["rp2350"];
        assert_eq!(b["lcd-cs"], Some(22));
        assert_eq!(b["lcd-rst"], None);
    }

    #[test]
    fn a_role_number_must_be_a_number_or_none() {
        let e = parse(
            "number",
            r#"
version = 1
[board.rp2350.roles]
lcd-cs = "GP22"
"#,
        )
        .expect_err("rejected");
        assert!(format!("{e:#}").contains("none"), "{e:#}");
    }

    #[test]
    fn reads_the_assets_table() {
        let m = parse(
            "assets",
            r#"
version = 1
[assets]
"doom1.wad" = "wad/doom1.wad"
"font.psf" = "assets/terminus-12.psf"
"#,
        )
        .expect("loads");
        let names: Vec<&String> = m.assets.keys().collect();
        assert_eq!(names, ["doom1.wad", "font.psf"], "in slot order");
        for (name, path) in &m.assets {
            assert!(
                path.starts_with(&m.dir),
                "{name}: {path:?} is not under the manifest"
            );
        }
        assert!(m.assets["doom1.wad"].ends_with("wad/doom1.wad"));
    }

    #[test]
    fn an_invalid_asset_name_is_rejected_offline() {
        // The firmware would refuse the directory; stop before writing it.
        for (name, text, needle) in [
            ("upper", "\"Doom1.WAD\" = \"a\"", "Doom1.WAD"),
            ("slash", "\"wad/doom1.wad\" = \"a\"", "wad/doom1.wad"),
            ("leading-dot", "\".hidden\" = \"a\"", ".hidden"),
        ] {
            let e = parse(name, &format!("version = 1\n[assets]\n{text}\n")).expect_err(name);
            assert!(format!("{e:#}").contains(needle), "{name}: {e:#}");
        }
    }

    #[test]
    fn at_most_eight_assets() {
        let mut text = String::from("version = 1\n[assets]\n");
        for i in 0..9 {
            text.push_str(&format!("a{i} = \"a{i}\"\n"));
        }
        let e = parse("nine-assets", &text).expect_err("nine do not fit");
        assert!(format!("{e:#}").contains("more than the 8"), "{e:#}");
    }

    #[test]
    fn a_firmware_variant_uses_its_boards_wiring() {
        // `esp32s3-psram` runs on the same circuit board as `esp32s3`, so one
        // table serves both (docs/app-workflow.md §4.7).
        let m = parse(
            "variant-wiring",
            "version = 1\n[board.esp32s3.roles]\nled = 2\nbuzzer = \"none\"\n",
        )
        .expect("loads");
        let plain = m.role_map(&profile::ESP32S3).expect("valid");
        let psram = m.role_map(&profile::ESP32S3_PSRAM).expect("valid");
        assert_eq!(
            crate::pack::describe_roles(&plain).as_deref(),
            Some("led=2")
        );
        assert_eq!(
            crate::pack::describe_roles(&psram),
            crate::pack::describe_roles(&plain)
        );
    }

    #[test]
    fn a_table_for_a_firmware_variant_is_rejected() {
        // It would never be read; say where the wiring goes instead.
        let e = parse(
            "variant-table",
            "version = 1\n[board.esp32s3-psram.roles]\nled = 2\n",
        )
        .expect_err("a variant has no table of its own");
        let msg = format!("{e:#}");
        assert!(msg.contains("[board.esp32s3.roles]"), "{msg}");
    }

    #[test]
    fn reads_a_display_entry_with_defaults() {
        let m = parse(
            "display-ok",
            r#"
version = 1
[board.esp32s3.display]
driver = "ili9341"
cs = 10
dc = 14
rst = 15
[board.esp32p4.display]
driver = "tab5"
"#,
        )
        .expect("loads");
        assert_eq!(
            m.display(&profile::ESP32S3),
            Some(DisplayConfig::Ili9341 {
                spi: 0,
                cs: 10,
                dc: 14,
                rst: 15,
                rotation: 90
            })
        );
        // The PSRAM firmware reads its board's entry.
        assert_eq!(
            m.display(&profile::ESP32S3_PSRAM),
            m.display(&profile::ESP32S3)
        );
        assert_eq!(m.display(&profile::ESP32P4), Some(DisplayConfig::Tab5));
        assert_eq!(m.display(&profile::RP2350), None);
        let w = m.wiring(&profile::ESP32P4).expect("wiring");
        assert_eq!(w.version(), 2);
    }

    #[test]
    fn rejects_a_bad_display_entry() {
        for (tag, body, want) in [
            (
                "no-cs",
                "driver = \"ili9341\"\ndc = 14\nrst = 15",
                "needs `cs`",
            ),
            ("driver", "driver = \"st7789\"", "unknown driver"),
            (
                "unknown-key",
                "driver = \"ili9341\"\ncs = 10\ndc = 14\nrst = 15\nmosi = 11",
                "unknown key",
            ),
            (
                "bus-pin",
                "driver = \"ili9341\"\ncs = 11\ndc = 14\nrst = 15",
                "spi / i2c",
            ),
            (
                "rotation",
                "driver = \"ili9341\"\ncs = 10\ndc = 14\nrst = 15\nrotation = 45",
                "malformed",
            ),
        ] {
            let text = format!("version = 1\n[board.esp32s3.display]\n{body}\n");
            let e = parse(&format!("display-{tag}"), &text).expect_err(tag);
            let msg = format!("{e:#}");
            assert!(msg.contains(want), "{tag}: {msg}");
        }
        let e = parse(
            "display-tab5-keys",
            "version = 1\n[board.esp32p4.display]\ndriver = \"tab5\"\ncs = 1\n",
        )
        .expect_err("tab5 with a pin");
        assert!(format!("{e:#}").contains("takes no other keys"));
        // The Tab5 panel exists only on the ESP32-P4.
        let e = parse(
            "display-tab5-s3",
            "version = 1\n[board.esp32s3.display]\ndriver = \"tab5\"\n",
        )
        .expect_err("tab5 on esp32s3");
        assert!(format!("{e:#}").contains("board esp32p4"));
        // A role on a display pin.
        let e = parse(
            "display-role",
            "version = 1\n[board.esp32s3.roles]\nled = 14\n\
             [board.esp32s3.display]\ndriver = \"ili9341\"\ncs = 10\ndc = 14\nrst = 15\n",
        )
        .expect_err("shared pin");
        assert!(format!("{e:#}").contains("two roles share one gpio"));
    }
}
