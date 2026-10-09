// ポートの build.rs が `include!` する。ビルドしたソースの `git describe` を
// `WASMICON_GIT` に埋め、ファームが起動時に名乗る（docs/app-workflow.md §3.8）。
//
// **dirty を落とさない。** 手元の未コミットの変更で焼いたファームが、
// コミット済みのものと同じ名前を名乗ると、検証の記録が曖昧になる。
// そのため、ファームに入るソース（ポート自身、ports/common、runtime、wit）が
// 変わったら build.rs を走らせ直す。それ以外（docs など）の変更では
// 走り直さないので、そこだけの dirty は次にソースが変わるまで反映されない。

fn emit_git_describe() {
    use std::process::Command;
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let run = |args: &[&str]| -> Option<String> {
        let o = Command::new("git").args(args).current_dir(&dir).output().ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    // git が無い・リポジトリの外でビルドしたときは unknown と名乗る（落とさない）。
    let describe = run(&["describe", "--always", "--dirty", "--abbrev=7"])
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=WASMICON_GIT={describe}");

    if let Some(git_dir) = run(&["rev-parse", "--absolute-git-dir"]) {
        for f in ["HEAD", "index", "packed-refs", "refs"] {
            println!("cargo:rerun-if-changed={git_dir}/{f}");
        }
    }
    for d in ["src", "../common/src", "../../runtime/src", "../../wit"] {
        println!("cargo:rerun-if-changed={dir}/{d}");
    }
}
