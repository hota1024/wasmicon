# Wasmicon handoff document (record)

> **Historical record.** Kept as written at the time and translated from Japanese; section numbers are stable. Current rules: `docs/decisions.md`. Current status: `README.md`, `docs/TODO.md`.

Created: 2026-09-10 / Author: Claude (Cowork session) / Owner: hota

**This document has finished its job as a handoff.** It stays as the record of what was decided, how, and how it was built.
Code no longer cites this document. Its permanent content moved to `docs/decisions.md` (§2 item N -> D-0N, §3 #N -> A-0N, §6 -> P-01...P-13); section numbers are kept for the historical cross-references in the other docs.

Where to look depends on what you need to know:

| You want to know | Look at |
|---|---|
| What this project is and how to build it | `README.md` |
| **Remaining work** | `docs/TODO.md` |
| What has been verified, and how far | `docs/verification-report.md` |
| The WIT -> Core Wasm lowering rules | `docs/abi-spec.md` (**normative**) |
| Why a technology was chosen | `docs/design-notes.md` |
| **Decisions that must not be changed** | `docs/decisions.md` (D-NN; originally this document's §2) |
| Pitfalls you are likely to hit while implementing | `docs/decisions.md` (P-NN; originally this document's §6) |

---

## 0. Wasmicon in 30 seconds

A WebAssembly execution environment for microcontrollers (ESP32-S3, RP2350; RP2040 has a port only and is not evaluated).

- **Runtime**: a from-scratch Core Wasm interpreter (the existing wasm3 / WAMR / wasmi are not used)
- **HAL**: GPIO / I2C / SPI / time / log defined in WIT. **The Component Model is not used.** WIT serves only as an IDL and is lowered to Core Wasm imports by the rules in `abi-spec.md`
- **Bindings**: Rust and AssemblyScript, generated from WIT. [C bindings were added later, 2026-10.]
- **Goal**: write an app in Rust and in AS that reads an I2C temperature / humidity sensor (SHT4x) and draws on an SPI display (ILI9341), run the **same Wasm binary** on ESP32-S3 and Pico 2 W, and verify that the host call traces match
- **Evaluation board change (owner decision, 2026-10-07)**: the second board was replaced, from Pico WH (RP2040) to **Pico 2 W (RP2350)**. RP2040 is no longer evaluated. The `ports/rp2040` code and its CI build stay, but there is no hardware verification and no I2C / SPI implementation for it. "Pico" and "both boards" in the completion criteria of Phase 4 to 6 below mean ESP32-S3 and Pico 2 W
- **The goal is reached (owner judgment, 2026-10-07)**. sensor-display (Rust / AS) runs on both boards; the traces match across the two boards up to the sensor read, and after the read, each board's trace matches, line for line, a host run fed with that board's response. With this the original goal was declared complete (docs/verification-report.md §11 / §12)

---

## 1. Current state -> moved to `README.md` and `docs/TODO.md`

The state keeps changing, so anything placed here would go stale.
Where we are is in `README.md`; remaining work is in `docs/TODO.md`, which is the source of truth.

---

## 2. Frozen decisions (do not change)

1. **The WIT subset** is exactly docs/abi-spec.md §2. `record` / `variant` / `option` / `tuple` are not used.
2. **Handle representation** is the WIT `resource` syntax. Import names are the same as the Canonical ABI: `[static]r.f` / `[method]r.f` / `[resource-drop]r`.
3. **Error code**: one `types.error-code` shared by the whole HAL. Functions return a single i32 status (0 = success, n>0 = discriminant+1).
4. **The T of `result<T,_>` goes through a trailing out-pointer**; a `list<u8>` return value uses a caller buffer `(buf, cap, len-out)`. `cabi_realloc` is not needed.
5. **Import module name** is `wasmicon:hal/<iface>@0.1.0`. The version is required and must match exactly.
6. **Guest exports** are `run: func()` and `memory`. `_start` / `_initialize` are not used.
7. **Supported feature set**: MVP + sign-ext + nontrapping-fptoint + bulk-memory (memory.* only) + multi-value + mutable-globals. The SIMD / threads / EH / tail-call / GC / reference-types extensions are not supported.
8. **SPI CS/DC are driven by the guest through gpio.** The SPI interface does not handle CS.
9. **The sensor is SHT4x (SHT40 / SHT41 / SHT45)** and the display is ILI9341. (Owner decision, 2026-09-29. Changed from SHT31/SHT30 because the board on hand is an SHT40. Overrides the SHT31 recommendation in docs/design-notes.md §7. **Commands and conversion formulas: apps/README.md §1 is normative.** The only differences from SHT3x are the measurement command and the humidity conversion; the response format, CRC-8 and temperature conversion are identical.)
10. **Definition of the determinism verification**: the trace of every host call and its result except `time` (in the docs/abi-spec.md §9 format) is identical on both boards.
11. **The runtime implementation language is Rust `no_std`** (owner decision, 2026-09-10). Zero dependency crates, no `alloc`, the arena is injected by the port. Overrides the C11 recommendation in docs/design-notes.md §4. The ABI, the WIT and the phase structure are not affected by this change.
12. **From-scratch implementation.** No code from wasm3 / WAMR / wasmi is taken (studying their algorithms is fine, reusing code is not). The core is written by Claude Code (owner decision, 2026-09-10).

---

## 3. Open questions and the defaults Claude Code may adopt

**This table is a record of decisions.** It keeps the items that were unanswered when work started and the defaults that were adopted.
All implementation runs on these defaults.

**Items still waiting for the owner's decision are collected in docs/TODO.md §2.** Nothing is appended here.

| # | Item | Default | Rationale |
|---|---|---|---|
| 1 | Runtime implementation language | **Decided -> §2-11 (Rust `no_std`)** | Owner judgment (2026-09-10): "embedded code can be written safely". The size-reduction measures went into the Phase 2 criteria |
| 2 | Absorbing GPIO number differences between boards | **Adopted (2026-09-10)**. `pin-by-role` in `wasmicon:hal/board@0.1.0`. Originally the role names were a fixed vocabulary (`led` / `lcd-cs` / `lcd-dc` / `lcd-rst`) with a default table in the firmware. **On 2026-10-07 the owner abolished the vocabulary and the default table**: role names are free-form, the wiring is written in the app's `wasmicon.toml`, and `deploy` writes it to the device's config slot (docs/app-workflow.md §3.9) | Required to verify with the same binary |
| 3 | Behavior during `sleep-ms` | Left to the port layer's HAL. host: `std::thread::sleep`; rp2040 / esp32s3: the `Delay` of each HAL crate (depends on #8) | The natural choice |
| 4 | Behavior after a trap | Log and halt (infinite loop / abort). No restart | Easier to debug |
| 5 | UTF-8 validation in `log` | None | As specified |
| 6 | `spi.transfer` | Keep in v0.1 | Low implementation cost |
| 7 | Who implements the core | **Decided -> §2-12 (from scratch, written by Claude Code)** | Owner judgment (2026-09-10) |
| 8 | **Port layer implementation approach** (new with the move to Rust) | **All Rust. host = `std`, rp2040 = `rp-hal` + `cortex-m-rt`, esp32s3 = `esp-hal` (`no_std`). ESP-IDF / Pico SDK are not used** | With the core in Rust, the Xtensa fork toolchain is required anyway, and mixing in a C SDK does not make anything lighter. One language and one toolchain is lighter and safer. **Approved 2026-09-10** (this document §8). All 3 ports implemented |
| 9 | **Splitting the Cargo workspaces** (new with the move to Rust) | **Approved (2026-09-10)**. One workspace per target. root = `runtime` + `tools/wasmicon-gen` + `ports/host`. The guest workspace root is `apps` (with `bindings/rust` among its members). `ports/rp2040` / `ports/esp32s3` are created in Phase 4. `.cargo/config.toml` is looked up **relative to the current directory**, so guests are built with `cd apps && cargo build --release` | In a single workspace the targets, profiles and toolchains collide |

Adopting default 2 added `wit/board.wit`, added `import board;` to `world app`, and updated docs/abi-spec.md §7 / §8 / §10.
At the same time **a normalization rule was added to docs/abi-spec.md §9**. A GPIO number looked up by role differs per board, so printing it as-is would make the traces of the same binary differ. Ports replace the number with the form `role:led`.

If default 8 is rejected (use the C SDKs), the C header (`runtime/include/wasmicon/hal.h`) has to come back as a Phase 1 output and the core has to grow an `extern "C"` API as a `staticlib`.

---

## 4. Repository layout -> moved to `README.md`

The layout can change, so the README is the source of truth. The reason for the separate workspaces is §3 #9.

---

## 5. Phases and completion criteria

Keep the order. Meet each phase's completion criteria before moving to the next.

### Phase 1: the generator `wasmicon-gen`

- Implemented in Rust with the `wit-parser` crate. Port the rules of `tools/wit2sig.py` to Rust and first make a test of matching `wit2sig.py`'s output.
- Outputs: (a) `runtime/src/generated.rs` -- the import table (module, name, sig string, host function slot) and the error-code enum; (b) `bindings/rust/src/generated.rs` -- `#[link(wasm_import_module=...)]` extern declarations and the enum; (c) `bindings/assemblyscript/assembly/generated.ts` -- `@external` declarations and the enum.
- WIT syntax outside the subset is rejected with an error.
- The cross-check against `wit2sig.py` is `tools/check-sigs.sh` (diff against the output of `wasmicon-gen --sigs`). The table in docs/abi-spec.md §7 itself is hard-coded in `tools/wasmicon-gen/tests/abi_spec.rs` and checked there (§7 is normative, so the expected values come from the spec, not from the WIT).
- Generated Rust is run through rustfmt before it is written out. Otherwise `cargo fmt --check` and the zero-diff check cannot both hold.
- **Completion criteria**: the 3 outputs match docs/abi-spec.md §7. CI regenerates on a WIT change and checks for a zero diff. -> **Reached (2026-09-10)**. The root job in `.github/workflows/ci.yml` checks this with `cargo test` (`tests/generated_up_to_date.rs`) and `--check`.
- `flags` is allowed by docs/abi-spec.md §2.1, so generation supports it, but the v0.1 `wit/` has none, so it does not appear in the real output (tests only).
- Remaining: the AssemblyScript output has not been compile-checked with `asc` (to be done in Phase 3 when the AS toolchain is installed).

### Phase 2: the runtime core (on the host PC)

- Rust `no_std`, **zero dependency crates**, no `alloc`. Receives the arena (`&'a mut [u8]`) from the port; internal structures hold indices / offsets into the arena, not raw pointers.
- lint: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` in CI. `unsafe` is confined to a minimum and always has a `// SAFETY:` comment directly above it.
- Size reduction (owner requirement): the root workspace's `[profile.release]` is `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1` (for size measurement). `panic = "abort"` and `strip = true` are not placed here but in the `[profile.release]` of the guest / port workspaces (host tools and `cargo test` need unwind). No `core::fmt` in the core. Types are written concretely so that generic monomorphization does not bloat the code.
  - Size measurement: `sh tools/measure-size.sh`. **As of 2026-09-10, 49.4 KiB for thumbv6m-none-eabi** (`.text` + `.rodata`, upper bound without LTO). Small enough against the RP2040's 2 MB flash
- Structure: `decode` (section parsing; code is kept as a slice `&'static [u8]` in flash) -> `validate` (type checking + building the side table of `br` targets) -> `interp` (stack machine, `match` dispatch).
- **Rust has no computed goto.** Start from a `match` loop. Explicit tail calls (`become`) are unstable, so do not rely on them. Optimize after the spec tests pass and after profiling.
- Wasm arithmetic is wrapping. Use `wrapping_*` / `rotate_*` / `checked_*` explicitly. Keep `overflow-checks` on in debug and fix whatever it catches to the wrapping the spec requires.
- `memory.grow` is limited to the maximum page count given by the port.
- Import resolution: link against the table in the generated `generated.rs` by exact match of module name + name + sig.
- The spec test runner lives in `runtime/tests/`. dev-dependencies are allowed there (read `.wast` directly with the `wast` crate, or use `wasm-tools json-from-wast`). The core itself stays at zero dependencies.
- **Completion criteria**: the WebAssembly spec testsuite (for the supported feature set) passes in full. The `apps/blink-rs` Wasm runs in `ports/host` and produces a mock GPIO trace. -> **Reached (2026-09-10)**.
  - "Passes in full" is defined by `FILES` in `runtime/tests/spec.rs` (74 core files). Exclusions are listed with reasons in `EXCLUDED` in the same file. 22507 commands pass
  - `apps/blink-rs` is created in Phase 3, so a hand-written `ports/host/tests/blink.wat` producing the same host call sequence verifies this first. Replaced by the real app in Phase 3
- Optimization comes after the spec tests pass and after profiling. Target: ILI9341 text drawing on RP2040 visibly finishes within about 1 second.

### Phase 3: Rust / AS bindings + blink

- Rust: a `no_std` crate, `wasm32-unknown-unknown`. Wrap the generated externs in safe wrappers `Pin` / `I2cBus` / `SpiBus` (`[resource-drop]` in `Drop`). Build flags: `-C target-feature=-reference-types,+sign-ext,+nontrapping-fptoint,+bulk-memory,+mutable-globals,+multivalue`, `--initial-memory=65536`, `-z stack-size=8192`, `panic=abort`, `opt-level=z`. **The source of truth is `tools/wasmicon-cli/src/flags.rs`** (`apps/.cargo/config.toml` and the `wasmicon new` scaffold are produced from it; docs/app-workflow.md §4.4).
- AS: `--runtime stub`, `--initialMemory 1`, `--disable simd,threads,exception-handling`, `--enable sign-extension,nontrapping-f2i,bulk-memory,mutable-globals`. For `env.abort`, provide a small shim on the host for AS (converted to a trap).
- **Completion criteria**: confirm with `wasm-tools validate --features=...` that the `blink-rs` and `blink-as` Wasm are within the supported feature set, and that they produce the same GPIO trace in `ports/host`. -> **Reached (2026-09-10)**. `ports/host/tests/apps.rs` builds both, checks the features, and checks that the `gpio` and `board` host call sequences are identical. blink-rs 796 bytes / blink-as 2.1 KB.
- AssemblyScript is built with the npm workspace at the repository root (`npm ci`) + `npx asc`. asc cannot resolve a scoped package (`@wasmicon/hal`) as `~lib`, so the apps import it by relative path.

### Phase 4: ports `rp2040` -> `esp32s3`

- **Pico first** (the more constrained one).
- The port layer approach is §3 #8 (all Rust). Confirm with the owner before starting.
- Pico: `rp2040-hal` + `cortex-m-rt`, `thumbv6m-none-eabi`, `memory` limit 2 pages. The Wasm binary is placed in `.rodata` (flash, XIP) with `include_bytes!` and passed to the runtime as `&'static [u8]`.
  - GPIO is driven dynamically by number, so the SIO / IO_BANK0 / PADS_BANK0 registers are written directly instead of using typed pins. Typed pins are for static assignment and do not fit the `pin.open(index)` form
  - Narrow `Config` for microcontrollers (value stack 512 / nesting 64 / locals 256 / call depth 32). With the host defaults, the validation work area is several hundred KB and does not fit in SRAM
  - **As of 2026-09-10: builds. Not yet verified on hardware** (flash 56 KB / RAM 172 KB)
- ESP32-S3: `esp-hal` (`no_std`), the Xtensa toolchain installed by espup (`channel = "esp"` in `rust-toolchain.toml`), limit 4 pages (no PSRAM).
  - No prebuilt core is distributed for Xtensa, so `.cargo/config.toml` specifies `build-std = ["core"]`
  - The linker (`xtensa-esp32s3-elf-gcc`) is installed by espup but is not on PATH. `ports/esp32s3/build.sh` sources `~/export-esp.sh` before calling cargo
  - GPIO is written through registers directly, as on RP2040 (IO_MUX MCU_SEL=1, GPIO matrix out_sel=128)
  - The arena is 300 KB. At 320 KB the linker fails with "cannot move location counter backwards" (the effective DRAM limit)
  - **As of 2026-09-10: builds. Not yet verified on hardware** (.text 80 KB / .bss 405 KB)
- Traces: a build with the cargo feature `trace` enabled writes the docs/abi-spec.md §9 format to serial (UART / USB-CDC).
- The board configuration (the table in docs/abi-spec.md §8) is collected in `ports/<board>/src/board.rs`, which implements `board.pin-by-role`.
- **Completion criteria**: `blink-rs` / `blink-as` run on both boards (ESP32-S3 and Pico 2 W) and the serial trace matches the host version. -> **blink not done** (not part of the goal judgment; the owner declared the goal reached on 2026-10-07). GPIO works on hardware on both boards through `lcd-demo-rs` / `sensor-display`, but blink (an external LED on the `led` role) has not been flashed yet (docs/TODO.md §1.3).

### Phase 5: the sensor + display app

- SHT4x: the address, measurement command, wait time, CRC-8 and conversion formulas are **normative in apps/README.md §1** (copying them here would be duplicate maintenance; this already happened once with `ili9341.rs` in three places -> docs/TODO.md §2.1). CRC-8 is verified in the guest. **The display values are computed in fixed point (integers x100); floating point is used deliberately in exactly one place (an f32 conversion)** as the subject of the determinism verification.
- ILI9341: the init sequence is the usual one (SWRESET, SLPOUT, PIXFMT=0x55 (RGB565), MADCTL, DISPON). Drawing has only 2 primitives, "fill rectangle" and "draw string in an 8x8 bitmap font". Row-sized buffers (at most 320x8x2 = 5 KB) are sent with `spi.write`. There is no full-screen framebuffer (it does not fit on RP2040).
- The Rust and AS versions draw **the same result**, so the font and coordinates are a shared spec (written in `apps/README.md`). -> **Written** (137 lines). When one side changes, change the other.
- **Completion criteria**: the display works in all 4 combinations (Rust/AS x ESP32-S3/Pico 2 W). -> **Reached (2026-10-07)**. All 4 combinations show the display on hardware, and each matched in full the trace of the host fed with the response the board read (docs/verification-report.md §12).
  - Both guests run in `ports/host` and a test checks that **the full trace (1215 lines at the time; 4183 lines after the screen redesign of 2026-10-05) is identical** (`ports/host/tests/apps.rs`). By docs/abi-spec.md §9, `time` does not appear in the trace, so full identity is exactly the §2-10 definition "every host call and result except `time` matches". The `spi.write` trace is the CRC-32 of the data, so a match means "the pixels being sent are identical"
  - The sensor is fed the recorded responses in `verify/sht4x-replay.txt` through `WASMICON_I2C_REPLAY`. Replaced on 2026-10-07 with responses recorded from hardware (the SHT40 on the Pico 2 W)

### Phase 6: cross-board verification

- In `verify/`: (1) a mock I2C mode that returns recorded SHT4x responses (`WASMICON_I2C_REPLAY` in the port layer), (2) a script that diffs the traces of the 2 boards, (3) a differential test of `ports/host` against wasmtime (interpreter correctness). -> **All 3 built (2026-09-10)**.
  - (1) `WASMICON_I2C_REPLAY` + `verify/sht4x-replay.txt`. The responses are **synthetic data**; replacing them with ones recorded from hardware remains
  - (2) `verify/diff-traces.sh`. Drops the serial banner and the guest's `[wasm]` lines and compares only the `> ` / `< ` lines. `--self-test` checks that the normalization is correct
  - (3) `verify/differential/`. **Shares the HAL** and runs the same `.wasm` on wasmtime 45 and on our interpreter. `Resolver::call` does not depend on the engine, so any difference is an interpreter bug. Identical for all 4 guests
- **Completion criteria**: run the same `.wasm` (the Rust version and the AS version) on both boards (ESP32-S3 and Pico 2 W); the trace except `time` and the SPI pixel CRCs are identical. Summarize the results in `docs/verification-report.md`. -> **Reached (owner judgment, 2026-10-07)**.
  - `lcd-demo-rs` (no sensor): 14,352 lines identical on the 2 boards (§6 / §7 / §8)
  - `sensor-display`, both Rust and AS: **every line up to the sensor read** matches on the 2 boards. After the read, **each board reads a different value**, so there is no literal full identity. Instead, it was confirmed in all 4 combinations that each board's trace matches in full "the host fed with the response that board read" (§11 / §12). **There is no way to inject the same response into both boards**, so the direct comparison of the 2 boards is indirect, through "each board = host". **The owner declared this complete** (2026-10-07)

---

## 6. Pitfalls and cautions

### Guest side

- **Rust's wasm32 default features**: recent rustc enables `reference-types` / `multivalue`. Read the `call_indirect` table index as LEB128 and reject anything but 0 (note in docs/abi-spec.md §6.1).
- **AS imports `env.abort`**: it can remain even with `--noAssert`. Provide `env.abort(msg, file, line, col)` on the host and convert it to a trap. Document it as the one exceptional import that is not in `world app`.

### Runtime side (Rust)

- **Float methods missing in `no_std`**: `floor` / `ceil` / `trunc` / `round` / `sqrt` and friends are not in `core` (they are in `std` or `libm`). With zero dependency crates they are implemented by hand. `f32.sqrt` is a mandatory Wasm instruction, so it cannot be avoided. `f32.nearest` (round half to even), NaN propagation in `min`/`max`, `copysign` and `trunc_sat` are also written by hand to the spec. `float_misc.wast` and `conversions.wast` in the spec tests are the hard cases.
- **RP2040 has no FPU and no hardware divide**: f32/f64 become `compiler_builtins` soft-float. IEEE compliance is kept but it is slow.
- **The ESP32-S3 FPU is f32 only**: f64 is soft-float. Some Xtensa FPU configurations cannot handle denormals, so run the denormal cases of the spec tests on the ESP32-S3 hardware too.
- **Reading bytecode directly from XIP**: RP2040 (Cortex-M0+) HardFaults on unaligned word access. Reads from a slice must always take the form `u32::from_le_bytes(bytes[i..i+4].try_into().unwrap())`; never use `ptr::read` / `align_to` / reference casts (HardFault and UB). Read LEB128 byte by byte.
- **Range checks on `memory`**: range-check `list<u8>` arguments and out-pointers on every host call. Use `checked_add` and `slice::get`; do not create silent truncation through `as` casts.
- **Panic messages remain even with `panic = "abort"`**: `panic!` and the implicit bounds checks of slices keep strings in the binary. Design the core to return `Result` and handle hot paths explicitly with `get()`. `panic_immediate_abort` needs build-std, so do not rely on it.
- **`core::fmt` is the main size cost**: no `write!` / `{:?}` / `Display` in the core. Trace output is written in the port layer, directly as bytes.

### Toolchain

- **Xtensa is not supported by upstream Rust**: ESP32-S3 needs the fork installed by espup (`channel = "esp"`). Building ESP32-S3 in CI would need the espup installation steps. RP2040 only needs `rustup target add thumbv6m-none-eabi`.

### Common

- **Handle 0 is invalid** (docs/abi-spec.md §5.1). Do not use table index 0.
- Do not forget **error code = discriminant + 1**. Use only the generated constants; never write literal numbers.
- **Determinism**: the order, arguments and results of every host call except `time` must match on both boards. Do not create behavior in the port layer that "happens to succeed" (for example reading an uninitialized GPIO).

---

## 7. Command cheat sheet -> moved to `README.md`

The README is the source of truth for build and verification commands. Anything that existed only here was moved to the README.

---

## 8. When the owner had to be asked (record)

Points where the owner's answer was needed to advance a phase. **Those still unanswered are collected in
docs/TODO.md §1.1.** Nothing is appended here.

- ~~Before Phase 1: approval of §3 defaults #1 (implementation language) and #7 (who implements)~~ -> answered 2026-09-10. §2-11 / §2-12
- ~~Before Phase 1: §3 #9 (splitting the Cargo workspaces)~~ -> approved 2026-09-10
- ~~Before Phase 4: approval of §3 #8 (all-Rust port layer)~~ -> the instruction "proceed to Phase 4" was taken as approval; all 3 ports implemented
- Hardware wiring (docs/abi-spec.md §8), how serial is connected, the ILI9341 model -> **confirmed (2026-10-07)**: I2C / SPI / UART0 work on both boards, and the ILI9341 (3.3V SPI version) shows the display. Only the external LED on `led` is unconfirmed (docs/TODO.md §1.1)
- Sensor model -> **answered (2026-09-29)**: the board on hand is an SHT40. Decision 9 in §2 was rewritten to SHT4x
- Evaluation board -> **answered (2026-10-07)**: RP2040 (Pico WH) is removed from evaluation and replaced by Pico 2 W (RP2350). `ports/rp2040` stays. Recorded in §0

---

## 9. References

- Component Model Canonical ABI (source of the import naming and flat types): https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md
- wit-parser: https://crates.io/crates/wit-parser
- WebAssembly spec testsuite: https://github.com/WebAssembly/testsuite
- Prior work: Wasefire (Google), WAMR, wasm3, wasmi (docs/design-notes.md §1)
