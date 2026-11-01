# touch-demo-rs

Draws where the touch panel is touched, through `wasmicon:device/display` and
`wasmicon:device/touch` (docs/device-layer.md §4, §11). The host owns the panel
and the touch controller, so the app has no driver, no pins and no bus.

## What it shows

| Element | What it checks |
|---|---|
| A dot under each touched point | `touch.panel.read` coordinates are display pixels after rotation |
| One color per point of a read (white, green, cyan, yellow, magenta) | Multi-touch reads |
| CLEAR, the gray button at the top left | Clears the canvas when first touched |
| DONE, the red button at the top right | Ends the app with `touch-demo done` |

The app gives up after about five minutes without DONE (`touch-demo timed out`).

## Running it

On the host, the mock touch panel replays a script, one line per read
(`touch-script.txt` has the format):

```sh
(cd apps && cargo build --release -p touch-demo-rs)
cd apps/touch-demo-rs
wasmicon run ../target/wasm32-unknown-unknown/release/touch_demo_rs.wasm \
    --touch-script touch-script.txt --display-out touch.png
```

Without `--touch-script` nothing touches the panel, and the app runs until it
times out.

On the M5Stack Tab5 the app draws on the panel's 720x1280 pixels, through the
`tab5` display and touch entries of `wasmicon.toml`. The firmware is
`ports/esp32p4` (either build) from the commit that added the touch entry: an
older one cannot read config slot version 3 and runs with no display at all.
`deploy` writes the entries to the config slot through the Tab5's USB-C:

```sh
wasmicon deploy ../target/wasm32-unknown-unknown/release/touch_demo_rs.wasm \
    --board esp32p4-psram --no-run
```

The trace goes out on UART0, which on the Tab5 reaches only the M5-Bus, so a
3.3 V USB serial adapter there takes it (`ports/esp32p4/src/main.rs`), not
`--monitor`. Power-cycle the board to start the app. DONE is at the top right
(x from 640).

The other boards have no touch backend yet (docs/TODO.md §7), so
`wasmicon check --board` refuses the app on them.

A hardware trace replays on the host: `wasmicon trace replay --touch <log>`
writes the touch reads in the script format, and the host run with that script
traces the same calls (docs/device-layer.md §11.5). For a Tab5 trace the host
display must have the panel's size: add `--display-size 720x1280`.
