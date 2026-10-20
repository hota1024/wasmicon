# Wasmicon

A WebAssembly runtime for microcontrollers. Evaluation boards: ESP32-S3 and RP2350 (Pico 2 W).
ESP32-C5 is a port in progress (boots and blinks on hardware; status in docs/TODO.md §1.3.1).
RP2040 has a port but is build-only; it was dropped from evaluation on 2026-10-07.

- **Runtime**: a from-scratch Core Wasm interpreter. `no_std`, zero dependency crates, no `alloc`
- **HAL**: GPIO / I2C / SPI / time / log / board, defined in WIT. **The Component Model is not used.**
  WIT serves only as the IDL; the rules in `docs/abi-spec.md` lower it to Core Wasm imports
- **Bindings**: Rust, AssemblyScript and C, generated from the WIT
- **Goal**: write an app that reads an I2C temperature / humidity sensor (SHT4x) and draws on an
  SPI display (ILI9341), in Rust and in AssemblyScript; run the **same Wasm binary** on ESP32-S3
  and Pico 2 W; verify that the host-call traces match

## Status

The software side of all six phases is complete. The checks CI runs are in
`.github/workflows/ci.yml`.

**The runtime ran on a Raspberry Pi Pico 2 W** (2026-09-26). `apps/lcd-demo-rs` ran on the board, its
host-call trace was identical to the host port's (14,352 lines), and a picture appeared on the
ILI9341 ([`docs/verification-report.md`](docs/verification-report.md) §6).

**It also ran on an ESP32-S3** (2026-09-26). `ports/esp32s3` got an SPI2 implementation, and
**the same `lcd_demo_rs.wasm` that was flashed to the Pico 2** (`fc470947...`) produced a trace
identical to the host port's and a picture on the ILI9341
([`docs/verification-report.md`](docs/verification-report.md) §7).
**That completes Phase 6, "the same binary produces the same trace on two boards".**
**sensor-display also ran on both boards** (2026-10-05 / 07). It read the SHT40 over I2C, all
four combinations (Rust and AS versions on both boards) showed the display, and each board's
trace was identical to a host run fed the responses that board had read
(docs/verification-report.md §11, §12). **The owner declared the original goal achieved on
this basis** (2026-10-07). Remaining work: [`docs/TODO.md`](docs/TODO.md).

| Verification | Status |
|---|---|
| **Replace the app without reflashing the firmware** (app slot) | **Achieved on RP2350 and ESP32-S3 hardware (2026-10-04)**. On both boards the trace of the app run from the slot was identical to the host's (line count: docs/verification-report.md §6). RP2040 is not evaluated |
| **Traces via the slot match on two boards that read flash differently** | **Achieved (2026-10-04)**. The Pico 2 W passes the XIP slice straight to decode; the ESP32-S3 reads the header through `esp-storage` and copies the body into the arena. The same `.wasm` gives identical traces on both (line count: docs/verification-report.md §6) |
| Wasm spec conformance (spec testsuite core: 74 files / 22507 commands) | Achieved |
| Interpreter correctness (differential test against wasmtime, 4 guests) | Achieved |
| The Rust and AS versions emit the same host-call sequence (success and failure paths) | Achieved |
| GPIO / SPI / ILI9341 drawing works on RP2350 hardware | Achieved (2026-09-26) |
| GPIO / SPI / ILI9341 drawing works on ESP32-S3 hardware | Achieved (2026-09-26) |
| The same binary produces the same trace on two boards | Achieved (2026-09-26, RP2350 and ESP32-S3) |
| I2C (SHT40) reads on both boards | Achieved (2026-10-05) |
| All four combinations (Rust/AS x 2 boards) show the display | **Achieved** (2026-10-07, ESP32-S3 and Pico 2 W) |
| With values read on hardware, the drawn values (including f32) match the host | **Achieved** (2026-10-07, all four combinations) |

Details: [`docs/verification-report.md`](docs/verification-report.md).

Core code size is **49.5 KiB** for thumbv6m-none-eabi (RP2040) and **46.0 KiB** for
thumbv8m.main-none-eabihf (RP2350) (`sh tools/measure-size.sh [target]`).

## Layout

```
wit/                  HAL definition. Source of truth; never edit the generated files by hand
tools/wasmicon-gen/   Generator that emits the four generated files from wit/
tools/wasmicon-cli/   CLI for app authors (binary name wasmicon). check / run / trace diff / doctor
runtime/              wasmicon-core: the no_std / zero-dependency / no-alloc interpreter
ports/common/         HAL shared by the ports; only board-specific operations go in the Board trait
ports/host/           PC port: mock HAL + trace. CI runs here
ports/rp2040/         Raspberry Pi Pico WH (separate workspace; build-only, not evaluated)
ports/rp2350/         Raspberry Pi Pico 2 / Pico 2 W (separate workspace, Cortex-M33)
ports/esp32s3/        ESP32-S3 DevKitC-1 (separate workspace, esp toolchain)
ports/esp32c5/        ESP32-C5 (Waveshare ESP32-C5-WIFI6-KIT; separate workspace, RISC-V; hardware bring-up in progress)
bindings/rust/        Rust bindings for guests
bindings/assemblyscript/  AssemblyScript bindings for guests
bindings/c/           C bindings for guests (header + build.sh; built with clang's wasm32 target)
apps/                 Guests (separate workspace). Rust and AS versions of blink and sensor-display
                      blink-c is the C version of blink (checks bindings/c)
                      lcd-demo-rs is a demo that uses only the ILI9341 (no sensor needed)
                      life-rs keeps drawing Conway's Life on the ILI9341 (with an FPS readout)
                      bench-rs measures interpreter and host-call speed (same .wasm on host and hardware)
                      rgb-led-rs drives the on-board RGB LED (esp32c5 GPIO27, esp32s3 GPIO48)
verify/               Verification tools: differential test against wasmtime, trace diff
docs/                 Spec, design, verification report, remaining work
```

`ports/rp2040` / `ports/rp2350` / `ports/esp32s3` / `ports/esp32c5` / `apps` / `verify/differential`
are **separate workspaces** because their targets, toolchains and profiles all differ.

## Build and verify

The commands can be pasted in order from the top. **Fetch the testsuite before `cargo test`.**
Without it the spec tests print "not fetched", skip entirely, and still end with
`test result: ok`.

```bash
# Testsuite for the spec tests (pinned SHA). Run before cargo test.
sh tools/fetch-testsuite.sh

# Host side (runtime, generator, host port)
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test

# Generated files match wit/
cargo run -p wasmicon-gen -- --check
sh tools/check-sigs.sh          # cross-check against the reference tools/wit2sig.py

# Guests (wasm32-unknown-unknown and AssemblyScript)
npm ci
(cd apps && cargo build --release)
(cd apps/sensor-display-as && npx asc assembly/index.ts --config asconfig.json --target release)
# C guest. Needs a clang with the wasm32 target and wasm-ld (wasi-sdk, or clang + lld).
# The host tests in cargo test call this too
sh bindings/c/build.sh apps/blink-c/build/blink_c.wasm apps/blink-c/src/main.c
# Guest unit tests. apps/ pins wasm32, so name the host triple explicitly.
# The generated bindings emit two clashing_extern_declarations warnings; that is
# a known issue visible only on the host target (docs/TODO.md §2.2)
(cd apps && cargo test --target "$(rustc -vV | sed -n 's/^host: //p')")

# Firmware for the boards. It contains no app (no need to build apps/ first).
# Apps are flashed separately; see "Flash an app to the slot" below
(cd ports/rp2040 && cargo build --release)
(cd ports/rp2350 && cargo build --release)
sh ports/esp32s3/build.sh build --release   # sources ~/export-esp.sh, then calls cargo
(cd ports/esp32c5 && cargo build --release)  # RISC-V; upstream Rust is enough

# Differential test against wasmtime (interpreter correctness)
(cd verify/differential && cargo test)

# Cross-check hardware traces (two boards)
sh verify/diff-traces.sh pico.log esp32s3.log
```

### Toolchain

Five `rust-toolchain.toml` files (root, `apps`, `ports/rp2040`, `ports/rp2350`,
`ports/esp32c5`) pin the same stable version, 1.99.0 (pinned 2026-10-08), so that they do
not drift from each other or from CI; bump them together. `ports/esp32s3` uses the espup
fork (`channel = "esp"`) because upstream Rust has no Xtensa target (docs/decisions.md P-10).

### Flash an app to the slot

**The firmware contains no app** (the built-in app was removed on 2026-10-07). Once the firmware
is flashed, apps are flashed to the slot and replaced there
([`docs/app-workflow.md`](docs/app-workflow.md)). With an empty slot the firmware prints
`wasmicon: slot empty, idle` and parks (it drives no pins).

**The wiring (role name -> GPIO) is not in the firmware either** (2026-10-07). Write it under
`[board.<name>.roles]` in the app's `wasmicon.toml`; `deploy` writes it to the device's config
slot together with the app. The boot banner shows `wasmicon: roles lcd-cs=17 ...`
([`docs/app-workflow.md`](docs/app-workflow.md) §3.9). Every app under `apps/` has one.

```bash
# Build the CLI. To put it on PATH: cargo install --path tools/wasmicon-cli
cargo build --release -p wasmicon-cli
w=./target/release/wasmicon
app=apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm

$w doctor                            # are the tools installed, and which boards can be flashed

# Build the app and check that it runs on that board
(cd apps && cargo build --release -p lcd-demo-rs)
$w check "$app" --board rp2350

# Flash and capture the trace in one go (nothing is flashed if the check fails)
$w deploy "$app" --board rp2350 --monitor -o pico.log

# Cross-check against a host run
$w run "$app" --trace > host.log
$w trace diff host.log pico.log
```

- **Hold BOOTSEL while plugging in the Pico's USB, then** `deploy`
  (the image goes through `picotool`; the stage-2 USB control channel will remove this step)
- **The ESP32-S3 needs no button** (`espflash` resets it over DTR/RTS). Flashing and the trace
  share one port, so `--monitor` delegates to `espflash monitor`
- To start a new app: `wasmicon new <dir> --lang rust --hal bindings`

### Other

```bash
wasm-tools component wit wit/                              # WIT syntax check
wasm-tools json-from-wast <file>.wast -o out/<file>.json   # convert a spec test (formerly wast2json)
wasm-tools validate --features=mvp,sign-extension,saturating-float-to-int,bulk-memory,multi-value,mutable-global app.wasm
rustup target add thumbv6m-none-eabi                       # RP2040
rustup target add thumbv8m.main-none-eabihf                # RP2350
rustup target add riscv32imac-unknown-none-elf             # ESP32-C5
espup install                                              # ESP32-S3 (Xtensa fork)
sh tools/measure-size.sh                                   # core code size
```

## Documents

| File | Contents |
|---|---|
| [`docs/TODO.md`](docs/TODO.md) | **Remaining work. The one place that says what is left** |
| [`docs/abi-spec.md`](docs/abi-spec.md) | WIT -> Core Wasm lowering rules. **Normative** |
| [`docs/decisions.md`](docs/decisions.md) | Frozen decisions (D-NN), adopted defaults (A-NN) and pitfalls (P-NN), with stable IDs that code cites |
| [`docs/style-guide.md`](docs/style-guide.md) | Language, comment, file-header, citation, error-string and commit conventions |
| [`docs/app-workflow.md`](docs/app-workflow.md) | Design of the app workflow: firmware that loads apps over USB / HTTP, and the `wasmicon` CLI |
| [`tools/wasmicon-cli`](tools/wasmicon-cli) | The `wasmicon` CLI: `new` / `check` / `run` / `pack` / `deploy` / `monitor` / `trace diff` / `doctor`. Also the source of truth for the guest build flags (`src/flags.rs`) |
| [`docs/verification-report.md`](docs/verification-report.md) | What has been verified, how far, and what has not |
| [`docs/design-notes.md`](docs/design-notes.md) | Background, direction and the reasons behind the technical choices |
| [`docs/handoff.md`](docs/handoff.md) | Historical record: the decisions, phases and pitfalls as written at the time. No longer cited from code; the live entries are in `docs/decisions.md` |
| [`apps/README.md`](apps/README.md) | Shared spec that keeps the Rust and AS versions drawing the same thing |
| [`apps/lcd-demo-rs/README.md`](apps/lcd-demo-rs/README.md) | Pico 2 (W) + ILI9341 demo: wiring, flashing, pitfalls |

Development continues on `main`. The `v2` branch was merged into `main` and deleted. The old
implementation (wasm decoder / llvm experiments, the 2024 prototype) is still reachable: it is
the second parent of merge commit `8c26071` in `main`'s history (`git log 8c26071^2`).

## Contributing

- Read [`AGENTS.md`](AGENTS.md) (rules, what needs the owner's approval, how to verify) and
  [`docs/style-guide.md`](docs/style-guide.md) (language, comments, citations, commit messages).
- The repository was translated from Japanese to English in one series of commits. To keep
  `git blame` useful, run once: `git config blame.ignoreRevsFile .git-blame-ignore-revs`.
