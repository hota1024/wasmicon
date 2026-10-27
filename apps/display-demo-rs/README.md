# display-demo-rs

Demo screen drawn through `wasmicon:device/display` (docs/device-layer.md §4).
The host owns the panel, so the app has no driver, no pins and no bus. It asks
the screen for its size and lays the picture out from it, so the same `.wasm`
draws on a 320x240 and a 720x1280 display.

## What it shows

Top to bottom:

| Element | What it checks |
|---|---|
| `WASMICON` on a blue band, scaled text | `write-rect`, one glyph per call |
| The screen size (`320X240`) | `width` / `height` |
| 8 color bars (red, orange, yellow, green, cyan, blue, magenta, white) | Color order of the backend |
| A 16-step gray ramp | RGB565 packing and the byte order of `write-rect` (high byte first) |
| `DEVICE DISPLAY` | Text |
| A green square moving right and back | One `present` per frame |
| A 1 px white frame | Rectangles that end exactly at the edge |

## Running it

On the host (a 320x240 mock):

```sh
(cd apps && cargo build --release -p display-demo-rs)
cd apps/display-demo-rs
wasmicon run ../target/wasm32-unknown-unknown/release/display_demo_rs.wasm --display-out demo.png
```

On a board, an ILI9341 wired as for `apps/lcd-demo-rs` (its README) is the
display. `wasmicon.toml` names it per board in `[board.<name>.display]`, and
`deploy` writes it to the config slot:

```sh
wasmicon deploy ../target/wasm32-unknown-unknown/release/display_demo_rs.wasm --board esp32s3
```

The firmware prints `wasmicon: roles none display=ili9341 ...` at boot. rp2040
has no SPI, so `check` refuses the app there.
