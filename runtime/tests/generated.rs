//! Locks in the meaning of the generated error codes.
//!
//! `from_status` returns `None` both for 0 (success) and for out-of-range
//! values. A binding that does not tell these apart treats an unknown status
//! as success and hands back the invalid handle 0 wrapped in `Ok`. This test
//! spells out that assumption.

use wasmicon_core::generated::ErrorCode;

#[test]
fn status_is_discriminant_plus_one() {
    // docs/abi-spec.md §4.4: for n > 0, the discriminant is n - 1.
    let mut i = 0;
    while i < ErrorCode::COUNT {
        let e = ErrorCode::from_u32(i).expect("in range but not recoverable");
        assert_eq!(e.status(), i + 1, "wrong status for discriminant {i}");
        assert!(
            ErrorCode::from_status(e.status()) == Some(e),
            "status {} does not round-trip",
            e.status()
        );
        i += 1;
    }
}

#[test]
fn from_status_returns_none_for_success_and_out_of_range() {
    assert!(
        ErrorCode::from_status(0).is_none(),
        "0 is success, not an error"
    );
    assert!(
        ErrorCode::from_status(ErrorCode::COUNT + 1).is_none(),
        "an out-of-range status must not decode"
    );
    assert!(ErrorCode::from_status(u32::MAX).is_none());
}
