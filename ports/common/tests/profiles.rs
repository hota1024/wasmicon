//! ボードプロファイルの値を固定する。
//!
//! `profile` はポートの `main.rs` / `board.rs` に散っていた値を集めたもの。
//! **集める前の数値をここに literal で書いてある**ので、移し間違いと、
//! あとからの不注意な変更の両方がここで落ちる。`config` は validate の
//! 上限なので、変わると**通るアプリが変わる**。
//!
//! **実機ボードは役割の既定の表を持たない**（2026-10-07 オーナー決定。
//! 配線表は設定スロットから読む。`docs/app-workflow.md` §3.9）。ここでは
//! 配線表の検査に使う GPIO の制約（本数・予約ピン・バスのピン）を固定する。

use wasmicon_core::Config;
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
fn arenas_are_what_the_ports_had() {
    // ポートの `static ARENA` / `static SCRATCH` がこの値を使うので、
    // 変えると実機のメモリ配置が変わる。
    assert_eq!(profile::RP2040.arena, 160 * 1024);
    assert_eq!(profile::RP2350.arena, 320 * 1024);
    assert_eq!(profile::ESP32S3.arena, 300 * 1024);
    for p in profile::PROFILES {
        assert_eq!(p.scratch, if p.name == "host" { 4 << 20 } else { 8 * 1024 });
    }
}

#[test]
fn each_board_arena_holds_the_pages_it_promises() {
    // `max_memory_pages` を満たすだけでは足りない。arena は decode / validate /
    // Exec / 線形メモリを全部ここから取るので、**上限のページ数が物理的に
    // 入らなければ、そのボードの Config は嘘**になる。
    for p in profile::PROFILES {
        if p.name == "host" {
            continue; // 下の test を見ること
        }
        let promised = p.config.max_memory_pages as usize * 64 * 1024;
        // **`<=` では足りない。** arena は decode / validate / Exec を先に
        // 取るので、ちょうど収まる大きさでは instantiate が必ず落ちる。
        // 実測で要るのは数 KB だが、32 KiB は空けておく
        // （ESP32-S3 が 4 ページ / 300 KiB で 44 KiB しか余らない一番きつい側）。
        let headroom = p.arena - promised;
        assert!(
            promised < p.arena && headroom >= 32 * 1024,
            "{}: {} ページ ({promised} B) に対して arena {} B は余裕 {headroom} B しかない",
            p.name,
            p.config.max_memory_pages,
            p.arena
        );
    }
}

#[test]
fn host_promises_more_pages_than_its_arena_can_hold() {
    // **host だけはこの不変条件を満たさない。** `Config::DEFAULT` は
    // 「ホスト PC 向けの既定値」で 65536 ページ（4 GiB）を名乗るが、
    // arena は 16 MiB しかない。256 ページ超のゲストは host でも
    // instantiate で落ちる。
    //
    // これは直すところではなく、**`wasmicon run` や `check --board host` が
    // 実機の根拠にならないことの、もう 1 つの理由**として記録しておく
    // （ページ上限の緩さに加えて、arena も実機より緩い）。
    let p = &profile::HOST;
    let promised = p.config.max_memory_pages as usize * 64 * 1024;
    assert!(
        promised > p.arena,
        "host が実機並みに締まったら、上の test に入れてよい"
    );
    // 実際に収まるのは 256 ページまで。
    assert_eq!(p.arena / (64 * 1024), 256);
}

#[test]
fn every_board_has_a_decided_slot() {
    // 3 ポートとも同じ置き方（1 MiB から 64 KiB）。読み方だけが違う
    // （Pico 系は XIP のスライス、ESP32-S3 は RAM への写し）。
    for (p, flash) in [
        (&profile::RP2040, 2usize << 20),
        (&profile::RP2350, 4usize << 20),
        (&profile::ESP32S3, 8usize << 20),
    ] {
        let sl = p
            .slot
            .unwrap_or_else(|| panic!("{} は決まっている", p.name));
        assert_eq!(
            sl.offset,
            1 << 20,
            "{}: 先頭 1 MiB はファームに空ける",
            p.name
        );
        assert_eq!(sl.len, 64 * 1024, "{}", p.name);
        assert!(
            (sl.offset + sl.len) as usize <= flash,
            "{}: フラッシュ {flash} B に収まらない",
            p.name
        );
    }
}

#[test]
fn the_host_mock_has_no_slot() {
    // mock にフラッシュは無い。
    assert!(profile::HOST.slot.is_none());
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
fn only_the_host_mock_has_a_role_table() {
    // 実機ボードは既定の表を持たない。host はフラッシュの無い mock なので
    // 表をプロファイルに持つ（`wasmicon run` とテストが使う）。
    for p in [&profile::RP2040, &profile::RP2350, &profile::ESP32S3] {
        assert!(p.roles.is_empty(), "{} は既定の表を持たない", p.name);
    }
    assert_eq!(
        profile::HOST.roles,
        [("led", 2), ("lcd-cs", 10), ("lcd-dc", 11), ("lcd-rst", 12)]
    );
}

#[test]
fn gpio_limits_are_what_the_ports_have() {
    // ポートの `board.rs` もコンパイル時に同じ値と突き合わせている。
    let pico_reserved = [0, 1, 23, 24, 25, 29];
    for p in [&profile::RP2040, &profile::RP2350] {
        assert_eq!(p.gpio_count, 30, "{}", p.name);
        assert_eq!(p.reserved, pico_reserved, "{}", p.name);
        assert_eq!(p.bus_pins, [4, 5, 16, 18, 19], "{}", p.name);
    }
    assert_eq!(profile::ESP32S3.gpio_count, 49);
    assert_eq!(
        profile::ESP32S3.reserved,
        [22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 43, 44]
    );
    assert_eq!(profile::ESP32S3.bus_pins, [8, 9, 11, 12, 13]);
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
fn by_name_resolves_every_profile_and_nothing_else() {
    for p in profile::PROFILES {
        let found = profile::by_name(p.name).expect("引けない");
        assert_eq!(found.name, p.name);
    }
    assert!(profile::by_name("rp2350 ").is_none(), "完全一致で引く");
    assert!(profile::by_name("esp32p4").is_none());
}
