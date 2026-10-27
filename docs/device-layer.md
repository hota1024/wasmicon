# Device layer design (`wasmicon:device`)

Created: 2026-10-27
Status: **approved, not implemented.** The owner agreed on the direction on 2026-10-27
(a separate package above the HAL, named `wasmicon:device`, with `display` as its
first interface) and settled docs/TODO.md §6-1 ... §6-7 the same day. The stages and
their status are in docs/TODO.md §6.
Related: docs/abi-spec.md, docs/app-workflow.md §3.9, §4.7, docs/decisions.md D-03, D-05, D-09

---

## 1. Why a second layer

`wasmicon:hal` exposes **peripherals**: pins and buses that every port has. Device
knowledge (ILI9341 commands, SHT4x conversion) lives in the guest
(docs/decisions.md D-08, D-09). That split works while the device sits on a bus the
HAL already has.

It stops working for displays that no HAL bus reaches. The M5Stack Tab5 has a
720x1280 MIPI-DSI panel on its ESP32-P4. A guest cannot drive it:

- DSI lanes are dedicated D-PHY pins with high-speed differential signaling. Only
  the chip's DSI host can drive them, and no `gpio` / `spi` / `i2c` call reaches it
- the panel runs in video mode and streams a frame buffer continuously by DMA. At
  720 x 1280 x 2 bytes = 1.8 MB the buffer does not fit in a guest's linear memory
  (the arena is 300 KiB on most ports)

A raw DSI interface in the HAL would serve one chip among the ports and push
chip-specific setup into the guest. A **device** interface, where the host owns the
panel and the guest only draws, serves every board with a screen:

| | `wasmicon:hal` | `wasmicon:device` |
|---|---|---|
| Abstracts | Peripherals (`gpio`, `spi`, `i2c`, ...) | Devices (`display`, ...) |
| Present on | Every port | Only boards that have the device; optional |
| Device knowledge | In the guest | In the host (the backend) |
| Implementations per interface | One per port | Several backends per interface (§5) |
| Version | `0.1.0`, frozen | Its own version, starts at `0.1.0`, moves independently |

The HAL stays small and frozen; the device layer can grow. The cost is that a
device backend is host code, which the HAL so far avoided. §2 limits that.

## 2. What may enter the device layer

**Rule (docs/decisions.md D-13):** an interface enters `wasmicon:device` only when it has **at
least two backends**, counting the host mock, of which at least one runs on an
evaluation board. One backend means the interface is a wrapper around one chip, and
the HAL plus a guest driver is the better home.

`display` meets it with three: the host mock frame buffer, an ILI9341 driver in the
host over SPI (ESP32-S3, Pico 2 W), and the Tab5 DSI panel (ESP32-P4).

Check (to add with the implementation): a test in `ports/common` lists the backends
of each `wasmicon:device` interface and fails below two.

## 3. Naming and packaging

- Package: `wasmicon:device@0.1.0`. Import module names follow docs/abi-spec.md §3.1
  unchanged: `wasmicon:device/display@0.1.0`. Field names follow docs/abi-spec.md §3.2.
- **One error type stays.** `display` uses `wasmicon:hal/types@0.1.0.{error-code}`.
  The codes and their lowering (discriminant + 1) are those of docs/decisions.md D-03.
  No new error code is needed.
- **The version moves independently** of `wasmicon:hal`. The firmware identity line
  (docs/app-workflow.md §3.8) gains the device package next to `abi=`.
- **WIT layout.** The cross-package `use` and an `include` of the HAL world resolve
  with `wasm-tools component wit` when the HAL package sits in `deps/` of the device
  package (tried on 2026-10-27 with the exact text of §4 and the world below). The files live in
  `wit-device/` (docs/TODO.md §6-4); `wit/` stays the HAL, untouched. There is no
  `deps/` copy of the HAL: two copies drift, so the generator and the signature check
  load `wit/` first and then `wit-device/`.
- **World.** The device package defines its own world that includes the HAL world:

```wit
world app {
    include wasmicon:hal/app@0.1.0;
    import display;
}
```

An app that imports no device interface is exactly a `wasmicon:hal` app, as today.

## 4. The `display` interface

```wit
package wasmicon:device@0.1.0;

/// A pixel display that the host drives.
/// The host owns the panel: its bus, initialization, orientation and any frame buffer.
/// The guest only draws rectangles. Text and images are drawn by the guest.
interface display {
    use wasmicon:hal/types@0.1.0.{error-code};

    /// How pixels are encoded in `write-rect` and `fill-rect`.
    enum pixel-format {
        /// 16 bits per pixel, 5-6-5, high byte first.
        rgb565,
    }

    /// A display.
    /// `index` is the board's display number. The backend behind it comes from the
    /// board configuration.
    resource screen {
        /// Opens the display. Returns `unsupported` if the board has no display at `index`.
        open: static func(index: u32) -> result<screen, error-code>;

        /// Returns the width in pixels, after the configured orientation.
        width: func() -> u32;

        /// Returns the height in pixels, after the configured orientation.
        height: func() -> u32;

        /// Returns the pixel format that `write-rect` and `fill-rect` take.
        format: func() -> pixel-format;

        /// Fills a rectangle with one color, given in the pixel format in the low bits.
        /// Returns `invalid-argument` if the rectangle leaves the screen or is empty.
        fill-rect: func(x: u32, y: u32, w: u32, h: u32, color: u32) -> result<_, error-code>;

        /// Writes `w * h` pixels row by row into a rectangle.
        /// Returns `invalid-argument` if the rectangle leaves the screen or is empty, or if
        /// `pixels` is not exactly `w * h` pixels long.
        write-rect: func(x: u32, y: u32, w: u32, h: u32, pixels: list<u8>) -> result<_, error-code>;

        /// Makes everything drawn so far visible.
        /// Backends that draw straight to the panel return at once; backends with a frame
        /// buffer flush or swap it here.
        present: func() -> result<_, error-code>;
    }
}
```

All types are inside the subset of docs/abi-spec.md §2.1. Notes:

- **Pixel byte order is fixed: RGB565, high byte first** (docs/TODO.md §6-3). This
  is the order the apps already send to the ILI9341, so guest drawing code carries
  over. A backend whose memory is little-endian (the DSI frame buffer) swaps while it
  copies. The order is part of the ABI because the `write-rect` CRC in the trace
  (§7) must be the same on every backend.
- **Only `rgb565` for now.** `format` exists so a later format does not change the
  signature of `write-rect`. A guest that sees a format it does not know stops.
- **`fill-rect` exists for speed.** Each host call is expensive and each byte
  crosses the interpreter (docs/TODO.md §4): clearing 720x1280 through `write-rect` would copy 1.8 MB
  out of the interpreter.
- **Orientation is configuration, not API** (§5). `width` / `height` report the
  result. The guest reads them and lays out its screen; the same `.wasm` runs on a
  320x240 and a 720x1280 display.
- **`present` is required for portable output** (docs/TODO.md §6-7). A guest that
  never calls it may show nothing on frame buffer backends. The ILI9341 backend has
  no buffer and returns 0.
- **One buffer.** `present` makes what was drawn visible; it does not swap a front
  and a back buffer, so drawing only a changed part stays correct. Whether `present`
  waits for vertical blank is the backend's choice; traces carry no time, so it does
  not change them.
- Bounds are checked with `checked_add` before any drawing (docs/decisions.md P-07).
  A failed call draws nothing.
- `screen` handles count toward the handle limit of docs/abi-spec.md §5.3 and are
  dropped when `run` returns (docs/abi-spec.md §5.2). Dropping a screen does not
  blank it.

## 5. Backends and configuration

### 5.1 What the firmware supports

The port profile already lists the interfaces a port implements
(`Profile::interfaces`, printed as `if=` in the identity line). It gains `display`
and the backend kinds the firmware has compiled in. `check --board` reads it.

### 5.2 Which backend a board uses: `wasmicon.toml`

The backend and its wiring are board configuration, like the wiring table
(docs/app-workflow.md §3.9). They go in the app's `wasmicon.toml`:

```toml
[board.esp32s3.display]
driver = "ili9341"
spi = 0          # bus index (docs/abi-spec.md §8)
cs = 10
dc = 14
rst = 15
rotation = 90    # 0, 90, 180, 270

[board.esp32p4.display]
driver = "tab5"  # the Tab5's own MIPI-DSI panel
```

- `deploy` writes it to the config slot with the roles. The body of the slot is text
  (docs/app-workflow.md §3.9), so this is new lines under config slot format
  version 2 (docs/TODO.md §6-5). **`deploy` writes version 2 only when the app has a
  display entry for that board**, and version 1 otherwise: a firmware that predates
  version 2 discards a whole table it cannot read, roles included, and apps without
  a display must keep running on it.
- Pins named here go through the same checks as roles: range, reserved pins, bus
  pins, no duplicates, also not shared with a role. The firmware runs the check
  again at boot. A bad entry leaves the board with no display and prints the reason.
- **Why numbers, not roles.** These pins are driven by the host, not opened by the
  guest, so the role indirection (which exists to keep traces equal) is not needed.
- A board with no `[board.<name>.display]` has no display. `check` / `deploy` refuse
  a display app for it (§6); `open` returning `unsupported` is the fallback for a
  bare `.wasm` flashed without the CLI.
- The host mock has its display in its profile, like its roles (320x240 by default,
  so lcd-demo-style apps compare against the evaluation boards).

### 5.3 The backends

| Backend | Boards | What the host does |
|---|---|---|
| `mock` | host (`wasmicon run`, tests) | Keeps a frame buffer; `run` can write it to a PNG after `run` returns |
| `ili9341` | ESP32-S3, Pico 2 W, ESP32-C5, ESP32-P4 | The ILI9341 init and drawing that the apps do today, moved into `ports/common` over the port's SPI and GPIO |
| `tab5` | ESP32-P4 on the Tab5 | Panel power and reset through the internal I2C IO expanders, backlight on GPIO22, DSI host + DPI video mode, frame buffer in PSRAM. While it is configured, GPIO22, 31 and 32 are reserved: a role on them is refused (docs/TODO.md §6-6) |

The `tab5` backend has known risks on the Tab5's v1.0 silicon (docs/TODO.md
§1.3.2): esp-hal 1.2.2 keeps DPI running through a VDMA interrupt, and this
silicon faults when an interrupt is enabled; PSRAM has not been tried. It is the
last backend to build.

## 6. Checking that a board can run an app

The owner chose to refuse early, before the app runs, and approved on 2026-10-27
that **the requirement is derived from the imports, not declared in
`wasmicon.toml`** (docs/TODO.md §6-2).

- `wasmicon.toml` has the rule "do not write what can be derived", and the imports
  already state which interfaces an app uses (docs/app-workflow.md §4.7). An import
  of `wasmicon:device/display@0.1.0` is the declaration.
- `check --board` and `deploy` refuse an app whose device imports the board profile
  does not list, and one whose `[board.<name>.display]` is missing or names a
  backend the firmware lacks. This happens before flashing, with no device
  connected.
- A firmware that lacks the interface still refuses at `instantiate` (link error,
  docs/abi-spec.md §3.1), so a bare `.wasm` sent without the CLI fails with a reason
  instead of running half-way (docs/decisions.md P-13).

## 7. Traces and determinism

The trace format of docs/abi-spec.md §9 does not change. `display` calls are host
calls, so they are traced with their module name like any other. Rules added:

- `write-rect`: `pixels` is written as its CRC-32 only, as for `spi.write`.
- `width`, `height` and `format` results are traced; they differ between backends of
  different size, so traces compare between boards with the same display geometry.
- **Backend-internal bus traffic is not traced.** The SPI and GPIO calls the
  `ili9341` backend makes are host internals, not guest calls. A display app traced
  on the ESP32-S3 (ILI9341) and on the host mock at 320x240 must be identical.

This keeps docs/decisions.md D-10 as it is: every host call the guest makes and its
result, except `time`, match across boards.

## 8. Changes to existing documents

docs/TODO.md §6-1 approved them. The docs/decisions.md rows are changed already; the
other documents change together with the code that implements them, so they never
describe something that does not exist:

| Document | Change |
|---|---|
| docs/decisions.md D-03 | "for the whole HAL" becomes "for `wasmicon:hal` and `wasmicon:device`" |
| docs/decisions.md D-05 | Import module names are `wasmicon:hal/<iface>@0.1.0` or `wasmicon:device/<iface>@<device version>` |
| docs/decisions.md D-09 | The ILI9341 stays the display of the HAL evaluation apps; through `display` it is one backend |
| docs/decisions.md (new D-13) | The device layer exists, with the rule of §2 |
| docs/abi-spec.md §1 | Scope names both packages |
| docs/abi-spec.md §2.3 | Two packages; `wasmicon:device` uses `wasmicon:hal` types with a cross-package `use`, and its world uses `include` (both outside the current subset) |
| docs/abi-spec.md §3.1 | A `wasmicon:device` example |
| docs/abi-spec.md §7 | A table for `wasmicon:device/display@0.1.0` |
| docs/abi-spec.md §9 | The rules of §7 above |
| docs/app-workflow.md §3.9, §4.7 | `[board.<name>.display]`, its checks and the config slot lines |

## 9. Implementation impact

- `tools/wasmicon-gen`: load two packages; emit `wasmicon:device` imports into all
  four generated files. `PACKAGE` (runtime, C header) becomes one constant per package.
- `tools/wit2sig.py`, `tools/check-sigs.sh`: cover the device package.
- `tools/wasmicon-cli`: `check.rs` stops assuming the `wasmicon:hal/` prefix;
  `manifest.rs` reads `[board.<name>.display]`; `run` gains the PNG output.
- `ports/common`: the `display` host functions, the `mock` and `ili9341` backends,
  the config slot lines, the trace rules.
- `ports/esp32p4`: the `tab5` backend.
- `apps`: a display demo that uses `display` only, plus `wasmicon.toml` for each board.
  lcd-demo-rs and sensor-display stay as they are (they are the HAL's evaluation).

## 10. Stages and verification

1. WIT, generator and bindings. **The recorded HAL app hashes must not move**
   (`lcd_demo_rs.wasm` `84d6de0f...`, 4,088 B, docs/TODO.md §2): an app that imports
   no device interface links no new import.
2. Host: `mock` backend, trace rules, `check` / `deploy` refusal, a demo app and a
   host trace test.
3. `ili9341` backend; the demo's trace on ESP32-S3 and Pico 2 W equals the host mock
   at 320x240, and the panel shows the picture (verified on hardware).
4. `tab5` backend on the ESP32-P4 (verified on hardware, with the risks of §5.3).
