//! `wasmicon new` — アプリの雛形を出す（`docs/app-workflow.md` §4.6 の 3）。
//!
//! **ビルドフラグは `flags` から書き出すだけ**にしてある。ここで文字列を
//! 持つと `apps/` と 3 重化して、食い違っても静かに通る（§4.4）。
//!
//! `wasmicon-hal` は **crates.io / npm に出していない**（`docs/TODO.md` §5 の
//! 「`new` と bindings の配布」）。なので依存はパスで書く。パスは
//! `--hal` で渡すか、無ければ**新しいディレクトリから上に `bindings/` を
//! 探す**。見つからなければ版指定で書き、その旨を出す（配布の決定を
//! ここで先取りしない）。
//!
//! 作ったあと `cargo build --release` が通り `wasmicon check` が 4 ボード
//! すべてで通ることを `tests/new.rs` が実際にやって見る。**雛形は
//! 「`apps/` と一致している」だけでは足りない**（一致していても動かない形は
//! あり得る）。

use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

use crate::{flags, pack};

/// 雛形の言語。`docs/app-workflow.md` §4.6 の `--lang` の値。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    AssemblyScript,
}

impl Lang {
    /// `--lang` の値から。
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "rust" | "rs" => Ok(Lang::Rust),
            "as" | "assemblyscript" => Ok(Lang::AssemblyScript),
            other => bail!("知らない言語 {other}（あるのは rust / as）"),
        }
    }
}

pub struct Options {
    /// 作る場所。最後の要素がアプリ名になる。
    pub dir: PathBuf,
    pub lang: Lang,
    /// `wasmicon.toml` の `[defaults] board` に書く値。
    pub board: Option<String>,
    /// バインディングの置き場所（`bindings/` を含むディレクトリ）。
    pub hal: Option<PathBuf>,
}

/// 書いたもの。
pub struct Created {
    pub dir: PathBuf,
    pub name: String,
    pub files: Vec<PathBuf>,
    /// バインディングをパスで参照できたか。できなければ版指定を書いてある。
    pub hal: Option<PathBuf>,
    /// 既存の cargo workspace の中に作ったか（rustflags が二重に効く）。
    pub inside_workspace: bool,
}

/// 雛形を書く。
///
/// # Errors
/// 名前が使えない、置き場所が空でない、書けないとき。
pub fn create(opts: &Options) -> Result<Created> {
    let name = app_name(&opts.dir)?;

    // **ボード名はここで弾く。** `manifest::load` は `[defaults] board` を
    // 検証しない（役割名は語彙と突き合わせるが、ボード名は素通り）ので、
    // 打ち間違えると `new` は成功して**そのあと全部のコマンドが落ちる**。
    if let Some(b) = &opts.board {
        pack::resolve_board(b)?;
    }

    // **空でないところには書かない**（cargo new と同じ）。見てから書く。
    if opts.dir.exists() {
        let mut entries = std::fs::read_dir(&opts.dir)
            .with_context(|| format!("{} を見られない", opts.dir.display()))?;
        if entries.next().is_some() {
            bail!("{} は空でない", opts.dir.display());
        }
    }
    std::fs::create_dir_all(&opts.dir)
        .with_context(|| format!("{} を作れない", opts.dir.display()))?;

    // 相対パスを出すために実体が要る（上で作ってある）。
    let dir = opts
        .dir
        .canonicalize()
        .with_context(|| format!("{} の実体を取れない", opts.dir.display()))?;

    let hal = match &opts.hal {
        Some(p) => Some(
            p.canonicalize()
                .with_context(|| format!("--hal {} が無い", p.display()))?,
        ),
        None => find_bindings(&dir),
    };
    let inside_workspace = enclosing_workspace(&dir).is_some();

    let mut files = Vec::new();
    let mut write = |rel: &str, body: &str| -> Result<()> {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("{} を作れない", parent.display()))?;
        }
        std::fs::write(&path, body).with_context(|| format!("{} を書けない", path.display()))?;
        files.push(PathBuf::from(rel));
        Ok(())
    };

    write(
        "wasmicon.toml",
        &manifest_toml(&name, opts.board.as_deref()),
    )?;

    match opts.lang {
        Lang::Rust => {
            write("Cargo.toml", &cargo_toml(&name, hal.as_deref(), &dir))?;
            write(".cargo/config.toml", flags::CARGO_CONFIG)?;
            write("rust-toolchain.toml", RUST_TOOLCHAIN)?;
            write("src/lib.rs", &rust_lib(&name))?;
            write(".gitignore", "/target\n")?;
        }
        Lang::AssemblyScript => {
            let wasm = format!("build/{}.wasm", name.replace('-', "_"));
            write("asconfig.json", &flags::asconfig(&wasm))?;
            write("package.json", &package_json(&name))?;
            write("assembly/index.ts", &as_index(&name, hal.as_deref(), &dir))?;
            write(".gitignore", "/build\n/node_modules\n")?;
        }
    }

    Ok(Created {
        dir,
        name,
        files,
        hal,
        inside_workspace,
    })
}

/// 雛形を書いて、次にやることを出す。
///
/// # Errors
/// `create` が失敗したとき。
pub fn run(opts: &Options) -> Result<bool> {
    let made = create(opts)?;

    // **渡されたパスで出す**（`made.dir` は canonicalize 済みの絶対パスで、
    // そのまま `cd` の案内に使うと読みにくい）。
    let shown = opts.dir.display();
    println!("{shown}/");
    for f in &made.files {
        println!("  {}", f.display());
    }

    if made.hal.is_none() {
        // **ここは黙ってはいけない。** 版指定の依存は今は解決できないので、
        // そのまま `cargo build` すると「見つからない」で止まる。
        println!();
        println!("※ バインディングが見つからなかったので依存を版指定で書いた。");
        println!("  まだ crates.io / npm に出していないので、");
        println!("  `--hal <wasmicon を置いた場所>` を渡し直すこと");
    }
    if made.inside_workspace {
        // cargo は入れ子の .cargo/config.toml の rustflags を**連結**する。
        // 同じ値が 2 回並ぶだけなので害は無いが、驚かないように言う。
        println!();
        println!("※ 既存の cargo workspace の中に作った。");
        println!("  rustflags は連結されるので同じ値が 2 回効く（害は無い）");
    }

    println!();
    println!("次にやること:");
    match opts.lang {
        Lang::Rust => println!("    cd {shown} && cargo build --release"),
        Lang::AssemblyScript => println!("    cd {shown} && npm install && npm run build"),
    }
    println!("    wasmicon check <出力された .wasm> --board <ボード>");
    Ok(true)
}

/// ディレクトリ名からアプリ名を取る。
fn app_name(dir: &Path) -> Result<String> {
    let Some(name) = dir.file_name().and_then(|s| s.to_str()) else {
        bail!("{} からアプリ名を取れない", dir.display());
    };
    // cargo のパッケージ名として通る範囲に絞る。弾くのは、あとで
    // `cargo build` が落ちるより早く言えるから。
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        bail!("アプリ名は英字で始めること: {name}");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && *c != '-' && *c != '_')
    {
        bail!("アプリ名に使えない文字: {bad:?}（英数字と - と _ だけ）");
    }
    Ok(name.to_string())
}

/// `bindings/` を含むディレクトリを上に向かって探す。
fn find_bindings(from: &Path) -> Option<PathBuf> {
    for dir in from.ancestors() {
        if dir.join("bindings/rust/Cargo.toml").is_file() {
            return Some(dir.join("bindings"));
        }
    }
    None
}

/// 既存の cargo workspace の中かどうか。
fn enclosing_workspace(from: &Path) -> Option<PathBuf> {
    for dir in from.ancestors().skip(1) {
        let manifest = dir.join("Cargo.toml");
        if let Ok(body) = std::fs::read_to_string(&manifest)
            && body.contains("[workspace]")
        {
            return Some(manifest);
        }
    }
    None
}

/// `from` から `to` への相対パス。
///
/// `std` に無いので自前で持つ。**両方とも実体のある絶対パス**
/// （`canonicalize` 済み）であることを前提にする。
fn relative(from: &Path, to: &Path) -> PathBuf {
    let f: Vec<Component> = from.components().collect();
    let t: Vec<Component> = to.components().collect();
    let same = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in same..f.len() {
        out.push("..");
    }
    for c in &t[same..] {
        out.push(c.as_os_str());
    }
    out
}

/// パスを TOML / TypeScript に書ける形（`/` 区切り）にする。
///
/// **root を特別扱いする。** `Component::RootDir` の `as_os_str()` は `/`
/// なので、素朴に `join("/")` すると `//Users/...` になる（実際に出した）。
fn slashed(p: &Path) -> String {
    let mut out = String::new();
    for c in p.components() {
        if c == Component::RootDir {
            out.push('/');
            continue;
        }
        if !out.is_empty() && !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(&c.as_os_str().to_string_lossy());
    }
    out
}

/// 雛形に書き込むパス。**相対と絶対の短い方**を選ぶ。
///
/// 相対にしておけばディレクトリごと移動しても壊れないが、共通の祖先が
/// 無いと `../` が延々と並ぶ（`/private/tmp` から `/Users` へ 8 段、という
/// のを実際に出した）。読めない方が害が大きいので長さで決める。
fn written_path(from: &Path, to: &Path) -> String {
    let rel = slashed(&relative(from, to));
    let abs = slashed(to);
    if rel.len() <= abs.len() { rel } else { abs }
}

const RUST_TOOLCHAIN: &str = "\
[toolchain]
channel = \"stable\"
components = [\"rustfmt\", \"clippy\"]
targets = [\"wasm32-unknown-unknown\"]
";

fn manifest_toml(name: &str, board: Option<&str>) -> String {
    let defaults = match board {
        Some(b) => format!("[defaults]\nboard = \"{b}\"\n"),
        // 既定のボードを勝手に決めない（`deploy` が「どこに書くか決まらない」と
        // 言うので、分からないまま焼かれることはない）。
        None => "# [defaults]\n# board = \"rp2350\"\n".to_string(),
    };
    format!(
        "\
# {name} の設定（docs/app-workflow.md §4.7）。
#
# 書くのは「アプリの性質」と「意図」だけ。アプリ名と言語は Cargo.toml /
# asconfig.json から取るので、ここには書かない。
version = 1

[requirements]
# このアプリが board.pin-by-role で引く役割名。**番号は書かない。**
# 書くと check の役割名の照合が「参考」から「保証」に変わる。
pin-roles = []

{defaults}"
    )
}

fn cargo_toml(name: &str, hal: Option<&Path>, dir: &Path) -> String {
    let dep = match hal {
        Some(b) => format!(
            "wasmicon-hal = {{ path = \"{}\" }}",
            written_path(dir, &b.join("rust"))
        ),
        // 出していないので、これは今は解決できない。`run` が警告を出す。
        None => "wasmicon-hal = \"0.1\"".to_string(),
    };
    format!(
        "\
# 単体で立つ workspace にしてある。これが無いと、既存の workspace の
# 中に置いたときに cargo が「workspace に入っていない」で止まる。
[workspace]

[package]
name = \"{name}\"
version = \"0.1.0\"
edition = \"2024\"

[lib]
# cdylib にすると memory が \"memory\" として export される（abi-spec §3.3）。
crate-type = [\"cdylib\"]

[dependencies]
{dep}

# ゲストはサイズが全て。トラップは abort に落とす。
[profile.release]
opt-level = \"z\"
lto = \"fat\"
codegen-units = 1
panic = \"abort\"
strip = true
"
    )
}

fn rust_lib(name: &str) -> String {
    format!(
        "\
//! {name}。

#![no_std]

use wasmicon_hal::log;

/// エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。
#[unsafe(no_mangle)]
pub extern \"C\" fn run() {{
    log::info(\"{name} start\");

    // ピン番号はボードごとに違うので**役割名で引く**（abi-spec §8）。
    // 同じ .wasm がどのボードでも動くのはこれがあるから。
    // 使う役割は wasmicon.toml の [requirements] pin-roles に書くこと。
    //
    // use wasmicon_hal::gpio::{{Level, Pin, PinMode}};
    // use wasmicon_hal::board;
    //
    // let Ok(index) = board::pin_by_role(\"led\") else {{
    //     log::error(\"led の割り当てが無い\");
    //     return;
    // }};
    // let Ok(pin) = Pin::open(index, PinMode::Output) else {{
    //     log::error(\"led のピンを開けない\");
    //     return;
    // }};
    // let _ = pin.write(Level::High);
}}

/// `no_std` なので自前で用意する。トラップに落として、ホストに
/// ログを出させて停止させる（docs/handoff.md §3 #4）。
///
/// `cfg(not(test))` なのは、`cargo clippy --all-targets` がテストハーネスを
/// 組むときに std の panic handler と衝突するため。
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {{
    core::arch::wasm32::unreachable()
}}
"
    )
}

fn package_json(name: &str) -> String {
    format!(
        "\
{{
  \"name\": \"{name}\",
  \"version\": \"0.1.0\",
  \"private\": true,
  \"type\": \"module\",
  \"devDependencies\": {{
    \"assemblyscript\": \"^0.28.8\"
  }},
  \"scripts\": {{
    \"build\": \"asc assembly/index.ts --config asconfig.json --target release\"
  }}
}}
"
    )
}

fn as_index(name: &str, hal: Option<&Path>, dir: &Path) -> String {
    // asc はスコープ付き npm パッケージを ~lib として解決できないので
    // **相対パスで参照する**（apps/*-as と同じ理由）。
    let import = match hal {
        Some(b) => written_path(
            &dir.join("assembly"),
            &b.join("assemblyscript/assembly/index"),
        ),
        None => "@wasmicon/hal".to_string(),
    };
    format!(
        "\
// {name}。

import {{ log }} from \"{import}\";

// エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。
export function run(): void {{
  log.info(\"{name} start\");

  // ピン番号はボードごとに違うので**役割名で引く**（abi-spec §8）。
  // 使う役割は wasmicon.toml の [requirements] pin-roles に書くこと。
  //
  // const index = board.pinByRole(\"led\");   // 割り当てが無ければ -1
  // if (index < 0) {{ log.error(\"led の割り当てが無い\"); return; }}
  // const pin = Pin.open(index as u32, GpioPinMode.Output);  // 失敗したら null
}}
"
    )
}
