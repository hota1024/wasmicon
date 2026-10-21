//! `wasmicon new`: writes an app scaffold (docs/app-workflow.md §4.6, step 3).
//!
//! The build flags are only written out from `flags`. Holding the strings
//! here would make a third copy next to `apps/`, and a mismatch would pass
//! silently (docs/app-workflow.md §4.4).
//!
//! `wasmicon-hal` is not published to crates.io / npm (docs/TODO.md §5,
//! "distributing `new` and the bindings"), so the dependency is written as a
//! path. The path comes from `--hal`, or else from a search for `bindings/`
//! upward from the new directory. If neither finds it, a version requirement
//! is written and the user is told (the distribution decision is not
//! preempted here).
//!
//! `tests/new.rs` builds the scaffold with `cargo build --release` and runs
//! `wasmicon check` on all four boards. Matching `apps/` byte for byte is not
//! enough: a scaffold can match and still not work.

use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

use crate::{flags, pack};

/// Scaffold language. The values of `--lang` (docs/app-workflow.md §4.6).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    AssemblyScript,
}

impl Lang {
    /// Parses the value of `--lang`.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "rust" | "rs" => Ok(Lang::Rust),
            "as" | "assemblyscript" => Ok(Lang::AssemblyScript),
            other => bail!("unknown language `{other}` (rust or as)"),
        }
    }
}

pub struct Options {
    /// Where to create. The last component becomes the app name.
    pub dir: PathBuf,
    pub lang: Lang,
    /// Value written to `[defaults] board` in `wasmicon.toml`.
    pub board: Option<String>,
    /// Where the bindings are (the directory that contains `bindings/`).
    pub hal: Option<PathBuf>,
}

/// What was written.
pub struct Created {
    pub dir: PathBuf,
    pub name: String,
    pub files: Vec<PathBuf>,
    /// The bindings path, when one could be referenced. Otherwise a version
    /// requirement was written.
    pub hal: Option<PathBuf>,
    /// Created inside an existing cargo workspace (rustflags apply twice).
    pub inside_workspace: bool,
}

/// Writes the scaffold.
///
/// # Errors
/// When the name is unusable, the directory is not empty, or a file cannot
/// be written.
pub fn create(opts: &Options) -> Result<Created> {
    let name = app_name(&opts.dir)?;

    // The board name is rejected here. `manifest::load` does not validate
    // `[defaults] board` (it checks role name format but passes board names
    // through), so with a typo `new` would succeed and every later command
    // would fail.
    if let Some(b) = &opts.board {
        pack::resolve_board(b)?;
    }

    // Never write into a non-empty directory (like cargo new). Look first.
    if opts.dir.exists() {
        let mut entries = std::fs::read_dir(&opts.dir)
            .with_context(|| format!("cannot list `{}`", opts.dir.display()))?;
        if entries.next().is_some() {
            bail!("`{}` is not empty", opts.dir.display());
        }
    }
    std::fs::create_dir_all(&opts.dir)
        .with_context(|| format!("cannot create `{}`", opts.dir.display()))?;

    // Relative paths need the real directory (created above).
    let dir = opts
        .dir
        .canonicalize()
        .with_context(|| format!("cannot resolve `{}`", opts.dir.display()))?;

    let hal = match &opts.hal {
        Some(p) => {
            let p = p
                .canonicalize()
                .with_context(|| format!("--hal `{}` does not exist", p.display()))?;
            // Look inside. With canonicalize alone any directory is accepted,
            // `<given>/rust` is written to Cargo.toml, and nothing is noticed
            // until `cargo build` fails with "no Cargo.toml" (the search
            // branch applies the same condition, so only this branch was
            // lax).
            if !p.join("rust/Cargo.toml").is_file() {
                bail!(
                    "--hal `{}` has no bindings (looked for {}/rust/Cargo.toml); \
                     pass the bindings directory itself",
                    p.display(),
                    p.display()
                );
            }
            Some(p)
        }
        None => find_bindings(&dir),
    };
    let inside_workspace = enclosing_workspace(&dir).is_some();

    let mut files = Vec::new();
    let mut write = |rel: &str, body: &str| -> Result<()> {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create `{}`", parent.display()))?;
        }
        std::fs::write(&path, body)
            .with_context(|| format!("cannot write `{}`", path.display()))?;
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

/// Writes the scaffold and prints what to do next.
///
/// # Errors
/// When `create` fails.
pub fn run(opts: &Options) -> Result<bool> {
    let made = create(opts)?;

    // Shown as the path that was passed (`made.dir` is canonicalized and
    // absolute, which reads badly in a `cd` hint).
    let shown = opts.dir.display();
    println!("{shown}/");
    for f in &made.files {
        println!("  {}", f.display());
    }

    if made.hal.is_none() {
        // This must not stay silent. A version requirement cannot be
        // resolved yet, so a plain `cargo build` stops with "not found".
        println!();
        println!("note: no bindings found, so the dependency is written with a version");
        println!("  They are not published to crates.io / npm yet, so run again with");
        println!("  `--hal <wasmicon's bindings directory>`");
        println!("  (`--hal bindings` from inside the repository)");
    }
    if made.inside_workspace {
        // cargo concatenates the rustflags of nested .cargo/config.toml
        // files. The same values just apply twice, which is harmless, but
        // say so to avoid surprise.
        println!();
        println!("note: created inside an existing cargo workspace");
        println!("  rustflags are concatenated, so the same values apply twice (harmless)");
    }

    println!();
    println!("Next:");
    match opts.lang {
        Lang::Rust => println!("    cd {shown} && cargo build --release"),
        Lang::AssemblyScript => println!("    cd {shown} && npm install && npm run build"),
    }
    println!("    wasmicon check <the .wasm that was built> --board <board>");
    Ok(true)
}

/// Derives the app name from the directory name.
fn app_name(dir: &Path) -> Result<String> {
    let Some(name) = dir.file_name().and_then(|s| s.to_str()) else {
        bail!("cannot derive an app name from `{}`", dir.display());
    };
    // Limited to what cargo accepts as a package name. Rejecting here is
    // earlier than a `cargo build` failure.
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        bail!("the app name must start with a letter: `{name}`");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && *c != '-' && *c != '_')
    {
        bail!("the app name contains {bad:?}; only letters, digits, - and _ are allowed");
    }
    Ok(name.to_string())
}

/// Searches upward for a directory that contains `bindings/`.
fn find_bindings(from: &Path) -> Option<PathBuf> {
    for dir in from.ancestors() {
        if dir.join("bindings/rust/Cargo.toml").is_file() {
            return Some(dir.join("bindings"));
        }
    }
    None
}

/// Whether the directory is inside an existing cargo workspace.
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

/// The relative path from `from` to `to`.
///
/// `std` has none, so it is written here. Both arguments must be absolute
/// paths of existing directories (`canonicalize`d).
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

/// Formats a path for TOML / TypeScript (`/` separators).
///
/// The root is special: `Component::RootDir` is `/` as an `OsStr`, so a
/// naive `join("/")` produces `//Users/...` (it happened).
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

/// The path written into the scaffold: the shorter of relative and absolute.
///
/// A relative path survives moving the directory, but without a common
/// ancestor it becomes a long run of `../` (eight levels from `/private/tmp`
/// to `/Users` happened). An unreadable path does more harm, so length
/// decides.
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
        // No default board is chosen here (`deploy` then says "no target
        // board", so nothing is flashed to a guessed board).
        None => "# [defaults]\n# board = \"rp2350\"\n".to_string(),
    };
    format!(
        "\
# App manifest (docs/app-workflow.md §4.7).
#
# Describes what {name} needs and how each board is wired. The app name and
# language come from Cargo.toml / asconfig.json and are not repeated here.
version = 1

[requirements]
# Role names this app looks up through board.pin-by-role (free-form: lowercase
# letters, digits and -). GPIO numbers go in the wiring tables below, not here;
# check / deploy cross-check the two.
pin-roles = []

{defaults}
# Per-board wiring. deploy writes it to the config slot and the firmware
# answers pin-by-role from this table alone (it has no default table;
# docs/app-workflow.md §3.9).
# [board.rp2350.roles]
# led = 15
#
# [board.esp32s3.roles]
# led = 2

# Read-only data the app opens through assets.blob (asset name -> file,
# relative to this file). assets write puts it in the board's assets slot
# (docs/app-workflow.md §3.10); run serves the files on the host.
# [assets]
# \"font.psf\" = \"assets/font.psf\"
"
    )
}

fn cargo_toml(name: &str, hal: Option<&Path>, dir: &Path) -> String {
    let dep = match hal {
        Some(b) => format!(
            "wasmicon-hal = {{ path = \"{}\" }}",
            written_path(dir, &b.join("rust"))
        ),
        // Not published, so this cannot be resolved yet. `run` prints a note.
        None => "wasmicon-hal = \"0.1\"".to_string(),
    };
    format!(
        "\
# A standalone workspace. Without this, cargo stops with \"not a member of
# the workspace\" when the app sits inside another workspace.
[workspace]

[package]
name = \"{name}\"
version = \"0.1.0\"
edition = \"2024\"

[lib]
# cdylib exports the linear memory as \"memory\" (docs/abi-spec.md §3.3).
crate-type = [\"cdylib\"]

[dependencies]
{dep}

# Size is everything for a guest. Traps become abort.
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
//! {name}: a Wasmicon app.

#![no_std]

use wasmicon_hal::log;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
#[unsafe(no_mangle)]
pub extern \"C\" fn run() {{
    log::info(\"{name} start\");

    // GPIO numbers differ per board, so pins are looked up by role name
    // (docs/abi-spec.md §8). That is what lets the same .wasm run on every
    // board. Declare the roles in [requirements] pin-roles of wasmicon.toml
    // and the numbers in [board.<name>.roles] (the firmware has no default
    // table).
    //
    // use wasmicon_hal::gpio::{{Level, Pin, PinMode}};
    // use wasmicon_hal::board;
    //
    // let Ok(index) = board::pin_by_role(\"led\") else {{
    //     log::error(\"no pin assigned to role led\");
    //     return;
    // }};
    // let Ok(pin) = Pin::open(index, PinMode::Output) else {{
    //     log::error(\"cannot open led pin\");
    //     return;
    // }};
    // let _ = pin.write(Level::High);
}}

/// `no_std` needs its own panic handler. It traps, so the host logs the
/// reason and halts (docs/decisions.md A-04).
///
/// `cfg(not(test))` because `cargo clippy --all-targets` builds the test
/// harness, whose std panic handler would conflict with this one.
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
    // asc cannot resolve a scoped npm package as `~lib`, so the bindings are
    // referenced by relative path (same reason as in apps/*-as).
    let import = match hal {
        Some(b) => written_path(
            &dir.join("assembly"),
            &b.join("assemblyscript/assembly/index"),
        ),
        None => "@wasmicon/hal".to_string(),
    };
    format!(
        "\
// {name}: a Wasmicon app.

import {{ log }} from \"{import}\";

// Entry point. The host calls it once, right after instantiation
// (docs/abi-spec.md §3.3).
export function run(): void {{
  log.info(\"{name} start\");

  // GPIO numbers differ per board, so pins are looked up by role name
  // (docs/abi-spec.md §8). Declare the roles in [requirements] pin-roles of
  // wasmicon.toml and the numbers in [board.<name>.roles] (the firmware has
  // no default table).
  //
  // const index = board.pinByRole(\"led\");   // -1 when no pin is assigned
  // if (index < 0) {{ log.error(\"no pin assigned to role led\"); return; }}
  // const pin = Pin.open(index as u32, GpioPinMode.Output);  // null on failure
}}
"
    )
}
