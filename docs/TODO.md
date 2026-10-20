# Remaining work

**The original goal has been reached** (owner decision, 2026-10-07: sensor-display runs on
ESP32-S3 and Pico 2 W and the traces match. `README.md`). What remains here are
improvements outside that goal, open questions, and post-MVP work. All remaining work is
collected here. Scattered lists go stale and start lying, so **remaining work is written in
this file only**. Frozen decisions and adopted defaults live in `docs/decisions.md`, the
history in `docs/handoff.md` (a historical record), and the verification status in
`docs/verification-report.md`.

---

## 1. Work that needs hardware

**A Raspberry Pi Pico 2 W and an ESP32-S3 DevKitC-1 are on hand** (both verified on
hardware with `lcd-demo-rs` on 2026-09-26. docs/verification-report.md §6, §7).
**The temperature / humidity sensor on hand is an SHT40** (2026-09-29. docs/decisions.md D-09
was changed from SHT31/SHT30 to SHT4x and the guest driver was fixed).
**RP2040 (Pico WH) was dropped from the evaluation** (owner decision, 2026-10-07. `README.md`).
The two boards of the Phase 4/5/6 completion criteria are **ESP32-S3 and Pico 2 W (RP2350)**.
The `ports/rp2040` code and its CI build stay, but there is no hardware verification and no
I2C / SPI implementation for it.
I2C and sensor-display were verified on both boards on 2026-10-05 / 07
(docs/verification-report.md §11, §12).

### 1.1 Questions for the owner

- [x] **Wiring on hardware** (closed 2026-10-07. Everything except `led` is verified on both boards; `led` is the item below). Whether the table in docs/abi-spec.md §8 (I2C/SPI pins, role name -> GPIO number) matches the hardware
      - RP2350 LCD side (`lcd-cs` / `lcd-dc` / `lcd-rst`, SCK=GP18 / MOSI=GP19) verified on
        2026-09-26, I2C (SDA=GP4 / SCL=GP5) on 2026-10-05
        (docs/verification-report.md §11). **Only `led` remains** (RP2040 is not evaluated)
      - ESP32-S3 LCD side (CS=GPIO10 / DC=GPIO14 / RST=GPIO15, SCK=GPIO12 /
        MOSI=GPIO11) verified on 2026-09-26. The wiring table is in
        `apps/lcd-demo-rs/README.md`. I2C (SDA=GPIO8 / SCL=GPIO9) was also
        verified on 2026-10-05. **Only `led` remains**
      - **I2C needs external 10 kΩ pull-ups on SDA / SCL.** Both ports enable the
        internal pull-ups (esp-hal's `connect_pin` always applies `Pull::Up`, so
        rp2350 was matched to that), but they are weak: 50..80 kΩ on RP2350 and about
        45 kΩ on ESP32-S3, barely enough for `standard` (100 kHz) over short wires.
        Sensor modules usually carry their own, so **if the module has them, none need
        to be added**
- [x] **How the serial port is connected** (closed 2026-10-07. UART0 verified on both boards). RP2350 is assumed to use UART0 (GP0/GP1), ESP32-S3 UART0 (GPIO43/44)
      - RP2350: UART0 + a USB serial adapter (CP2102N), verified on 2026-09-26
      - ESP32-S3: UART0 (GPIO43/44), also verified on 2026-09-26. **The bridge on the
        `USB-UART` side of the board on hand is a CH343 (VID 0x1A86 / PID 0x55D3), which
        macOS shows as `/dev/cu.usbmodem*`** (not a CP2102N, so looking for `usbserial`
        finds nothing). Flashing also works through the `USB-OTG` side (USB-Serial-JTAG)
      - **When using the `USB-UART` side of the ESP32-S3, do not connect an external USB
        serial adapter to UART0.** The CH343 TX is wired straight to GPIO44 on the board,
        so the two drivers fight and flashing stops working (hit on 2026-10-07.
        docs/verification-report.md §12)
- [x] **Module part numbers** (closed 2026-10-07. The ILI9341 is the 3.3 V SPI version and displays on both boards. The SHT40 reads at address 0x44). The ILI9341 is assumed to be the 3.3 V logic SPI version
      - The sensor is **fixed as SHT40** (2026-09-29). But **the I2C address depends on
        the suffix** (-AD1B is 0x44). The driver keeps `0x44`, so **confirm against the
        marking of the part on hand or a bus scan**. If it differs, fix **three places**:
        `ADDRESS` in `apps/sensor-display-rs/src/sht4x.rs` and
        `apps/sensor-display-as/assembly/sht4x.ts`, and `SHT4X_ADDR` in
        `ports/host/tests/apps.rs` (the value the host test expects in the trace;
        without it the test fails with the misleading message that the measurement
        command differs)
- [x] **Role names** (owner decision 2026-10-07: the defaults are final). `led` / `lcd-cs` / `lcd-dc` / `lcd-rst`. Changing them means three places (the comment in `wit/board.wit`, the table in docs/abi-spec.md §8, `profile` in `ports/common`). **Changing only a number is one place, `profile`**; adding to the vocabulary (names) also means `ROLE_NAMES` (`assert_role_names` rejects a mismatch at compile time) [Superseded: names are free-form and wiring lives in wasmicon.toml, §5-9]
- [x] **An external LED serves as `led`** (closed 2026-10-07. **`led` is no longer a firmware role**. Role names have no fixed vocabulary; the wiring goes in the app's `wasmicon.toml` (§5-9). The blink apps' `apps/blink-*/wasmicon.toml` assign `led` to GP15 / GPIO2. Actually lighting an external LED stays in the Phase 4 blink item). No board has an onboard LED on a plain GPIO (Pico W/WH and Pico 2 W drive it through the CYW43439, the DevKitC-1 has a WS2812). Only the Pico 2 (no radio) has GP25 as a plain LED, but it is kept external to match the Pico 2 W
- [x] **RP2350 board variant** -> fixed as **Pico 2 W** (RP2350A, GP0..GP29). `NUM_GPIO` stays 30
- [x] **Treat RP2350 as Arm only?** (owner decision 2026-10-07: Arm (Cortex-M33) only. The RISC-V Hazard3 cores are out of scope). `ports/rp2350` targets Cortex-M33 (`thumbv8m.main-none-eabihf`) only. Whether RISC-V (Hazard3) produces the same trace is not part of the v0.1 verification

### 1.2 Implementation

- ~~I2C / SPI for `ports/rp2040`~~ -> **not evaluated** (2026-10-07). They keep returning
      `unsupported`. Note for the record in case it comes back: the rp2350 implementation
      can be carried over as is (RP2040 and RP2350 have the same DW_apb_i2c for I2C and the
      same PL022 for SPI; the only differences are the missing `ISO` bit in PADS and the
      spelling of FUNCSEL)
- [x] **SPI for `ports/rp2350`**. SPI0 (PL022) is driven through registers directly.
      **Verified on hardware on 2026-09-26** (docs/verification-report.md §6).
      When adding it to rp2040, match the frequency rounding (largest value not above
      the request) and "do not return until `BSY` clears after sending"
      (on esp32s3 the `esp-hal` driver already satisfies both)
- [x] **I2C for `ports/rp2350`**. I2C0 (DW_apb_i2c) is driven through registers directly
      (2026-10-04). Initialization and transfer follow `i2c/controller.rs` in
      `rp235x-hal`, but return error codes instead of `assert!` (the panic handler
      cannot report a reason -> §1.4). Waits are bounded by TIMER0 wall time, and
      **on timeout the transfer is aborted through `IC_ENABLE.ABORT` before**
      `timeout` is returned (otherwise the bus stays held).
      **The SCL counters are derived from `clk_sys`** (the PL022 SPI runs on `clk_peri`,
      but DW_apb_i2c runs on `clk_sys`. Both default to 150 MHz, so mixing them up still
      works by accident). ~~Not yet verified on hardware~~ -> verified on hardware on 2026-10-05 (docs/verification-report.md §11)
- [x] **SPI for `ports/esp32s3`**. SPI2 (FSPI) uses the `esp-hal` `spi::master`
      driver (unlike GPIO, not direct register access). The points matched to rp2350
      are in the module comment of `ports/esp32s3/src/board.rs`.
      **Verified on hardware on 2026-09-26** (14,352 trace lines identical + a picture
      on the ILI9341. docs/verification-report.md §7)
- [x] **I2C for `ports/esp32s3`**. I2C0 uses the `esp-hal` `i2c::master` driver
      (2026-10-04; like SPI, not direct register access). Error mapping matches rp2350
      (NACK -> `nack`, arbitration loss -> `io`, `esp-hal`'s `Error::Timeout` ->
      `timeout`). ~~Not yet verified on hardware~~ -> verified on hardware on 2026-10-05 (docs/verification-report.md §11)

### 1.3 Verification (Phase 4 / 5 / 6 completion criteria)

- [x] **`lcd-demo-rs` displays on the Pico 2 W** (reached 2026-09-26. Details in
      docs/verification-report.md §6). The trace is identical to the host port over
      14,352 lines and a picture appeared on the screen. **The I2C side remains**
- [ ] **`blink-rs` / `blink-as` run on both boards (ESP32-S3 and Pico 2 W) and the serial trace matches the host version (Phase 4)**.
  GPIO itself works on both boards through `lcd-demo-rs` / `sensor-display`, but blink has
  not been flashed yet. It needs the external LED for the `led` role (§1.1). **This is the only
  Phase 4 item left**. The owner declared the original goal reached on 2026-10-07, so **it does
  not count toward the goal** (optional extra verification)
- [x] **`lcd-demo-rs` displays on the ESP32-S3** (reached 2026-09-26.
      docs/verification-report.md §7). The trace is identical to the host over 14,352
      lines and a picture appeared on the screen. **The I2C side remains**
- [x] **All four combinations (Rust/AS x ESP32-S3/Pico 2 W) display (Phase 5)**. Reached 2026-10-07
  (docs/verification-report.md §12. See the AS item below)
- [x] Run the same `.wasm` on both boards; the trace except `time` and the SPI pixel CRCs are identical (Phase 6)
      -> **Reached on 2026-09-26 with RP2350 and ESP32-S3** (both identical to the host
      reference over 14,352 lines, including 3,272 `spi.write` CRC-32 values.
      docs/verification-report.md §6, §7). **Matching displays are a separate item** (above)
  - **sensor-display matches on both boards for every line up to the sensor read**. After
    the read the values differ per board, so each board was compared against the host fed
    with that board's responses, in all four combinations (docs/verification-report.md §11, §12). **There is no way to
    inject the same response into both boards**
  - [x] **Phase 6 is therefore considered complete** (owner decision 2026-10-07: the original
    goal is reached). To strictly check "the whole sensor-display trace is identical on
    both boards", add an I2C replay mode to the ports and feed both boards the same response
  - Procedure: docs/verification-report.md §5
  - Comparison: `sh verify/diff-traces.sh a.log b.log`
- [ ] **Floating-point match between boards**. sensor-display is the only app using f32, in the temperature bar computation. ESP32-S3 and RP2350 have an f32-only hardware FPU (denormal handling depends on configuration). This is the real measurement target of Phase 6 (RP2040 soft-float is no longer evaluated)
  - RP2350 is built with the hard-float ABI (`thumbv8m.main-none-eabihf`). `cortex-m-rt` enables the FPU, FPSCR stays at its default (round to nearest, flush-to-zero off), so it should be IEEE compliant. Confirm on hardware
  - **One data point on RP2350 on 2026-10-07** (docs/verification-report.md §12).
    Feeding the response read by the Pico 2 W to the host gave a trace identical to the
    board, including the temperature bar (Rust and AS versions). **The ESP32-S3 gave the
    same result the same day** (all four combinations). **There is still no way to feed
    the same input to both boards**, so the claim is limited to "each board == host"
  - The RP2350 DCP (the f64 coprocessor) is not used. Enabling `dcp-fast-f64` in `rp235x-hal` would replace `__aeabi_dadd` / `__aeabi_dmul`. Faster, but whether the results match is unverified, so it stays out until Phase 6 passes
- [x] **Read the SHT40 on hardware** (reached 2026-10-05. docs/verification-report.md §11).
  sensor-display-rs ran on ESP32-S3 and Pico 2 W; both read on the first try and displayed.
  **The 3,756 lines up to the sensor read are identical across host and both boards**.
  Address 0x44 was correct
- [x] **Run the AS version (`sensor-display-as`) on hardware** (2026-10-07, reached on both
  Pico 2 W and ESP32-S3. The responses read on hardware were fed to the host Rust and AS
  versions and the traces are identical. docs/verification-report.md §12)
- [ ] **Find out why USB reset stopped working on the ESP32-S3** (2026-10-07).
  **Narrowed down** (docs/verification-report.md §14): with the `USB-OTG` side connected
  to the Mac, even `RST` enters download mode (`boot:0x20`). With only the `USB-UART` side
  connected, `RST` boots normally, and with the CP2102N removed, flashing through the CH343
  works too. **What remains is only that the app does not run after `deploy --monitor`
  (the reset done by `espflash monitor`)**. The original record follows:
  flashing from USB-OTG stops in download mode (`boot:0x21`); the CH343 auto-reset drives
  EN but not GPIO0. In docs/verification-report.md §10 (2026-10-04) `deploy --monitor` worked as one command, so
  something changed. The workaround is to boot with the `RST` button
  (docs/verification-report.md §12 "Where the ESP32-S3 got stuck"). Until fixed,
  `deploy` on the ESP32-S3 is two steps: flash, then press `RST`
- [x] Replace `verify/sht4x-replay.txt` with **a response recorded on hardware** (2026-10-07.
  SHT40 on the Pico 2 W, T=25.63C / RH=74.49%). The owner decided to emit the bytes read
  (`data=`) in the trace (docs/abi-spec.md §9), and `wasmicon trace replay` extracted them
- [x] Record the results in docs/verification-report.md §11, §12

### 1.3.1 ESP32-C5 support (started 2026-10-12. Waveshare ESP32-C5-WIFI6-KIT)

`ports/esp32c5` is built like `ports/esp32s3` (GPIO through registers directly, SPI / I2C
through the `esp-hal` 1.2 drivers). Pins: docs/abi-spec.md §8. Each stage is verified on
hardware and committed separately.

- [x] **(1) The firmware boots and announces itself** (2026-10-12. The chip is ESP32-C5 rev v1.0, 16 MB flash.
  The ESP-IDF second-stage bootloader runs and the firmware prints `wasmicon id esp32c5 ...`. With the app
  slot and config slot empty it prints `magic mismatch` and parks)
- [x] **(2) `deploy` runs blink from the slot** (2026-10-12. The wiring table `led`=GPIO4 is read and
  the blink-rs trace is identical to the host over 20 lines)
- [x] **Light the on-board RGB LED (WS2812B, GPIO27) with `apps/rgb-led-rs`** (2026-10-20. Firmware and
  app built from cfb7f75. The LED lit red -> green -> blue -> white -> rainbow -> off as intended (owner
  checked by eye), and the trace is identical to the host over 978 lines. `spi.bus` index 1 on the C5
  is a bus that routes only the SPI2 MOSI to GPIO27, docs/abi-spec.md §8; no wiring needed)
- [ ] **(3) SPI: lcd-demo-rs / life-rs draw on the ILI9341** (the lcd-demo trace matches the host).
  **lcd-demo-rs done** (2026-10-20): the picture appears and the trace is identical to the host over
  14,352 lines (docs/verification-report.md §20). **life-rs is still to run**
- [ ] **(4) I2C: sensor-display-rs / -as read the SHT40 and the whole trace is identical to the host**
- Decided and known so far:
  - **Less RAM than the S3.** The main RAM usable with the esp-hal linker script is 314 KiB.
    The arena is 288 KiB (4 pages + the 32 KiB floor) and the scratch area (8 KiB) sits in
    the region reclaimed from the bootloader (`ram(reclaimed)`). The native stack is
    20,168 B (the S3 gets by with about 17 KB)
  - **No FPU** (RV32IMAC). f32 arithmetic is done in software. Whether the sensor-display
    temperature bar matches the host is checked in (4) (the floating-point item in §1.3)
  - Flashing uses the "UART" Type-C port (CH343, with the auto-download circuit). Booting
    with the native USB port connected may enter download mode as on the S3 (unconfirmed)
  - **Once the firmware has run and read the serial port, `espflash` fails with
    `Failed to connect`** (same symptom as the S3. See the S3 reset item in §1.3). Unplugging
    and replugging USB restores it. Dropping DTR/RTS, `-c esp32c5` and `--before usb-reset`
    did not help. After `espflash reset` the output stops in the middle of the ROM's first
    line; the RESET button boots normally

### 1.4 What to suspect first on hardware

**RP2350 and ESP32-S3 are verified on hardware** (both on 2026-09-26). `lcd-demo-rs` ran on the
Pico 2 W and the ESP32-S3 DevKitC-1, the host call trace is identical to the host port
(14,352 lines on both, including 3,272 `spi.write` CRC-32 values), and a picture appeared
(docs/verification-report.md §6, §7). Of the concerns below, **those about GPIO / SPI
register settings** are resolved on RP2350 / ESP32-S3 (the `ARENA` and native stack items
are not resolved; they are still valid warnings). But **the ESP32-S3 actually hit "the trace
is identical yet the pins do not move"** (`out_sel`, below), so **a matching trace alone is
no proof that GPIO works**. **RP2040 remains unobserved** (not evaluated, 2026-10-07). If it
comes back, expect "blink does not light" as the first outcome.

- RP2040: SIO / IO_BANK0 / PADS_BANK0 (FUNCSEL=5)
- RP2350: the same registers. In addition, **the reset value of `ISO` (the isolation latch) in PADS_BANK0 is 1**.
  If it is not cleared the pad stays isolated: the registers read correctly but the GPIO does nothing.
  `gpio_configure` does `iso().clear_bit()` on every write to PADS (`write()` starts from the reset
  value, so leaving it out re-isolates the pad)
- ESP32-S3: GPIO / IO_MUX (MCU_SEL=1, GPIO matrix **out_sel=256**).
  **This was hit on hardware on 2026-09-26.** `out_sel` was set to 128 (the value for the
  RISC-V parts such as C3 / C6; ESP32 / S2 / S3 use 256), so `GPIO_OUT` / `GPIO_ENABLE`
  read back correctly but no pin moved at all. The symptom: the host call trace was
  identical to the host over 14,352 lines while the ILI9341 stayed white.
  docs/verification-report.md §7
- ESP32-S3: SPI2 is left to the `esp-hal` driver, so the port holds no signal numbers of its own.
  A mix-up like the one above cannot happen there
- **I2C worked on the first try on both ESP32-S3 and RP2350 on 2026-10-05**
  (docs/verification-report.md §11). Kept for when it is ported to RP2040 or the wiring
  changes. The first expected failure is "`i2c.bus.open` succeeds but `write` returns
  `nack`". Check in this order:
  1. **External pull-ups** (§1.1). Without them SDA/SCL cannot return high and the
     address is never ACKed. The internal pull-ups alone are too weak
  2. **Address** (§1.1). SHT4x addresses vary by suffix. On `nack`, look here first.
     A different variant means fixing three places
  3. **SDA / SCL swapped**. `open` still succeeds when they are swapped
  4. RP2350 only: **`ISO` in PADS**. The same pitfall as GPIO / SPI; `i2c_open` clears it
     together with `pue`. If the registers read correctly but there is no waveform, look here
  5. ESP32-S3 only: the `esp-hal` driver owns the GPIO matrix signal numbers, so a mix-up
     like the `out_sel` one in docs/verification-report.md §7 cannot happen
  - **A `timeout` means the bus is held** (SCL stuck low). rp2350 gives up after 25 ms per
    byte of TIMER0 wall time. It never stops silently, so the last trace line shows the reason

- ESP32-S3: **the native stack is what is left of `ARENA`**. The `esp-hal` linker script
  puts `.stack` in the remainder of dram_seg, so with `ARENA` at 300 KB only 17.4 KiB is
  left (right after `.bss` at 315,496 B, up to the end of dram_seg at 0x3FCDB700). The
  interpreter keeps call frames in an array inside the arena and does not recurse natively,
  but growing `ARENA` shrinks this. An overflow enters the panic handler (which cannot
  report a reason) and stops silently, so suspect this when "the trace stops midway and
  nothing follows"
  - **About 4 KiB of those 17.4 KiB are used by `esp-storage`.** Its `read` **creates a
    4096-byte sector buffer on the stack on every call** (`FlashSectorBuffer`; its
    documentation says so). The slot read calls it many times, but sequentially, so the
    peak is one buffer. **When growing `ARENA`, budget with this 4 KiB subtracted**
    (the 17.4 KiB figure does not include it yet)

---

## 2. Decisions that need no hardware

- [x] **Pin the toolchain?** -> **Pinned to 1.99.0** (owner decision 2026-10-08; the same stable version CI was passing on). The four `rust-toolchain.toml` files (root / `apps` / `ports/rp2040` / `ports/rp2350`) are raised together. The CI runner is pinned to `ubuntu-24.04` and the actions were raised to the v5 releases that run on Node.js 24. **With 1.99.0 the bytes of `lcd_demo_rs.wasm` change** (`fc470947...` -> `84d6de0f...`, same size 4,088 B), but the host trace is identical to the 1.97.1 build over 14,352 lines (the recorded traces stay valid. docs/verification-report.md §5.1). The original record follows: `rust-toolchain.toml` had `channel = "stable"`, floating. A new clippy lint or a change in rustfmt output makes CI fail out of nowhere (the very first CI run did exactly that: 1.97.1 locally / 1.98.0 in CI). The "generated files diff zero" check in particular depends on rustfmt output, so it passes locally and fails in CI. Pinning means manual version bumps
- [ ] **State the external pull-up on `lcd-rst` in the docs/abi-spec.md §8 table?**
      `pin.drop` returns the pin to input with no pull, as docs/abi-spec.md §5.2 says, so **the LCD `RESET`
      floats once the demo finishes drawing**. If the module has no pull-up, the panel
      resets and the screen goes white (observed on ESP32-S3 hardware on 2026-09-26.
      docs/verification-report.md §7 "Cause 3")
      - **Not a bug; a consequence of docs/abi-spec.md §5.2.** The idle level of a line is for the external
        circuit to decide
      - **Whether it happens depends on the board** (measured 2026-09-26). With the same
        module the ESP32-S3 goes white and **the Pico 2 W keeps the picture**. The mechanism
        is the same in both ports (the pin returns to high impedance); whether the floating
        line crosses the panel's threshold depends on pad leakage and capacitance. **It is
        not a difference in port code**
      - In practice a 10 kΩ pull-up on `RESET` is enough. **Confirmed on ESP32-S3 hardware
        on 2026-09-26** (the picture stayed with the shipping configuration). **Zero code
        changes, and neither the `.wasm` hash nor the recorded traces move**, so this is the
        preferred option. That the Pico 2 W kept the picture without a pull-up is an
        observation, not a guarantee, so "add it on both boards" is the safe side
      - **Leaving handles unreleased in the guest does not avoid it either.** docs/abi-spec.md §5.2 also
        says "when `run` returns, the host drops all remaining handles", so the release
        always happens. Only an external pull-up or a change to docs/abi-spec.md §5.2 would help
      - Changing docs/abi-spec.md §5.2 (drop no longer returns the pin to input) is **an ABI change**, so it
        is recorded here instead of implemented (docs/decisions.md D-03)
      - Deciding "whether the picture survives after `run()` returns is not part of the
        spec" and only recording it would also be consistent. The current text guarantees
        it nowhere
- [ ] **Align the "drop -> log" order in docs/abi-spec.md §6.6 with the implementation?**
      (2026-10-04). The implementation **prints the reason first, then cleans up**. The
      cleanup can wait without bound (the `BSY` wait in `spi_close`, etc. §2.1), so doing it
      first means that when a peripheral hangs the firmware stops silently with no reason
      printed, breaking "log and halt" (docs/decisions.md A-04)
      - The only observable difference is whether the reason is printed; the handles are
        released in either order. The ABI (import names, signatures, error codes) is not
        touched
      - Whether to fix the docs/abi-spec.md §6.6 text or revert the implementation to the text is the owner's
        call
- [ ] **Make open questions 2-5 of docs/abi-spec.md §10 final?** All are implemented with their defaults and work
  - #2 behavior during `sleep-ms` -> left to each port's HAL (implemented)
  - #3 behavior after a trap -> log and halt, no restart (implemented)
  - #4 UTF-8 validation of `log` -> none (implemented)
  - #5 keep `spi.transfer` in v0.1 -> kept (implemented)

### 2.1 Open questions from the SPI implementation (`/code-review xhigh` findings, 2026-09-26)

All of these came up when SPI was added to `ports/rp2350`. **1 and 2 concern the error
status of imports, so they are ABI changes** (docs/decisions.md D-03). They are recorded
here instead of implemented.

- [ ] **Align the failure conditions of `spi.bus.open` across all ports?** `ports/rp2350`
      returns `invalid-argument` for `frequency-hz == 0` and `unsupported` for `index == 1`,
      but the `ports/host` mock succeeds on both and `ports/common` does not check the
      frequency. **The same `.wasm` produces different traces on host and hardware**, which
      conflicts with "the same binary produces the same trace on both boards"
      (docs/decisions.md D-10). Neither `lcd-demo-rs` nor `sensor-display` hits it, but any
      app that does will mismatch
      - If aligned, the check belongs in `ports/common` (every port gets the same verdict)
      - `wit/spi.wit` says "frequency-hz is rounded to the nearest value the host supports"
        and does not define 0 as a failure. `docs/abi-spec.md` only makes zero-length
        transfers `invalid-argument`. **Which way to go is the owner's decision**
      - `ports/esp32s3` got the same verdicts as rp2350 (`index != 0` -> `unsupported`,
        `frequency-hz == 0` -> `invalid-argument`). **Only requests below the minimum are
        not aligned**: rp2350 clamps to the slowest divider and succeeds, esp32s3 returns
        `unsupported` because `esp-hal` rejects out-of-range values (below 78.125 kHz at
        APB 80 MHz). Neither demo hits it
      - **I2C has the same kind of discrepancy** (found when it was implemented on
        2026-10-04). Both ports return `unsupported` for `i2c.bus.open` with `index != 0`
        and `invalid-argument` for addresses outside 7 bits and zero-length transfers, but
        **the `ports/host` mock checks none of them**. SHT4x uses only index 0 / 0x44, so it
        is not hit; aligning it means putting the checks in `ports/common`, as for SPI
        - Zero length never actually arrives, because `ports/common` rejects it first. Both
          ports still reject it explicitly because `esp-hal`'s `transaction_impl` **drops an
          empty read from the transfer** (left alone it returns `Ok(0)` and mismatches rp2350)
        - Timeouts **use the same budget on both ports** (25 ms per byte). esp32s3 sets
          `SoftwareTimeout::PerByte` explicitly. `Config::default()` is
          `SoftwareTimeout::None`, which leaves only the FSM hardware timeout (about 0.2 s),
          an order of magnitude off
- [ ] **Bound the SPI wait loops?** In `ports/rp2350`, the `BSY` wait in `spi_drain`, the
      RESETS completion wait, and the `TNF` / `RNE` waits in `spi_write` / `spi_transfer`
      all spin without bound. If the clock stops or the peripheral hangs, the firmware stops
      silently with the last trace line on the UART. That is exactly the failure mode
      `ports/rp2350/src/main.rs` and §1.4 promise to make diagnosable
      - `types.error-code` already has `timeout`, so the type is sufficient. But **a state
        that never returned before would start returning**, which is an ABI change
- [ ] **Deduplicate the guest `ili9341.rs`?** `apps/sensor-display-rs` and
      `apps/lcd-demo-rs` each have a copy. **Since 2026-10-05 `draw_text` differs**
      (only `sensor-display` has a scale factor; `lcd-demo-rs` is frozen to protect the
      recorded traces). The bounds check, init sequence and `fill_rect` are still identical.
      The split is intentional, but **behavior did diverge once**: the u16 wrap-around bug in
      the bounds check was fixed only on the `lcd-demo-rs` side in `321fe3d`, and the two
      behaved differently for a while (both the Rust and AS `sensor-display` were fixed and
      realigned on 2026-09-28)
      - Extracting a shared crate means making the font table a parameter. The `apps/`
        workspace gets one more member
      - If they stay separate, write in `apps/README.md` that fixing one means checking the
        other (three places including the AS version)
### 2.2 Open questions from the guest bounds check (`/code-review` findings, 2026-09-28)

These came up when the u16 wrap-around in `apps/sensor-display-*` was fixed. **None touch the ABI.**

- [ ] **Run the guest unit tests in CI?** (2026-09-28). Unit tests for `in_bounds` were
      added to `apps/sensor-display-rs` and `apps/lcd-demo-rs`, and on 2026-09-29 the
      `sht4x` conversion (CRC / temperature / humidity clamps) too, but **neither runs in
      CI**. `apps/.cargo/config.toml` pins wasm32, so they only run with the host triple
      given explicitly:

      ```
      (cd apps && cargo test --target "$(rustc -vV | sed -n 's/^host: //p')")
      ```

      - **Only the humidity clamp reaches CI by another route** (2026-10-01). The host test
        `sensor_display_agrees_at_humidity_clamp_bounds` cross-checks the Rust and AS
        versions on a synthetic response that hits the bounds, so that one runs in CI. It is
        also **the only clamp check on the AS side**. But it is **only a partial
        mitigation; the item itself is unresolved** (`in_bounds` and most of the `sht4x`
        conversion are still outside CI)
      - Locally (macOS / aarch64) both debug and release pass, and reverting `in_bounds` to
        the wrapping version was confirmed to fail. **Whether undefined `extern "C"`
        symbols link on Linux is unconfirmed**, so add it to the `guest` job
        (`ubuntu-latest`) only after one trial run in CI. In the job, use the portable form
        above or `x86_64-unknown-linux-gnu` (hard-coding `aarch64-apple-darwin` fails in CI
        for lack of std)
      - **`clashing_extern_declarations` must be fixed first** (item below). The `guest` job
        stops on `-D warnings`
- [ ] **Fix the `clashing_extern_declarations` warnings from the generated `bindings/rust`?**
      `i2c` and `spi` declare `[static]bus.open` / `[method]bus.write` with different
      signatures and tell them apart through `#[link(wasm_import_module = ...)]`. The lint
      looks at the attribute on wasm32 but **not on host targets**, so building the guest
      for the host shows two warnings (first seen on 2026-09-28 when the unit tests were
      added)
      - The fix belongs in `tools/wasmicon-gen`. Generated files are not edited by hand
        (`wit/AGENTS.md`)
      - Until fixed, the CI item above cannot pass `-D warnings`
- [ ] **The call sites of the bounds check are not locked in** (2026-09-28). The unit tests
      added only check `in_bounds` / `inBounds`, so **reverting a call site to the wrapping
      expression goes unnoticed** (reverting both `fill_rect` and `draw_text` makes
      `in_bounds` dead code and clippy fails, but reverting one passes. The AS side has not
      even that safety net)
      - Locking them in for real needs a test that hits `fill_rect` / `draw_text` with
        hostile coordinates. The guest exports only `run`, so calling them from the host
        means adding exports, and **the `lcd_demo_rs.wasm` hash changes, forcing a re-record
        of the traces**
      - **The AS side has no unit tests** (this repo has no AS test infrastructure). It is
        the side where `noAssert: true` removes the bounds checks, so the riskier side is
        the untested one

---

## 3. Known limits (not a problem today)

Written down for when a fix becomes necessary.

- **`spi.transfer` and `i2c.write-read` are limited to 128 bytes** (`SCRATCH` in `ports/common`). Source and destination are both in guest memory and may overlap, so the source is copied once. Beyond the limit: `unsupported`. Enough for the v0.1 uses (6 bytes for the SHT4x, the ILI9341 ID read)
- **`draw_text` takes up to 40 characters** (apps/README.md §2; the `lcd-demo-rs` copy takes 12). Beyond that it returns failure without drawing
- **`fill_rect` raises `dc` again on every row.** Within one CS-low transaction `dc` only needs to go high once after RAMWR, so every raise after the first is a wasted host call.
  In `lcd-demo-rs` about 2,659 of the 7,176 calls are these, and they nearly double the run
  time of a tracing build. **Fixing it changes the trace**, so the 14,352-line figure
  recorded in docs/verification-report.md §6 and `README.md` would have to be re-recorded
  (the send procedure in apps/README.md §2 itself does not change)
- **The text format of Wasm is not read**. The 538 `(module quote ...)` cases in the spec tests are skipped for this reason (the runner prints the breakdown of the 548 skips at run time)
- **No linking of multiple modules**. `linking.wast` / `imports.wast` and the like are out of scope
- **`panic` reasons cannot reach the serial port on hardware**. The board owns the serial port and the panic handler cannot reach it. Runtime failures are caught and printed by `main`, so only bugs in the port itself end up here

---

## 4. Possible future work (post-MVP)

Items on the roadmap in docs/design-notes.md §6 that are outside v0.1.

- AoT compilation (`wasmicon_compiler`)
- Dynamic loading over HTTP -> **entered the design in §5** (`docs/app-workflow.md`)
- Full adoption of the Component Model (resource types / async / component binary)
- Interpreter optimization. It is still a plain `match` loop. The goal of drawing ILI9341
  text on the RP2040 within one second has not been measured
    - **Speed was measured for the first time on 2026-10-05** (`spike/` in the separate
      repository `wasmicon-doom`, C guest, no trace; the full numbers are in its README).
      In an integer loop it is **about 90 cycles per Wasm instruction**, nearly the same on
      RP2350 (150 MHz) and ESP32-S3 (240 MHz) (about 4 ns per instruction on the host)
    - **One host call is expensive** (2026-10-11, the measurement mode of `apps/life-rs`,
      `--features bench`. Pico 2 W at 150 MHz, trace=off). Drawing the game of life takes
      14 host calls per cell and **950 µs per cell**, with 93% of the time in drawing.
      Raising SPI from 16 to 48 MHz saves only 4% (the transfer is about 60 µs per cell),
      so **the remaining ~890 µs is about 63 µs (about 9,500 cycles) per host call**. The
      breakdown is unmeasured (time in the binding's Wasm, the import path, or waiting in
      the port). Found along the way: `Hal::call` in `ports/common` **builds the trace
      strings and the `spi.write` CRC-32 on every call even with trace off** (a few percent
      expected)
    - **Benchmark app `apps/bench-rs`** (2026-10-11). The same `.wasm` runs on host and
      hardware and logs the time of an integer loop / fib / sieve / game-of-life computation /
      four kinds of host call. Baseline on the Pico 2 W (150 MHz, trace=off), per call:
      `time.now-us` 18.7 µs, `gpio.write` 21.3 µs, `spi.write` 1 B 55.0 µs / 98 B 175.8 µs,
      integer loop 12.7 µs per iteration (140x the host)
    - [x] **No formatting when trace is off** (2026-10-11). `Buf::off`, plus an `is_on` check
      before the CRC-32 and the hex dump. In the host profile 22% of the time was CRC-32.
      On hardware `spi.write` 1 B 55.0 -> 31.8 µs, 98 B 175.8 -> 106.8 µs, `gpio.write`
      21.3 -> 19.1 µs. The trace content when tracing is unchanged
    - **Instruction cache misses were not the bottleneck** (2026-10-11, experiment only, not
      merged). Placing the hot functions (the interp loop, arithmetic, LEB128, calls,
      `Hal::call`, RP2350 gpio / spi) in SRAM shrinks computation by only 6-8% and
      `spi.write` 1 B by 26%. Too little for the per-board linker directives and
      `unsafe(link_section)` it needs, so it was dropped. **About 100 cycles per instruction
      is the nature of the interpreter itself**
    - [x] **`opt-level = 3` for the whole firmware** (2026-10-11, `[profile.release]` of the
      three ports). On the Pico 2 W, z -> 3 is 16-35% faster on computation and 7-37% on
      host calls. The firmware grows from 71 to 116 KB (flash has plenty of room). **A
      per-package override that sets only the runtime to 3 had no effect** (the final
      optimization of fat LTO runs at the firmware's z. Combined with the LEB128 shortcut it
      even made the game of life 36% slower)
    - [x] **Shortcut for LEB128 values that fit in the first byte** (2026-10-11, `leb_*` in
      `runtime/src/interp.rs`. Two or more bytes go to the `#[cold]` `*_slow` path). In the
      host profile `leb_u32` was 12%. The same `bench-rs` run twice (under 0.3% variation
      between runs), O3 without -> with the shortcut: integer loop 16.33 -> 8.99 s, fib
      1.143 -> 1.084 s, sieve 2.851 -> 2.813 s, game of life 1.154 -> 1.181 s (+2%),
      `gpio.write` 18.75 -> 13.6 µs, `spi.write` 1 B 27.8 -> 18.2 µs / 98 B 100.2 -> 91.7 µs,
      `time.now-us` 13.6 -> 13.8 µs. The hardware trace (sensor-display, 4,182 lines up to
      the sensor value) is identical to the host
    - **Measured on the ESP32-S3 (240 MHz) as well** (2026-10-11, the same `bench-rs` twice).
      aadf8d6 (z, formatting skip already in) -> now (O3 + shortcut): integer loop 8.600 ->
      6.983 s, fib 1.068 -> 0.833 s, sieve 2.892 -> 2.223 s, game of life 1.173 -> 0.865 s,
      `time.now-us` 13.78 -> 10.75 µs, `gpio.write` 12.43 -> 9.85 µs, `spi.write` 1 B
      19.55 -> 15.33 µs / 98 B 81.52 -> 74.59 µs. Even at O3 the native stack (about 17 KB)
      did not overflow and the run completed
    - [x] **Look up `func_type` without scanning** (2026-10-12). The type of an imported
      function was found by scanning the import table from the start on every call, so
      `imported_func_types` (4 B per import) is now built at decode time. Defined functions
      were looked up twice, in `call` and `enter`; now once. On the ESP32-S3 every item
      shrank by 0.2-4.2% (fib -2.6%, `spi.write` 1 B -4.2%). On the Pico 2 W fib -4.9% and
      `spi.write` 1 B -6.1%, but `spi.write` 98 B +11.6% (below)
    - **Pico 2 W numbers move by about +/-10% with code placement** (2026-10-12). The change
      above moved `spi.write` 98 B from 91.7 to 102.3 µs, yet not one instruction of the
      port's `spi_write` machine code changed; the function merely shifted 88 B earlier
      (going back to the previous firmware reproduces 91.7 µs). It runs from XIP, so
      placement changes how the cache hits. **Do not read a difference under 10% on the Pico
      as the effect of a change** (judge it next to the ESP32-S3). To stabilize, put the hot
      loop in RAM (the RAM execution experiment above; not merged for now)
    - Next candidates (not started): what remains in the host profile is the `run` body at
      24%. Shortcuts for hot instructions are the candidate. The binary search in the branch
      side table is under 1% and not the bottleneck now
    - [x] **Run the ESP32-S3 CPU clock at 240 MHz?** -> **Set to 240 MHz** (owner decision 2026-10-09. `with_cpu_clock(CpuClock::max())` in `ports/esp32s3/src/main.rs`). Verified on hardware that the sensor-display trace is identical to the host (docs/verification-report.md §15). The original record follows:
      `ports/esp32s3` kept `esp_hal::Config::default()` and ran at **80 MHz**.
      A local build confirmed that `with_cpu_clock(CpuClock::max())` makes every
      measurement exactly 3x faster (not committed)

---

## 5. App development workflow (loader and the `wasmicon` CLI)

**The design is owned by `docs/app-workflow.md`. Only remaining work and open questions live here.**
On 2026-10-04 the owner set the direction: **provide a firmware per board, and let that
firmware load apps over USB / HTTP.**

**Decided the same day: the firmware carries no app.** The built-in app baked in with
`include_bytes!` goes away; with an empty slot the firmware prints the reason and parks,
and the port layer blinks the LED directly as a sign of life
(docs/app-workflow.md §3.3). The other reason: **the current `include_bytes!` makes the
firmware build depend on the `apps/` build, so the firmware cannot be produced as a
standalone release deliverable.**

This does not push §1 aside. **Stage 0 shortens the SHT40 work in §1.3** (capture and
cross-check move into the CLI). **The Phase 4 / 5 / 6 completion criteria in §1.3 do not
change.**

### Open questions (owner decisions; the text cites them as §5-1 ... §5-9)

- [ ] **§5-1 Settle the behavior after a trap as "stay halted, but accept the next app"?**
      Today it is "log and halt, no restart" (docs/decisions.md A-04), and `main` enters
      `loop { wfi() }`. The loader adds a "wait for a new app" state **without automatically
      re-running the same app**. docs/abi-spec.md §6.6 leaves post-trap behavior to the port,
      so **this is not an ABI change**, but it changes how the A-04 record reads, so it needs
      a decision. Same subject as #3 of "make open questions 2-5 of docs/abi-spec.md §10 final" in §2
- [x] **§5-2 Board variant** (measured 2026-10-04). Output of `espflash board-info`:
      **`Flash size: 8MB`**, `Features: WiFi, BLE, Embedded Flash`,
      `esp32s3 (revision v0.2)`, 40 MHz crystal
      - **PSRAM does not appear in Features** -> apparently not fitted (N8, not N8R8).
        **One option of §5-3 disappears** (the arena cannot move to PSRAM, so HTTP leaves
        only lowering `max_memory_pages` from 4 to 2)
      - The evidence is espflash's output, so **confirming against the module marking
        (WROOM-1-N8 or N8R8) would make it certain**. Matters when deciding §5-3
      - The RP2350 side is settled as the Pico 2 W (4 MB flash) (§1.1)
- [ ] **§5-3 What to do about the ESP32-S3 RAM budget** (prerequisite for HTTP). DRAM is
      nearly exhausted (`.bss` 315 KB / `ARENA` 300 KB / native stack 17.4 KiB. §1.4).
      Adding `esp-wifi` leaves two options:
      - Lower `max_memory_pages` from 4 to 2. docs/abi-spec.md §6.2 says "the port decides
        the limit", so **it is not a spec violation**, but **"the same binary runs on every
        board" effectively breaks**
      - ~~Put the arena in PSRAM~~ -> **not an option**. The measurement of 2026-10-04 showed
        no PSRAM (§5-2). Not fully closed until the marking confirms it, but it does not
        appear in the Features of `espflash board-info`
- [ ] **§5-4 Do Wi-Fi on the Pico 2 W / Pico WH?** They use the CYW43439, and the practical
      driver `cyw43` assumes embassy (async). From the current blocking setup this means
      **rewriting the port**. Recommendation: limit v1 HTTP to the ESP32-S3
- [ ] **§5-5 Direction of HTTP.** Device as server (`POST /app` + mDNS; easy dev loop) or
      pull from a URL (easy OTA, works across NAT). Both can come later, but decide which
      goes in first
- [ ] **§5-6 Technical points to confirm** (check before implementing; do not proceed on
      guesses)
      - **RP2350 flash writes.** `rom_data::flash_range_erase` / `flash_range_program`
        **cannot be called from code executing from XIP**. The write routine will have to
        live in RAM (`#[link_section = ".data"]`) with interrupts disabled
      - **Can UF2 write to an arbitrary address?** On RP2350 without a partition table the
        absolute family ID (0xe48bff57) may be required. If it works, `deploy` needs no
        picotool
      - **Do not assume an arbitrary offset is XIP-mapped on the ESP32-S3.** The design
        reads into RAM through `esp-storage`
- [ ] **§5-7 Build the ESP32-S3 firmware as a release deliverable in CI?** Not run in CI
      today because of Xtensa (the comment in `.github/workflows/ci.yml`). If app authors
      are not to build firmware (docs/app-workflow.md §4.5), a job that installs espup is
      needed
- [ ] **§5-8 How firmware versions are assigned.** One version shared by the three ports
      (all `0.1.0`) or independent per port. **The ABI version (`wasmicon:hal@0.1.0`) comes
      from `wit/` and is a different thing from the firmware version.** Do not bump it on
      your own (docs/decisions.md D-05). Compatibility is judged per axis, not by one
      version (docs/app-workflow.md §3.8), so the firmware version is only a provenance
      record
- [x] **§5-9 Make the role map a device-side setting? When?** -> **Yes. The minimal stage-1
      version was implemented on 2026-10-07** (owner decision). Decided: **role names are
      not the firmware's vocabulary** (`ROLE_NAMES` removed; names are free-form) / **the
      firmware has no default table** (empty or corrupt means no roles are handed out) /
      **no pin is driven while parked on an empty slot**. Design: docs/app-workflow.md §3.9
      (docs/app-workflow.md §3.9, §4.7). The original record: role -> GPIO lives in the
      firmware's `profile::<board>.roles`, so **changing the wiring means reflashing the
      firmware**, which conflicts with "flash the firmware once"
      - **Determinism is not broken**. docs/abi-spec.md §9 normalizes numbers handed out by
        role to `role:`, so changing the table does not change the trace lines
      - But **the trace of an app that hard-codes pin numbers changes** (a line that was
        `role:` becomes a raw number). That only concerns apps docs/abi-spec.md §9 already excludes, but the
        symptom needs an explanation
      - The minimal version works at stage 1 (the config sector is written from outside
        with `espflash write-bin` / UF2, and the firmware just reads and applies it).
        Through the protocol is stage 2

### Implementation (stages: docs/app-workflow.md §5)

- [x] **Implement the docs/abi-spec.md §5.2 handle cleanup in `ports/common`** (done
      2026-10-04. **The spec and the implementation disagreed regardless of the loader**).
      docs/abi-spec.md §5.2 says "when `run` returns, the host drops every remaining handle", but
      **neither host nor rp2040 / rp2350 / esp32s3 did it**. It went unnoticed because the
      `Drop` impls in the Rust bindings (`bindings/rust/src/hal.rs`) release handles on the
      guest side. With a loader, **the next app hits `busy`** (always when the previous
      app trapped)
      - `Hal::release_all()` was added and is called **in four places, host and the three
        ports**, once `run` has returned (also when it left by a trap)
      - **The cleanup emits no trace lines** (it talks to `board` directly, bypassing
        `Hal::call`, which writes the trace). The host's 10 trace comparison tests passing
        confirms that the recorded 14,352 lines (docs/verification-report.md §6, §7) are
        unchanged
      - The release order is fixed: gpio -> i2c -> spi (the same across ports)
      - Locked in by the 4 tests in `ports/common/tests/release_all.rs`. Emptying
        `release_all` was confirmed to fail 3 of them
- [x] **Collect the board profiles in `ports/common`** (done 2026-10-04). `Config` (the
      three `main.rs`), the role assignments (`ROLES` in each `board.rs`) and **the
      implemented interfaces** are now in `ports/common/src/profile.rs`. Each port reads
      from there, so the `cargo build` of the rp2040 / rp2350 CI jobs checks that the values
      agree. The values are locked in `ports/common/tests/profiles.rs` to the numbers from
      before the move
      - Without the implemented interfaces, `check` for `ports/rp2040` **passes statically**
        (SPI / I2C return `unsupported` only at run time. §1.2)
      - **`assert_role_names` checks at compile time that the roles are contained in
        `ROLE_NAMES`.** Outside it, `pin-by-role` succeeds but the trace is not normalized
        to `role:`; a raw GPIO number appears and **the two boards' traces mismatch**
        (docs/abi-spec.md §9). **This check did not exist before**
      - `Config::DEFAULT` was added to `runtime`, and `Default` returns it (the host profile
        uses it in a const context; two copies would diverge)
- [x] **Create `tools/wasmicon-cli` (binary name `wasmicon`)** (done 2026-10-04).
      Stage 0 is `new` / `check` / `run` / `monitor` / `trace diff` / `doctor`.
      Stage 1's `pack` / `deploy` landed the same day and were run on hardware.
      **`size` was left out** (reason below; `tools/measure-size.sh` stays)
      (`/code-review` findings applied)
      - **`check` looks at the actual arena size, not only `Config`.** Linear memory takes
        all that is left of the arena, so even within the page limit, "the remainder after
        decode / validate / `Exec` cannot hold it" happens
        (`ports/rp2040` has 160 KiB; with 2 pages only about 20 KiB is left).
        `Profile` carries `arena` / `scratch`, and **the ports' `static ARENA` uses them
        too**
      - The role-name cross-check **does not affect the exit code** (the scan is
        informational). Import / export defects do not depend on the board, so they are
        counted once as "common to all boards" rather than mixed into per-board counts
      - **No panic** on a broken `.wasm` (an import type index out of range). A diagnostic
        tool that crashes is useless
      - **`run` is checked down to its type** (anything but `func()` fails at run time with
        `wrong arity`, so checking only that the export exists lets it through). A missing
        memory definition also fails (docs/abi-spec.md §6.2)
      - The failing stage (decode / validate / instantiate) is reported separately. Calling
        an arena shortage "validate failed" steers people toward shrinking
        `max_memory_pages`
      - Without `--i2c-replay`, **`WASMICON_I2C_REPLAY` is read**. Ignoring it silently
        falls back to "no sensor" and mismatches the recorded traces
      - **The test helpers (`repo_root` / guest builds) are not shared.** They duplicate
        `ports/host/tests/apps.rs` verbatim, and the `Mutex` does not cross processes, so a
        runner that runs test binaries in parallel can still read a half-written `.wasm`.
        A shared helper is the right fix (`repo_root` is now spread over 7 files)
      - `run` only calls `wasmicon-host` as a library (`--trace` / `--i2c-replay`).
        `--i2c-replay` calls `load_i2c_replay` directly, so the environment variable
        `WASMICON_I2C_REPLAY` stays on the host binary side
      - `trace diff` has its normalization in Rust, and **`tests/trace.rs` cross-checks that
        it gives the same verdict as `verify/diff-traces.sh`** (the line count too, not
        only the verdict; the same shape as `tools/check-sigs.sh` against `wit2sig.py`).
        The script stays for CI's `--self-test`. **Removing the NUL stripping was confirmed
        to fail the cross-check**
      - **Lines are held as `Vec<u8>`.** Converting to `String` collapses every non-UTF-8
        byte to `U+FFFD`, which **calls two logs with different noise "a match"**. The
        script also got `LC_ALL=C` (with a UTF-8 locale, macOS `tr` stops on an invalid
        byte and local and CI results differ)
      - `run --trace` **prints the trace up to the trap even when the guest traps**
        (`run_wasm_capture` was added; that is exactly when it is needed)
      - **`monitor` was implemented on 2026-10-04 and run on hardware**
        (`deploy --monitor` captured 14,352 lines, identical to the host.
        docs/verification-report.md §9). The three capture traps (`stty` while the port is
        open / attaching before output starts / treating the data as bytes) are contained
        in the module. It stops on silence (default 3 seconds)
      - **No `size`.** The core's code size is a concern of repository maintenance, not of
        app authors, so `tools/measure-size.sh` stays. The app's own size is the first line
        of `check`
      - **No `build` either** (decided 2026-10-04). It would be a thin wrapper that picks
        `cargo build --release` / `npm run build` by language, and the build flags live in
        the files `new` writes (docs/app-workflow.md §4.4), so there is nothing to add. The
        inner loop in docs/app-workflow.md §4.6 is written with `cargo build`
      - The verdict (`facts` / `judge`) is separated from printing, so tests can call the
        verdict alone. `tools/wasmicon-cli/tests/check.rs` locks it in with the real `apps/`:
        **`blink-rs` passes on all 4 boards; `sensor-display-rs` fails only on rp2040**, for
        the missing I2C / SPI. A minimal hand-built `.wasm` that asks for 4 pages shows
        "passes on host and rp2350, fails on rp2040"
      - **The limits of the role-name cross-check were measured** (both recorded in
        docs/app-workflow.md §4.3): `led` inside a log string is picked up
        (`sensor crc failed`), and **AssemblyScript guests never match** (strings are
        UTF-16)
      **Zero firmware changes, no hardware needed**; it helps the §1.3 work
      - `check` runs decode / validate / instantiate **on the real runtime**. `ports/host`
        runs with `Config::default()` (`max_memory_pages` = 65536), so **host execution is
        more permissive than every board**. That is why `--board` exists
      - `monitor` enumerates `/dev/cu.usb*` and does not assume `usbserial` (§1.1).
        **Stage 0's `monitor` comes before the identity** (recording it at the top of the
        log belongs to "make the firmware state who it is" below; this alone is not enough)
      - **`run --i2c-replay` corresponds to the existing environment variable
        `WASMICON_I2C_REPLAY`.** Its target, `verify/sht4x-replay.txt`, is to be replaced
        with a hardware recording in §1.3
      - **There is no "check only" entry point today.** `wasmicon_host::run_wasm_opts` goes
        all the way to `run`. Either the CLI assembles `wasmicon-core` +
        `wasmicon_port::Hal` + `wasmicon_host::hal::HostBoard` directly (recommended) or
        `ports/host` gets a `check_wasm(wasm, &Config)`
      - The `wasmicon-host` crate stays and is used as a library
        (`verify/differential` depends on it)
- [x] **A single source of truth for the app build flags** (done 2026-10-04.
      docs/app-workflow.md §4.4). The truth is **the file contents themselves** in
      `tools/wasmicon-cli/src/flags.rs`, and `tools/wasmicon-cli/tests/flags.rs` checks
      the match with the real files in `apps/` **byte for byte** (the same shape as
      `wasmicon-gen --check`; the check rides on the `cargo test` already in CI, so no new
      job is needed)
      - **A one-character difference in a comment fails it.** Loosening it brings back "the
        values agree but nobody knows which is the truth", so this is deliberate
      - `[profile.release]` is not included. A mismatch there only makes the app **bigger**;
        it does not break silently. Limited to the two files docs/app-workflow.md §4.4 names
      - It also checks that **the Rust and AS versions agree on the initial memory in
        pages** and that it **fits the tightest board (rp2040's 2 pages)** (read from
        `profile::PROFILES`, so a new board is covered automatically)
- [x] **`wasmicon new`** (done 2026-10-04. docs/app-workflow.md §4.6, step 3). The scaffold
      is written out from `flags` only (holding the strings here would make a third copy)
      - **"Matches `apps/`" is not enough.** `tests/new.rs` really runs `cargo build` on the
        scaffold and checks that **`check` passes on all 4 boards** and that the
        `wasmicon.toml` it wrote can be read back (with `deny_unknown_fields`, an extra key
        makes it unreadable)
      - `Cargo.toml` gets a `[workspace]` (without it, cargo stops with "not in the
        workspace" inside an existing workspace). In that location the `rustflags` are
        **concatenated** and the same value applies twice, so `new` says so
      - **Dependencies are written as paths** (`wasmicon-hal` is unpublished). `--hal`, or
        else search upward for `bindings/`. If none is found, write a version requirement
        and **say so**. Relative or absolute, **whichever is shorter** (without a common
        ancestor the output really had eight `../` in a row)
- [x] **Slot format** (done 2026-10-04. docs/app-workflow.md §3.4). Lives in
      `ports/common/src/slot.rs` so that **the writer (CLI) and the reader (firmware) share
      one shape**. CRC-32 reuses the existing `crc32` (the same as the trace)
      - **Empty (erased `0xff` / unused `0x00`) is not a failure.** It is told apart from a
        wrong magic (the log means something different)
      - Locked in by the 9 tests in `ports/common/tests/slot.rs` (round trip, empty, wrong
        magic, wrong version, length overflow, CRC, trailing bytes ignored) and the 4 in
        `tools/wasmicon-cli/tests/pack.rs` (a round trip that **reads what the CLI wrote
        through the firmware's path**, and a CRC check against a known value)
      - `SlotError` has no `Debug` (to keep `core::fmt` out of the ports). Tests use
        `reason()` and `matches!`
      - The CLI side is `wasmicon pack`. At stage 1 it is **input for an external flasher**
        (`espflash write-bin <offset>` / `picotool load -o <offset>`)
- [ ] **The supervisor loop** (docs/app-workflow.md §3.1).
      **The three ports now read the slot** (2026-10-04. RP2350 and ESP32-S3 verified on
      hardware, RP2040 code only). What remains is the return to the parked state and the per-cycle
      rebuild (arena / `Hal` / **the role table**; carrying it over can change the trace.
      `release_all.rs` locks in the current state)
      - **RP2040 and RP2350 read it in place** (XIP is memory-mapped, so no copy to RAM). The
        read lives in one place, `slot::read_xip` in `ports/common` (to keep the `unsafe`
        that builds the raw slice in one spot). **It can be tried on the host too** (pass a
        static standing in for flash)
      - **RP2350 verified on the Pico 2 W on 2026-10-04** (three cases: empty -> built-in /
        the slot app runs / corrupted CRC -> built-in. On top of that, **the trace of the
        app run from the slot is identical to the host over 14,352 lines**.
        docs/verification-report.md §9). **RP2040 unverified** (no Pico WH on hand. The
        code goes through the same `slot::read_xip`)
      - Two `picotool` argument pitfalls were hit and fixed on hardware (the extension is
        `.bin`, `-o` is an absolute address). Neither shows up on the host
      - **The firmware checks at boot that the slot does not overlap the end of the firmware
        (`__flash_binary_end`).** If it does, it falls back to the built-in app without
        reading (never feed your own code to the runtime as wasm). RP2040 lacked this
        symbol, so it was added to `memory.x` (measured `0x1000de40`; the slot is at
        `0x10100000`, so nearly 1 MB is free)
      - **`ports/esp32s3` was implemented on 2026-10-04 and verified on hardware the same
        day** (reads the header through `esp-storage`, takes only the body's length from
        the arena and reads the body. The app is a few KB against a 64 KiB slot, so no
        static region grows). **The trace of the app run from the slot is identical to the
        host over 14,352 lines**. Moreover, **the Pico 2 W (XIP) and the ESP32-S3 (copied
        to RAM through `esp-storage`) are identical on the same `.wasm`** - determinism
        holds across two boards that read flash differently
        (docs/verification-report.md §10)
      - **`partitions.csv` was not needed** (measured: `espflash flash` erases only the
        app's sectors, so the marker at the 7 MB point survived). The location matches the
        Pico family: 64 KiB from `0x100000`
      - **On the ESP32-S3, flashing and the trace share one port.** `monitor` only applies
        `stty`, so it cannot reset the board itself, and the ordering deadlocks. Solved by
        **handing the whole port to `espflash monitor`** (`--before default-reset --after
        hard-reset` are its defaults, so it can open the port and then boot the board).
        `espflash reset` lingers, so it is not called
      - **The failure cases were also tried on hardware the same day** (the same three as
        docs/verification-report.md §9): correct image -> runs / one byte of the body
        flipped -> `slot crc mismatch, running built-in` / filled with `0xff` ->
        `slot empty, running built-in`. **No button presses**, so it is easier to iterate
        than the Pico
      - **Remaining**: `Truncated` (an image that lies only about the length) was not
        tried. The `Overlap` check exists only on the `read_xip` side, so **the ESP32-S3
        has none** (it reads through `esp-storage`). Whether it is right that preventing
        overlap falls to `espflash` is an open question
- [x] **Remove the built-in app** (done 2026-10-07. Verified on ESP32-S3 hardware that "the
      slot app runs" and "an empty slot gives `slot empty, idle`".
      docs/verification-report.md §13. The LED blink is unconfirmed for lack of an external
      LED).
      An empty slot prints `wasmicon: slot empty, idle`, enters `idle::heartbeat` in
      `ports/common`, and blinks the `led` role at 1 Hz [Later replaced by `idle::park`, which drives no pins; see §5-9]
      (stage 1. **Only once the three ports can read the slot** - removing it first would
      leave the firmware running nothing. **RP2350 and ESP32-S3 are verified on
      hardware**. RP2040 was dropped from evaluation (2026-10-07), so **the precondition is
      met**. RP2040 goes through the same `slot::read_xip`, so it can be removed there
      too). Drop the `include_bytes!` and the `guest-lcd-demo` feature from
      `ports/*/src/main.rs`; an empty slot prints the reason and parks; the port layer
      blinks the LED as a sign of life (`Board` directly, not through Wasm)
      - **Fix CI at the same time**: the port jobs no longer need the `apps` pre-build, and
        rp2350's `cargo clippy --release --features guest-lcd-demo` line goes away
      - **The recorded traces live on**. `deploy` `lcd_demo_rs.wasm` (`fc470947...`) to the
        slot and **the bytes are identical, so the host-call sequence is identical** (the
        XIP address or a copy to RAM does not show in the trace)
      - **What is lost**: the footing to confirm on hardware, right after flashing, that
        "the runtime gets from decode to run". The host tests and the first `deploy`
        replace it
- [x] **Stage 1 of `deploy`** (done 2026-10-04, all three ports). Folds the check, the
      image preparation, `picotool load -t bin -o <address>` and the reset into one
      command. **Run end to end as one command on the Pico 2 W**
      (docs/verification-report.md §9)
      - **Nothing that will not run is flashed** (`check --board` runs before flashing).
        `tests/deploy.rs` locks in that sending sensor-display to rp2040 stops **without
        even writing the image**
      - The trace capture procedure (`stty` while `cat` holds the port open) is printed at
        the end
      - **The ESP32-S3 also went through on 2026-10-04** (`espflash write-bin <offset>`;
        the command `pack` prints is per board too). `--monitor` delegates to
        `espflash monitor` (`monitor::capture_cmd`). **One command flashes and captures,
        14,352 lines identical to the host**. Unlike the Pico, **no button presses**
- [x] **Make the firmware state who it is** (2026-10-09, **the banner part**. `info` comes
      with the USB control channel (stage 2)). The line after the banner is
      `wasmicon id <board> fw=... git=...(-dirty) abi=... trace=on|off pages=... if=... slot=v1/... roles=v1/...`
      (`identity` in `ports/common`. git is filled in by each port's `build.rs` from
      `ports/common/build/git.rs`. The ABI version is `generated::PACKAGE`, generated from
      `wit/`). `trace diff` warns when the two logs' identities differ (the verdict is
      unchanged). **`monitor` captures the raw serial output, so the identity lands at the
      top of the log by itself**. Verified on ESP32-S3 and Pico 2 W hardware
      (docs/verification-report.md §16, §17).
      The original record follows: (`info` and the banner. docs/app-workflow.md §3.8).
      Today the banner is the single line `wasmicon rp2350`, **with neither version nor
      git** (`ports/rp2350/src/main.rs:140`). `docs/verification-report.md` **records the
      guest's SHA-256 but nothing about the firmware**, so which build produced the 14,352
      lines can only be inferred from dates and git history
      - What it carries: board name / firmware version + `git describe` + dirty / ABI
        version / configuration (`trace` on or off, built-in app) / `Config` / slot
        capacity and format version / protocol version / role names
      - `git describe` is filled in by `build.rs`. **Do not drop dirty**
      - `monitor` records the identity at the top of the captured log; `trace diff`
        excludes the identity line from the comparison but **warns when it differs**
      - **A board flashed with `trace` off leaves `monitor` silent.** Reporting
        `trace: off` saves suspecting the wiring
- [ ] **Firmware distribution** (release manifest and `flash --fw` / `fw list`).
      Attach a manifest (file + sha256 + the 5 axes of docs/app-workflow.md §3.8) to the
      board x version deliverables. Pairs with §5-7
- [x] **Read `wasmicon.toml`** (done 2026-10-04. docs/app-workflow.md §4.7). It holds
      `[requirements] pin-roles` (the role names this app looks up), the default board,
      the replay path and the intended wiring.
      - **Searched upward from the current directory** (like cargo). `check` / `run` work
        without it
      - **With `pin-roles` the role cross-check becomes a guarantee** (the scan is not
        printed). Names are cross-checked against `ROLE_NAMES`, so **a typo stops without
        a device connected**
      - `[defaults] board` is the default for `--board`, `i2c-replay` the default for
        `--i2c-replay` (resolved **relative to the toml's location**)
      - **Unknown keys are errors** (`deny_unknown_fields`). A `version` newer than the CLI
        is an error. `"none"` means "this role does not exist"
      - **`[board.<name>.roles]` is only read and validated**, not used yet. Pushing it to
        the device is `config apply` (after the docs/app-workflow.md §3.9 / §5-9 decision)
      - **`wasmicon new` writes it as part of the scaffold** (2026-10-04). `tests/new.rs`
        checks that **it can read back what it wrote**
      **The ABI-conformant build flags are not allowed in it** (it would turn the
      triplication of docs/app-workflow.md §4.4 into quadruplication). Machine-specific
      values go in `wasmicon.local.toml` (gitignored)
      - With `[requirements] pin-roles`, the role-name cross-check of
        docs/app-workflow.md §4.3 **turns from informational into a guarantee**. The check
        only compares against `ROLE_NAMES` in `ports/common`, so **a typo stops without a
        device connected**
      - `deploy` cross-checks the toml against the device's effective map every time and
        **never applies it silently** (the driven pins change, which is physically
        dangerous)
- [x] **Put a `wasmicon.toml` in each of the 5 apps under `apps/` (dogfooding)**
      (2026-10-07. Became mandatory once the firmware lost its default table.
      `tests/apps.rs` checks that "each app has a toml and the declared roles agree with
      the rp2350 / esp32s3 wiring tables") (the spot where docs/app-workflow.md §4.7 says
      "dogfooding this repo's `apps/` means five of them"). **There is none today**, so the
      CLI's manifest path is exercised only by unit tests with synthetic files
      - **This matters most for AssemblyScript.** The role-name scan is a guess that picks
        up strings from the data segment, and **AS is UTF-16, so it never matches**
        (measured and recorded in docs/app-workflow.md §4.3). `blink-as` /
        `sensor-display-as` are cross-checked only once they declare
      - `missing_declared_roles` is part of `fails()`, so declaring **turns the
        informational scan into a hard check**. The `ROLES` of the three ports are identical
        (`led` / `lcd-cs` / `lcd-dc` / `lcd-rst`), so it passes on every board
      - `[defaults] board` lets `check` / `run` work from the app's directory with no
        flags. `sensor-display-*` point `[defaults] i2c-replay` at
        `verify/sht4x-replay.txt` (the file to be replaced with a hardware recording in
        §1.3)
      - **Existing tests do not break.** `manifest::find` searches upward, so
        `apps/<app>/wasmicon.toml` only applies when run from that directory (the existing
        tests run from the repository root with an explicit `--board`)
      - **Add a "zero is not silently success" check, by the same reasoning as
        `tests/flags.rs`**: check that every app under `apps/` has a `wasmicon.toml` (a new
        app without one fails it)
      - **The structure of `apps/` is not replaced** (considered and rejected, 2026-10-04).
        `apps/` is a monorepo that builds its 5 apps in one workspace on every CI run;
        `wasmicon new` produces one standalone app. Splitting them would split `target/`
        five ways and compile `wasmicon-hal` five times, and would mean rewriting the
        `include_bytes!` sites (5), the test helpers that use `-p` (4) and the 3 CI jobs.
        The only gain is that the `.cargo/config.toml` watch list grows from 1 to 5, and
        **that check is already covered by `tests/flags.rs` and `tests/new.rs`**
- [x] **The role-map config slot** (2026-10-07, minimal stage-1 version.
      docs/app-workflow.md §3.9). 4 KiB right after the app slot, holding `"WMCR"` + the
      table as text. `deploy` writes it together with the app as one image, and the
      firmware reads it at boot (`roles` in `ports/common`). **Remaining**: the `config`
      command over the protocol and reading it back through `info` (stage 2). The original
      plan follows: a config sector + a `config` command + device-side validation
      (rejecting reserved pins, bus pins and duplicates). ~~Boot with defaults if the CRC
      does not match~~ -> **there is no default table, so it boots handing out no roles**
      and prints the reason in the banner (2026-10-07). The CRC-32 of the effective map
      goes into the identity together with `info` (docs/app-workflow.md §3.8)
- [ ] **`deploy` flashes without a wiring table when it finds no `wasmicon.toml`**
      (found 2026-10-20, docs/verification-report.md §18). `wasmicon.toml` is searched upward
      from the current directory, not from the `.wasm`. Run from the repository root,
      `deploy apps/blink-c/build/blink_c.wasm` found no manifest, flashed an empty role
      table and printed no warning. The board then booted with `roles none` and
      `pin-by-role("led")` returned `unsupported`. The README example (deploy
      `lcd_demo_rs.wasm` from the root) takes the same path. Options:
      - also search upward from the `.wasm` location (finds `apps/blink-c/wasmicon.toml`;
        a Rust app's `.wasm` sits under `apps/target/`, so that one still needs the
        current directory)
      - stop `deploy` when no manifest is found, unless a flag such as `--no-roles` says
        an empty table is intended
- [ ] **USB control channel** (stage 2). ESP32-S3: `usb::usb_serial_jtag` from `esp-hal`
      (no extra crate); RP2350 / RP2040: `rp235x-hal::usb` + `usbd-serial`. **Control
      over USB, trace over UART0**
      - **Turn the `hw-probe` feature into a `probe` command** (docs/app-workflow.md §3.6).
        Triage without reflashing, one build configuration fewer, and the variants
        converge on two: `trace` on / off
- [ ] **HTTP** (stage 3). The §5-3 / §5-4 / §5-5 decisions come first
- [ ] **Put the C bindings (`bindings/c`) on the CLI** (on 2026-10-19 only the generator
      output and `apps/blink-c` went in. `cargo test` checks on the host that its trace is
      identical to blink-rs's). Remaining:
      - `wasmicon new --lang c`, and the language detection in `deploy` / `pack` (today the
        `.wasm` location can only be derived from `Cargo.toml` / `asconfig.json`. C is
        built by hand with `sh bindings/c/build.sh` and the `.wasm` is passed in)
      - **The build-flag truth is in `bindings/c/build.sh`.** It has no copy, so it is not
        in `tools/wasmicon-cli/src/flags.rs` yet; adding `new --lang c` means moving it to
        flags.rs and cross-checking the scaffold byte for byte
        (docs/app-workflow.md §4.4)
      - **`blink-c` runs on both evaluation boards** (2026-10-20). The same `blink_c.wasm`
        gave a 20-line trace on the ESP32-S3 and the Pico 2 W that is identical in full to
        the host and to blink-rs (docs/verification-report.md §18). There is no C version
        of sensor-display, so I2C / SPI from C have run only on the host mock
      - CI builds it with the clang + lld (18.x) from apt on ubuntu-24.04. Locally it was
        checked with wasi-sdk 33 (clang 22). The output bytes change with the version, but
        the checks are only the trace and the feature set
- [ ] **Distributing the bindings** (crates.io / npm). **`new` landed on 2026-10-04**, but
      dependencies are written as paths (above). Where to publish and how to version are
      not decided yet. Once published, change `new`'s default from a path to a version
