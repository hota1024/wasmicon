# lcd-demo-rs

Demo that only draws on an ILI9341. No sensor needed: one LCD is enough.

`sensor-display-rs` reads an SHT4x over I2C, so without the sensor it shows
nothing (it returns as soon as I2C fails to open). This demo uses only SPI
and GPIO, so the display alone is enough.

Rust only. There is no AssemblyScript counterpart (unlike `sensor-display-*`,
this app is not for comparing two implementations).

The same `.wasm` runs on the Raspberry Pi Pico 2 (W) and the Freenove
ESP32-S3-WROOM board. The guest asks the host for pin numbers through
`board.pin-by-role`, so nothing board-specific is in the guest
(docs/abi-spec.md §8). Portability is checked by flashing the same binary to
both boards and cross-checking the traces (docs/verification-report.md §5).

## What it shows

320x240, landscape. Top to bottom:

| Element | What it checks |
|---|---|
| `WASMICON` / `RP2350` on a blue band | Text drawing |
| 8 color bars (red, orange, yellow, green, cyan, blue, magenta, white) | Color order. If red and blue are swapped, the BGR bit of MADCTL (`0x36`) |
| A 16-step gray ramp | RGB565 packing. Tinted steps mean a bit position is off |
| A green square moving right and back | `time.sleep-ms` and repeated drawing |
| `HELLO PICO 2` / `ILI9341 SPI` | Text drawing |
| A 1 px white frame around the screen | The edge of the address window: rectangles with exactly `x + w == 320` / `y + h == 240` |

The screen says `RP2350` and `HELLO PICO 2` on the ESP32-S3 too. This is
intentional. The point of this demo is to confirm that one binary produces
the same host-call sequence on two boards; per-board text would leave nothing
to compare. Changing the text changes the `.wasm`, and the 14,352-line trace
recorded in docs/verification-report.md §6 would have to be re-recorded.

## Wiring

### Raspberry Pi Pico 2 W

This wiring was verified on a Pico 2 W on 2026-09-26
(docs/verification-report.md §6). To use other pins: the roles (`lcd-cs` /
`lcd-dc` / `lcd-rst`) are in `[board.rp2350.roles]` of `wasmicon.toml`
(`deploy` writes them to the config slot; the firmware need not be
reflashed), and the SPI pins are `SPI0_SCK` / `SPI0_MOSI` / `SPI0_MISO` in
`ports/rp2350/src/board.rs`.

| ILI9341 module | Pico 2 W | Physical pin |
|---|---|---|
| `VCC` | 3V3 OUT | 36 |
| `GND` | GND | 23 / 28 etc. |
| `CS` | GP17 | 22 |
| `RESET` | GP21 | 27 |
| `DC` / `D/C` / `RS` | GP20 | 26 |
| `SDI` / `MOSI` | GP19 | 25 |
| `SCK` / `SCL` | GP18 | 24 |
| `LED` / `BL` | 3V3 OUT | 36 (the same pin as VCC) |
| `SDO` / `MISO` | GP16 (may stay unconnected) | 21 |

All the LCD signals are on the right column of the board. With the USB
connector at the top, the right column runs 21, 22, 23, ... 40 from bottom to
top (the left column runs 1..20 from top to bottom). Pin 21 is the
bottom-right corner. Only the serial GP0/GP1 are at the top left (pins 1 and
2).

There is only one `3V3(OUT)`, pin 36. It feeds both `VCC` and `LED`, so route
it to a breadboard 3.3 V rail and branch from there (or join the two on the
LCD side). The neighboring pin 37, `3V3_EN`, is the regulator enable input,
not a supply. There are 8 GND pins (3 / 8 / 13 / 18 / 23 / 28 / 33 / 38).

- A 10 kΩ pull-up on `RESET` (`RESET` to `3V3`) is the safe choice. On the
  Pico 2 W at hand the image stayed without it, but that is an observation,
  not a guarantee (see "The image disappears and the screen turns white
  after drawing" below; on the ESP32-S3 it turned white without one)
- The module must be a 3.3 V logic SPI module. 5 V-only boards and boards
  configured for the 8080 parallel interface will not work
- Without `LED` (backlight) connected, the screen stays dark even though it
  is drawing
- The demo never reads MISO, so `SDO` may stay unconnected
- On modules with a touch panel, all `T_*` pins may stay unconnected

Serial (trace and log) is UART0: GP0=TX (pin 1) / GP1=RX (pin 2),
115200 8N1. It is not USB, so a USB-serial adapter is needed (share GND).

### Freenove ESP32-S3-WROOM board

This wiring was verified on a Freenove board on 2026-09-26
(docs/verification-report.md §7). To use other pins: the roles are in
`[board.esp32s3.roles]` of `wasmicon.toml`, and the SPI pins are `SPI2_SCK` /
`SPI2_MOSI` / `SPI2_MISO` in `ports/esp32s3/src/board.rs`.

| ILI9341 module | Freenove ESP32-S3-WROOM board |
|---|---|
| `VCC` | `3V3` |
| `GND` | `G` (`GND`) |
| `CS` | `GPIO10` |
| `RESET` | `GPIO15` |
| `DC` / `D/C` / `RS` | `GPIO14` |
| `SDI` / `MOSI` | `GPIO11` |
| `SCK` / `SCL` | `GPIO12` |
| `LED` / `BL` | `3V3` (same as `VCC`) |
| `SDO` / `MISO` | `GPIO13` (may stay unconnected) |

The Freenove board's silkscreen shows the GPIO numbers directly, so there is no
physical-pin lookup as on the Pico. All six signals above are in the
`IO10`..`IO15` run.

Add a 10 kΩ pull-up on `RESET` (`RESET` to `3V3`). The image appears without
it, but on the ESP32-S3 at hand it disappeared and the screen turned white
the moment drawing finished (see "The image disappears and the screen turns
white after drawing" below).

`GPIO11` / `GPIO12` / `GPIO13` are also the IO_MUX default pins of SPI2
(FSPI). The driver routes through the GPIO matrix, so this is not required,
but the defaults are used.

`3V3` is on both columns (the first two pins of the left column and one on
the right), so `VCC` and `LED` can be fed from different pins. Unlike the
Pico, there is no single-pin constraint.

Serial (trace and log) is UART0: GPIO43=TX / GPIO44=RX, 115200 8N1. On the
Freenove board the USB-C port labeled `USB-UART` connects to it through a
USB-serial bridge, so one cable does both flashing and trace capture (no
separate USB-serial adapter). The other port, `USB-OTG`, is native USB and
this firmware prints nothing there (flashing alone works through
USB-Serial-JTAG).

The bridge chip varies between boards. The official schematic has a CP2102N
(`/dev/cu.usbserial-*` on macOS), but the board at hand has a CH343 (VID
0x1A86 / PID 0x55D3) that shows up as `/dev/cu.usbmodem*`. Searching only for
`usbserial` finds nothing, so check with `ls /dev/cu.usb*`.

- On both boards, connect `LED` (backlight) to 3.3 V. Without it the screen
  stays dark even though it is drawing
- Two ILI9341 modules are not needed; move the one module and flash the
  boards in turn

## Build and flash

The firmware carries no app (docs/app-workflow.md §3.3). Flash the firmware
once, then flash this demo to the app slot. The same `lcd_demo_rs.wasm` is
flashed to both boards (the guest is not rebuilt).

```bash
# Build the guest and the CLI
(cd apps && cargo build --release -p lcd-demo-rs)
cargo build --release -p wasmicon-cli
w=./target/release/wasmicon
app=apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm
```

### Raspberry Pi Pico 2 (W)

```bash
# 1. Build the firmware (it does not depend on apps/)
(cd ports/rp2350 && cargo build --release)

# 2. Hold BOOTSEL while plugging in USB, then flash the firmware.
#    Without -x the board stays in BOOTSEL, so step 3 can flash right after
picotool load -u -v -t elf ports/rp2350/target/thumbv8m.main-none-eabihf/release/wasmicon-rp2350

# 3. Flash the demo to the slot (it starts here). The trace comes from UART0
$w monitor --port /dev/cu.usbserial-XXXX --idle 0 -o pico.log &   # open first
$w deploy "$app" --board rp2350
```

`picotool` 2.0 or later is required (RP2350 support). `elf2uf2-rs` is for
RP2040 and does not work. From the second time on, if the firmware is
unchanged, only the slot needs reflashing (step 3 only).

### Freenove ESP32-S3-WROOM board

```bash
# 1. Build and flash the firmware (build.sh sources ~/export-esp.sh)
sh ports/esp32s3/build.sh build --release
espflash flash --non-interactive \
  ports/esp32s3/target/xtensa-esp32s3-none-elf/release/wasmicon-esp32s3

# 2. Flash the demo to the slot and capture the trace in the same command
$w deploy "$app" --board esp32s3 --monitor -o esp32s3.log
```

`espflash` connects to the `USB-UART` port of the Freenove board. If several USB
serial devices are visible, pass `--port` (the name depends on the bridge
chip; see "Wiring" above).

Sometimes the reset over USB does not happen and the app does not run (hit on
2026-10-07; docs/verification-report.md §12 "Where the ESP32-S3 got stuck").
Then flash through the `USB-OTG` side, capture the trace on the `USB-UART`
side, and start the board with its EN (`RST`) button.

A NUL byte in a captured log fails `wasmicon trace diff` and
`verify/diff-traces.sh`. NULs come from line noise at reset or power-on; left
in, `grep` treats the file as binary and returns a single line.

### Faster without tracing

The default `trace` feature writes every host call to the UART. This demo
makes about 7,200 host calls, so at 115200 baud it takes about a minute to
finish drawing (you can watch the screen fill from the top).

To just see the picture, build the firmware without it (the app need not be
reflashed):

```bash
(cd ports/rp2350 && cargo build --release --no-default-features)
sh ports/esp32s3/build.sh build --release --no-default-features
```

For the first bring-up, though, keep tracing on. When SPI is not working, the
trace is the only clue to how far the app got.

## The image disappears and the screen turns white after drawing (both boards)

Not a bug. When the guest leaves `run()` it releases its handles, and
docs/abi-spec.md §5.2 returns the pins to input with no pull. The LCD's
`RESET` then floats, and if the module has no pull-up the panel resets and
the screen turns white (observed on an ESP32-S3 on 2026-09-26;
docs/verification-report.md §7 "Cause 3").

A 10 kΩ pull-up from `RESET` to `3V3` fixes it (verified on an ESP32-S3 on
2026-09-26: the image stays with the as-shipped configuration). Keep the
existing `lcd-rst` wire. When the ESP32 drives `RESET` low, only 330 µA flows
through the pull-up, and the reset pulse of the init sequence goes out as
before. Any value from 4.7 kΩ to 100 kΩ works.

Not leaving a reset input floating is sound circuit practice, and it touches
neither the code nor the `.wasm`. Leaving the handles unreleased on the guest
side does not avoid it: docs/abi-spec.md §5.2 also says that the host drops
every remaining handle when `run` returns, so the release always happens.

Whether the symptom appears depends on the board. Measured on 2026-09-26 with
the same module:

| Board | Without pull-up |
|---|---|
| ESP32-S3 | Turns white |
| Raspberry Pi Pico 2 W | Image stays |

The mechanism is the same on both ports (the pin returns to high impedance
per docs/abi-spec.md §5.2, and software does not set the idle level of the
line), but whether the floating line crosses the panel's threshold depends on
pad leakage and capacitance, hence the different result. The Pico 2 W keeping
the image is an observation, not a guarantee. A pull-up on both boards is the
safe choice.

## When it does not work

This setup is verified on both a Pico 2 W and a Freenove ESP32-S3-WROOM board
(docs/verification-report.md §6 / §7). If it still shows nothing, suspect in
this order:

1. Nothing on the UART: wiring (TX/RX direction, GND), 115200 8N1, and
   whether flashing succeeded. The banner `wasmicon rp2350` comes first
2. `role not found`: the wiring table is not in the config slot. Look at the
   `wasmicon: roles ...` line of the banner. If it says
   `roles empty, no roles`, run `deploy` again from the directory that has
   `wasmicon.toml` (the table is written along with the app)
3. `spi open failed` / `gpio open failed`: the port side
4. The trace runs to the end but the screen is dark: backlight (`LED` pin),
   `RESET` wiring, power. The ILI9341 is 3.3 V
5. Something shows but it is garbled: suspect the DC wiring first, then the
   SPI clock. Lower `SPI_HZ` (`src/lib.rs`) to around 4 MHz to narrow it
   down. On a breadboard with long wires, 15 MHz sometimes does not get
   through
6. Red and blue are swapped, or the colors are inverted: the MADCTL (`0x36`)
   value. `init` in `src/ili9341.rs` sends `0x28` (landscape, BGR)

### When only the ESP32-S3 shows nothing

`ports/esp32s3` also runs on hardware (docs/verification-report.md §7). In
the order the problems were actually hit in docs/verification-report.md §7:

1. Not even the banner `wasmicon esp32s3` appears: wrong UART port. The
   Freenove board has two USB-C ports, and the trace comes out of the `USB-UART`
   side (USB-serial bridge to GPIO43/44). The `USB-OTG` side is native USB
   and this firmware prints nothing there. The device name depends on the
   bridge chip, so check with `ls /dev/cu.usb*`
2. `spi open failed`: `spi_open` returned `unsupported`. That happens only
   when the requested frequency is outside the producible range (below
   78.125 kHz or above 80 MHz with APB at 80 MHz). `SPI_HZ` is 16 MHz, so
   normally it does not
3. The trace matches the host exactly but the screen is pure white: the
   two-stage case actually hit in docs/verification-report.md §7. The trace
   says nothing about this (every host call returns success). Narrow it down
   by driving the port layer directly, bypassing the guest:
   `sh ports/esp32s3/build.sh run --release --features hw-probe`
   (`ports/esp32s3/src/probe.rs`)
   - First `SIG_GPIO_OUT` in `ports/esp32s3/src/board.rs`. On the S3 it is
     256 (128 is the C3 / C6 value). If it is wrong, `GPIO_OUT` /
     `GPIO_ENABLE` read back correctly but the pin never moves at the pad
   - Then the breadboard contacts. In docs/verification-report.md §7,
     reseating the wires made it work
   - If `SPI2_SCK` / `SPI2_MOSI` were moved from GPIO12 / GPIO11, were both
     the constants and the table in docs/abi-spec.md §8 updated
4. It stops partway and starts over: power. Feeding the backlight from the
   breadboard 3V3 can be too much (the Freenove board's 3V3 is the on-board
   regulator output)
5. The image appears but turns white after drawing: see "The image
   disappears and the screen turns white after drawing" above. A pull-up on
   `RESET` fixes it
6. The trace stops partway and nothing follows: the native stack. The
   `esp-hal` linker script puts `.stack` in whatever DRAM is left, so growing
   `ARENA` (300 KB) in `ports/esp32s3/src/main.rs` shrinks the stack
   (17.4 KiB today). An overflow enters the panic handler, but the reason
   cannot be printed (the serial port lives on the board side and is
   unreachable from the panic handler), so the only symptom is silence
