# Shared spec of the sample apps

`sensor-display-rs` (Rust) and `sensor-display-as` (AssemblyScript) must
produce the same host-call sequence and the same pixel output. This document
fixes the drawing details so that they do. Change both when you change one.

The check is `ports/host/tests/apps.rs`. It cross-checks the two traces, so a
difference in the CRC-32 of any `spi.write` fails it.

## 0. The ILI9341 driver exists in four places

There are three copies of `ili9341.rs` / `ili9341.ts` in the apps and a fourth,
the display backend `ports/common/src/ili9341.rs` (docs/device-layer.md §5.3).
When you fix one, look at the others. The backend follows the init sequence and
the send order of §2; `ports/host/tests/ili9341.rs` decodes what it sends and
compares the picture with the host mock.

| File | Check |
|---|---|
| `apps/sensor-display-rs/src/ili9341.rs` | Trace cross-check against the AS version |
| `apps/sensor-display-as/assembly/ili9341.ts` | Trace cross-check against the Rust version |
| `apps/lcd-demo-rs/src/ili9341.rs` | No automated check (this app has no AS version) |

`apps/life-rs/src/ili9341.rs` is not a copy but a separate minimal driver
(init, rectangle fill, `blit` and `draw_text` only). Its init sequence is the
one in §2, but it sends the pixels of a rectangle in as few SPI transfers as
possible, so the send order differs from §2 (the game of life redraws dozens
of cells per generation). When you fix the init sequence, look here too.

The two Rust copies have had different `draw_text` since 2026-10-05:
`sensor-display` has the scaled version (§2 below); `lcd-demo-rs` keeps the
unscaled 8x8, 12-character version. `lcd-demo-rs` has a 14,352-line trace and
the SHA-256 of its `.wasm` recorded in docs/verification-report.md §6 / §7,
and touching it would mean re-recording them, so the two were not aligned.
The bounds check, the init sequence and `fill_rect` are still identical, so
when you fix those, look at all three.
`lcd-demo-rs` has no automated guard, so this is where a fix gets forgotten.
It has happened: the u16 wrap in the bounds check was fixed in `321fe3d` for
`lcd-demo-rs` only and stayed in the two `sensor-display` copies (aligned on
2026-09-28; docs/TODO.md §2.1 has an item on removing the duplication).

## 1. Sensor: SHT4x (SHT40 / SHT41 / SHT45)

- I2C bus index 0, `standard` (100 kHz), address `0x44`
  - The address depends on the part suffix (-AD1B is 0x44). For another
    variant, change `ADDRESS` in both the Rust and the AS version
- Write the single-shot measurement command `0xFD` (high precision). It is
  one byte (the SHT3x `0x2400` is two)
- Wait 15 ms (`time.sleep-ms(15)`). The SHT4x high-precision measurement
  takes at most 8.3 ms, so this leaves margin
- Read 6 bytes: `T MSB, T LSB, CRC, RH MSB, RH LSB, CRC` (same as SHT3x)
- The guest checks the CRC-8: polynomial `0x31`, initial value `0xFF`, no
  reflection (same as SHT3x)
  - Each 2-byte value is checked. On a mismatch, show no value and log
    `sensor crc failed` from §4

### Conversion (fixed point)

No floating point. Values are integers scaled by 100 (centi units).

```
temp_centi     = -4500 + (17500 * raw_t) / 65535
humidity_centi =  -600 + (12500 * raw_h) / 65535   -> clamped to 0..10000
```

The temperature formula is the same as SHT3x. Only humidity differs (SHT3x:
`(10000 * raw_h) / 65535`). The raw formula can go below 0 and above 100, so
it is clamped as the datasheet says. Both `17500 * 65535 = 1_146_862_500` and
`12500 * 65535 = 819_187_500` fit in i32. Division truncates (the same in
both languages). The clamp is done in integers, so it stays true that the
only f32 use is the temperature bar below.

### Exception: one place uses floating point

As a determinism test case, the length of the temperature bar, and only
that, is computed in f32.

```
t      = f32(temp_centi) / 100.0
b      = (t + 45.0) * 1.4
b      = clamped: below 0.0 becomes 0.0, above 308.0 becomes 308.0
bar_px = i32(b)
```

This checks that f32 division, addition and multiplication and the
truncation to `i32` match on both boards.

The clamp is done in f32, so the conversion always sees an in-range value.
Rust's `as` lowers to `i32.trunc_sat_f32_s` and AssemblyScript's `<i32>` may
lower to `i32.trunc_f32_s`; out of range, one would saturate and the other
would trap.

## 2. Display: ILI9341

- SPI bus index 0, 24 MHz, `mode0`
- CS / DC / RST are driven through `gpio` (docs/decisions.md D-08). The pin
  numbers come from `board.pin-by-role` for `lcd-cs` / `lcd-dc` / `lcd-rst`
- The screen is 320x240, colors are RGB565 (sent big-endian: high byte first)

### Init sequence

Pull `rst` low for 10 ms, then high and wait 120 ms, then send these commands
in order.

| Command | Arguments | Meaning |
|---|---|---|
| `0x01` | none | SWRESET. Wait 120 ms after sending |
| `0x11` | none | SLPOUT. Wait 120 ms after sending |
| `0x3A` | `0x55` | PIXFMT = RGB565 |
| `0x36` | `0x28` | MADCTL = landscape (320x240), with the BGR bit set (see below) |
| `0x29` | none | DISPON |

`0x28` is `0b0010_1000`: MV (swap rows and columns, that is landscape) and
BGR (bit 3) are set. Despite the bit's name, the guest sends plain RGB565.
Most ILI9341 modules have panels whose subpixels are in BGR order, and this
bit compensates for that; `0xF800` was verified to show as red on a Pico 2 W
on 2026-09-26 (docs/verification-report.md §6). If red and blue appear
swapped, suspect this bit.

One command is sent in this order: pull `cs` low, send the command byte with
`dc` low, send the arguments and pixels with `dc` high, then return `cs` to
high. Pixels that span several rows are also sent within one `cs` low.

### Drawing primitives

Only two, and both have the shape "select a rectangle, stream pixels into it".

```
fill_rect(x, y, w, h, color)
draw_text(x, y, text, scale, fg, bg)   -- 8x8 bitmap font, monospace, scaled by scale
```

The rectangle is selected with CASET (`0x2A`), then PASET (`0x2B`), then
RAMWR (`0x2C`). CASET / PASET take 4 bytes (high and low byte of `x0`, high
and low byte of `x1`).

Pixels are sent row by row. One row is 320 x 2 = 640 bytes of buffer. There
is no full-screen frame buffer (it does not fit on RP2040).

Text is drawn scaled by `scale` (each dot becomes a `scale` x `scale`
square). The rectangle is `characters x 8 x scale` wide and `8 x scale` high.
For each font row, build one scaled pixel row and `spi.write` the same row
`scale` times (within one `cs` low: 8 font rows x `scale` = `8 x scale`
writes). One screen row of 640 bytes is enough buffer (the guest shadow stack
is 8 KiB).

Bounds (the same in both languages):

- `fill_rect` fails without drawing if `x + w > 320` or `y + h > 240`
- `draw_text` fails without drawing if the text is longer than 40 characters
  or `scale == 0` (never truncate silently). Width and height are compared
  with the screen before narrowing to u16
- Both succeed and do nothing for `w == 0` / `h == 0` / an empty string

### Colors

| Use | RGB565 |
|---|---|
| Background | `0x0841` |
| Header band | `0x1A3F` |
| Card background | `0x2104` |
| Value and header text | `0xFFFF` |
| Labels and tick marks | `0xA514` |
| Temperature card accent | `0xFC00` |
| Humidity card accent | `0x07FF` |
| Gauge track | `0x4208` |

The gauge fill changes color by zone on the `bar_px` axis:

| Zone (`bar_px`) | Temperature | RGB565 |
|---|---|---|
| 0..77 | -45..10 °C | `0x041F` |
| 77..105 | 10..30 °C | `0x07E0` |
| 105..133 | 30..50 °C | `0xFFE0` |
| 133..308 | 50..175 °C | `0xF800` |

### Screen layout

Drawn top to bottom in this order. The frame (1-5) is drawn before the sensor
is read; the values (6-7) are drawn once the reading succeeds. If the reading
fails, only the frame remains.

| # | Element | Calls |
|---|---|---|
| 1 | Background | `fill_rect(0, 0, 320, 240, background)` |
| 2 | Header | `fill_rect(0, 0, 320, 28, band)`, `draw_text(8, 6, "WASMICON", 2, text, band)`, `draw_text(272, 10, "SHT40", 1, text, band)` |
| 3 | Card (temperature, `x = 4`) | "One card" below. Label `TEMP`, unit `C`, temperature accent |
| 4 | Card (humidity, `x = 162`) | "One card" below. Label `HUMIDITY`, unit `%`, humidity accent |
| 5 | Gauge frame | `fill_rect(4, 162, 312, 72, card)`, `fill_rect(6, 178, 308, 16, track)`, tick marks `fill_rect(6 + t, 196, 1, 5, label)` for `t = 63, 98, 133, 203` in that order (0 / 25 / 50 / 100 °C), tick labels `draw_text(x, 206, s, 1, label, card)` for `(6, "-45")`, `(65, "0")`, `(131, "50")`, `(197, "100")`, `(282, "175C")` in that order |
| 6 | Values | Temperature, then humidity, centered in the card: `draw_text(x + (154 - characters x 24) / 2, 84, value, 3, text, card)` |
| 7 | Gauge fill | For each zone above, in order: if `bar_px > zone start`, `fill_rect(6 + start, 178, min(bar_px, zone end) - start, 16, zone color)`. If `bar_px` is 0, nothing is drawn |

One card is `fill_rect(x, 36, 154, 116, card)`, `fill_rect(x, 36, 154, 4, accent)`,
`draw_text(x + 8, 48, label, 1, label color, card)`, `draw_text(x + 130, 46, unit, 2, accent, card)`, in that order.

A value is formatted as "centi value -> sign + integer part + `.` + two
decimals" (no label or unit). The integer part is not zero-padded. Negative
values get a leading `-` (the decimals are those of the absolute value). The
longest are `-45.00` / `130.00` / `100.00`, 6 characters (144 px), which fit
in the 154 px card.

## 3. Font

8x8, one byte per row, bit 7 is the leftmost pixel. Only these 31 characters
are supported; anything else is drawn as the space.

```
' '  0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
'-'  0x00,0x00,0x00,0x7E,0x00,0x00,0x00,0x00
'.'  0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x00
'0'  0x3C,0x66,0x6E,0x76,0x66,0x66,0x3C,0x00
'1'  0x18,0x38,0x18,0x18,0x18,0x18,0x7E,0x00
'2'  0x3C,0x66,0x06,0x0C,0x18,0x30,0x7E,0x00
'3'  0x3C,0x66,0x06,0x1C,0x06,0x66,0x3C,0x00
'4'  0x0C,0x1C,0x3C,0x6C,0x7E,0x0C,0x0C,0x00
'5'  0x7E,0x60,0x7C,0x06,0x06,0x66,0x3C,0x00
'6'  0x1C,0x30,0x60,0x7C,0x66,0x66,0x3C,0x00
'7'  0x7E,0x06,0x0C,0x18,0x30,0x30,0x30,0x00
'8'  0x3C,0x66,0x66,0x3C,0x66,0x66,0x3C,0x00
'9'  0x3C,0x66,0x66,0x3E,0x06,0x0C,0x38,0x00
':'  0x00,0x18,0x18,0x00,0x18,0x18,0x00,0x00
'%'  0x62,0x66,0x0C,0x18,0x30,0x66,0x46,0x00
'C'  0x3C,0x66,0x60,0x60,0x60,0x66,0x3C,0x00
'H'  0x66,0x66,0x66,0x7E,0x66,0x66,0x66,0x00
'T'  0x7E,0x18,0x18,0x18,0x18,0x18,0x18,0x00
'A'  0x18,0x3C,0x66,0x7E,0x66,0x66,0x66,0x00
'D'  0x78,0x6C,0x66,0x66,0x66,0x6C,0x78,0x00
'E'  0x7E,0x60,0x60,0x78,0x60,0x60,0x7E,0x00
'I'  0x3C,0x18,0x18,0x18,0x18,0x18,0x3C,0x00
'M'  0x63,0x77,0x7F,0x6B,0x63,0x63,0x63,0x00
'N'  0x66,0x76,0x7E,0x7E,0x6E,0x66,0x66,0x00
'O'  0x3C,0x66,0x66,0x66,0x66,0x66,0x3C,0x00
'P'  0x7C,0x66,0x66,0x7C,0x60,0x60,0x60,0x00
'R'  0x7C,0x66,0x66,0x7C,0x78,0x6C,0x66,0x00
'S'  0x3C,0x66,0x60,0x3C,0x06,0x66,0x3C,0x00
'U'  0x66,0x66,0x66,0x66,0x66,0x66,0x3C,0x00
'W'  0x63,0x63,0x63,0x6B,0x7F,0x77,0x63,0x00
'Y'  0x66,0x66,0x66,0x3C,0x18,0x18,0x18,0x00
```

## 4. Behavior on failure

If any of `board` / `gpio` / `spi` / `i2c` fails, log the reason with
`log.error` and return from `run`. Do not trap. Both languages log the same
message.

| Situation | Message |
|---|---|
| A role name cannot be resolved | `role not found` |
| A pin cannot be opened | `gpio open failed` |
| SPI cannot be opened | `spi open failed` |
| I2C cannot be opened | `i2c open failed` |
| Display init or drawing failed | `display failed` |
| The sensor does not respond | `sensor read failed` |
| The CRC does not match | `sensor crc failed` |

### Releasing handles

Whichever path leaves `run`, the handles that were opened are released in
the same order: `i2c`, `spi`, `rst`, `dc`, `cs`, skipping what was never
opened.

In Rust, `Drop` runs in reverse declaration order, so the handles are
declared in one scope in this order. AssemblyScript has no `Drop`, so `run`
has a single exit that calls `close()` explicitly.

If the order differed on a failure path, the two languages' traces would
diverge when an error occurs on hardware, and the equality check would only
cover the happy path.
