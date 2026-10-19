# wit/ (the HAL)

The single source of truth for the HAL. Everything under `bindings/*/generated.*`
and `runtime/src/generated.rs` is derived from these files.

- Use only the WIT subset in docs/abi-spec.md §2 (docs/decisions.md D-01).
  The generator rejects anything else; do not widen it without the owner.
- `///` doc comments are copied verbatim into all four generated bindings as
  rustdoc, JSDoc and C comments. Write them as API documentation for a binding
  user: one summary sentence, then behavior and error cases.
- Changing import names, signatures or the error enum changes the ABI
  (docs/abi-spec.md §3, §4, §5). That is an owner decision.
- After any edit: `wasm-tools component wit wit/`, then
  `cargo run -p wasmicon-gen`, and commit the four regenerated files in the
  same commit. `cargo run -p wasmicon-gen -- --check` and
  `sh tools/check-sigs.sh` must pass.
