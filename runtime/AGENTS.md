# runtime/ (wasmicon-core)

The Core Wasm interpreter. It runs on the host and on every board unchanged.

- `#![no_std]`, zero dependency crates, no `alloc`. The port hands in an arena
  (`&mut [u8]`) and every internal structure is an index or offset into it
  (docs/decisions.md D-11). Adding a crate or `alloc` is an owner decision.
- Wasm arithmetic wraps: use `wrapping_*`, `rotate_*` and `checked_*`
  explicitly. Debug builds keep `overflow-checks` on so a stray `+` is caught.
- No `core::fmt` in `src/`: no `write!`, `format_args!`, `{:?}` or `Display`.
  Formatting lives in the port layer (docs/decisions.md P-09). Return
  `Result` instead of panicking; use `get()` on hot paths (docs/decisions.md P-08).
- Bytecode may live in XIP flash: read multi-byte values only through
  `from_le_bytes` on a slice, never `ptr::read` or `align_to` (docs/decisions.md P-06).
  Range-check guest memory with `checked_add` and `get` (docs/decisions.md P-07).
- Every `unsafe` block has a full `// SAFETY:` comment; clippy enforces it.
- `src/generated.rs` is generator output. Edit `wit/` and run
  `cargo run -p wasmicon-gen`, then commit all four generated files together.
- Spec tests (`tests/spec.rs`) need `sh tools/fetch-testsuite.sh` first;
  without it they skip and still report `ok`. Excluded `.wast` files are listed
  with reasons in `EXCLUDED`.
- Verify: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  from the repository root. Measure size with `sh tools/measure-size.sh`.
