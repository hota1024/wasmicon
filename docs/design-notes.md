# Wasmicon design notes

> **Historical record.** Kept as written at the time and translated from Japanese; section numbers are stable. Current rules: `docs/decisions.md`. Current status: `README.md`, `docs/TODO.md`.

Created: 2026-09-10 (§8 updated the same day, when ABI spec v0.1 was written)
Status: original concept (2026-09). Superseded in part; see docs/decisions.md D-09 (sensor), D-11 (runtime language), A-08 (port layer). It collects the concept and the judgments that had to be made before implementation.

## 1. Concept

- Conventional MCU development environments differ per hardware and are a burden to manage
- WebAssembly can be compiled from many languages and is a fast, safe instruction set. It can run on MCUs
- But WebAssembly has no standard API for accessing MCU peripherals
  - The wasi-i2c / wasi-spi / wasi-gpio proposals of the Bytecode Alliance embedded WG have stalled at Phase 1
  - They assume the Component Model. The Canonical ABI requires cabi_realloc and resource tables, which is heavy for the RP2040 class
- So a lightweight API layer is needed that reaches peripherals directly from a Core Wasm runtime

### Components

- **Wasmicon Runtime**: a from-scratch Core Wasm interpreter, light enough to run on ESP32-S3 / RP2040
- **Wasmicon HAL**: GPIO / SPI / I2C (plus time / log) defined in WIT. The Component Model is not used; WIT is used only as an IDL, with a limited subset of types and syntax
- **Wasmicon Bindings**: Rust, AssemblyScript

### Goal

Implement, in Rust and in AssemblyScript, an app that reads values from an I2C temperature / humidity sensor and draws them on an SPI-connected ILI9341. Run it on ESP32-S3 and Raspberry Pi Pico WH and verify that the results are the same.

### Comparison and prior work

- **Wasefire (Google)**: defines an applet API as Core Wasm imports. The same decision not to use the Component Model
- **WAMR**: supports ESP-IDF. The representative existing runtime
- **wasm3**: a lightweight interpreter. Closure-compilation approach (maintenance is slowing down)
- **wasmi**: written in Rust, works in no_std. Register-machine translation approach

What sets Wasmicon apart is the combination: WIT as the IDL with its own lightweight ABI / a from-scratch runtime / both Rust and AS bindings / determinism verification across several MCUs.

## 2. The Wasmicon ABI (the first thing to decide)

Spell out how each WIT type is lowered to a Core Wasm function signature. If this is ambiguous, the Rust bindings, the AS bindings and the runtime's host function table drift apart and breed bugs.

-> The concrete spec is in `abi-spec.md`. Below is a summary of the approach.

### WIT types in use (the subset)

- bool, u8, u16, u32, s32, u64, f32, f64
- enum, flags
- list<u8>
- resource (treated as a handle)

### Lowering rules

| WIT | Core Wasm |
|---|---|
| bool, u8, u16, u32, s32, enum, flags | i32 |
| u64 | i64 |
| f32 / f64 | f32 / f64 |
| list<u8> | 2 arguments (ptr: i32, len: i32). Same as the Canonical ABI |
| resource | u32 handle (i32). [method] functions get mechanically generated import names such as `[method]bus.write` |
| result<T, error-code> | returns a single i32 error code; T goes through an out-pointer. A deliberate deviation from the Canonical ABI (which uses a retptr). Deviations are documented explicitly |

Design policy: stay within a subset of the Canonical ABI where possible, so that a future bridge to the Component Model can be a mechanical conversion. Deviations are stated explicitly.

### Generator

WIT is the single source of truth. A small generator built on the `wit-parser` crate (the WIT parser inside wit-bindgen, usable on its own) generates all of the following at once:

- the import table for the runtime (Rust; `runtime/src/generated.rs`)
- the Rust `extern "C"` declarations (`#[link(wasm_import_module = "wasmicon:hal/i2c@0.1.0")]`)
- the AssemblyScript `@external` declarations

## 3. HAL interface design

Required interfaces: gpio, spi, i2c, plus **time** (sleep-ms, now-us) and **log** (debug output). The ILI9341 needs DC/RST/CS GPIOs and SPI bulk writes, and the sensor needs a measurement wait, so time is mandatory.

API granularity principle: the interpreter overhead is paid per host call. SPI writes are not "one host call per pixel" but "assemble a buffer in linear memory and transfer it at once with `spi.write(list<u8>)`". The API granularity directly determines performance.

Framebuffer: 320x240x2 = 150KB does not fit on RP2040. Draw with small row- or rectangle-sized buffers, and keep a tiny drawing library on the guest side: a small font and fill-rect, about that much.

## 4. Runtime implementation approach

### Implementation language

**Decision (2026-09-10, owner judgment): Rust `no_std`**. The reason: "embedded development can be done safely". The condition: **keep it as light as possible**.

- Zero dependency crates, no `alloc`, the arena is injected by the port layer (the no-malloc policy is unchanged)
- `panic = "abort"` / `opt-level = "z"` / `lto` / `codegen-units = 1`. No `core::fmt` in the core
- Dispatch starts as a `match` loop (Rust has no computed goto). Optimize after profiling

Rejected option: **C11, zero dependencies**. It had the advantages that both platforms' official SDKs could be used as-is and that GCC's computed goto makes dispatch fast, but safety was given priority.

The cost of choosing Rust (accepted):
- ESP32-S3 (Xtensa) is not supported by upstream Rust. A fork toolchain via espup is needed. RP2040 works as-is with upstream `thumbv6m-none-eabi`
- Without computed goto, dispatch speed starts from a plain `match`
- In `no_std` some float methods (`floor` / `ceil` / `trunc` / `sqrt` and others) are not in `core`. With zero dependencies they are implemented by hand

### Interpreter design

- Full validation at load time (required by the spec; the basis of safety)
- Start with the "direct execution of validated bytecode" style: a side table of precomputed `br` targets built during validation
- wasm3's closure compilation and wasmi's register-machine translation are fast but complex. Pass the full spec test first; optimize after profiling
- The code section is not copied to RAM; pointers into flash (memory-mapped through XIP on both boards) are used directly. Only the side table, globals, table and linear memory live in RAM

### Memory constraints

- RP2040: 264KB SRAM, Cortex-M0+ 133MHz, no FPU, no hardware divide
- ESP32-S3: 512KB SRAM (plus optional PSRAM), Xtensa LX7 240MHz, hardware FPU for f32 only
- Of the 64KB linear memory pages, 1 to 2 is the limit on RP2040
  - Rust: narrow `--initial-memory` and `-z stack-size`
  - AS: `--initialMemory`, `--runtime stub`

### Supported feature set

Baseline: **MVP + sign-extension + non-trapping float-to-int + bulk-memory + multi-value + mutable-globals**

Reason: recent Rust wasm32-unknown-unknown targets emit these enabled by default. With MVP only, Rust's output does not run (it can be suppressed with `-C target-cpu=mvp`, but supporting them outright is healthier).

Not needed: SIMD, threads, exceptions, GC, tail-call

## 5. How determinism is verified

RP2040 has no FPU; ESP32-S3 has a hardware FPU for f32 only. Wasm floating point must be IEEE-compliant and deterministic, so f32.nearest, the NaN handling of min/max, trunc_sat, and the 32-bit implementation of i64 arithmetic are exactly where differences are likely to show up.

### Trace comparison

Compare the trace of host calls, not what the LCD looks like.

- Add a **trace mode** to the HAL port layer that writes every I2C/SPI transaction (bytes sent, bytes received; timestamps excluded) to serial
- Include the CRC of the pixel data sent over SPI and diff between the two boards
- The raw sensor values differ physically, so during verification replace I2C with fixed responses (or replay of recorded transactions) and mechanically confirm that the module binary, inputs and outputs are identical

### Host PC port

Build a host port with a mock HAL and run differential tests against wasmtime as the reference implementation. CI can run it too.

## 6. Roadmap

1. Write the ABI spec and the WIT (**done**: `abi-spec.md`, `wit/`)
2. Finish the runtime on the host PC until the spec tests (converted with wast2json) pass in full
3. Port to the Pico first (doing the more constrained one first tightens the design)
4. Port to ESP32-S3
5. Generator and bindings (Rust / AS)
6. Apps: blink -> I2C sensor -> SPI display
7. Cross-board verification (trace comparison)

Note: the detailed order of the implementation phases and their completion criteria are in docs/handoff.md §5, which is the source of truth (restructured to put the generator first).

## 7. Parts selection

- Recommended temperature / humidity sensor: **SHT4x (SHT40 / SHT41 / SHT45)**. No init sequence, a single 1-byte command reads 6 bytes, comes with CRC-8, and is a straightforward subject for determinism verification. Works at 3.3V on both boards
  - **SHT31 / SHT30 was recommended originally.** Changed to SHT4x on 2026-09-29 (the sensor on hand was an SHT40; docs/handoff.md §2 decision 9, now docs/decisions.md D-09). All of the advantages above hold for SHT3x too, so either is fine if available. **But the code assumes SHT4x** (the measurement command and the humidity conversion formula differ)
- Display: ILI9341 (SPI)

## 8. Decided and open items

Decided (2026-09-10):
- resources are written with the WIT `resource` syntax. Import names follow the Canonical ABI
- the error code is one `types.error-code` shared by the whole HAL
- **the runtime implementation language is Rust `no_std`** (owner decision; see §4)
- **from-scratch implementation; the core is written by Claude Code** (owner decision)

Open (remaining work: `docs/TODO.md` is the source of truth; the adopted defaults are recorded in docs/handoff.md §3, now docs/decisions.md A-01 to A-09):
- how to absorb GPIO number differences between boards (docs/abi-spec.md §8; now docs/decisions.md A-02)
- the port layer implementation approach (all Rust, or C SDK + FFI; now docs/decisions.md A-08)
