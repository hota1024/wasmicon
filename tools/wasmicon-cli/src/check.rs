//! `wasmicon check` — アプリがそのボードで走るかを**実ランタイムで**検査する。
//!
//! 見るもの（`docs/app-workflow.md` §4.3）:
//!
//! - import の**名前とシグネチャの完全一致**（abi-spec §6.4 / §7）。表は
//!   `runtime` の生成物（= `wit/` 由来）
//! - `run` と `memory` の export（abi-spec §3.3 / §6.2）
//! - **そのボードの `Config` での validate**（`max_memory_pages` など）
//! - **アプリが import するインターフェースがそのポートで実装済みか。**
//!   `ports/rp2040` は SPI / I2C が `unsupported` を返すので、これが無いと
//!   静的検査は通って実機で初めて落ちる
//!
//! **できないこと: 役割名の列挙。** `pin-by-role` の引数は実行時に渡る
//! `string` なので、`.wasm` から確実には取れない。ここでは既知の役割名が
//! バイト列に現れるかを見るだけで、**参考**であって保証ではない
//! （`wasmicon.toml` の `[requirements] pin-roles` が入れば保証になる。§4.7）。
//!
//! 判定（`facts` / `judge`）と印字を分けてあるのは、判定だけをテストから
//! 呼べるようにするため。

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use wasmicon_core::module::{ExportDesc, ImportDesc};
use wasmicon_core::{Arena, Exec, decode, generated, instantiate, validate};
use wasmicon_port::profile::{self, Interfaces, Profile};
use wasmicon_port::{Hal, ROLE_NAMES};

/// 事実を取り出すだけの decode に使う作業領域（ボードに依存しない）。
/// ボードごとの検査は**そのボードの arena の実寸**を使う（`Profile::arena`）。
const FACTS_ARENA: usize = 4 << 20;

/// `env.abort` は `world app` に無い例外的な import（`docs/handoff.md` §6）。
/// AssemblyScript が必ず入れてくるので、未知の import として扱わない。
const AS_ABORT: (&str, &str) = ("env", "abort");

/// import 1 件の判定。
#[derive(PartialEq, Eq)]
pub enum ImportState {
    /// 名前もシグネチャも一致した。
    Ok,
    /// 表に無い（リンクエラーになる）。
    Unknown,
    /// 名前はあるがシグネチャが違う（リンクエラーになる）。
    SigMismatch { want: &'static str },
    /// AssemblyScript の `env.abort`。
    AsAbort,
    /// 関数でない import（abi-spec §6.2: import memory は不可）。
    NotAFunc,
    /// 型インデックスが型セクションの外を指している（壊れた `.wasm`）。
    BadTypeIndex { idx: u32 },
}

impl ImportState {
    /// リンクを壊すか。
    #[must_use]
    pub fn is_failure(&self) -> bool {
        !matches!(self, ImportState::Ok | ImportState::AsAbort)
    }
}

pub struct ImportCheck {
    pub module: String,
    pub name: String,
    pub state: ImportState,
}

/// ボードに依存しない事実。arena を畳んだあとも使えるよう所有権を持つ。
pub struct Facts {
    pub imports: Vec<ImportCheck>,
    /// インターフェースごとの import 数。`wasmicon:hal/<iface>@0.1.0` の `<iface>`。
    pub per_iface: BTreeMap<String, usize>,
    pub has_run: bool,
    pub has_memory_export: bool,
    pub mem_pages: Option<u32>,
    pub table_elems: Option<u32>,
    /// バイト列に現れた既知の役割名（**参考**）。
    pub roles_seen: Vec<&'static str>,
}

impl Facts {
    /// リンクを壊す import の数。
    #[must_use]
    pub fn import_failures(&self) -> usize {
        self.imports.iter().filter(|i| i.state.is_failure()).count()
    }

    /// **ボードに依存しない**失敗の数（import と export）。
    #[must_use]
    pub fn failures(&self) -> usize {
        self.import_failures() + usize::from(!self.has_run || !self.has_memory_export)
    }
}

/// 1 ボード分の判定。
pub struct Verdict {
    pub board: &'static str,
    /// `Ok(そのボードの上限)` か、validate が落ちた理由。
    pub validate: std::result::Result<u32, String>,
    /// 使っているのにそのポートで未実装のインターフェース。
    pub unimplemented: Vec<String>,
    /// プロファイルに無い役割名（**参考**。`fails()` には数えない）。
    pub missing_roles: Vec<&'static str>,
}

impl Verdict {
    /// **このボードに固有の**失敗の数。
    ///
    /// import や export の不備はボードに依存しないので、ここには入れない
    /// （モジュールの節で 1 回だけ数える。`Facts::failures`）。全ボードに
    /// 同じ数を足すと、どれがボード固有の問題なのか読めなくなる。
    ///
    /// **役割名は数えない。** 走査は参考（部分一致で誤検出し、
    /// AssemblyScript には当たらない）なので、終了コードを左右させない。
    #[must_use]
    pub fn fails(&self) -> usize {
        usize::from(self.validate.is_err()) + self.unimplemented.len()
    }

    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.fails() == 0
    }
}

pub struct Options {
    pub path: PathBuf,
    /// `--board`。`None` なら全ボードを見る（同一バイナリが本題なので既定）。
    pub board: Option<String>,
}

/// 検査して結果を出す。全ボードが通れば `true`。
pub fn run(opts: &Options) -> Result<bool> {
    let wasm =
        std::fs::read(&opts.path).with_context(|| format!("{} を読めない", opts.path.display()))?;

    let boards: Vec<&'static Profile> = match &opts.board {
        Some(name) => vec![profile::by_name(name).with_context(|| {
            let known: Vec<&str> = profile::PROFILES.iter().map(|p| p.name).collect();
            format!("知らないボード {name}（あるのは {}）", known.join(" / "))
        })?],
        None => profile::PROFILES.to_vec(),
    };

    let facts = facts(&wasm)?;
    print_module(&opts.path, &wasm, &facts);

    let mut all_ok = facts.failures() == 0;
    for board in boards {
        let v = judge(&wasm, board, &facts);
        println!();
        print_verdict(&v, &facts, board);
        all_ok &= v.is_ok();
    }
    Ok(all_ok)
}

/// decode して、ボードに依存しない事実を取り出す。
///
/// # Errors
/// decode が失敗したとき（壊れた `.wasm`、対応外の機能セットなど）。
pub fn facts(wasm: &[u8]) -> Result<Facts> {
    let mut buf = vec![0u8; FACTS_ARENA];
    let mut arena = Arena::new(&mut buf);
    let m = decode::decode(wasm, &mut arena).map_err(|e| anyhow::anyhow!(reason(e)))?;

    let mut imports = Vec::new();
    let mut per_iface = BTreeMap::new();
    for imp in m.imports {
        let state = match imp.desc {
            ImportDesc::Func(ty) => match m.types.get(ty as usize) {
                // decode は import の型インデックスを検査しない（壊れた
                // `.wasm` を診断するのがこのコマンドの仕事なので、
                // ここで panic してはいけない）。
                None => ImportState::BadTypeIndex { idx: ty },
                Some(ft) => match generated::resolve(imp.module, imp.name) {
                    Some(d) if ft.sig_matches(d.sig) => ImportState::Ok,
                    Some(d) => ImportState::SigMismatch { want: d.sig },
                    None if (imp.module, imp.name) == AS_ABORT => ImportState::AsAbort,
                    None => ImportState::Unknown,
                },
            },
            _ => ImportState::NotAFunc,
        };
        if state == ImportState::Ok
            && let Some(iface) = iface_of(imp.module)
        {
            *per_iface.entry(iface.to_string()).or_insert(0) += 1;
        }
        imports.push(ImportCheck {
            module: imp.module.to_string(),
            name: imp.name.to_string(),
            state,
        });
    }

    let has_run = m
        .exports
        .iter()
        .any(|e| e.name == "run" && matches!(e.desc, ExportDesc::Func(_)));
    let has_memory_export = m
        .exports
        .iter()
        .any(|e| e.name == "memory" && matches!(e.desc, ExportDesc::Memory(_)));

    Ok(Facts {
        imports,
        per_iface,
        has_run,
        has_memory_export,
        mem_pages: m.mems.first().map(|l| l.min),
        table_elems: m.tables.first().map(|l| l.min),
        roles_seen: roles_in_bytes(wasm),
    })
}

/// 1 ボード分を判定する。
#[must_use]
pub fn judge(wasm: &[u8], p: &'static Profile, f: &Facts) -> Verdict {
    let validate = instantiate_with(wasm, p).map(|()| p.config.max_memory_pages);

    let unimplemented = f
        .per_iface
        .keys()
        .filter(|iface| implemented(&p.interfaces, iface) == Some(false))
        .cloned()
        .collect();

    let missing_roles = f
        .roles_seen
        .iter()
        .copied()
        .filter(|r| !p.roles.iter().any(|(name, _)| name == r))
        .collect();

    Verdict {
        board: p.name,
        validate,
        unimplemented,
        missing_roles,
    }
}

/// そのボードの実寸で decode → validate → `Exec` → instantiate まで通す。
///
/// **`validate` だけでは足りない。** 線形メモリは arena の残り全部を取るので
/// （`Arena::alloc_rest`）、`max_memory_pages` に収まっていても
/// 「decode / validate / `Exec` が先に取った残りに `min_pages * 64 KiB` が
/// 入らない」ことがある。実機はそこで `instantiate` が落ちる。
/// ここでボードの `arena` / `scratch` の実寸を使うのは、その余裕まで
/// 再現するため（`ports/rp2040` は 160 KiB しかなく、2 ページで
/// 20 KiB ほどしか余らない）。
///
/// リンクは `Hal` が解決する（import 名とシグネチャの完全一致。abi-spec §6.4）。
/// ボードは host の mock を使う。`instantiate` はゲストを実行しないので、
/// どのボード実装でも結果は変わらない。
///
/// # Errors
/// decode / validate / `Exec` / instantiate のいずれかが失敗したとき。
/// 文字列は `reason() [kind]` の形。
pub fn instantiate_with(wasm: &[u8], p: &Profile) -> std::result::Result<(), String> {
    let mut buf = vec![0u8; p.arena];
    let mut scratch_buf = vec![0u8; p.scratch];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);

    let m = decode::decode(wasm, &mut arena).map_err(reason)?;
    let v = validate::validate(&m, &p.config, &mut arena, &mut scratch).map_err(reason)?;
    // Exec は線形メモリ（arena の残り全部）より先に確保する。ポートと同じ順番。
    let _exec = Exec::new(&p.config, &mut arena).map_err(reason)?;
    let mut hal = Hal::new(wasmicon_host::hal::HostBoard::new(), false);
    // 線形メモリはここで arena の残りから取られる。足りなければ実機と同じ
    // ように落ちる（それがこの関数の目的）。
    let _inst = instantiate(m, v, &p.config, &mut arena, &mut hal).map_err(reason)?;
    Ok(())
}

/// `wasmicon:hal/<iface>@0.1.0` から `<iface>` を取る（abi-spec §3.1）。
fn iface_of(module: &str) -> Option<&str> {
    module
        .strip_prefix("wasmicon:hal/")?
        .split('@')
        .next()
        .filter(|s| !s.is_empty())
}

/// 既知の役割名がバイト列に現れるかを見る（**参考**。§4.3）。
///
/// 部分一致なので、ログ文字列の中の `led`（`failed` など）も拾う。境界を
/// 見ないのは、データセグメントの文字列が長さ前置で**隣と連結して**置かれる
/// ため（`lcd-cslcd-dc…`）。境界を要求すると本物を落とす。
/// **AssemblyScript のゲストには原理的に当たらない。** AS の文字列リテラルは
/// UTF-16 で置かれるので、ASCII の部分一致では見つからない（実測:
/// `sensor_display_as.wasm` は「見つからない」になる）。バインディングが
/// 呼び出し時に UTF-8 へ変換するため、UTF-8 の役割名はバイナリに現れない。
///
/// どちらに転んでも保証にはならないので、精度を上げるより
/// `wasmicon.toml` の宣言（§4.7）に寄せる。
fn roles_in_bytes(wasm: &[u8]) -> Vec<&'static str> {
    ROLE_NAMES
        .iter()
        .copied()
        .filter(|role| {
            let pat = role.as_bytes();
            wasm.windows(pat.len()).any(|w| w == pat)
        })
        .collect()
}

fn implemented(i: &Interfaces, iface: &str) -> Option<bool> {
    match iface {
        "gpio" => Some(i.gpio),
        "i2c" => Some(i.i2c),
        "spi" => Some(i.spi),
        "time" => Some(i.time),
        "log" => Some(i.log),
        "board" => Some(i.board),
        // types は関数を持たないので import に現れない（abi-spec §7）。
        _ => None,
    }
}

/// `wasmicon_core::Error` は `Debug` を実装しない（コアで `core::fmt` を
/// 使わないため）。表示は `reason()` と `kind().name()` で組む。
fn reason(e: wasmicon_core::Error) -> String {
    format!("{} [{}]", e.reason(), e.kind().name())
}

// ---- 印字 ----

fn print_module(path: &Path, wasm: &[u8], f: &Facts) {
    let name = path.file_name().unwrap_or(path.as_os_str());
    println!("{}  {} B", name.to_string_lossy(), thousands(wasm.len()));
    println!();

    row("abi", "wasmicon:hal@0.1.0", "import 名で強制される");

    let bad = f.import_failures();
    if bad == 0 {
        let breakdown: Vec<String> = f
            .per_iface
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        let mut note = format!("({})", breakdown.join(" / "));
        if f.imports.iter().any(|i| i.state == ImportState::AsAbort) {
            note.push_str(" + env.abort");
        }
        row(
            "imports",
            &format!("{} 件すべて一致", f.imports.len()),
            &note,
        );
    } else {
        row("imports", &format!("{bad} 件が一致しない"), "");
        for i in &f.imports {
            let what = match &i.state {
                ImportState::Unknown => "表に無い（リンクエラー）".to_string(),
                ImportState::SigMismatch { want } => {
                    format!("シグネチャが違う（{want} を期待）")
                }
                ImportState::NotAFunc => "関数でない（abi-spec §6.2）".to_string(),
                ImportState::BadTypeIndex { idx } => {
                    format!("型 {idx} が型セクションの外を指している（壊れた .wasm）")
                }
                ImportState::Ok | ImportState::AsAbort => continue,
            };
            println!("         - {}/{}  {what}", i.module, i.name);
        }
    }

    let exports = match (f.has_run, f.has_memory_export) {
        (true, true) => ("run, memory", "あり"),
        (true, false) => ("run", "memory の export が無い（abi-spec §6.2）"),
        (false, true) => ("memory", "run の export が無い（abi-spec §3.3）"),
        (false, false) => ("なし", "run と memory の export が無い"),
    };
    row("exports", exports.0, exports.1);

    match f.mem_pages {
        Some(p) => row("memory", &format!("初期 {p} ページ"), ""),
        None => row("memory", "定義なし", "abi-spec §6.2 は 1 つ要求する"),
    }
    match f.table_elems {
        Some(n) => row("table", &format!("{n} 要素"), ""),
        None => row("table", "なし", ""),
    }

    // ここまでがボードに依存しない判定。ボードごとの節に混ぜると、
    // 同じ失敗が全ボードに重複して出て、どれがボード固有か読めなくなる。
    if f.failures() > 0 {
        println!("→ 全ボード共通で落ちる（{} 件）", f.failures());
    }
}

fn print_verdict(v: &Verdict, f: &Facts, p: &Profile) {
    println!("[{}]", v.board);

    match &v.validate {
        Ok(limit) => row(
            "validate",
            &format!("初期 {} ページ ≤ {limit}", f.mem_pages.unwrap_or(0)),
            "ok",
        ),
        Err(e) => row("validate", e, "← 落ちる"),
    }

    for iface in &v.unimplemented {
        row(
            iface,
            "このポートは未実装（実機では unsupported）",
            "← 落ちる",
        );
    }

    if f.roles_seen.is_empty() {
        row("roles", "見つからない", "（参考）");
    } else {
        let seen = f.roles_seen.join(", ");
        if v.missing_roles.is_empty() {
            row("roles", &seen, &format!("{} にある（参考）", p.name));
        } else {
            row(
                "roles",
                &seen,
                &format!("{} に無い: {}（参考）", p.name, v.missing_roles.join(", ")),
            );
        }
    }

    match (v.fails(), f.failures()) {
        (0, 0) => println!("→ 通る"),
        (0, n) => println!("→ 落ちる（全ボード共通の {n} 件。上に出した）"),
        (m, 0) => println!("→ 落ちる（このボードで {m} 件）"),
        (m, n) => println!("→ 落ちる（このボードで {m} 件 + 全ボード共通の {n} 件）"),
    }
}

/// 列を揃える。`{:<n}` は文字数で数えるので、日本語が入ると崩れる。
fn row(label: &str, value: &str, note: &str) {
    if note.is_empty() {
        println!("{}{value}", pad(label, 9));
    } else {
        println!("{}{}{note}", pad(label, 9), pad(value, 44));
    }
}

fn pad(s: &str, w: usize) -> String {
    let mut out = String::from(s);
    for _ in display_width(s)..w {
        out.push(' ');
    }
    out
}

/// 端末での表示幅。CJK は 2 桁として数える。
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| if is_wide(c) { 2 } else { 1 })
        .sum::<usize>()
}

fn is_wide(c: char) -> bool {
    matches!(u32::from(c),
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6)
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iface_names_come_from_the_module_name() {
        assert_eq!(iface_of("wasmicon:hal/gpio@0.1.0"), Some("gpio"));
        assert_eq!(iface_of("wasmicon:hal/i2c@0.1.0"), Some("i2c"));
        // abi-spec §3.1 はバージョン必須・完全一致なので、他の形は拾わない。
        assert_eq!(iface_of("env"), None);
        assert_eq!(iface_of("wasmicon:hal/"), None);
    }

    #[test]
    fn interfaces_map_to_the_profile_fields() {
        let i = profile::RP2040.interfaces;
        assert_eq!(implemented(&i, "i2c"), Some(false));
        assert_eq!(implemented(&i, "spi"), Some(false));
        assert_eq!(implemented(&i, "gpio"), Some(true));
        // types は関数を持たないので import に現れない。
        assert_eq!(implemented(&i, "types"), None);
    }

    #[test]
    fn role_scan_is_a_substring_match() {
        // 本物は拾う。
        assert_eq!(roles_in_bytes(b"....lcd-cs...."), vec!["lcd-cs"]);
        // 連結していても拾う（データセグメントは長さ前置で隣と繋がる）。
        let all = roles_in_bytes(b"ledlcd-cslcd-dclcd-rst");
        assert_eq!(all, vec!["led", "lcd-cs", "lcd-dc", "lcd-rst"]);
        // ログ文字列の中の led も拾ってしまう。**だから（参考）**。
        assert_eq!(roles_in_bytes(b"sensor crc failed"), vec!["led"]);
    }

    #[test]
    fn role_scan_is_blind_to_assemblyscript_strings() {
        // AS の文字列リテラルは UTF-16 で置かれるので ASCII の部分一致では
        // 当たらない。実測でも sensor_display_as.wasm は「見つからない」になる。
        // **これが（参考）の限界で、wasmicon.toml の宣言が要る理由**（§4.7）。
        let utf16: Vec<u8> = "lcd-cs".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(roles_in_bytes(&utf16).is_empty());
    }

    #[test]
    fn columns_count_japanese_as_two() {
        assert_eq!(display_width("ok"), 2);
        assert_eq!(display_width("通る"), 4);
        // "1 " が 2 桁 + "ページ" が 3 文字 x 2 桁。
        assert_eq!(display_width("1 ページ"), 2 + 6);
        assert_eq!(pad("ok", 4), "ok  ");
        assert_eq!(pad("通る", 6), "通る  ");
    }

    #[test]
    fn sizes_are_grouped() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(775), "775");
        assert_eq!(thousands(4549), "4,549");
        assert_eq!(thousands(1_146_862), "1,146,862");
    }
}
