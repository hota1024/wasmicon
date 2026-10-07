//! `wasmicon.toml` — プロジェクトに 1 つ置く設定（`docs/app-workflow.md` §4.7）。
//!
//! 書くのは「アプリの性質」と「配線」。**配線表（`[board.<name>.roles]`）は
//! `deploy` が設定スロットに書き、ファームはその表だけで `pin-by-role` に
//! 答える**（§3.9。ファームは既定の表を持たない）。
//!
//! ```toml
//! version = 1
//!
//! [requirements]
//! # このアプリが `board.pin-by-role` で引く役割名。番号は書かない
//! pin-roles = ["lcd-cs", "lcd-dc", "lcd-rst"]
//!
//! [defaults]
//! board = "rp2350"
//! i2c-replay = "fixtures/sht4x.txt"
//!
//! [board.rp2350.roles]   # 机の上の配線。deploy が設定スロットに書く
//! lcd-cs = 17
//! lcd-dc = 20
//! lcd-rst = 21
//! ```
//!
//! 決めたこと（§4.7「スキーマの規則」）:
//!
//! - **導出できるものは書かない。** アプリ名と言語は `Cargo.toml` /
//!   `asconfig.json` から取る（二重に持つと必ず drift する）
//! - **1 アプリに 1 つ。** workspace でもアプリごとに置く
//! - **必須のみ。** `led` が無くても動く degradation は v0.1 では表現しない
//! - **役割名は自由。** ファームに語彙は無い。英小文字で始まる 16 文字までの
//!   `a-z` / `0-9` / `-`（`wasmicon_port::roles::valid_name`）
//! - **「この役割は無い」は `"none"`**（キーを省略したのと同じで、配らない）
//! - **パスは `wasmicon.toml` のあるディレクトリ基準。** CLI の cwd 基準に
//!   すると、どこから呼んだかで壊れる
//! - **未知のキーはエラー。** 黙って無視すると「設定したのに効いていない」に
//!   気付けない
//! - **`version` が CLI の対応より新しければエラー**、古ければ受ける
//! - **toml は任意。** `build` / `check` / `run` は無くても動く

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use wasmicon_port::profile::{self, Profile};
use wasmicon_port::roles::{self, Limits, RoleMap};

/// この CLI が読める `version`。
pub const SUPPORTED_VERSION: u32 = 1;

/// ファイル名。`cargo` と同じく上に向かって探す。
pub const FILE_NAME: &str = "wasmicon.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Raw {
    version: u32,
    #[serde(default)]
    requirements: RawRequirements,
    #[serde(default)]
    defaults: RawDefaults,
    /// `[board.<name>.roles]`。
    #[serde(default)]
    board: BTreeMap<String, RawBoard>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawRequirements {
    /// このアプリが `pin-by-role` で引く役割名。
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
    /// 役割名 → GPIO 番号、または `"none"`。
    #[serde(default)]
    roles: BTreeMap<String, toml::Value>,
}

/// 読み込んだ `wasmicon.toml`。
#[derive(Debug)]
pub struct Manifest {
    /// 読んだファイルの場所。相対パスの基準。
    pub dir: PathBuf,
    /// このアプリが引く役割名（**宣言**。配線表にあることを check / deploy が見る）。
    pub pin_roles: Vec<String>,
    /// `--board` が無いときに使うボード。
    pub default_board: Option<String>,
    /// `--i2c-replay` が無いときに使うファイル（`dir` 基準で解決済み）。
    pub default_i2c_replay: Option<PathBuf>,
    /// ボードごとの配線表（`deploy` が設定スロットに書く。§3.9）。
    /// 読んだ時点でボードの制約（範囲・予約ピン・バスのピン・重複）を検査済み。
    pub board_roles: BTreeMap<String, BTreeMap<String, Option<u32>>>,
}

impl Manifest {
    /// そのボードの配線表を、設定スロットに書く形にする。
    ///
    /// 表が無ければ空の表（役割を 1 つも配らない）。
    ///
    /// # Errors
    /// 表がボードの制約に合わないとき（`load` で検査済みなので、実際には
    /// プロファイルを差し替えたときだけ起きる）。
    pub fn role_map(&self, board: &'static Profile) -> Result<RoleMap> {
        match self.board_roles.get(board.name) {
            Some(table) => build_map(table, board),
            None => Ok(RoleMap::EMPTY),
        }
    }
}

/// `[board.<name>.roles]` の 1 枚を `RoleMap` にする。`"none"` は配らない。
fn build_map(table: &BTreeMap<String, Option<u32>>, board: &'static Profile) -> Result<RoleMap> {
    let limits = Limits::of(board);
    let mut map = RoleMap::EMPTY;
    for (role, pin) in table {
        if let Some(pin) = *pin {
            map.insert(role.as_bytes(), pin, &limits).map_err(|e| {
                anyhow::anyhow!(
                    "[board.{}.roles] {role} = {pin}: {}",
                    board.name,
                    e.reason()
                )
            })?;
        }
    }
    Ok(map)
}

/// `dir` から上に向かって `wasmicon.toml` を探す。
///
/// # Errors
/// 見つかったファイルが読めない、または内容が不正なとき。
/// **見つからないのはエラーではない**（toml は任意）。
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

/// 1 ファイル読む。
///
/// # Errors
/// 読めない、TOML として壊れている、未知のキーがある、`version` が新しい、
/// 役割名の書式が違う、配線表がボードの制約に合わないとき。
pub fn load(path: &Path) -> Result<Manifest> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("{} を読めない", path.display()))?;
    let raw: Raw = toml::from_str(&text)
        .with_context(|| format!("{} を読めない（未知のキーや型違い）", path.display()))?;

    if raw.version > SUPPORTED_VERSION {
        bail!(
            "{} の version = {} はこの wasmicon より新しい（読めるのは {} まで）",
            path.display(),
            raw.version,
            SUPPORTED_VERSION
        );
    }

    let dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    // 役割名は語彙と突き合わせない（ファームに語彙は無い）。書式だけ見る。
    // **タイポは「宣言（pin-roles）と配線表の食い違い」として check / deploy が
    // 止める**（デバイスに繋がなくてよい）。
    let mut pin_roles = Vec::new();
    for name in &raw.requirements.pin_roles {
        valid_role(name)
            .with_context(|| format!("{}: [requirements] pin-roles が不正", path.display()))?;
        pin_roles.push(name.clone());
    }

    let mut board_roles = BTreeMap::new();
    for (board, b) in raw.board {
        let Some(p) = profile::by_name(&board) else {
            bail!("{}: [board.{board}] というボードは無い", path.display());
        };
        let mut map = BTreeMap::new();
        for (role, value) in b.roles {
            valid_role(&role)
                .with_context(|| format!("{}: [board.{board}.roles] が不正", path.display()))?;
            let pin = role_number(&value, &role, board.as_str())?;
            map.insert(role, pin);
        }
        // ボードの制約（範囲・予約ピン・バスのピン・重複）はここで見る。
        // ファームも起動時に同じ検査をするが、焼く前に止める方が早い。
        build_map(&map, p).with_context(|| format!("{}: 配線表が不正", path.display()))?;
        board_roles.insert(board, map);
    }

    Ok(Manifest {
        dir: dir.clone(),
        pin_roles,
        default_board: raw.defaults.board,
        default_i2c_replay: raw.defaults.i2c_replay.map(|p| dir.join(p)),
        board_roles,
    })
}

/// 役割名の書式（`wasmicon_port::roles::valid_name`）。
fn valid_role(name: &str) -> Result<()> {
    if roles::valid_name(name.as_bytes()) {
        Ok(())
    } else {
        bail!(
            "{name:?} は役割名に使えない（英小文字で始まる {} 文字までの a-z / 0-9 / -）",
            roles::MAX_NAME
        )
    }
}

/// 番号、または `"none"`（このボードにこの役割は無い）。
fn role_number(value: &toml::Value, role: &str, board: &str) -> Result<Option<u32>> {
    match value {
        toml::Value::Integer(n) => u32::try_from(*n)
            .map(Some)
            .map_err(|_| anyhow::anyhow!("[board.{board}.roles] {role} = {n} は GPIO 番号でない")),
        toml::Value::String(s) if s == "none" => Ok(None),
        other => bail!(
            "[board.{board}.roles] {role} には GPIO 番号か \"none\" を書く（{other} ではない）"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `wasmicon.toml` を書いて読む。
    ///
    /// **テストごとにディレクトリを分ける。** 同じプロセス内で並列に走るので、
    /// 1 つのディレクトリを共有すると互いのファイルを書き合う。プロセス ID も
    /// 入れる（固定名だと worktree を並べたとき別プロセスとも衝突する）。
    fn parse(name: &str, text: &str) -> Result<Manifest> {
        let dir = std::env::temp_dir()
            .join(format!("wasmicon-manifest-{}", std::process::id()))
            .join(name);
        std::fs::create_dir_all(&dir).expect("作れない");
        let p = dir.join(FILE_NAME);
        std::fs::write(&p, text).expect("書けない");
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
        .expect("読める");
        assert_eq!(m.pin_roles, ["lcd-cs", "lcd-dc"]);
        assert!(m.default_board.is_none());
    }

    #[test]
    fn a_malformed_role_name_is_rejected_offline() {
        // 語彙は無いが、書式は見る（本文の `=` や改行、トレースの `role:` と
        // 衝突させない）。
        let e = parse(
            "bad-role",
            r#"
version = 1
[requirements]
pin-roles = ["LCD_CS"]
"#,
        )
        .expect_err("弾く");
        assert!(format!("{e:#}").contains("LCD_CS"), "{e:#}");
    }

    #[test]
    fn any_well_formed_role_name_is_accepted() {
        // ファームに語彙は無いので、`status-led` のような名前も書ける。
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
        .expect("読める");
        let map = m.role_map(&profile::ESP32S3).expect("作れる");
        assert_eq!(map.get(b"status-led"), Some(2));
    }

    #[test]
    fn the_wiring_is_checked_against_the_board_offline() {
        // トレースの UART（GP0）、I2C のピン（GP4）、同じ番号の二重割り当て、
        // 存在しないボード。どれも焼く前に止まる。
        for (name, text, needle) in [
            ("reserved", "[board.rp2350.roles]\nx = 0\n", "reserved"),
            ("bus", "[board.rp2350.roles]\nx = 4\n", "spi / i2c"),
            ("dup", "[board.rp2350.roles]\na = 15\nb = 15\n", "share"),
            ("range", "[board.rp2350.roles]\nx = 30\n", "out of range"),
            ("board", "[board.nope.roles]\nx = 2\n", "ボードは無い"),
        ] {
            let e = parse(name, &format!("version = 1\n{text}")).expect_err(name);
            assert!(format!("{e:#}").contains(needle), "{name}: {e:#}");
        }
    }

    #[test]
    fn unknown_keys_are_rejected() {
        // 黙って無視すると「設定したのに効いていない」に気付けない。
        let e = parse(
            "unknown-key",
            r#"
version = 1
[requirements]
pin_roles = ["led"]
"#,
        )
        .expect_err("キーは kebab-case だけ");
        assert!(format!("{e:#}").contains("読めない"), "{e:#}");
    }

    #[test]
    fn a_newer_version_is_rejected() {
        let e = parse("version", "version = 2\n").expect_err("読めない版");
        assert!(format!("{e:#}").contains("新しい"), "{e:#}");
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
        .expect("読める");
        assert_eq!(m.default_board.as_deref(), Some("rp2350"));
        let p = m.default_i2c_replay.expect("ある");
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
        .expect("読める");
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
        .expect_err("弾く");
        assert!(format!("{e:#}").contains("none"), "{e:#}");
    }
}
