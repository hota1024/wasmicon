//! 設定スロット（配線表）の形式と検査を固定する（`docs/app-workflow.md` §3.9）。
//!
//! 書く側（CLI）と読む側（ファーム）が同じ `roles` を使うので、ここが両方の契約。
//! **形を変えたら `FORMAT_VERSION` を上げる。**

use wasmicon_port::fmt::Buf;
use wasmicon_port::profile;
use wasmicon_port::roles::{self, Limits, RoleMap, RolesError};

/// Pico 2 W の制約（GP0/1 は UART、GP4/5 と 16/18/19 はバス）。
fn pico() -> Limits<'static> {
    Limits::of(&profile::RP2350)
}

/// 表から設定スロットの画像（ヘッダ + 本文）を作る。CLI と同じ手順。
fn image(map: &RoleMap) -> Vec<u8> {
    let mut body = [0u8; roles::MAX_BODY];
    let n = roles::encode_body(map, &mut body);
    let mut v = roles::header(&body[..n]).to_vec();
    v.extend_from_slice(&body[..n]);
    v
}

fn table(entries: &[(&str, u32)]) -> RoleMap {
    roles::from_table(entries, &pico()).unwrap_or_else(|e| panic!("作れない: {}", e.reason()))
}

fn parsed(img: &[u8]) -> RoleMap {
    roles::parse(img, &pico()).unwrap_or_else(|e| panic!("読めない: {}", e.reason()))
}

fn parse_err(img: &[u8], what: &str) -> RolesError {
    match roles::parse(img, &pico()) {
        Err(e) => e,
        Ok(_) => panic!("{what} を期待したが読めてしまった"),
    }
}

#[test]
fn a_written_table_reads_back() {
    let map = table(&[("lcd-cs", 17), ("lcd-dc", 20), ("lcd-rst", 21)]);
    let back = parsed(&image(&map));
    assert_eq!(back.len(), 3);
    assert_eq!(back.get(b"lcd-cs"), Some(17));
    assert_eq!(back.get(b"lcd-dc"), Some(20));
    assert_eq!(back.get(b"lcd-rst"), Some(21));
    assert_eq!(back.get(b"led"), None, "書いていない役割は無い");
}

#[test]
fn the_body_is_human_readable() {
    // フラッシュを吸い出しても読める（§3.9 がテキストを選んだ理由）。
    let img = image(&table(&[("status-led", 2), ("lcd-cs", 17)]));
    assert_eq!(&img[..4], b"WMCR");
    assert_eq!(&img[roles::HEADER_LEN..], b"status-led=2\nlcd-cs=17\n");
}

#[test]
fn an_erased_slot_is_empty_not_an_error() {
    // 消去済み（0xff）とまだ書いていない（0x00）は「空」。ファームは役割を
    // 配らずに走る（既定の表は持たない）。
    for fill in [0xffu8, 0x00] {
        let e = parse_err(&[fill; 4096], "空");
        assert!(e.is_empty(), "{}", e.reason());
    }
}

#[test]
fn a_corrupted_table_is_rejected_whole() {
    let mut img = image(&table(&[("lcd-cs", 17), ("lcd-dc", 20)]));
    let last = img.len() - 2;
    img[last] ^= 0x01; // 本文の 1 文字（"20" の "0"）を壊す
    assert!(parse_err(&img, "CRC 違い") == RolesError::BadCrc);

    let mut img = image(&table(&[("lcd-cs", 17)]));
    img[0] = b'X';
    assert!(parse_err(&img, "magic 違い") == RolesError::BadMagic);
}

#[test]
fn a_well_formed_but_invalid_entry_drops_the_whole_table() {
    // CRC は合っているが中身がボードに合わない表（別のボード向けに書いた、
    // プロファイルが変わった、など）。**1 項目でもおかしければ全部捨てる。**
    for (body, want) in [
        (&b"lcd-cs=0\n"[..], RolesError::Reserved), // GP0 はトレースの UART
        (b"lcd-cs=4\n", RolesError::BusPin),        // GP4 は I2C0
        (b"lcd-cs=30\n", RolesError::OutOfRange),   // RP2350A は GP29 まで
        (b"a=15\nb=15\n", RolesError::DuplicatePin), // 同じピンに 2 役
        (b"a=15\na=17\n", RolesError::DuplicateName), // 同じ名前が 2 回
        (b"LCD=15\n", RolesError::BadName),         // 大文字
        (b"lcd-cs\n", RolesError::BadLine),         // = が無い
        (b"lcd-cs=x\n", RolesError::BadNumber),     // 数でない
        (b"lcd-cs=99999999999\n", RolesError::BadNumber), // u32 を溢れる
    ] {
        let mut img = roles::header(body).to_vec();
        img.extend_from_slice(body);
        let got = parse_err(&img, std::str::from_utf8(body).unwrap_or("?"));
        assert!(
            got == want,
            "{:?}: {} を期待したが {}",
            std::str::from_utf8(body),
            want.reason(),
            got.reason()
        );
    }
}

#[test]
fn names_are_limited_to_a_safe_alphabet() {
    assert!(roles::valid_name(b"status-led"));
    assert!(roles::valid_name(b"a1"));
    assert!(roles::valid_name(b"abcdefghijklmnop"), "16 文字までは通す");
    assert!(!roles::valid_name(b"abcdefghijklmnopq"), "17 文字は弾く");
    assert!(!roles::valid_name(b""));
    assert!(!roles::valid_name(b"1led"), "数字で始めない");
    assert!(!roles::valid_name(b"lcd_cs"));
    assert!(!roles::valid_name(b"lcd=cs"), "本文の区切りと衝突する");
}

#[test]
fn at_most_max_roles_entries() {
    let limits = Limits {
        gpio_count: 64,
        reserved: &[],
        bus_pins: &[],
    };
    let mut map = RoleMap::EMPTY;
    for i in 0..roles::MAX_ROLES as u32 {
        let name = format!("r{i}");
        assert!(map.insert(name.as_bytes(), i, &limits).is_ok());
    }
    assert!(map.insert(b"one-more", 63, &limits) == Err(RolesError::TooMany));
    // 上限いっぱいでも本文は MAX_BODY に収まる。
    let mut big = RoleMap::EMPTY;
    for i in 0..roles::MAX_ROLES as u32 {
        let name = format!("abcdefghijklmn{i:02}");
        assert!(big.insert(name.as_bytes(), i + 40, &limits).is_ok());
    }
    let mut body = [0u8; roles::MAX_BODY];
    let n = roles::encode_body(&big, &mut body);
    assert!(n < roles::MAX_BODY, "{n} バイト");
    assert!(roles::parse_body(&body[..n], &limits).is_ok_and(|m| m.len() == roles::MAX_ROLES));
}

#[test]
fn the_boot_line_lists_the_table_or_the_reason() {
    let describe = |r: Result<RoleMap, RolesError>| {
        let mut line = [0u8; 192];
        let mut out = Buf::new(&mut line);
        roles::describe(&r, &mut out);
        String::from_utf8(out.as_bytes().to_vec()).expect("ASCII")
    };
    assert_eq!(
        describe(Ok(table(&[("lcd-cs", 17), ("lcd-dc", 20)]))),
        "wasmicon: roles lcd-cs=17 lcd-dc=20"
    );
    assert_eq!(describe(Ok(RoleMap::EMPTY)), "wasmicon: roles none");
    assert_eq!(
        describe(Err(RolesError::Empty)),
        "wasmicon: roles empty, no roles"
    );
}

#[test]
fn every_profile_puts_the_roles_slot_right_after_the_app_slot() {
    // deploy はアプリスロットと設定スロットを 1 本の画像で書く（pack::with_roles）。
    for p in profile::PROFILES {
        match (p.slot, p.role_slot) {
            (Some(s), Some(r)) => {
                assert_eq!(r.offset, s.offset + s.len, "{}", p.name);
                assert_eq!(r.len, roles::SLOT_LEN, "{}", p.name);
            }
            (None, None) => {}
            _ => panic!("{}: スロットと設定スロットは揃えて持つ", p.name),
        }
        // 実機ボードは既定の表を持たない（host の mock だけが持つ）。
        assert_eq!(p.roles.is_empty(), p.name != "host", "{}", p.name);
    }
}
