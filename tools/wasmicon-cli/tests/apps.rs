//! この repo の `apps/` が自分の `wasmicon.toml` を持っていることを見る
//! （ドッグフード。`docs/app-workflow.md` §4.7）。
//!
//! **ファームは役割の既定の表を持たない**（§3.9）ので、役割を引くアプリは
//! 配線表が無いと実機で `unsupported` になる。アプリを足したのに toml を
//! 置き忘れた、宣言と配線表が食い違った、をここで止める。
//! **0 件を黙って成功にしない**（`tests/flags.rs` と同じ理屈）。

use std::path::{Path, PathBuf};

use wasmicon_cli::{check, manifest};
use wasmicon_port::profile;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

/// `apps/` の直下で、Rust（`Cargo.toml`）か AssemblyScript（`asconfig.json`）の
/// アプリになっているディレクトリ。
fn app_dirs() -> Vec<PathBuf> {
    let apps = repo_root().join("apps");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&apps)
        .expect("apps/ を読めない")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("Cargo.toml").is_file() || p.join("asconfig.json").is_file())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn every_app_has_a_manifest_whose_roles_are_wired_on_both_boards() {
    let dirs = app_dirs();
    assert!(dirs.len() >= 5, "apps/ のアプリが見つからない: {dirs:?}");
    for dir in dirs {
        let path = dir.join(manifest::FILE_NAME);
        assert!(path.is_file(), "{} が無い", path.display());
        let m = manifest::load(&path).unwrap_or_else(|e| panic!("{e:#}"));
        assert!(
            !m.pin_roles.is_empty(),
            "{}: どのアプリも役割を引くので pin-roles を書く",
            path.display()
        );
        // 評価ボード（docs/handoff.md §0）では宣言した役割が全部配られる。
        for board in [&profile::RP2350, &profile::ESP32S3] {
            let provided = check::provided_roles(board, Some(&m));
            for role in &m.pin_roles {
                assert!(
                    provided.contains(role),
                    "{}: {role} が [board.{}.roles] に無い",
                    path.display(),
                    board.name
                );
            }
            // 設定スロットに書ける表になっている。
            let map = m.role_map(board).unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(map.len(), m.pin_roles.len(), "{}", path.display());
        }
    }
}
