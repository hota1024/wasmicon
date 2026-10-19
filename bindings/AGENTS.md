# bindings/

Guest-side bindings for Rust, AssemblyScript and C. Each `generated.*` file is
`wasmicon-gen` output; the hand-written parts are `rust/src/lib.rs`,
`rust/src/hal.rs`, `assemblyscript/assembly/index.ts`, `c/wasmicon_hal.h` and
`c/build.sh`.

- A WIT change lands in all three languages in one commit, by regenerating
  (`cargo run -p wasmicon-gen`); never edit a `generated.*` file by hand.
- The Rust crate builds inside the `apps` workspace:
  `cd apps && cargo clippy --all-targets -- -D warnings`. Every `unsafe fn` has a
  `# Safety` section; every call into `raw::*` has a `// SAFETY:` comment, and
  clippy `undocumented_unsafe_blocks` is on.
- Handles: 0 is invalid and `Drop` issues `[resource-drop]`
  (docs/abi-spec.md §5.2). Error codes are discriminant + 1; use the generated
  constants (docs/decisions.md P-11, P-12).
- AssemblyScript has no `Drop`; callers release handles explicitly. `asc`
  cannot resolve the scoped package as `~lib`, so apps import by relative path.
- C: `bindings/c/build.sh` holds the clang flags (no libc, `-nostdlib`);
  `wasmicon_hal.h` must stay C99 `-pedantic` and C++ clean.
