# Wasmicon

WebAssembly runtime for microcontrollers: a from-scratch `no_std` Core Wasm
interpreter (`runtime/`), a HAL defined in WIT (`wit/`), generated bindings for
Rust / AssemblyScript / C (`bindings/`), board ports (`ports/`), sample apps
(`apps/`) and the `wasmicon` CLI (`tools/`). Evaluation boards: ESP32-S3 and
RP2350 (Pico 2 W). ESP32-C5 is a port in progress (boots and blinks on hardware;
status in `docs/TODO.md` §1.3.1). RP2040 builds but is not evaluated.

## Read first

1. `README.md` – what exists, layout, build and flash instructions
2. `docs/TODO.md` – remaining work and open owner questions. The only place for them.
3. `docs/abi-spec.md` – normative WIT -> Core Wasm lowering rules
4. `wit/` – the HAL. Source of truth; every `generated.*` file is generator output
5. `docs/decisions.md` – frozen decisions (D-NN), adopted defaults (A-NN), pitfalls (P-NN)
6. `docs/style-guide.md` – language, comment, citation and commit conventions

Area rules live in nested `AGENTS.md` files: `runtime/`, `wit/`, `ports/`, `apps/`,
`tools/`, `bindings/`.

## Ask the owner before

Stop and write up the reason instead of implementing, if a change would:

- change the ABI: import module or field names, signatures, error code =
  discriminant + 1, handle 0 = invalid (`docs/abi-spec.md` §3–§5)
- use WIT syntax outside the subset in `docs/abi-spec.md` §2
- overturn a frozen decision (`docs/decisions.md` D-01…D-12)
- add a dependency crate or `alloc` to `runtime/`
- change the trace format (`docs/abi-spec.md` §9) or a guest log string that a
  trace-equality test compares

If you proceed on an adopted default while an owner question is open, say so in
the commit body.

## Verify

Root workspace (what CI's `root` job runs):

```sh
sh tools/fetch-testsuite.sh            # once; without it the spec tests silently skip
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cargo run -p wasmicon-gen -- --check   # generated files match wit/
sh tools/check-sigs.sh                 # signatures vs. the reference tools/wit2sig.py
sh tools/check-english.sh && sh tools/check-citations.sh
```

Other workspaces are separate: `cd apps`, `cd ports/<board>`, `cd verify/differential`
(see the nested `AGENTS.md`). `ports/esp32s3` builds through `ports/esp32s3/build.sh`.

## Conventions

- English only: code, comments, docs, commit messages, CLI and firmware output.
  CI runs `tools/check-english.sh`.
- Cite documents as `docs/abi-spec.md §4.1` or `docs/decisions.md D-07` (path +
  section, checked by `tools/check-citations.sh`). Code never cites `docs/handoff.md`.
- Comments explain why. Every `unsafe` block has a `// SAFETY:` comment stating
  the invariant in full (clippy `undocumented_unsafe_blocks` is on in every crate).
  No dates, phase numbers, verification status or build steps in source headers.
- Prefer a test or script to a prose rule. When you add a rule, add its check.
- Commits: Conventional Commits, English subject and body; the body says what was
  verified and what was not. Translation and rename commits never change behavior.
