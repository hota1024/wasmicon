# Decisions

The frozen decisions, adopted defaults and known pitfalls that code and tests
rely on. Each entry has a stable ID; cite it as `docs/decisions.md D-07`.
The entries were moved here from docs/handoff.md §2, §3 and §6, which is kept
as a historical record. Changing a `D` entry needs the owner's approval; write
up the reason instead of implementing.

## Frozen decisions (D)

| ID | Decision |
|---|---|
| D-01 | The WIT subset is exactly docs/abi-spec.md §2. No `record`, `variant`, `option` or `tuple`. |
| D-02 | Handles use the WIT `resource` syntax. Import names follow the Canonical ABI: `[static]r.f`, `[method]r.f`, `[resource-drop]r`. |
| D-03 | One error type for `wasmicon:hal` and `wasmicon:device`, `wasmicon:hal/types.error-code`. Every function returns a single `i32` status: 0 = success, n > 0 = discriminant + 1. |
| D-04 | The `T` of `result<T, _>` is returned through a trailing out-pointer. A `list<u8>` return value uses a caller buffer `(buf, cap, len-out)`. There is no `cabi_realloc`. |
| D-05 | The import module name is `wasmicon:hal/<iface>@0.1.0`, or `wasmicon:device/<iface>@<version>` for the device layer, whose version moves independently (owner decision, 2026-10-27). The version is required and must match exactly. |
| D-06 | The guest exports `run: func()` and `memory`. `_start` and `_initialize` are not used. |
| D-07 | Supported feature set: MVP + sign-ext + nontrapping-fptoint + bulk-memory (`memory.*` only) + multi-value + mutable-globals. SIMD, threads, exception handling, tail calls, GC and reference types are not supported. |
| D-08 | The guest drives SPI CS and DC through `gpio`. The SPI interface does not handle CS. |
| D-09 | The sensor is SHT4x (SHT40 / SHT41 / SHT45) and the display is ILI9341 (owner decision, 2026-09-29). This is the display of the HAL evaluation apps; through `wasmicon:device/display` the ILI9341 is one backend among several (2026-10-27). Commands and conversion formulas: apps/README.md §1 is normative. |
| D-10 | Determinism means: the trace of every host call and its result, except `time`, in the docs/abi-spec.md §9 format, is identical on both boards. |
| D-11 | The runtime is Rust `no_std` with zero dependency crates and no `alloc`. The arena is injected by the port (owner decision, 2026-09-10). |
| D-12 | The runtime is written from scratch. No code from wasm3, WAMR or wasmi is taken (studying their algorithms is fine). The core is written by Claude Code (owner decision, 2026-09-10). |
| D-13 | Devices that no HAL interface reaches get the device layer, the package `wasmicon:device` (docs/device-layer.md). The host owns the device and the guest uses it. An interface enters only with at least two backends, counting the host mock, of which one runs on an evaluation board (owner decision, 2026-10-27). |

## Adopted defaults (A)

Defaults chosen when a question was open at the start. All implementation runs
on them. Open questions that still wait for the owner live in docs/TODO.md §2.

| ID | Default | Status |
|---|---|---|
| A-01 | Runtime implementation language. | Decided: see D-11. |
| A-02 | Board-specific GPIO numbers are resolved through `board.pin-by-role` in `wasmicon:hal/board@0.1.0`. Originally a fixed vocabulary (`led`, `lcd-cs`, `lcd-dc`, `lcd-rst`) with a default table in the firmware. Role-derived numbers are normalized to `role:<name>` in traces (docs/abi-spec.md §9). | **Superseded 2026-10-07**: role names are free-form, the wiring lives in the app's `wasmicon.toml`, and `deploy` writes it to the device's config slot (docs/app-workflow.md §3.9). |
| A-03 | Behavior during `sleep-ms` is left to the port HAL: `std::thread::sleep` on host, the HAL crate's `Delay` on boards. | In force. |
| A-04 | After a trap: log the reason and halt (infinite loop or abort). No restart. | In force. |
| A-05 | `log` does not validate UTF-8. | In force. |
| A-06 | `spi.transfer` stays in v0.1. | In force. |
| A-07 | Who implements the core. | Decided: see D-12. |
| A-08 | The port layer is all Rust: host = `std`, rp2040 / rp2350 = `rp-hal` + `cortex-m-rt`, esp32s3 / esp32c5 / esp32p4 = `esp-hal` (`no_std`). ESP-IDF and the Pico SDK are not used. | Approved 2026-09-10. |
| A-09 | One Cargo workspace per target: root = `runtime` + `tools/*` + `ports/common` + `ports/host`; guest root = `apps` (includes `bindings/rust`); each `ports/<board>` and `verify/differential` are separate. `.cargo/config.toml` is looked up from the current directory, so build guests with `cd apps && cargo build --release`. | Approved 2026-09-10. |
| A-10 | Read-only assets (a WAD, fonts, images) are a third flash slot right after the config slot, running to the end of flash, written only by `wasmicon assets write` and read by the guest through `wasmicon:hal/assets@0.1.0` (`blob.open` / `len` / `read` by range, at most four open; docs/abi-spec.md §7, docs/app-workflow.md §3.10). Names are 1 to 32 bytes of `a-z` / `0-9` / `-` / `_` / `.`, the table is flat (no directories), the directory's CRC is checked at boot and a body's CRC once at its first `open`. `error-code` gained `not-found` for it. A read-write store (`storage`) is a separate, later interface. | Approved 2026-10-22. |
| A-11 | On the RP2350 and the RP2040, USB is a composite device with two CDC ACM interfaces: a console that carries plain output lines and the control channel of docs/app-workflow.md §3.6. A plain serial terminal reads the console; frames never share a port with lines (docs/app-workflow.md §3.5, docs/TODO.md §5-11). The ESP32-S3's USB-Serial-JTAG has one fixed CDC interface, so this does not apply there. Whether the log becomes a port requirement and whether the trace goes to USB stay open in docs/TODO.md §5-11. | Approved 2026-10-29. |

## Pitfalls (P)

### Guest

| ID | Pitfall |
|---|---|
| P-01 | Recent rustc enables `reference-types` and `multivalue` on wasm32 by default. Read the `call_indirect` table index as LEB128 and reject anything but 0 (docs/abi-spec.md §6.1). |
| P-02 | AssemblyScript imports `env.abort` even with `--noAssert`. The host provides a shim that turns it into a trap. It is the one import outside `world app`. |

### Runtime

| ID | Pitfall |
|---|---|
| P-03 | `core` has no `floor`, `ceil`, `trunc`, `round` or `sqrt`. With zero dependencies they are implemented by hand. `f32.sqrt` is a mandatory instruction; `nearest` rounds half to even; `min` / `max` propagate NaN; `copysign` and `trunc_sat` follow the spec. `float_misc.wast` and `conversions.wast` are the hard cases. |
| P-04 | RP2040 has no FPU and no hardware divide: f32 / f64 go through `compiler_builtins` soft-float. IEEE-correct but slow. |
| P-05 | The ESP32-S3 FPU is f32-only; f64 is soft-float. The Xtensa FPU may not handle denormals in some configurations, so the denormal spec cases are run on hardware too. |
| P-06 | Bytecode is read directly from XIP flash. Cortex-M0+ HardFaults on unaligned word access. Read through `u32::from_le_bytes(bytes[i..i + 4].try_into()...)`; never use `ptr::read`, `align_to` or reference casts (HardFault and UB). Read LEB128 byte by byte. |
| P-07 | Range-check `list<u8>` arguments and out-pointers on every host call with `checked_add` and `slice::get`. No silent truncation through `as`. |
| P-08 | `panic = "abort"` still keeps panic messages and implicit bounds-check strings in the binary. The core returns `Result` and uses `get()` on hot paths. `panic_immediate_abort` needs build-std and is not relied on. |
| P-09 | `core::fmt` is the main size cost. The core uses no `write!`, `{:?}` or `Display`. Trace output is written byte-wise in the port layer. |

### Toolchain

| ID | Pitfall |
|---|---|
| P-10 | Xtensa is not supported by upstream Rust. ESP32-S3 needs the espup fork (`channel = "esp"`). RP2040, RP2350, ESP32-C5 and ESP32-P4 only need `rustup target add`. |

### Common

| ID | Pitfall |
|---|---|
| P-11 | Handle 0 is invalid (docs/abi-spec.md §5.1). Never use table index 0. |
| P-12 | Error code = discriminant + 1. Use only the generated constants, never literal numbers. |
| P-13 | Determinism: the order, arguments and results of every host call except `time` must match across boards. Ports must not have behavior that "happens to succeed", such as reading an uninitialized GPIO. |
