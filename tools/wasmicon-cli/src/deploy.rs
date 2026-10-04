//! `wasmicon deploy` — アプリをボードのスロットに送る（`docs/app-workflow.md` §3.5）。
//!
//! **1 段は外のフラッシャに渡す。** USB の制御チャネル（2 段）が入るまでは
//! `picotool` を呼ぶので、Pico は BOOTSEL 押下が残る。2 段で消える。
//!
//! **送る前に `check` を通す。** そのボードで走らないものをスロットに書いても、
//! 実機で落ちるまで分からない（§4.3）。`check` が落ちたら焼かない。
//!
//! 判定と画像の用意（`prepare`）を、フラッシャの呼び出し（`run`）から
//! 分けてある。前者はテストから呼べる。

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

use wasmicon_port::profile::{Profile, Slot};

use crate::manifest::Manifest;
use crate::{check, monitor, pack};

pub struct Options {
    pub path: PathBuf,
    /// 送り先。`--board` か `wasmicon.toml` の `[defaults] board`。
    /// **省略できない**（どこに書くか決まらない）。
    pub board: Option<String>,
    /// `wasmicon.toml`。`[requirements] pin-roles` があれば照合に使う。
    pub manifest: Option<Manifest>,
    /// 焼いたあとリセットしない（既定はリセットして走らせる）。
    pub no_run: bool,
    /// 焼いてリセットしたあと、そのままトレースを取り込む。
    pub monitor: bool,
    /// `--monitor` のときの取り込み先（`None` なら標準出力だけ）。
    pub out: Option<PathBuf>,
    /// `--monitor` のときのシリアルの口（`None` なら `/dev/cu.usb*` から選ぶ）。
    pub port: Option<PathBuf>,
}

/// 焼く前に決まること。副作用は画像の書き出しだけ。
///
/// `Debug` は付けない（`Profile` / `Slot` がポート側で持っていない。
/// あちらは `core::fmt` を避けている）。テストは `match` で取り出す。
pub struct Plan {
    pub board: &'static Profile,
    pub slot: Slot,
    /// 書き出したスロット画像。
    pub image: PathBuf,
    /// `picotool` に渡すアドレス（**オフセットではない**）。
    pub addr: u64,
    pub wasm_len: usize,
    pub image_len: usize,
    pub crc: u32,
}

/// 検査して画像を用意する。
///
/// # Errors
/// ボードが決まらない、置き場所が未決、スロットに入らない、`check` が落ちた、
/// 読み書きができないとき。
pub fn prepare(opts: &Options) -> Result<Plan> {
    let Some(name) = opts
        .board
        .clone()
        .or_else(|| opts.manifest.as_ref().and_then(|m| m.default_board.clone()))
    else {
        bail!(
            "どのボードに送るか決まらない。--board を渡すか、\
             wasmicon.toml に [defaults] board を書くこと"
        );
    };
    let board = pack::resolve_board(&name)?;

    let wasm =
        std::fs::read(&opts.path).with_context(|| format!("{} を読めない", opts.path.display()))?;

    // **走らないものを焼かない。** 役割名は宣言があれば保証になる（§4.7）。
    let facts = check::facts(&wasm)?;
    let declared: Option<&[&'static str]> = opts
        .manifest
        .as_ref()
        .filter(|m| !m.pin_roles.is_empty())
        .map(|m| m.pin_roles.as_slice());
    if facts.failures() > 0 {
        bail!(
            "check が落ちた（全ボード共通で {} 件）。`wasmicon check {}` を見ること",
            facts.failures(),
            opts.path.display()
        );
    }
    let verdict = check::judge(&wasm, board, &facts, declared);
    if !verdict.is_ok() {
        bail!(
            "check が落ちた（{} で {} 件）。`wasmicon check {} --board {}` を見ること",
            board.name,
            verdict.fails(),
            opts.path.display(),
            board.name
        );
    }

    let image = pack::build(&wasm);
    let slot = pack::fits(board, image.len())?;

    // 画像はアプリの隣に置く（`pack` と同じ名前）。消さないのは、同じものを
    // もう一度焼いたり、デバイスのログと CRC を突き合わせたりするため。
    let mut out = opts.path.clone();
    out.set_extension("bin");
    std::fs::write(&out, &image).with_context(|| format!("{} を書けない", out.display()))?;

    Ok(Plan {
        board,
        slot,
        image: out,
        addr: pack::XIP_BASE + u64::from(slot.offset),
        wasm_len: wasm.len(),
        image_len: image.len(),
        crc: pack::crc_of(&image),
    })
}

/// 検査して焼く。
///
/// # Errors
/// `prepare` が失敗したとき。フラッシャが失敗した場合は `Ok(false)`
/// （使い方の誤りではないので）。
pub fn run(opts: &Options) -> Result<bool> {
    let plan = prepare(opts)?;

    println!(
        "{} → {} のスロット（{:#x}）  {} B（wasm {} B、crc32 {:08x}）",
        plan.image.display(),
        plan.board.name,
        plan.addr,
        plan.image_len,
        plan.wasm_len,
        plan.crc
    );

    // 1 段は picotool（§3.5）。`-t bin` を明示するのは、ファイル名の拡張子に
    // 判定を任せないため（picotool は拡張子で種別を決める）。
    // `-o` は picotool の help が "Load offset (memory address)" と書いている
    // とおり**アドレス**で、フラッシュのオフセットを渡すと弾かれる。
    let ok = spawn(
        "picotool",
        &[
            "load",
            &plan.image.to_string_lossy(),
            "-t",
            "bin",
            "-o",
            &format!("{:#x}", plan.addr),
        ],
    )?;
    if !ok {
        eprintln!(
            "  BOOTSEL を押しながら USB を挿してから、もう一度実行すること\n  \
             （USB の制御チャネルが入れば押下は要らなくなる。§3.5）"
        );
        return Ok(false);
    }

    if opts.no_run {
        println!("→ 書いた。リセットすると走る");
        return Ok(true);
    }

    // **`--monitor` ならリセットの前に開いて baud を当てる。**
    // 流れ始めてから当てると行が混ざる（`monitor` の罠 2）。
    let session = if opts.monitor {
        let port = monitor::choose(&monitor::ports(), opts.port.as_deref())?;
        let s = monitor::open(&port, monitor::DEFAULT_BAUD)?;
        eprintln!("{} を開いた（リセット前）", s.port().display());
        Some(s)
    } else {
        None
    };

    if !spawn("picotool", &["reboot"])? {
        eprintln!("  書けたがリセットできなかった。USB を抜き差しすること");
        return Ok(false);
    }

    let Some(session) = session else {
        println!("→ 走っている。トレースを取り込むなら:");
        println!("    wasmicon monitor -o app.log   # そのあとリセット");
        println!("  次からは deploy --monitor で 1 回で済む");
        return Ok(true);
    };

    println!("→ 走っている。トレースを取り込む:");
    monitor::capture(
        session,
        &monitor::Options {
            port: None,
            baud: monitor::DEFAULT_BAUD,
            out: opts.out.clone(),
            idle: Some(monitor::DEFAULT_IDLE),
            timeout: None,
        },
    )
}

/// 外のコマンドを呼ぶ。見つからないのはエラー、失敗は `false`。
fn spawn(cmd: &str, args: &[&str]) -> Result<bool> {
    let status = Command::new(cmd).args(args).status().with_context(|| {
        format!("{cmd} を起動できない（`wasmicon doctor` で入っているか見ること）")
    })?;
    Ok(status.success())
}
