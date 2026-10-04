//! `wasmicon check` の判定を `apps/` の実物で固定する。
//!
//! 要点は 2 つ:
//!
//! - **`blink-rs` は 4 ボードすべてで通る**（gpio / time / log / board しか
//!   使わないので、SPI / I2C が未実装の rp2040 でも走る）
//! - **`sensor-display-rs` は rp2040 だけで落ちる**（I2C と SPI が未実装）。
//!   これが `check --board` の存在理由で、**host 実行では分からない**
//!
//! ゲストのビルドは `ports/host/tests/apps.rs` と同じやり方。

use std::path::{Path, PathBuf};
use std::process::Command;

use wasmicon_cli::check::{self, Facts};
use wasmicon_port::profile::{self, Interfaces, Profile};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("リポジトリルートが見つからない")
}

/// guest workspace でアプリをビルドして `.wasm` を返す。
fn build(pkg: &str) -> Vec<u8> {
    // 同じプロセス内のテストを直列化する。
    //
    // **プロセスをまたぐ競合は防げない。** `ports/host/tests/apps.rs` が
    // 同じ `.wasm` を同じ target に作るので、テストバイナリが並走する
    // ランナー（nextest など）では、片方のリンカが書いている途中で
    // もう片方が読む形が残る。cargo のロックはビルドを直列化するが
    // 読み出しは守らない。共有のヘルパに切り出す件は docs/TODO.md §5。
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let root = repo_root();
    // cargo test はテストプロセスに RUSTUP_TOOLCHAIN を渡す。立っていると
    // rustup が toolchain override を見ないので、apps/rust-toolchain.toml の
    // targets（wasm32 の自動導入）が効かない。外してから起動する。
    let status = Command::new("cargo")
        .current_dir(root.join("apps"))
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(["build", "--release", "-p", pkg])
        .status()
        .expect("cargo を起動できない");
    assert!(status.success(), "{pkg} のビルドに失敗した");

    let wasm = root
        .join("apps/target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", pkg.replace('-', "_")));
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("{} を読めない: {e}", wasm.display()))
}

fn facts_of(wasm: &[u8]) -> Facts {
    check::facts(wasm).unwrap_or_else(|e| panic!("decode できない: {e:#}"))
}

#[test]
fn blink_rs_passes_on_every_board() {
    let wasm = build("blink-rs");
    let f = facts_of(&wasm);

    assert!(
        matches!(f.run_export, Some(Ok(()))) && f.has_memory_export,
        "run: func() と memory がある"
    );
    assert_eq!(f.import_failures(), 0, "import は全て表と一致する");
    // blink は gpio / time / board しか触らない。
    assert!(!f.per_iface.contains_key("i2c"));
    assert!(!f.per_iface.contains_key("spi"));

    for p in profile::PROFILES {
        let v = check::judge(&wasm, p, &f, None);
        assert!(
            v.is_ok(),
            "{} で落ちた（{} 件、未実装 {:?}）",
            p.name,
            v.fails(),
            v.unimplemented
        );
    }
}

#[test]
fn sensor_display_fails_only_on_rp2040() {
    let wasm = build("sensor-display-rs");
    let f = facts_of(&wasm);

    assert_eq!(f.import_failures(), 0);
    assert!(f.per_iface.contains_key("i2c"), "SHT4x を I2C で読む");
    assert!(f.per_iface.contains_key("spi"), "ILI9341 を SPI で描く");

    let rp2040 = check::judge(&wasm, &profile::RP2040, &f, None);
    assert!(!rp2040.is_ok(), "rp2040 は I2C / SPI が未実装なので落ちる");
    let mut unimpl = rp2040.unimplemented.clone();
    unimpl.sort();
    assert_eq!(unimpl, ["i2c", "spi"], "落ちる理由はこの 2 つだけ");
    assert!(
        rp2040.validate.is_ok(),
        "初期 1 ページは RP2040 の 2 ページ上限に収まる"
    );

    for p in [&profile::RP2350, &profile::ESP32S3, &profile::HOST] {
        let v = check::judge(&wasm, p, &f, None);
        assert!(v.is_ok(), "{} では通る（{} 件落ちた）", p.name, v.fails());
    }
}

#[test]
fn memory_limit_is_per_board() {
    // 「host で通っても実機で落ちる」を実際に見る（§4.3）。
    // 4 ページを要求する .wasm は host と rp2350 では通り、rp2040 では落ちる。
    let wasm = wat_memory_pages(4);
    let f = facts_of(&wasm);

    assert_eq!(f.mem_pages, Some(4));
    assert!(
        check::judge(&wasm, &profile::HOST, &f, None)
            .validate
            .is_ok(),
        "host は 65536 ページまで"
    );
    assert!(
        check::judge(&wasm, &profile::RP2350, &f, None)
            .validate
            .is_ok(),
        "rp2350 は 4 ページまで"
    );
    assert!(
        check::judge(&wasm, &profile::RP2040, &f, None)
            .validate
            .is_err(),
        "rp2040 は 2 ページまでなので validate が落ちる"
    );
}

#[test]
fn the_arena_is_part_of_the_judgement() {
    // **validate だけでは足りない。** 線形メモリは arena の残り全部を取るので、
    // max_memory_pages に収まっていても「先に取られた残りに min_pages * 64 KiB
    // が入らない」ことがある。実機はそこで instantiate が落ちる。
    //
    // 2 ページ (128 KiB) を要求するモジュールを、arena が 100 KiB しかない
    // ボードに当てる。Config は RP2040 のまま（2 ページ上限）なので
    // validate は通り、**instantiate だけが落ちる**。
    let wasm = wat_memory_pages(2);
    let tight = Profile {
        name: "tight",
        config: profile::RP2040.config,
        interfaces: Interfaces::ALL,
        roles: &[],
        arena: 100 * 1024,
        scratch: 8 * 1024,
    };
    let (stage, err) =
        check::instantiate_with(&wasm, &tight).expect_err("arena が足りないので落ちる");
    assert_eq!(
        stage.label(),
        "instantiate",
        "validate ではなく instantiate で落ちたと言う（{err}）"
    );
    assert!(
        err.contains("arena") || err.contains("Arena") || err.contains("out of"),
        "arena が足りないことが分かるメッセージ: {err}"
    );

    // 同じモジュールは実寸の RP2040（160 KiB）なら通る。
    assert!(
        check::instantiate_with(&wasm, &profile::RP2040).is_ok(),
        "RP2040 の実寸なら 2 ページが収まる"
    );
}

#[test]
fn every_board_can_instantiate_its_advertised_maximum() {
    // **上限いっぱいのページ数が実際に収まるか**を全ボードで見る。
    // ESP32-S3 が一番きつい（4 ページ = 256 KiB を 300 KiB の arena）。
    // TODO §1.4 のとおり DRAM はほぼ使い切っているので、arena を削ると
    // ここで `max_memory_pages: 4` が嘘になることが分かる。
    for p in profile::PROFILES {
        if p.name == "host" {
            continue; // host は 65536 ページを名乗るが arena は 256 ページ分
        }
        let pages = u8::try_from(p.config.max_memory_pages).expect("ページ数が u8 に入る");
        let wasm = wat_memory_pages(pages);
        assert!(
            check::instantiate_with(&wasm, p).is_ok(),
            "{}: 名乗っている {pages} ページが arena {} B に収まらない",
            p.name,
            p.arena
        );
    }
}

#[test]
fn the_run_export_must_take_and_return_nothing() {
    // abi-spec §3.3 は `run: func()`。引数が付いていると実行時に
    // `wrong arity` で落ちるので、**export の有無だけでは足りない**。
    let ok = wat_memory_pages(1);
    let f = facts_of(&ok);
    assert!(matches!(f.run_export, Some(Ok(()))), "() -> () は通る");
    assert_eq!(f.failures(), 0);

    let bad = wat_run_with_param();
    let f = facts_of(&bad);
    assert!(
        matches!(f.run_export, Some(Err(_))),
        "引数付きの run は落とす"
    );
    assert_eq!(f.failures(), 1, "run の型違いを 1 件として数える");
}

#[test]
fn a_declaration_turns_the_role_check_into_a_guarantee() {
    // 走査は参考（部分一致で誤検出し、AssemblyScript には当たらない）。
    // `wasmicon.toml` の宣言があれば列挙が確定するので、**送る前に**
    // 「このボードにこの役割は無い」と言える（§4.7）。
    //
    // 役割を 1 つだけ持つプロファイルを組んで、宣言した 2 つのうち
    // 片方が無い状態を作る。
    let wasm = wat_memory_pages(1);
    let f = facts_of(&wasm);
    let only_led: &[(&str, u32)] = &[("led", 2)];
    let board = Profile {
        name: "led-only",
        config: profile::RP2350.config,
        interfaces: Interfaces::ALL,
        roles: only_led,
        arena: profile::RP2350.arena,
        scratch: profile::RP2350.scratch,
    };
    // `Profile` は `&'static` を要求するので leak する（テストの中だけ）。
    let board: &'static Profile = Box::leak(Box::new(board));

    // 宣言が無いと、このモジュールには役割名の文字列が無いので何も言えない。
    let without = check::judge(&wasm, board, &f, None);
    assert!(without.is_ok(), "宣言が無ければ役割では落とさない");

    // 宣言があると、無い役割が失敗になる。
    let declared: &[&str] = &["led", "lcd-cs"];
    let with = check::judge(&wasm, board, &f, Some(declared));
    assert_eq!(with.missing_declared_roles, ["lcd-cs"]);
    assert!(!with.is_ok(), "宣言した役割が無いので落ちる");
    assert_eq!(with.fails(), 1);

    // 全部あるボードなら通る。
    let ok = check::judge(&wasm, &profile::RP2350, &f, Some(declared));
    assert!(ok.is_ok(), "rp2350 には 4 つ揃っている");
}

/// `run` が `(i32) -> ()` のモジュール。abi-spec §3.3 違反。
fn wat_run_with_param() -> Vec<u8> {
    let mut w = Vec::new();
    w.extend_from_slice(b"\0asm");
    w.extend_from_slice(&1u32.to_le_bytes());
    // type: (i32) -> ()
    w.extend_from_slice(&[0x01, 0x05, 0x01, 0x60, 0x01, 0x7f, 0x00]);
    w.extend_from_slice(&[0x03, 0x02, 0x01, 0x00]);
    w.extend_from_slice(&[0x05, 0x03, 0x01, 0x00, 0x01]);
    let exports: &[u8] = &[
        0x02, 0x03, b'r', b'u', b'n', 0x00, 0x00, 0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02,
        0x00,
    ];
    w.push(0x07);
    w.push(u8::try_from(exports.len()).expect("export セクションが大きすぎる"));
    w.extend_from_slice(exports);
    w.extend_from_slice(&[0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b]);
    w
}

/// `(module (memory n) (export "memory" ...) (func (export "run")))` を
/// 手で組む。wat の組み立て器を持ち込まずに最小の `.wasm` を作る。
fn wat_memory_pages(pages: u8) -> Vec<u8> {
    let mut w = Vec::new();
    w.extend_from_slice(b"\0asm");
    w.extend_from_slice(&1u32.to_le_bytes());

    // type: () -> ()
    w.extend_from_slice(&[0x01, 0x04, 0x01, 0x60, 0x00, 0x00]);
    // func: 1 つ、型 0
    w.extend_from_slice(&[0x03, 0x02, 0x01, 0x00]);
    // memory: min = pages、max なし
    w.extend_from_slice(&[0x05, 0x03, 0x01, 0x00, pages]);
    // export: "run" = func 0、"memory" = mem 0
    let exports: &[u8] = &[
        0x02, // 2 件
        0x03, b'r', b'u', b'n', 0x00, 0x00, //
        0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00,
    ];
    w.push(0x07);
    w.push(u8::try_from(exports.len()).expect("export セクションが大きすぎる"));
    w.extend_from_slice(exports);
    // code: 空の body
    w.extend_from_slice(&[0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b]);
    w
}
