# apps/ (guest workspace)

Sample apps compiled to `wasm32-unknown-unknown`, plus `bindings/rust`. This is
a separate Cargo workspace; `.cargo/config.toml` only applies when you build
from here.

- Verify: `cd apps && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo build --release`.
  AssemblyScript apps: `npm ci && npm run build` from the repository root.
  Unit tests run on the host triple:
  `cargo test --target "$(rustc -vV | sed -n 's/^host: //p')"` (two
  `clashing_extern_declarations` warnings are known, docs/TODO.md §2.2).
- `apps/.cargo/config.toml` is a byte-exact copy of `CARGO_CONFIG` in
  `tools/wasmicon-cli/src/flags.rs`, and every `asconfig.json` is a byte-exact copy of `ASCONFIG`.
  Edit `flags.rs` first; a test compares them.
- Every app has a `wasmicon.toml` (docs/app-workflow.md §4.7). Roles it looks
  up must be wired for every evaluation board in `[board.<name>.roles]`.
- The Rust and AssemblyScript versions of sensor-display must produce identical
  pixels; `apps/README.md` is the shared spec. Change both when you change one.
- Guest log strings that a trace-equality test compares are byte-identical
  across the Rust, AssemblyScript and C versions (today: the four blink error
  strings). Non-ASCII bytes are hex-escaped in traces.
- The ILI9341 driver is copied in three apps on purpose (apps/README.md §0);
  fix all copies together.
