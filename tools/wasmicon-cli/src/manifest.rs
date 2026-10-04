//! `wasmicon.toml` — プロジェクトに 1 つ置く設定（`docs/app-workflow.md` §4.7）。
//!
//! **書くのは「アプリの性質」と「意図」だけ。** 真実がデバイス側にあるものは
//! 宣言にとどめる（§3.9）。
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
//! [board.rp2350.roles]   # 机の上の配線（= 意図）
//! lcd-cs = 22
//! ```
//!
//! 決めたこと（§4.7「スキーマの規則」）:
//!
//! - **導出できるものは書かない。** アプリ名と言語は `Cargo.toml` /
//!   `asconfig.json` から取る（二重に持つと必ず drift する）
//! - **1 アプリに 1 つ。** workspace でもアプリごとに置く
//! - **必須のみ。** `led` が無くても動く degradation は v0.1 では表現しない
//! - **「この役割は無い」は `"none"`。** キーを省略すればファームの既定どおり
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

use wasmicon_port::ROLE_NAMES;

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
    /// このアプリが引く役割名（**宣言**。これがあると照合が保証になる）。
    pub pin_roles: Vec<&'static str>,
    /// `--board` が無いときに使うボード。
    pub default_board: Option<String>,
    /// `--i2c-replay` が無いときに使うファイル（`dir` 基準で解決済み）。
    pub default_i2c_replay: Option<PathBuf>,
    /// 配線の**意図**（`config apply` が使う。§3.9。まだ未実装）。
    pub board_roles: BTreeMap<String, BTreeMap<&'static str, Option<u32>>>,
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
/// 役割名が語彙に無いとき。
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

    // 役割名は語彙と突き合わせる。**デバイスに繋がなくてもタイポが止まる**
    // （「このボードにあるか」はデバイスの申告が要るが、「そんな役割名は
    // 存在しない」はここで分かる。§4.3）。
    let mut pin_roles = Vec::new();
    for name in &raw.requirements.pin_roles {
        pin_roles.push(
            known_role(name)
                .with_context(|| format!("{}: [requirements] pin-roles が不正", path.display()))?,
        );
    }

    let mut board_roles = BTreeMap::new();
    for (board, b) in raw.board {
        let mut map = BTreeMap::new();
        for (role, value) in b.roles {
            let role = known_role(&role)
                .with_context(|| format!("{}: [board.{board}.roles] が不正", path.display()))?;
            map.insert(role, role_number(&value, role, board.as_str())?);
        }
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

/// 語彙（`ports/common` の `ROLE_NAMES`）に無い名前を弾く。
fn known_role(name: &str) -> Result<&'static str> {
    ROLE_NAMES
        .iter()
        .copied()
        .find(|r| *r == name)
        .with_context(|| {
            format!(
                "{name} という役割名は無い（あるのは {}）",
                ROLE_NAMES.join(" / ")
            )
        })
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
    fn an_unknown_role_name_is_rejected_offline() {
        // デバイスに繋がなくてもタイポが止まる（§4.3）。
        let e = parse(
            "unknown-role",
            r#"
version = 1
[requirements]
pin-roles = ["lcd-cd"]
"#,
        )
        .expect_err("弾く");
        assert!(format!("{e:#}").contains("lcd-cd"), "{e:#}");
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
