//! ボードプロファイルの値を固定する。
//!
//! `profile` はポートの `main.rs` / `board.rs` に散っていた値を集めたもの。
//! **集める前の数値をここに literal で書いてある**ので、移し間違いと、
//! あとからの不注意な変更の両方がここで落ちる。`config` は validate の
//! 上限なので、変わると**通るアプリが変わる**。
//!
//! 役割名の語彙の検査（`ROLE_NAMES` に無い名前を弾く）は `profile` 側で
//! `const _: () = assert_role_names(..)` として**コンパイル時に**走る。
//! ここではその関数が実際に弾くことだけを見る。

use wasmicon_core::Config;
use wasmicon_port::ROLE_NAMES;
use wasmicon_port::profile::{self, Profile};

/// 集める前に各 `main.rs` にあった値（`max_memory_pages` 以外は 3 ポート共通）。
fn assert_mcu_limits(p: &Profile, pages: u32) {
    assert_eq!(p.config.max_value_stack, 512, "{}", p.name);
    assert_eq!(p.config.max_control_depth, 64, "{}", p.name);
    assert_eq!(p.config.max_locals, 256, "{}", p.name);
    assert_eq!(p.config.max_memory_pages, pages, "{}", p.name);
    assert_eq!(p.config.max_table_elems, 256, "{}", p.name);
    assert_eq!(p.config.max_call_depth, 32, "{}", p.name);
    assert_eq!(p.config.operand_stack_slots, 1024, "{}", p.name);
}

#[test]
fn mcu_configs_are_what_the_ports_had() {
    // RP2040 は SRAM 264 KB なので 2 ページ。他は 4 ページ。
    assert_mcu_limits(&profile::RP2040, 2);
    assert_mcu_limits(&profile::RP2350, 4);
    assert_mcu_limits(&profile::ESP32S3, 4);
}

#[test]
fn host_config_is_the_runtime_default() {
    // host は意図的に緩い。`Config::DEFAULT` と一致していることを見ておく
    // （profile 側が独自の値を持ち始めると、`wasmicon run` と `check --board host`
    // が食い違う）。
    let d = Config::default();
    let h = profile::HOST.config;
    assert_eq!(h.max_value_stack, d.max_value_stack);
    assert_eq!(h.max_control_depth, d.max_control_depth);
    assert_eq!(h.max_locals, d.max_locals);
    assert_eq!(h.max_memory_pages, d.max_memory_pages);
    assert_eq!(h.max_table_elems, d.max_table_elems);
    assert_eq!(h.max_call_depth, d.max_call_depth);
    assert_eq!(h.operand_stack_slots, d.operand_stack_slots);
}

#[test]
fn host_is_looser_than_every_board() {
    // これが成り立つ限り「host で通っても実機で通るとは限らない」が言える。
    // `wasmicon check --board <board>` が要る理由（docs/app-workflow.md §4.3）。
    for p in profile::PROFILES {
        if p.name == "host" {
            continue;
        }
        assert!(
            profile::HOST.config.max_memory_pages > p.config.max_memory_pages,
            "host より厳しいボードがある: {}",
            p.name
        );
    }
}

#[test]
fn roles_are_what_the_ports_had() {
    // abi-spec §8 の表。RP2040 と RP2350 はヘッダが同じなので同一。
    let pico = [("led", 15), ("lcd-cs", 17), ("lcd-dc", 20), ("lcd-rst", 21)];
    assert_eq!(profile::RP2040.roles, pico);
    assert_eq!(profile::RP2350.roles, pico);
    assert_eq!(
        profile::ESP32S3.roles,
        [("led", 2), ("lcd-cs", 10), ("lcd-dc", 14), ("lcd-rst", 15)]
    );
    assert_eq!(
        profile::HOST.roles,
        [("led", 2), ("lcd-cs", 10), ("lcd-dc", 11), ("lcd-rst", 12)]
    );
}

#[test]
fn rp2040_declares_i2c_and_spi_unimplemented() {
    // `spi_open` / `i2c_open` が `unsupported` を返す（docs/TODO.md §1.2）。
    // これを持たないと rp2040 向けの静的検査が通ってしまう（§4.3）。
    let i = profile::RP2040.interfaces;
    assert!(!i.i2c, "rp2040 の I2C は未実装");
    assert!(!i.spi, "rp2040 の SPI は未実装");
    assert!(i.gpio && i.time && i.log && i.board);

    for p in [&profile::RP2350, &profile::ESP32S3, &profile::HOST] {
        let i = p.interfaces;
        assert!(
            i.gpio && i.i2c && i.spi && i.time && i.log && i.board,
            "{} は全インターフェース実装済み",
            p.name
        );
    }
}

#[test]
fn every_role_name_is_in_the_vocabulary() {
    // profile 側のコンパイル時検査と同じことを実行時にも見る
    // （どのプロファイルも検査から漏れていないことの確認）。
    for p in profile::PROFILES {
        for (role, _) in p.roles {
            assert!(
                ROLE_NAMES.contains(role),
                "{} の役割名 {role} が ROLE_NAMES に無い（abi-spec §9）",
                p.name
            );
        }
    }
}

#[test]
#[should_panic(expected = "ROLE_NAMES")]
fn assert_role_names_rejects_an_unknown_name() {
    // コンパイル時の検査が本当に弾くことを、同じ関数を実行時に呼んで確かめる。
    profile::assert_role_names(&[("buzzer", 22)]);
}

#[test]
fn by_name_resolves_every_profile_and_nothing_else() {
    for p in profile::PROFILES {
        let found = profile::by_name(p.name).expect("引けない");
        assert_eq!(found.name, p.name);
    }
    assert!(profile::by_name("rp2350 ").is_none(), "完全一致で引く");
    assert!(profile::by_name("esp32p4").is_none());
}
