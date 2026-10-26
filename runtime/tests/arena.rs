//! `Arena::alloc_rest` alignment: linear memory starts on a `REST_ALIGN`
//! boundary whatever was allocated before it.

use wasmicon_core::Arena;
use wasmicon_core::arena::REST_ALIGN;

#[repr(align(16))]
struct Buf([u8; 256]);

#[test]
fn the_rest_starts_aligned_after_an_odd_allocation() {
    // `instantiate` allocates one `bool` per data segment just before linear
    // memory; two segments left the old start at 4n + 2.
    for skew in 0..REST_ALIGN {
        let mut buf = Buf([0; 256]);
        let end = buf.0.as_ptr() as usize + buf.0.len();
        let mut arena = Arena::new(&mut buf.0);
        arena.alloc(skew, false).ok().expect("fits");
        let rest = arena.alloc_rest();
        assert_eq!(rest.as_ptr() as usize % REST_ALIGN, 0, "skew {skew}");
        assert_eq!(rest.as_ptr() as usize + rest.len(), end, "runs to the end");
        assert!(
            rest.len() > 256 - skew - REST_ALIGN,
            "skips less than REST_ALIGN"
        );
        assert_eq!(arena.used(), arena.capacity(), "nothing left after it");
    }
}

#[test]
fn the_rest_is_empty_when_the_padding_does_not_fit() {
    let mut buf = Buf([0; 256]);
    let mut arena = Arena::new(&mut buf.0);
    arena.alloc(250, 0u8).ok().expect("fits");
    assert!(arena.alloc_rest().is_empty());
}
