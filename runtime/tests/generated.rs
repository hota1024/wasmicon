//! 生成されたエラーコードの意味づけを固定する。
//!
//! `from_status` は 0（成功）と範囲外の両方で `None` を返す。この 2 つを
//! 区別せずに書くとバインディング側で「未知のステータス = 成功」になり、
//! 無効ハンドル 0 を包んで返してしまう。ここでその前提を明文化しておく。

use wasmicon_core::generated::ErrorCode;

#[test]
fn status_is_discriminant_plus_one() {
    // abi-spec §4.4: n > 0 のとき n - 1 が discriminant。
    let mut i = 0;
    while i < ErrorCode::COUNT {
        let e = ErrorCode::from_u32(i).expect("範囲内なのに復元できない");
        assert_eq!(e.status(), i + 1, "discriminant {i} のステータスが違う");
        assert!(
            ErrorCode::from_status(e.status()) == Some(e),
            "ステータス {} から復元できない",
            e.status()
        );
        i += 1;
    }
}

#[test]
fn from_status_returns_none_for_success_and_out_of_range() {
    assert!(
        ErrorCode::from_status(0).is_none(),
        "0 は成功であってエラーではない"
    );
    assert!(
        ErrorCode::from_status(ErrorCode::COUNT + 1).is_none(),
        "範囲外のステータスは復元できない"
    );
    assert!(ErrorCode::from_status(u32::MAX).is_none());
}
