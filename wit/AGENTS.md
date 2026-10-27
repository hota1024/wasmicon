# wit/ (the HAL)

The single source of truth for the HAL. Everything under `bindings/*/generated.*`
and `runtime/src/generated.rs` is derived from these files.

- Use only the WIT subset in docs/abi-spec.md §2 (docs/decisions.md D-01).
  The generator rejects anything else; do not widen it without the owner.
- `///` doc comments are copied verbatim into the three guest bindings as
  rustdoc, JSDoc and C comments; `runtime/src/generated.rs` carries only the
  enum docs. Write them as API documentation for a binding user: one summary
  sentence, then behavior and error cases.
- Changing import names, signatures or the error enum changes the ABI
  (docs/abi-spec.md §3, §4, §5). That is an owner decision.
- `wit-device/` is the device layer, `wasmicon:device` (docs/device-layer.md,
  docs/decisions.md D-13). It follows the same rules, uses the HAL's
  `error-code`, and has no `deps/` copy of the HAL; `tools/check-sigs.sh`
  checks that it resolves. A new device interface needs two backends first.
- After any edit: `wasm-tools component wit wit/`, then
  `cargo run -p wasmicon-gen`, and commit the four regenerated files in the
  same commit. `cargo run -p wasmicon-gen -- --check` and
  `sh tools/check-sigs.sh` must pass.
