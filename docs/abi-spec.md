# Wasmicon ABI specification v0.1 (draft)

Created: 2026-09-10
Scope: `wasmicon:hal@0.1.0` (the WIT definitions under `wit/`)
Related: `design-notes.md`

This document defines how the Wasmicon HAL, written in WIT, is lowered to the **imports and exports of a Core WebAssembly module**. The Component Model is not used. WIT serves only as the IDL; the rules here drive the mechanical generation of the Rust bindings, the AssemblyScript bindings, the C bindings and the runtime's host function table.

Terms: "guest" = the Wasm module; "host" = the Wasmicon runtime and its port layer.

---

## 1. Design principles

1. **WIT is the source of truth.** The three bindings are never maintained as hand-written copies; they are generated from the WIT.
2. **Stay within a subset of the Canonical ABI.** Use the same lowering as the Component Model's Canonical ABI wherever possible, so that a bridge to components stays open. Deviations are stated explicitly in §4.
3. **Require no allocator on the guest side.** No `cabi_realloc`. The host never allocates guest memory. Variable-length data is always written into a buffer the caller (the guest) provides.
4. **Coarse-grained host calls.** The interpreter's overhead is paid per call, so bulk-transfer APIs are the default.
5. **Determinism.** For the same module and the same inputs, every platform produces the same sequence of host calls with the same results.

---

## 2. The WIT subset in use

The Wasmicon WIT uses only the syntax and types below. The generator rejects anything else.

### 2.1 Allowed types

| Category | Types |
|---|---|
| Scalars | `bool`, `u8`, `u16`, `u32`, `s32`, `u64`, `s64`, `f32`, `f64` |
| Enumerations | `enum` (at most 256 cases) |
| Bit sets | `flags` (at most 32 cases) |
| Byte strings | `list<u8>` |
| Strings | `string` (**argument position only**) |
| Handles | `resource` (methods, static functions, implicit drop) |
| Results | `result<T, error-code>`, `result<_, error-code>` (`T` is one of the scalars above, an enum, a resource handle or `list<u8>`) |

### 2.2 Types and syntax not used

`record`, `variant`, `option`, `tuple`, `list<T>` (other than `u8`), `string` return values, explicit `own<T>` / `borrow<T>`, `constructor` (it cannot return a `result`, so a static `open` is used instead), `char`, `s8`, `s16`, nested `result`.

These were judged unnecessary for v0.1. Lowering rules will be added when one is needed.

### 2.3 Syntactic constraints

- All interfaces live in the single package `wasmicon:hal@X.Y.Z`.
- `world app` describes the guest's complete set of imports and exports.
- Shared types live in `interface types`, and each interface pulls them in with `use types.{...}`.
- Doc comments (`///`) are copied verbatim into the comments of the generated files.

---

## 3. Naming rules

### 3.1 Import module names

Each WIT interface maps to one Core Wasm import module name. The form is the same as the Canonical ABI:

```
<namespace>:<package>/<interface>@<version>
```

Examples: `wasmicon:hal/gpio@0.1.0`, `wasmicon:hal/i2c@0.1.0`

The version is **always included**. The runtime resolves it by exact match and refuses to instantiate a module that imports an unknown module name or function name (link error).

### 3.2 Import field names

| WIT | Core Wasm import name |
|---|---|
| Free function `foo: func(...)` | `foo` |
| Static function `open` of resource `r` | `[static]r.open` |
| Method `write` of resource `r` | `[method]r.write` |
| Drop of resource `r` (implicit) | `[resource-drop]r` |

This too is the same naming as the Canonical ABI.

### 3.3 Exports

The exports of `world app` are exported by the guest module under their own names. In v0.1:

| Export name | Type | Description |
|---|---|---|
| `run` | `func() -> ()` | Entry point. The host calls it once, right after instantiation |
| `memory` | `memory` | Linear memory. **Required.** The host uses it to read and write `list<u8>` / `string` arguments and out-pointers |

`_start`, `_initialize` and `cabi_realloc` are not used (they are ignored if exported).

---

## 4. Type lowering rules

### 4.1 Scalars, enums and flags (arguments)

| WIT | Core Wasm | Value representation |
|---|---|---|
| `bool` | `i32` | 0 = false, anything else = true (host to guest is always 0/1) |
| `u8`, `u16`, `u32` | `i32` | Zero-extended. For `u8`/`u16` the host **ignores** (masks off) the upper bits outside the range |
| `s32` | `i32` | As is |
| `u64`, `s64` | `i64` | As is |
| `f32`, `f64` | `f32`, `f64` | As is |
| `enum` | `i32` | Discriminant in declaration order, starting at 0. Out of range: the host returns `invalid-argument` |
| `flags` | `i32` | Bits assigned from bit 0 in declaration order. An undefined bit set: `invalid-argument` |
| Resource handle | `i32` | See §5 |

Same as the Canonical ABI.

### 4.2 `list<u8>` and `string` (arguments)

Expanded into **two `i32`s**, `(ptr: i32, len: i32)`. `ptr` is a byte offset into the guest's `memory`. Same as the Canonical ABI.

- `string` is a UTF-8 byte sequence. The host does **not** validate UTF-8 (the string is only used for log output).
- The host checks that `[ptr, ptr+len)` lies inside `memory` and **traps** if it does not (not an error code: this is a guest bug and is treated as part of Wasm memory safety).
- `len = 0` is allowed and the host does not dereference `ptr`. A transfer length of 0 for I2C/SPI, however, returns `invalid-argument`.
- The host **only reads** the argument buffer and never modifies it.
- The host does not retain `ptr` after the host call returns.

### 4.3 Return values (functions without `result`)

| WIT | Core Wasm |
|---|---|
| No return value | No result |
| Scalar / enum / flags | The one corresponding value (same representation as §4.1) |

Example: `now-us: func() -> u64` -> `(func (result i64))`

### 4.4 `result<T, error-code>` (**deviation from the Canonical ABI**)

> In the Canonical ABI, `result<T, E>` is flattened into multiple values, and two or more values go through a retptr. For ease of use on an MCU, Wasmicon unifies this as follows.

The function **always returns a single `i32`**, which is the status code.

- `0` = success (`Ok`)
- `n > 0` = error; `n - 1` is the discriminant of `error-code` (for example `timeout` has discriminant 3, so its status is `4`)

When `T` is present (that is, not `result<_, ...>`), `T` is handed to the guest through **an out-pointer appended at the end of the argument list**.

| `T` | Appended arguments | What the host writes |
|---|---|---|
| `bool`, `u8`, `u16`, `u32`, `s32`, `enum`, `flags` | `out: i32` | A **u32 (4 bytes, little-endian)** at `out`. `bool` is 0/1; `u8`/`u16` are zero-extended to 4 bytes |
| `u64`, `s64` | `out: i32` | 8 bytes little-endian |
| `f32` / `f64` | `out: i32` | IEEE 754, 4 / 8 bytes little-endian |
| Resource handle | `out: i32` | The u32 handle, 4 bytes |
| `list<u8>` | `buf: i32, cap: i32, len-out: i32` | At most `cap` bytes into `[buf, buf+cap)`, and the number of bytes actually written into `len-out` as a u32 |

Rules:
- The out-pointer is written only on success. On error its contents are undefined (the host need not write it).
- The out-pointer and `[buf, buf+cap)` must lie inside `memory`; out of range traps. No alignment is required (the host writes byte by byte).
- In a function that returns `list<u8>`, if the requested length (such as a `len` argument) exceeds `cap`, the host returns `invalid-argument`. The guest is always responsible for passing a sufficient `cap`.
- In v0.1, `i2c.read` / `i2c.write-read` / `spi.transfer` / `assets.blob.read` always set `len-out` equal to the requested length on success. `len-out` exists for future variable-length APIs (UART receive and the like).

### 4.5 Summary of deviations

| Item | Canonical ABI | Wasmicon ABI |
|---|---|---|
| `result<T, E>` | Flattened; multiple values through a retptr | A single `i32` status plus a trailing out-pointer |
| `list<u8>` return value | The host allocates through `cabi_realloc` and returns (ptr, len) | A caller buffer `(buf, cap, len-out)` |
| `string` return value | The host allocates through `cabi_realloc` and returns (ptr, len) | Not used |
| `cabi_realloc` | Required (when there are variable-length return values) | Not needed |
| Everything else (scalars, enums, flags, list arguments, handles, import names) | - | **Identical** |

Should a bridge to the Component Model be needed later, only adapters for `result` and `list<u8>` return values would have to be written.

---

## 5. Resources and handles

### 5.1 Representation

A resource handle is an `i32` (interpreted as an unsigned u32). The meaning of the value is up to the host implementation, with these guarantees:

- `0` is reserved as the **invalid handle**. The host never returns `0` as a valid handle. The guest can use `0` as a "not acquired" sentinel.
- Handle namespaces are **independent per resource type**. `1` of `gpio.pin` and `1` of `i2c.bus` are unrelated.
- The host keeps a handle table per type and returns `invalid-handle` (no trap) for a handle of the wrong type, out of range, or already dropped.

### 5.2 Lifecycle

- A successful static `open` returns a new handle. The guest owns it.
- Calling `[resource-drop]r` `(func (param i32))` makes the host release the peripheral (a GPIO returns to input with no pull; a bus is disabled). Dropping an invalid handle is **ignored** (neither an error nor a trap).
- When `run` returns, the host drops every handle that is still open.
- Opening the same peripheral or pin twice gives `busy`.

### 5.3 Handle limits

The host guarantees at least this many simultaneous handles per type: `gpio.pin` 16, `i2c.bus` 2, `spi.bus` 2, `assets.blob` 4. Beyond that: `out-of-memory`.

---

## 6. Guest module requirements

### 6.1 Supported Wasm feature set

The runtime implements the following and rejects, at validation time, any module that contains other instructions or sections.

MVP + `sign-extension` + `nontrapping-float-to-int` + `bulk-memory` (`memory.copy`, `memory.fill`, `data.drop`; the `table.*` instructions are excluded) + `multi-value` + `mutable-globals`.

Not supported: SIMD, threads/atomics, exception-handling, tail-call, GC, the reference-types extension instructions (`ref.null`, `ref.func`, `table.get/set/grow/size/fill/copy/init`), multi-memory, memory64.

> Note: recent Rust enables `reference-types` and `multivalue` by default on `wasm32-unknown-unknown`. With `reference-types` enabled, the table index of `call_indirect` is encoded as LEB128 (with a single table the value is 0, so the byte sequence is unchanged). The runtime reads the `call_indirect` table index as LEB128 and rejects anything other than 0. On the Rust side, use `-C target-feature=-reference-types`, or `-C target-cpu=mvp` plus the required features added individually with `+` (**the actual flags are defined in `tools/wasmicon-cli/src/flags.rs`, the source of truth**; docs/app-workflow.md §4.4).

### 6.2 Memory

- Define exactly one `memory` and export it as `"memory"`. An imported memory is not allowed.
- The minimum page count must not exceed the platform's limit. **The port decides the limit.** Reference values: RP2040 = 2 pages (128 KiB), RP2350 = 4 pages (256 KiB), ESP32-S3 (no PSRAM) = 4 pages (256 KiB).
- `memory.grow` succeeds up to the limit and returns `-1` beyond it (no trap).
- Data segments may be passive or active.

### 6.3 Tables

- Zero or one `funcref` table. The port decides the element limit (reference: 256).

### 6.4 Imports

- Only imports that come from the interfaces listed in `world app`. An unknown import is a link error.
- Not every HAL function has to be imported (only the ones in use).
- The type signature of each import must **match the table in §7 exactly**. A mismatch is a link error.

### 6.5 The start function

A `start` section is allowed. The host calls `start`, then `run`.

### 6.6 Traps

When the guest traps (out-of-bounds access, `unreachable`, integer division by zero, stack overflow and so on), the host discards the instance, drops all handles, and logs the trap reason. What happens next (restart or halt) is up to the port.

---

## 7. v0.1 import list (normative table)

The generator's output must match this table. `ec` is the status `i32` of §4.4.

### `wasmicon:hal/gpio@0.1.0`

| Import name | Core Wasm type | Notes |
|---|---|---|
| `[static]pin.open` | `(param i32 i32 i32) (result i32)` | `(index, mode, out-handle) -> ec` |
| `[method]pin.set-mode` | `(param i32 i32) (result i32)` | `(self, mode) -> ec` |
| `[method]pin.read` | `(param i32 i32) (result i32)` | `(self, out-level) -> ec` |
| `[method]pin.write` | `(param i32 i32) (result i32)` | `(self, level) -> ec` |
| `[method]pin.toggle` | `(param i32) (result i32)` | `(self) -> ec` |
| `[resource-drop]pin` | `(param i32)` | `(self)` |

### `wasmicon:hal/i2c@0.1.0`

| Import name | Core Wasm type | Notes |
|---|---|---|
| `[static]bus.open` | `(param i32 i32 i32) (result i32)` | `(index, speed, out-handle) -> ec` |
| `[method]bus.write` | `(param i32 i32 i32 i32) (result i32)` | `(self, address, data-ptr, data-len) -> ec` |
| `[method]bus.read` | `(param i32 i32 i32 i32 i32 i32) (result i32)` | `(self, address, len, buf, cap, len-out) -> ec` |
| `[method]bus.write-read` | `(param i32 i32 i32 i32 i32 i32 i32 i32) (result i32)` | `(self, address, data-ptr, data-len, len, buf, cap, len-out) -> ec` |
| `[resource-drop]bus` | `(param i32)` | `(self)` |

### `wasmicon:hal/spi@0.1.0`

| Import name | Core Wasm type | Notes |
|---|---|---|
| `[static]bus.open` | `(param i32 i32 i32 i32) (result i32)` | `(index, frequency-hz, mode, out-handle) -> ec` |
| `[method]bus.write` | `(param i32 i32 i32) (result i32)` | `(self, data-ptr, data-len) -> ec` |
| `[method]bus.transfer` | `(param i32 i32 i32 i32 i32 i32) (result i32)` | `(self, data-ptr, data-len, buf, cap, len-out) -> ec`. Requires `cap >= data-len` |
| `[resource-drop]bus` | `(param i32)` | `(self)` |

### `wasmicon:hal/time@0.1.0`

| Import name | Core Wasm type | Notes |
|---|---|---|
| `now-us` | `(result i64)` | |
| `sleep-ms` | `(param i32)` | |
| `sleep-us` | `(param i32)` | |

### `wasmicon:hal/log@0.1.0`

| Import name | Core Wasm type | Notes |
|---|---|---|
| `log` | `(param i32 i32 i32)` | `(level, msg-ptr, msg-len)` |

### `wasmicon:hal/board@0.1.0`

| Import name | Core Wasm type | Notes |
|---|---|---|
| `pin-by-role` | `(param i32 i32 i32) (result i32)` | `(role-ptr, role-len, out-index) -> ec` |

Role names are **not a fixed vocabulary** (owner decision, 2026-10-07). A name is up to 16 characters of `a-z` / `0-9` / `-`, starting with a lowercase letter. Which GPIO a name maps to is decided by the wiring table of §8.
If the board has no assignment for the name, the call returns `unsupported`.

### `wasmicon:hal/assets@0.1.0`

| Import name | Core Wasm type | Notes |
|---|---|---|
| `[static]blob.open` | `(param i32 i32 i32) (result i32)` | `(name-ptr, name-len, out-handle) -> ec` |
| `[method]blob.len` | `(param i32 i32) (result i32)` | `(self, out-len) -> ec` |
| `[method]blob.read` | `(param i32 i32 i32 i32 i32 i32) (result i32)` | `(self, offset, len, buf, cap, len-out) -> ec` |
| `[resource-drop]blob` | `(param i32)` | `(self)` |

Read-only data the developer ships next to the app, written to the device's assets slot by `wasmicon assets write` (docs/app-workflow.md §3.10). A name is 1 to 32 bytes of `a-z` / `0-9` / `-` / `_` / `.`, starting with a letter or a digit; any other shape is `invalid-argument`, and a well-formed name that is not in the slot is `not-found`. The first successful `open` of an asset checks the body's CRC-32 against the directory; a mismatch is `io`, on every later `open` too. The same asset may be open more than once. `read` is never short: `offset + len` past the asset, `len = 0` or `len > cap` is `invalid-argument`, and the bytes go straight into `[buf, buf+len)`. Dropping a blob releases nothing on the board; the handle is simply freed.

### `wasmicon:hal/types@0.1.0`

No functions. The `error-code` discriminants (the status of §4.4 is `+1`):

| Discriminant | Case | Status value |
|---|---|---|
| 0 | `invalid-argument` | 1 |
| 1 | `invalid-handle` | 2 |
| 2 | `busy` | 3 |
| 3 | `timeout` | 4 |
| 4 | `nack` | 5 |
| 5 | `io` | 6 |
| 6 | `unsupported` | 7 |
| 7 | `out-of-memory` | 8 |
| 8 | `not-found` | 9 |

---

## 8. Board configuration (the port layer's responsibility)

How `index` is interpreted, and which pins carry SDA/SCL/SCK/MOSI/MISO, is decided by the port layer's board configuration. The guest sees only `index`. Initial assignment for the target app:

| Purpose | WIT designation | ESP32-S3 (DevKitC-1) | ESP32-C5 (WIFI6-KIT) | Raspberry Pi Pico WH | Raspberry Pi Pico 2 / 2 W |
|---|---|---|---|---|---|
| I2C bus (SHT4x) | `i2c.bus` index 0 | I2C0: SDA=GPIO8, SCL=GPIO9 | I2C0: SDA=GPIO8, SCL=GPIO9 | i2c0: SDA=GP4, SCL=GP5 | i2c0: SDA=GP4, SCL=GP5 |
| SPI bus (ILI9341) | `spi.bus` index 0 | SPI2: SCK=GPIO12, MOSI=GPIO11, MISO=GPIO13 | SPI2: SCK=GPIO24, MOSI=GPIO23, MISO=GPIO5 | spi0: SCK=GP18, MOSI=GP19, MISO=GP16 | spi0: SCK=GP18, MOSI=GP19, MISO=GP16 |
| ILI9341 CS | `gpio.pin` | GPIO10 | GPIO10 | GP17 | GP17 |
| ILI9341 DC | `gpio.pin` | GPIO14 | GPIO0 | GP20 | GP20 |
| ILI9341 RST | `gpio.pin` | GPIO15 | GPIO1 | GP21 | GP21 |
| On-board RGB LED (WS2812B) | `spi.bus` index 1 | SPI2: MOSI=GPIO48 only (SCK / MISO are not routed) | SPI2: MOSI=GPIO27 only (SCK / MISO are not routed) | none | none |

The Pico 2 / Pico 2 W header has the same pinout as the Pico / Pico WH, so the GP numbers are kept the same.

The ESP32-C5 assignment was taken from the schematic of the Waveshare ESP32-C5-WIFI6-KIT (ESP32-C5-WROOM-1) (2026-10-12).
**GPIO6 is tied through 0 Ω to the battery-voltage divider on the board**, so it is not used. GPIO27 is the RGB LED
(WS2812), GPIO28 is the BOOT button, and GPIO2/3/7/25/26 are strapping pins, so they were avoided.
GPIO11/12 are the trace UART0 (CH343), 13/14 are native USB, and 15..=22 are flash /
PSRAM; none of these are exposed to the guest.

On the ESP32-C5, `spi.bus` index 1 is for the on-board RGB LED (WS2812B, wired directly to GPIO27 through 0 Ω)
(2026-10-12). Only the MOSI of SPI2 is routed to GPIO27, so no wiring is needed. The guest expands each WS2812
bit into several SPI bits and calls `write` (`apps/rgb-led-rs`). It is the same SPI2 as index 0, so
**the two cannot be open at the same time** (the second `open` gets `busy`).

The ESP32-S3 port does the same with GPIO48 (2026-10-20), where the Freenove ESP32-S3-WROOM board and the
DevKitC-1 carry their on-board WS2812. On the Pico boards `spi.bus` index 1 is `unsupported`.

The mapping from role names to GPIO numbers that `board.pin-by-role` (§7) answers from (the **wiring table**)
**lives in the device's config slot, not in the firmware** (owner decision, 2026-10-07;
docs/app-workflow.md §3.9).

- **Role names are not the firmware's vocabulary.** They are an agreement between the app
  (`[requirements] pin-roles` in `wasmicon.toml`) and the wiring (`[board.<name>.roles]`):
  `wasmicon deploy` writes the wiring table to the config slot, and the firmware answers
  according to the table it read at boot
- **The firmware has no default table.** If the config slot is empty or corrupt, it hands out
  no roles at all (`pin-by-role` always returns `unsupported`) and drives no pin on its own
- GPIOs that cannot go into the wiring table: out-of-range numbers, reserved pins (the trace UART,
  flash, the radio chip), **pins the port uses for SPI / I2C** (the table above), and duplicates of
  the same number. The CLI rejects them before flashing; the firmware rejects them again at boot
  (`roles` in `ports/common`)
- Only the host (mock) keeps its table in the profile (`led`=2 / `lcd-cs`=10 /
  `lcd-dc`=11 / `lcd-rst`=12), because it has no flash

The wiring used by the evaluation setup (SHT40 + ILI9341) is in `apps/*/wasmicon.toml`:

| Role name | ESP32-S3 (DevKitC-1) | ESP32-C5 (WIFI6-KIT) | Raspberry Pi Pico 2 W | Apps that use it |
|---|---|---|---|---|
| `lcd-cs` | GPIO10 | GPIO10 | GP17 | lcd-demo / sensor-display |
| `lcd-dc` | GPIO14 | GPIO0 | GP20 | lcd-demo / sensor-display |
| `lcd-rst` | GPIO15 | GPIO1 | GP21 | lcd-demo / sensor-display |
| `led` | GPIO2 (external) | GPIO4 (external; 10 kΩ pull-up on the board) | GP15 (external) | blink |

`led` is just one of the names blink looks up (nothing special to the firmware).
No board has an on-board LED on a plain GPIO, so an external LED is used
(the Pico 2 W's LED is on the CYW43439 side; the ESP32-S3 DevKitC-1's is a WS2812).

**Because GPIO numbers differ per board**, achieving "the same binary, the same result" for the target app needs a mechanism that keeps the guest from hard-coding pin numbers. v0.1 adopted (b) below (§10 item 1, docs/decisions.md A-02); both alternatives are kept as the record:

- (a) The guest is built separately per board (switched with `cfg` / constants)
- (b) Add a `wasmicon:hal/board` interface and delegate a "role name -> GPIO number" lookup to the host, such as `pin-by-role: func(role: string) -> result<u32, error-code>`

From the determinism point of view, (b) is preferable (the binaries become fully identical).

---

## 9. Determinism and traces

When the port layer is built with `WASMICON_TRACE` enabled, it writes every host call to the serial port in this format:

```
> <module>/<name>(<args...>)
< <status> [<out values...>]
```

- A `list<u8>` argument is a hex dump; beyond 32 bytes, the first 16 bytes plus a CRC-32.
- **The bytes received by the read calls (`i2c.read` / `i2c.write-read` / `spi.transfer` / `assets.blob.read`) are
  also written, in the result on success, as `len=<n> data=<same notation as arguments>`** (added 2026-10-05). Example:
  `< 0 [len=6 data=0x6421e074e970]`. This makes what the board read reproducible from the trace alone
  (`wasmicon trace replay` extracts it in the `--i2c-replay` format). The value is decided by the external
  device, so **if the sensor reading differs, a mismatch here** is correct.
- Calls to the `time` interface are **not included** in the trace (they depend on real time).
- For `spi.write`, `data` is the CRC-32 only (to check that pixel data matches).
- **A GPIO number that `board.pin-by-role` returned is written as the role name.** The numbers differ per
  board, so writing them as is would make the traces of the same binary differ.
  The port remembers which numbers it handed out through `pin-by-role` and for which role, and writes the
  `index` argument of `gpio.pin.open` in the form `role:led`. The return value of `pin-by-role` itself is written the same way.
  The host **can only judge by the numeric value** of the argument, so a guest that hard-codes a pin number
  which happens to equal that board's role assignment also gets the role name substituted.
  Hard-coded pin numbers already break cross-board identity, so a guest that is subject to
  determinism verification (docs/decisions.md D-10) must always use `pin-by-role`.

"The same result" is defined as: diff this trace from the two boards, and every host call and result except `time` matches.

---

## 10. Open questions

Items 2 to 5 are all implemented on their defaults and working. Whether to freeze them is tracked in docs/TODO.md §2.

1. ~~How to absorb the per-board GPIO number differences ((a) or (b) of §8)~~ -> **(b) adopted (2026-09-10)**. `pin-by-role` in `wasmicon:hal/board@0.1.0`. Role names: docs/app-workflow.md §3.9.
2. The host's behavior during `sleep-ms` (yield to other tasks, or a plain busy wait). Left to the port layer's HAL (docs/decisions.md A-03).
3. Behavior after a trap (restart / halt / call `run` again).
4. Whether to validate UTF-8 in the `string` of `log` (currently: no).
5. Whether to keep `spi.transfer` in v0.1 (the target app does not need it; it is only useful for reading the ILI9341's ID).

---

## Appendix A. Guest code example (Rust, what the generated file looks like)

```rust
// Generated: bindings/rust/src/generated.rs
// One mod per interface (to keep i2c.bus.open and spi.bus.open from colliding)
pub mod gpio {
    #[link(wasm_import_module = "wasmicon:hal/gpio@0.1.0")]
    unsafe extern "C" {
        #[link_name = "[static]pin.open"]
        pub fn pin_open(index: u32, mode: u32, out: *mut u32) -> u32;
        #[link_name = "[method]pin.write"]
        pub fn pin_write(this: u32, level: u32) -> u32;
        #[link_name = "[resource-drop]pin"]
        pub fn pin_drop(this: u32);
    }
}

// Hand-written: the safe wrapper
pub struct Pin(u32);
impl Pin {
    pub fn open(index: u32, mode: PinMode) -> Result<Pin, ErrorCode> {
        let mut h = 0u32;
        match unsafe { pin_open(index, mode as u32, &mut h) } {
            0 => Ok(Pin(h)),
            n => Err(ErrorCode::from_status(n)),
        }
    }
    pub fn write(&self, level: Level) -> Result<(), ErrorCode> {
        match unsafe { pin_write(self.0, level as u32) } { 0 => Ok(()), n => Err(ErrorCode::from_status(n)) }
    }
}
impl Drop for Pin { fn drop(&mut self) { unsafe { pin_drop(self.0) } } }
```

## Appendix B. Guest code example (AssemblyScript, what the generated file looks like)

```ts
// Generated: wasmicon:hal/i2c@0.1.0
@external("wasmicon:hal/i2c@0.1.0", "[static]bus.open")
declare function i2c_bus_open(index: u32, speed: u32, out: usize): u32;
@external("wasmicon:hal/i2c@0.1.0", "[method]bus.write-read")
declare function i2c_bus_write_read(self: u32, address: u32, dataPtr: usize, dataLen: u32,
                                     len: u32, buf: usize, cap: u32, lenOut: usize): u32;
@external("wasmicon:hal/i2c@0.1.0", "[resource-drop]bus")
declare function i2c_bus_drop(self: u32): void;
```

## Appendix C. Host-side import table (Rust, what the generated file looks like)

```rust
// Generated: runtime/src/generated.rs
pub struct ImportDesc {
    pub module: &'static str,  // "wasmicon:hal/gpio@0.1.0"
    pub name: &'static str,    // "[static]pin.open"
    pub sig: &'static str,     // "iii:i"  params:results, i=i32 I=i64 f=f32 F=f64
    pub host_fn: HostFn,       // the slot the port layer dispatches on
}

#[repr(u16)]
pub enum HostFn { GpioPinOpen, GpioPinSetMode, /* ... */ LogLog }

pub static IMPORTS: [ImportDesc; 19] = [ /* ... */ ];

pub fn resolve(module: &str, name: &str) -> Option<&'static ImportDesc>;
```

The runtime looks up the `ImportDesc` by module + name, checks that `sig` matches the guest's type exactly, and then links `host_fn`. The port layer supplies the host function bodies.

## Appendix D. Guest code example (C, what the generated file looks like)

C has a flat namespace, so every name is prefixed with `wasmicon_`.

| WIT | C |
|---|---|
| Import function | `wasmicon_<iface>_<function>` (the AS function name with `wasmicon_` in front: `wasmicon_gpio_pin_open`) |
| Enum type | `wasmicon_<iface>_<type>_t`. Types from `types` drop the interface name (`wasmicon_error_code_t`) |
| Enum case | Upper-case type name + `_` + case name (`WASMICON_GPIO_PIN_MODE_OUTPUT`). The case count is `<UPPER_TYPE>_COUNT` |
| flags | No type, constants only. Bits are `<UPPER_TYPE>_<BIT>`; the mask of all defined bits is `<UPPER_TYPE>_ALL` |
| Arguments | snake_case. `self` stays as is (it is not a reserved word in C / C++) |
| `list<u8>` / `string` pointers | `const uint8_t *` / `const char *` (both are i32 in Core Wasm) |

Enum arguments in import declarations are raw `uint32_t`, as in Rust. The guest's export is marked with
`__attribute__((export_name("run")))`. The build flags (the feature set of §6.1 and the memory limit)
live in `bindings/c/build.sh`.

```c
// Generated: bindings/c/generated.h
typedef enum wasmicon_gpio_pin_mode {
    WASMICON_GPIO_PIN_MODE_INPUT = 0,
    /* ... */
    WASMICON_GPIO_PIN_MODE_OUTPUT = 3,
} wasmicon_gpio_pin_mode_t;

WASMICON_IMPORT("wasmicon:hal/gpio@0.1.0", "[static]pin.open")
uint32_t wasmicon_gpio_pin_open(uint32_t index, uint32_t mode, uint32_t *out);
WASMICON_IMPORT("wasmicon:hal/gpio@0.1.0", "[resource-drop]pin")
void wasmicon_gpio_pin_drop(uint32_t self);

// App (apps/blink-c/src/main.c)
#include "wasmicon_hal.h"

WASMICON_EXPORT("run") void run(void) {
    uint32_t pin;
    if (wasmicon_gpio_pin_open(15, WASMICON_GPIO_PIN_MODE_OUTPUT, &pin) != WASMICON_OK) {
        return;
    }
    wasmicon_gpio_pin_write(pin, WASMICON_GPIO_LEVEL_HIGH);
    wasmicon_gpio_pin_drop(pin);
}
```
