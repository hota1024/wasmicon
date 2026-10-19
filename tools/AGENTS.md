# tools/

`wasmicon-gen` turns `wit/` into generated code; `wasmicon-cli` is the
`wasmicon` command apps are built, checked and flashed with. Shell and Python
helpers live beside them.

- `wasmicon-gen` writes the four files listed in `src/lib.rs::outputs`. Any
  change to `emit.rs`, `lower.rs` or a WIT doc comment regenerates them; commit
  the outputs in the same commit. `cargo run -p wasmicon-gen -- --check` and
  `tests/generated_up_to_date.rs` fail otherwise. `tools/wit2sig.py` is the
  reference signature derivation; `sh tools/check-sigs.sh` cross-checks it.
- `wasmicon-cli` error strings are lowercase, without a trailing period, with
  values in backticks. Integration tests assert on substrings of these messages
  and on the `match: N lines` line of `verify/diff-traces.sh`; change both sides.
- `src/flags.rs` is the source of truth for guest build flags. `apps/.cargo/config.toml`
  and every `asconfig.json` are byte-exact copies checked by `tests/flags.rs`.
- `src/new.rs` scaffolds must stay English and pass `tools/check-english.sh`
  when generated.
- Scripts are POSIX `sh` with a header of purpose, `Requires:` and `Usage:`.
- Verify from the repository root:
  `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test -p wasmicon-gen -p wasmicon-cli`,
  `cargo run -p wasmicon-gen -- --check`, `sh tools/check-sigs.sh`.
