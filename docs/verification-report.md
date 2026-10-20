# Wasmicon verification report

Created: 2026-09-10 / Subject: **the commit that contains this report** (the point where the Phase 6 tools were added)

The report cannot contain its own commit hash; use `git log --oneline -- docs/verification-report.md`
to find the commit that first added this file.

The deliverable of docs/handoff.md §5 Phase 6. It separates **what has been verified, and how far**, from
**what has not been verified yet**.

---

## 0. Conclusion

| Item | Status |
|---|---|
| Runtime Wasm spec compliance (spec testsuite) | **Achieved** |
| Interpreter correctness (differential against wasmtime) | **Achieved** |
| Rust and AS versions produce the same host call sequence | **Achieved** (on host, success and failure paths) |
| GPIO / SPI / ILI9341 drawing works on RP2350 hardware | **Achieved** (2026-09-26, `lcd-demo-rs`. §6) |
| Host call trace matches on ESP32-S3 hardware | **Achieved** (2026-09-26, 14,352 lines identical. §7) |
| ILI9341 shows an image on ESP32-S3 hardware | **Achieved** (2026-09-26. §7) |
| The same binary produces the same trace on 2 boards | **Achieved** (2026-09-26, RP2350 and ESP32-S3. §6 / §7) |
| I2C (SHT40) reads on RP2350 / ESP32-S3 hardware | **Achieved** (2026-10-05, sensor-display-rs. §11) |
| With the values the board read, drawing the values (including f32) matches host | **Achieved** (2026-10-07, ESP32-S3 / Pico 2 W x Rust / AS, all 4. §12) |
| Display works in all 4 combinations (Rust/AS x 2 boards) | **Achieved** (2026-10-07, ESP32-S3 and Pico 2 W. §12) |

**The original goal is reached (owner decision, 2026-10-07).** sensor-display runs on both boards; the traces
match across the two boards up to the sensor read, and after the read each board matches host in full (§11 / §12).

**The evaluation board changed on 2026-10-07** (owner decision). The second board went from Pico WH (RP2040) to
**Pico 2 W (RP2350)**; RP2040 is no longer evaluated. The RP2040 statements below stay as the record of the time.

**Phase 6's "the same binary produces the same trace on two boards" is met.**
**The four-way display (Phase 5) was also met on 2026-10-07.** I2C read the SHT40 on two boards on
2026-10-05 (§11), and all four combinations of Rust / AS x 2 boards matched in full the trace of the host fed with
the response the board read (§12). What remains is the Phase 4 blink, and how to treat the sensor-display
trace, which cannot be compared directly across the two boards -> docs/TODO.md §1.3.

On 2026-09-26 SPI2 was implemented in `ports/esp32s3`, and the **same `lcd_demo_rs.wasm`** as on RP2350
(SHA-256 `fc470947...`) ran on hardware. **The host call trace was identical, 14,352 lines**, the same count
and content as the RP2350 measurement. **An image appeared on the screen too.** So **Phase 6's "the same
binary produces the same host call sequence on two boards" is reached on hardware** (§6 / §7).

Along the way the ESP32-S3 port had one real bug (the value of `GPIO_FUNCn_OUT_SEL`). It was **the kind of
failure that an identical trace cannot detect**, so §7 keeps the whole story.

---

## 1. Runtime spec compliance

**22507 commands pass** in the 74 core files of the WebAssembly spec testsuite (pinned SHA `34f1f40`).

- Subject: `FILES` in `runtime/tests/spec.rs`
- Excluded: 11 categories (SIMD / GC / EH / tail-call / multi-memory / memory64 / linking and others) listed
  with reasons in `EXCLUDED` in the same file
- 548 skipped: mostly `(module quote ...)` text-format modules. There is no WAT parser, so they cannot be
  handled. The binary-format equivalents are executed

The hand-written floats (`sqrt` / `floor` / `nearest` and others that `no_std` lacks) were separately confirmed
to match std bit for bit on 400,000 random inputs (`runtime/tests/float.rs`).
**That is a match on the host (AArch64)**; a match on RP2040, which falls back to soft-float, or on Xtensa,
which has an f32-only FPU, is not confirmed.

## 2. Interpreter correctness (differential against wasmtime)

For all four guests in `apps/`, **the same `.wasm` was run on our interpreter and on wasmtime 45, and the traces
are identical** (`verify/differential/tests/agree.rs`).

The key is that **the HAL is shared**. `wasmicon-port`'s `Hal` implements `wasmicon_core::Resolver`, but its
`call` does not depend on the engine (it only touches the argument slots and a slice of guest memory). The same
`Hal` is driven from wasmtime too, so a difference in the traces would be a **difference between engines**, that
is, a bug in our interpreter.

This verification works independently of hardware and covers the fixed-point arithmetic, the single f32
operation, and the order of more than 1200 host calls.

## 3. Rust and AS versions match

Both guests run in `ports/host` and **the full traces are identical**.

| Guest | Lines | Path |
|---|---|---|
| blink | 22 | success |
| sensor-display | 4183 | success (1215 before the screen redesign of 2026-10-05) |
| sensor-display | short | SPI returns `unsupported` (mimics the state of the hardware ports) |
| sensor-display | short | sensor does not respond |

By docs/abi-spec.md §9, `time` does not appear in the trace, so **full identity is exactly the definition in
docs/handoff.md §2-10 (now docs/decisions.md D-10): "every host call and result except `time` matches"**.
The `spi.write` trace is the CRC-32 of the data, so a match means "the pixels being sent are identical".

The failure paths are checked too because the SPI of `ports/rp2040` and the I2C of all three ports still return
`unsupported` (SPI is implemented on both rp2350 and esp32s3). Aligning only the success path would make the
comparison meaningless the moment it is taken to hardware.

## 4. Not yet verified

### 4.1 Running on hardware

**RP2350 (§6) and ESP32-S3 (§7) are verified on hardware. RP2040 builds but has never been flashed.**

- RP2040 GPIO is written through registers directly (SIO / IO_BANK0 / PADS_BANK0); it type-checks but nothing
  has been observed. "blink does not light" should be expected as the first thing to happen on hardware
- The I2C / SPI of `ports/rp2040` return `unsupported` (not evaluated. 2026-10-07). The I2C of
  `ports/rp2350` / `ports/esp32s3` **read the SHT40 on hardware on 2026-10-05** (§11). sensor-display-rs was
  confirmed up to the display on both boards. **The AS version has not run on hardware yet** [Stale as of 2026-10-19: the AS version ran on both boards on 2026-10-07; see §12.]
- SPI2 and GPIO of `ports/esp32s3` worked on hardware (§7). But **a host call match only says that "the guest
  passed the right bytes to the HAL".** Whether the register writes reach the pads does not show in the trace.
  The bug in §7 hit exactly that
- Of the wiring in docs/abi-spec.md §8, the RP2350 SPI / LCD side (`lcd-cs` / `lcd-dc` / `lcd-rst`,
  SCK=GP18 / MOSI=GP19) was confirmed on hardware (§6). I2C was confirmed on RP2350
  (SDA=GP4 / SCL=GP5) and ESP32-S3 (SDA=GPIO8 / SCL=GPIO9) (§11).
  **`led` and RP2040 are still unconfirmed by the owner**

### 4.2 Floating-point match between boards

The temperature bar computation, the only place sensor-display uses f32, **has been confirmed to match on one
platform only, the host**. [Stale as of 2026-10-19: it matched on both boards with the responses each board read; see §12.] RP2040 uses `compiler_builtins` soft-float, and ESP32-S3 and RP2350 have an f32-only
hardware FPU (denormal handling depends on configuration), so this is the real measurement target of Phase 6.

RP2350 is built with the hard-float ABI (`thumbv8m.main-none-eabihf`). The DCP that speeds up f64 is not used
(enabling `dcp-fast-f64` of `rp235x-hal` replaces `__aeabi_dadd` / `__aeabi_dmul`, and the results have not
been checked for a match).

### 4.3 Recorded I2C responses

**Replaced on 2026-10-07 with responses recorded from the SHT40 on hardware (T=25.63°C / RH=74.49%)** (§12).
Until then it was synthetic data giving T=23.44°C / RH=51.08%.
(The bytes were still from the SHT31 days. The response format and CRC-8 are the same for SHT3x / SHT4x and
**only the humidity conversion formula differs**, so the displayed value changed from 45.66% to 51.08%.)

### 4.4 CI

[Stale as of 2026-10-19: CI has six jobs on ubuntu-24.04 and the toolchain is pinned to 1.99.0; see `.github/workflows/ci.yml`.]

**First run on 2026-09-10, all four jobs green** (`v2` branch, ubuntu-latest).

The first attempt failed in the `root` job's clippy. CI's stable was 1.98.0 while the local one was 1.97.1, and
`map_or_identity`, new in 1.98, fired. Fixed; green on the second run.

- `root`: fmt / clippy / spec tests (fetches the testsuite at the pinned SHA and runs it) /
  zero diff of generated files / check-sigs / diff-traces self-test
- `guest`: fmt / clippy / wasm32 build of the apps workspace
- `rp2040`: thumbv6m build
- `differential`: the four differential tests against wasmtime

On 2026-09-22 the `rp2350` job (thumbv8m.main-none-eabihf build) was added. fmt / clippy / build all pass
locally, but **it has not yet been seen running in CI**.

**The match with wasmtime was also confirmed on x86_64 Linux** (local is AArch64 macOS). This is supporting
evidence that the interpreter's match does not depend on the host architecture, but the match across the
RP2040 / Xtensa soft-float is still unverified (§4.2).

`rust-toolchain.toml` floats on `channel = "stable"`. A new clippy lint or a change in rustfmt output can make
CI fail suddenly (which is exactly what happened here). **Whether to pin it is open.**

---

## 5. Procedure for verifying on hardware

1. Implement I2C / SPI in `ports/rp2040` / `ports/rp2350` / `ports/esp32s3`
   (**SPI is done on both rp2350 and esp32s3. I2C and rp2040 remain**) [Stale as of 2026-10-19: I2C is done on both boards; see §11.]
2. Check the wiring in docs/abi-spec.md §8 and align the role-name table with the hardware
3. Flash
   - RP2040: `picotool load` the ELF, or make a UF2 with `elf2uf2-rs` and use BOOTSEL
   - RP2350: `picotool load -u -v -x -t elf` with the ELF (RP2350 needs picotool 2.0 or later.
     `elf2uf2-rs` is for RP2040 and does not work)
   - ESP32-S3: `sh ports/esp32s3/build.sh run --release` (espflash)
4. Capture serial (115200 8N1 in all cases) to a file
   - RP2040 / RP2350: UART0 (GP0=TX, GP1=RX)
   - ESP32-S3: UART0 (GPIO43/44; the DevKitC-1's `USB-UART` port is wired straight through the USB-serial
     bridge. The output of `espflash flash --monitor` can be captured as-is).
     **The bridge chip varies between units** (the official schematic has a CP2102N, but the board on hand has
     a CH343 and shows up as `/dev/cu.usbmodem*` on macOS. See the conditions table in §7).
     Do not hard-code the device name; check with `ls /dev/cu.usb*`
5. Cross-check: `sh verify/diff-traces.sh pico.log esp32s3.log`
   - the banner and the guest's `[wasm]` lines are dropped automatically
   - `time` does not appear in the trace, and GPIO numbers looked up by role are already normalized to
     `role:led`, so no further processing is needed
6. Record the SHT4x responses from hardware and replace `verify/sht4x-replay.txt`

### 5.1 Measuring the two-board match with the LCD alone (the path that does not wait for I2C)

`lcd-demo-rs` uses only SPI and GPIO. SPI is implemented on both rp2350 and esp32s3, so **Phase 6's "same
binary, same trace" can be measured on the LCD side first, without waiting for I2C**. Build the guest once and
flash the same one to both boards.

**The built-in apps were removed on 2026-10-07, so `--features guest-lcd-demo` below no longer exists** (kept
as the record of the time). Now the same `.wasm` is `deploy`ed to the slot (`apps/lcd-demo-rs/README.md`).
**The bytes are identical, so the host call sequence is identical**; §9 / §10 confirm the same 14,352-line
match through the slot.

```bash
(cd apps && cargo build --release)
shasum -a 256 apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm
# expected (when this record was taken; Rust 1.97.1):
# fc470947ac08b230ccdc59e04e09aa11105a5c8754efd00533858470f11d2453
# since the toolchain was pinned to 1.99.0 on 2026-10-08 it is 84d6de0f... (same 4,088 B).
# only the code generation changed; the host trace is identical to the 1.97.1 version, 14,352 lines

(cd ports/rp2350 && cargo build --release --features guest-lcd-demo)
picotool load -u -v -x -t elf \
  ports/rp2350/target/thumbv8m.main-none-eabihf/release/wasmicon-rp2350
# -> capture pico.log

sh ports/esp32s3/build.sh run --release --features guest-lcd-demo | tee esp32s3.log
# if the monitor adds colors or prefixes and diff-traces.sh cannot get even 1 line, see
# apps/lcd-demo-rs/README.md (capturing a trace to a file)

sh verify/diff-traces.sh pico.log esp32s3.log
```

`lcd-demo-rs` does not use f32, so **a match here moves the floating point of §4.2 forward by nothing**. It
only says that the SPI bytes (the CRC-32 of `spi.write`) and the GPIO order are the same on the two boards.

---

> **Historical record.** Kept as written at the time and translated from Japanese; section numbers are stable. Current rules: `docs/decisions.md`. Current status: `README.md`, `docs/TODO.md`.

## 6. RP2350 hardware measurement (2026-09-26)

The record of the first run of `ports/rp2350` on hardware. **Not the Phase 6 completion criterion itself**
(that one is defined on two boards; docs/handoff.md §5). Of Phase 5's "the display works", only the display side,
isolated without waiting for the SHT31.
(At the time of this measurement decision 9 was SHT31/SHT30. The sensor was changed to SHT4x on 2026-09-29,
**after this record**. The body of the record is kept as it was.)

### Conditions

| | |
|---|---|
| Board | Raspberry Pi Pico 2 W |
| Guest | `apps/lcd-demo-rs` (Rust). SPI and GPIO only; no I2C |
| Wiring | the defaults of docs/abi-spec.md §8 (CS=GP17 / DC=GP20 / RST=GP21 / SCK=GP18 / MOSI=GP19, MISO not connected) |
| SPI | 16 MHz requested -> 15 MHz actual (`clk_peri` 150 MHz; the round-down rule is `spi_divisors`) |
| Serial | UART0 (GP0/GP1) 115200 8N1, through a CP2102N |
| Flashing | `picotool` 2.3.1, `picotool load -u -v -t elf` -> `picotool reboot -f` |
| Build | `cargo build --release --features guest-lcd-demo` (`trace` enabled) |

### Reproduction

The `host.log` to compare against is made with the host port (`pico.log` is the UART captured as-is; the
banner and log lines may be mixed in).

```bash
(cd apps && cargo build --release)
cargo run -q -p wasmicon-host -- --trace \
  apps/target/wasm32-unknown-unknown/release/lcd_demo_rs.wasm > host.log
sh verify/diff-traces.sh pico.log host.log
```

The host side alone is checked by CI on every run (`lcd_demo_rs_runs_on_host` in
`cargo test -p wasmicon-host --test apps`). The hardware log exists only locally.

### Results

- **The host call trace is identical to the host port.** `sh verify/diff-traces.sh pico.log host.log`
  -> `match: 14352 lines`. **All 3,272 `spi.write` CRC-32 values are the same**, so the bytes
  that reached the ILI9341 match the host model's prediction bit for bit
- Not a single failure status (`pin.open` x 3, `spi.bus.open`, and every following
  `pin.write` / `spi.write` are `< 0`)
- Handles were released in the order `spi` -> `rst` -> `dc` -> `cs` (apps/README.md §4)
- **An image appeared on the screen.** The left end of the color bars is red (the BGR bit of MADCTL is
  right), and the text is readable (the `draw_text` path and the DC wiring are right)

This settled on hardware the 2 RP2350-specific concerns:

- **`ISO` in PADS_BANK0** (reset value 1). Had it not been cleared, `pin.write` would have returned success
  while the GPIO stayed unresponsive
- **Releasing SPI0 from RESETS**. Had it been forgotten, the register writes from `spi.bus.open` on would
  have gone nowhere

### What this measurement does not say

- **Floating-point agreement (§4.2) has not advanced at all.** `lcd-demo-rs` does not use f32.
  That is about sensor-display's temperature bar, which needs I2C
- **How the gray ramp looks is unconfirmed.** Of the RGB565 bit positions, only "the left end is red", that
  is, R and B are not swapped, has been seen
- **The match between the two boards (Phase 6) is not reached.** The comparison partner was the host's mock
  HAL, not a second board
- **The `led` role (external LED) is unverified.** `lcd-demo-rs` does not touch the LED
- **Nothing is said about ESP32-S3** (measured separately in §7)

---

## 7. ESP32-S3 hardware measurement (2026-09-26)

The record of the first run of `ports/esp32s3` on hardware. **A matching trace and an image on the screen are
two different things**, so they are written separately. Both were reached in the end, but **the trace was
identical on the first try while the screen stayed pure white**, and from there 3 causes (1 of them a real bug)
were eliminated. That story is kept too: the same shape of failure will happen in other ports.

### Conditions

| | |
|---|---|
| Board | a board carrying an ESP32-S3-WROOM-1 (2 ports, `USB-UART` / `USB-OTG`). chip rev v0.2, flash 8 MB, no PSRAM |
| Guest | `apps/lcd-demo-rs`. **The identical file flashed to the RP2350** (SHA-256 `fc470947ac08b230ccdc59e04e09aa11105a5c8754efd00533858470f11d2453`) |
| Wiring | the defaults of docs/abi-spec.md §8 (CS=GPIO10 / DC=GPIO14 / RST=GPIO15 / SCK=GPIO12 / MOSI=GPIO11, MISO not connected) |
| SPI | 16 MHz requested. APB is 80 MHz, so 80/5 gives exactly 16 MHz |
| Serial | UART0 (GPIO43/44) 115200 8N1 |
| Flashing | `espflash` 4.5. Works from either the `USB-OTG` side (USB-Serial-JTAG) or the CH343 on the `USB-UART` side |
| Build | `sh ports/esp32s3/build.sh build --release --features guest-lcd-demo` (`trace` enabled) |

### Reached: the trace is identical to host

```
sh verify/diff-traces.sh esp32s3.log host.log
-> match: 14352 lines
```

**The same 14,352 lines as the RP2350 measurement (§6)**, all the same including the 3,272 `spi.write` CRC-32
values. Not a single failure status, and it reached `[wasm] lcd-demo done`.

With this, **"the same binary produces the same host call sequence on two boards" is confirmed on hardware**
(each board is identical to the same host reference, so the two boards match each other).

`spi.bus.open(0, 16000000, 0)` and all three `pin.open` calls succeed too.
So **the GPIO / SPI2 register operations return success**.

### Reached: an image on the screen

Everything `lcd-demo-rs` draws appeared. **The left end of the color bars is red** (the BGR bit of MADCTL is
right), **the text is readable** (the `draw_text` path and the DC wiring are right).
With a 10 kΩ pull-up on `RESET`, **the image stays after drawing finishes**
(cause 3). Observed at the same granularity as the RP2350 in §6.

But 3 things had to be eliminated to get there (next sections).
**The initial state was "the trace is identical but the screen is pure white"**.
The first drawing fills the whole screen black, so if even a little SPI reaches the panel it goes black first.
An all-white screen means the backlight is on but not a single byte of valid data has arrived.

**A matching trace does not guarantee this.** A match only says "the guest passed the right bytes to the
HAL"; whether the register writes **reach the pads** does not show in the trace (it can take the form of
returning success while nothing happens. This is what docs/TODO.md §1.4 listed as the first ESP32-S3
concern).

Tried:

- **Lowered the SPI clock from 16 MHz to 1 MHz**. The trace still matched at 14,352 lines; the screen stayed
  pure white. **Signal quality is not the cause**
- **Added an isolation path that drives the port layer directly** (`ports/esp32s3/src/probe.rs`,
  `--features hw-probe`). Calls `Board` without the guest or the interpreter. Result:

  ```
  probe: configure cs: ok / dc: ok / rst: ok
  probe: spi_open(0, 1MHz, mode0): ok
  probe: init sequence: ok
  probe: 0xD3 (want 00 00 93 41) -> 00 00 00 00 00
  ```

  The ID read was all zeros, but **MISO is not connected, so it is no evidence either way**.

### Cause 1 (software): the value of `GPIO_FUNCn_OUT_SEL` differs on the S3

The USB-serial adapter's `RXD` was moved from `GPIO43` to `GPIO14`, and the received bytes were counted while
`probe` toggled GPIO14 at 1 Hz (to a UART receiver a 500 ms low looks like a BREAK, so if it toggles, one `00`
arrives per low period).

| | Bytes received in 20 s |
|---|---|
| `out_sel = 128` (before the fix) | **1** |
| `out_sel = 256` (after the fix) | **28** |

The same adapter got 442,982 bytes from `GPIO43`, so the adapter is alive.
So before the fix **the pin was not moving at the pad**.

`SIG_GPIO_OUT` in `ports/esp32s3/src/board.rs` (the number of the "GPIO output" signal written to
`GPIO_FUNCn_OUT_SEL`) was changed **from 128 to 256**. **This number differs per chip**
(`OutputSignal::GPIO` in `esp-metadata-generated`): ESP32 / S2 / S3 use 256; the RISC-V chips such as C3 / C6
have a signal map 128 entries shorter, so 128. Writing 128 on the S3 connects the `I2S0O_SD1` output to the
pad, so reading back `GPIO_OUT` / `GPIO_ENABLE` looks right while the pin does not move.

**What makes this failure nasty:** `pin.open` / `pin.write` all return success, and the host call trace is
identical to host. `gpio_write` reads `GPIO_ENABLE` to check that the pin is an output, and that passes too.
**Looking only at the trace, it reads as "working".** docs/TODO.md §1.4 had listed "out_sel" as the first
ESP32-S3 concern, and that was right.

SPI2 was not affected because the `esp-hal` driver carries its own signal numbers. What was broken were the 3
lines `lcd-cs` / `lcd-dc` / `lcd-rst`, and if CS never goes low not 1 byte enters the panel. Pure white is
that shape.

### Cause 2 (wiring): bad contact

After fixing `out_sel` the screen was still pure white. SCK (GPIO12) was putting out 15,770 bytes' worth of
clock, and CS / DC / RST were measured toggling, so the software side had done all it could. **After
reseating the breadboard wires, the image appeared.** One wire of the readback adapter was also found to have
come loose along the way, so the other wires were likely marginal too.

Lesson: **"the registers are right" and "the signal reaches the other side" are different things**, and the
latter is invisible in the trace. On a breadboard, leave room to suspect the wiring first.

### Cause 3 (consequence of the spec): `RESET` floats after drawing and the image disappears

After the image started appearing, one symptom remained: **when the demo finishes drawing, the screen goes
back to white**. This is not a bug but a consequence of docs/abi-spec.md §5.2, and **not port-specific
either**.

The last 4 entries of the trace are the answer:

```
[wasm] lcd-demo done
> spi@0.1.0/[resource-drop]bus(1)     <- SCK / MOSI / MISO return to input
> gpio@0.1.0/[resource-drop]pin(3)    <- lcd-rst returns to input, no pull = RESET floats
> gpio@0.1.0/[resource-drop]pin(2)    <- lcd-dc
> gpio@0.1.0/[resource-drop]pin(1)    <- lcd-cs
```

`pin.drop` returns the pin to input with no pull, as §5.2 says. The ILI9341 module on hand has no pull-up on
`RESET`, so the line floats, the panel resets, and the screen goes back to white.

**How it was confirmed (2 steps, both measured):**

1. Flashing an isolation build that stops `gpio_release` from returning the pin to input, **the image
   stayed**. That it is not a chip reboot was confirmed separately (observed for 150 s untouched: 1 boot, a
   14,352-line trace, no output after `lcd-demo done`)
2. **With a 10 kΩ pull-up from `RESET` to `3V3`, the image stayed in the shipping configuration (without the
   isolation feature).** That is the measured fix.
   `gpio_release` returns the pin to input as §5.2 says, so only the pull-up holds `RESET` high

**The only fix is in the external circuit, and it was confirmed to work on hardware.** Add a pull-up of about
10 kΩ from 3V3 to `RESET` (keep the existing `GPIO15` -> `RESET`. When the ESP32 drives `RESET` low, only
330 µA flows through the pull-up, and the reset pulse of the init sequence comes out fine). Ending without
releasing the handles on the guest side does not avoid it either: §5.2 also says "when `run` returns, the
host drops every remaining handle", so the release always happens. Reasons:

- Zero code change. The `.wasm` stays `fc470947...` and the recorded trace stays valid
- **Not letting a reset input float is proper circuit design.** Since §5.2 says "return to input, no pull",
  **the idle level of the line is for the external circuit to decide**, not something the host guarantees
- Most ILI9341 modules have a pull-up on `RESET`. The unit on hand just did not

#### It did not happen on the Pico 2 W (measured 2026-09-26)

**Whether this symptom appears differs per board.** Moving the same module to the Pico 2 W and running
without the pull-up, **the image stayed after drawing finished**. This originally said "`gpio_release` means
the same on both ports, so it should happen on the Pico 2 W too", but **in measurement it did not**, so this
is a correction.

| Board | Without pull-up | With pull-up |
|---|---|---|
| ESP32-S3 | **goes back to white** | stays |
| Raspberry Pi Pico 2 W | **stays** | (not tried) |

Two things must be kept apart:

- **The mechanism is the same on both ports.** By §5.2 the pin returns to high impedance, so **software does
  not decide the idle level of the line**. This is not port-specific
- **The electrical outcome differs per board.** Whether a floating line crosses the panel's threshold is
  decided by the pad's leakage and capacitance and the module's input spec. On the ESP32-S3 it crossed; on the
  Pico 2 W it did not. **Not a difference in port code**

So **the `RESET` pull-up is needed not as a "bug workaround" but to "not depend on a floating line"**. That it
works on the Pico 2 W is not a guarantee, only the observation that it happened not to cross (it can change
with temperature, unit and wire length).

docs/TODO.md §2 lists "whether to state the external pull-up on `lcd-rst` in the docs/abi-spec.md §8 table"
as an item for the owner.

### Tools used for isolation

`ports/esp32s3/src/probe.rs` (`--features hw-probe`). Drives `Board` directly without the guest or the
interpreter and does the following:

- reads the ILI9341 ID (`0xD3` / `0x04`)
- **fills the whole screen with 1 color** (see by eye whether SPI is alive from the port layer alone)
- **toggles DC at 1 Hz** (probe from outside whether it reaches the pad)

In the situation "the trace matches but no image appears", this was the only place that could show "whether
the registers reach the pads". The `rp2040` bring-up will need the same thing.

Note that **reading MOSI back in software alone does not work.** Pointing MISO at the same pin as MOSI should
read it back, but on the ESP32-S3 GPIO11 is FSPID itself and `with_mosi` selects the direct IO_MUX function,
so pointing `with_miso` at the same pin rewrites `mcu_sel` to the GPIO function and **disconnects the direct
output** (trying it returns `ff ff ff ff`). Left as a comment in `probe.rs`.

---

## 8. Direct cross-check of the two boards (2026-09-26)

§6 and §7 compared each board against the host reference. **This is the board-to-board cross-check that step
5 of §5 describes.**

```bash
(cd apps && cargo build --release)      # build fc470947... exactly once
(cd ports/rp2350 && cargo build --release --features guest-lcd-demo)
sh ports/esp32s3/build.sh build --release --features guest-lcd-demo
# confirm by byte comparison that the same .wasm is in both ELFs, then flash

sh verify/diff-traces.sh pico.log esp32s3.log
-> match: 14352 lines
```

| | |
|---|---|
| Guest | `apps/lcd-demo-rs`. SHA-256 `fc470947ac08b230ccdc59e04e09aa11105a5c8754efd00533858470f11d2453` |
| Board 1 | Raspberry Pi Pico 2 W (RP2350 A2 / QFN60). UART0 (GP0) 115200 8N1 |
| Board 2 | a board carrying an ESP32-S3-WROOM-1 (chip rev v0.2, flash 8 MB). UART0 (GPIO43) 115200 8N1 |
| Result | **14,352 lines identical** (including the 3,272 `spi.write` CRC-32 values) |

All 3 cross-checks match (Pico <-> host, ESP32-S3 <-> host, Pico <-> ESP32-S3). **docs/handoff.md §5 Phase 6
(2), "run the same binary on two boards; the trace except `time` and the SPI pixel CRCs are identical", is
reached.**

That the same `.wasm` is in both ELFs was confirmed before flashing by searching for the guest's bytes inside
the ELFs (`include_bytes!` embeds them as-is).

### A bug in `verify/diff-traces.sh` was hit here

The first cross-check came out as a mismatch, "`pico.log` has 1 line, `esp32s3.log` has 14352 lines". **It
was the script, not the trace contents.**

The board's serial emits a NUL from line noise at the moment of reset or power-on. `pico.log` contained
**1 NUL in 441,254 bytes**, and that alone made `grep` treat the file as binary and return the 1 line
`Binary file matches`. There are really 14,352 lines.

Fixed by dropping NUL in `normalize()` and adding `grep -a`. **This script decides the completion criterion,
so this kind of breakage leads straight to a misdiagnosis** (this time it was a false negative, but it was
about to be read as "the two boards do not match" and the hardware was about to be suspected).

**What this match does not say:**

- **Floating-point agreement (§4.2) has not moved.** `lcd-demo-rs` does not use f32
- **That the displays are the same does not follow from this match.** What matches is the host call
  sequence; whether it reaches the pads is separate (the `out_sel` bug of §7 is exactly that example). The
  displays were checked by eye on both boards (§6 / §7)
- **The behavior after drawing finished differed between the two boards.** Without a pull-up the ESP32-S3 went
  back to white and the Pico 2 W kept the image (§7 "Cause 3"). The traces are identical, so this is one more
  example that **a host call match does not guarantee that the display is kept**

In the course of this cross-check, 1 bug in `verify/diff-traces.sh` was hit and fixed (above). **A breakage
that first appeared with hardware logs**; it does not appear in host-to-host comparisons or the self-test
(no NUL gets mixed in).

---

## 9. App slot measurement (2026-10-04, Pico 2 W)

**Confirmed on hardware that the app can be swapped without reflashing the firmware.**
The first stage of docs/app-workflow.md §3.1 / §3.4.

The firmware is `ports/rp2350` (default features = trace enabled, built-in app = `blink_rs`). `lcd_demo_rs`
was put in the slot. **The built-in app and the slot app are different**, so the head of the trace shows which
one ran (no LCD wiring needed).

| Observed | Expected | Board output |
|---|---|---|
| Slot empty | falls back to built-in | `wasmicon: slot empty, running built-in` -> blink trace |
| App in slot | that one runs | `wasmicon: slot 4088 B crc32=0f19d973` -> lcd-demo trace (657 `spi` entries) |
| CRC corrupted by 1 bit | falls back to built-in | `wasmicon: slot crc mismatch, running built-in` -> blink trace |

The CRC matched the value `wasmicon pack` reported (`0f19d973`).

### The trace of the app run from the slot is identical to host

**14,352 lines.** The same count as recorded in §6 with the `include_bytes!` build, not 1 line different.
Measured proof that **distributing through the slot does not change the guest's behavior**. The cross-check
was done with both `wasmicon trace diff` and `verify/diff-traces.sh`, and **both gave the same verdict on the
hardware log** (the part of the `tests/trace.rs` cross-check that had only seen synthetic data).

```
$ wasmicon run --trace lcd_demo_rs.wasm > host.log     # 14,354 lines
$ wasmicon trace diff host.log pico.log
match: 14352 lines
```

The captured log had 1 NUL at its head (line noise at the moment of reset). Normalization drops it, so it
does not affect the verdict -- the same breakage hit in §8.

### By-product: handle cleanup does not pollute the trace

With the slot empty, the trace **ends with the guest's own `[resource-drop]pin(1)`**. `Hal::release_all`
(docs/abi-spec.md §5.2) runs after that, but **not a single line is added.** Hardware confirmation that the
recorded 14,352 lines (§6 / §7) do not change.

### 2 things hit in the `picotool` arguments

Neither shows up on host. The command `wasmicon pack` printed did not work.

- **The extension must be `.bin`.** `picotool load` decides the file type by extension, so `.slot` stops
  with `does not have a recognized file type (extension)`
- **`-o` is an absolute address.** Passing the flash offset (`0x100000`) is rejected with
  `invalid memory range 0x00100000-0x00101008`.
  Correct is `0x10100000`, with the XIP base added

### Capture procedure (needed when building `monitor`)

**On macOS, `stty -f` alone does not keep the baud rate.** It opens, configures and closes, so the next open
goes back to 9600. Once, a log of nothing but noise was actually captured (all 11,799 bytes garbled).

**The right order is "keep it open with `cat` and apply `stty`"**:

```
cat /dev/cu.usbserial-21420 > pico.log &
stty -f /dev/cu.usbserial-21420 115200 raw -echo   # apply while it is open
# reset the board here (unplug and replug USB)
```

**Applying `stty` while data is flowing mixes lines** (bytes drop, as in `<")`). Apply it before the reset. A
log captured in this order matched at 14,352 lines (above). Even in a garbled log the leading `slot ...` line
was readable, so the verdict could be made, but it cannot be used for the trace cross-check.

### Confirming that the command `pack` prints works

After the 2 bugs were fixed, **the command printed by `wasmicon pack --board rp2350` was run without changing
a character and it worked** (`picotool load -o 0x10100000 <file>.bin`). The trace match above was taken in
that state.

### `wasmicon deploy` worked in 1 command (same day)

`deploy`, which folds check / image preparation / write / reset into one, was run on hardware. **The slot
content was swapped from lcd-demo (4,088 B) to blink (775 B)**, so the swap shows at the head of the trace.
[The blink sizes and CRCs here predate the 2026-10 translation of the blink log strings; the lcd-demo-rs hash `fc470947...` is unaffected because that app has no Japanese literals.]

```
$ wasmicon deploy blink_rs.wasm --board rp2350
blink_rs.bin -> rp2350 slot (0x10100000)  791 B (wasm 775 B, crc32 d6c9dce3)
The device was rebooted into application mode.
-> running. To see the trace: ...

(serial from the board)
wasmicon rp2350
wasmicon: slot 775 B crc32=d6c9dce3
[wasm] blink start
```

The CRC matched the value `deploy` reported, and **the trace was identical to host (20 lines)**.

`deploy` **does not flash what will not run** (it passes `check --board` before flashing).
`tools/wasmicon-cli/tests/deploy.rs` locks in that sending sensor-display to rp2040 stops **without even
writing the image**.

### `deploy --monitor` worked in 1 command (same day)

Flash, run and capture folded into one. **The 3 commands done by hand until now, and the baud rate trap,
became 1 line.**

```
$ wasmicon deploy lcd_demo_rs.wasm --board rp2350 --monitor -o pico.log
opened /dev/cu.usbserial-21420 (before reset)
(stops after 3 s of silence)
silence: 441291 B / trace 14352 lines

$ wasmicon trace diff host.log pico.log
match: 14352 lines
```

**Exactly 14,352 lines were captured and matched host in full.** Nothing dropped, nothing mixed (when
captured by hand, applying `stty` while data was flowing mixed lines). This is because `monitor` **opens the
port and applies the baud rate before the reset**.

When to stop is decided by **silence**. When the app leaves `run` the firmware goes idle and prints nothing,
so "done after N seconds of silence" is the natural boundary (default 3 s). Waiting for Ctrl-C cannot be
automated.

**0 trace lines is a failure.** So that "nothing was captured" is not silently a success (the shape of the
wrong-baud-rate log captured in §9).

### Not yet seen

- **The check for the firmware's tail overlapping the slot** (`__flash_binary_end`) has not fired. Measured,
  the firmware ends at `0x10011100` and the slot is at `0x10100000`, so nearly 1 MB is free. A path that
  cannot be exercised without creating an overlap
- **RP2040 is unverified** (no Pico WH at hand). The code goes through the same `slot::read_xip` as RP2350

---

## 10. The ESP32-S3 slot (2026-10-04, measured)

Measured with `espflash board-info`:

```
Chip type:         esp32s3 (revision v0.2)
Crystal frequency: 40 MHz
Flash size:        8MB
Features:          WiFi, BLE, Embedded Flash
```

**Flash is 8 MB** (docs/TODO.md §5-2 resolved). **PSRAM does not appear in Features** -- it looks like it is
not fitted (N8, not N8R8). **One option of §5-3 disappears** (the arena cannot be moved to PSRAM, so for HTTP
the only choice is to lower `max_memory_pages` from 4 to 2). The evidence is espflash's output, so
**confirming against the module marking would make it certain**.

### `partitions.csv` was not needed (measured)

With `espflash flash`'s default partition table, `factory` occupies **almost the whole 8 MB** (the output says
`App/part. size: 145,904/8,323,072 bytes`). The design said "so the slot cannot be placed at a fixed offset",
but **measured, it can**:

1. Wrote a 256 B marker at the 7 MB point (`0x700000`) with `espflash write-bin`
2. Flashed the firmware with `espflash flash`
3. Read it back with `espflash read-flash 0x700000 256` -> **the marker was still there, intact**

`espflash flash` **erases only the app's sectors**. Being inside `factory` is harmless in itself, so the slot
went to a fixed offset (64 KiB from `0x100000`, aligned with the Pico family) without adding a
`partitions.csv`.

### The serial port is shared with flashing (where it got stuck)

**On the ESP32-S3, flashing / reset and the trace share the same port** (CH343 -> GPIO43/44). Unlike the
Pico, which uses a separate USB-serial converter. Therefore:

- while `monitor` holds the port open, `espflash` cannot use it (`Resource busy`)
- if `espflash` resets first, **the app's output is over before the port is opened**
- **`espflash reset` does not exit; it stays** (holding DTR/RTS). Calling it left the board stopped at
  `boot:0x0 (DOWNLOAD)`.
  **`write-bin` already does `--after hard-reset` by default, so it must not be called separately**

**The answer was "hand the whole port to espflash".** `monitor` only applies `stty`, so it cannot reset by
itself (it does not touch DTR/RTS). `espflash monitor` does `--before default-reset --after hard-reset` by
default, that is, it **opens first and then boots the chip**, so the ordering problem disappears within 1
process:

```sh
espflash monitor --port <dev> -c esp32s3 --non-interactive
```

- **`-c esp32s3` is required.** Without it, depending on `--before`, it fails with `chip_not_provided`
- **`--before no-reset-no-sync` cannot be used.** The app is running (= not in the bootloader), so it ends
  in `Timeout while running ReadReg command`. The default `default-reset` (enter the bootloader with DTR/RTS,
  then hard-reset) works
- it runs with `--non-interactive` (no interactive screen)

`deploy --monitor --board esp32s3` delegates to this tool (`monitor::capture_cmd`). The flashing side uses
`--after no-reset` so that **it does not run**, and the reset is left to `espflash monitor`. If the delegation
fails, it reports **that the board is in the bootloader** (`espflash reset` does not help).

The summary line shows **the slot offset**. The absolute address passed to `picotool` (`0x10100000`) is that
tool's argument, not a property of the slot -- on the ESP32-S3 it is a meaningless number.

### **2 readers fighting over the same port** (why captures came up short)

The first capture was cut off at 13,634 lines. The cause: **the next capture was opened while the previous
one was still alive**. macOS `cu.*` is not exclusive, so both can read, and **the bytes are split between the
2 processes** (the short side plus the later-opened side added up to about 14,352 lines, which was
consistent). If a capture is short, first suspect a leftover with
`pgrep -f 'wasmicon|espflash|cat /dev/cu'`.

### Matched all the way to running from the slot (the main point)

Captured in 1 try with `espflash monitor`:

```
wasmicon: slot 4088 B crc32=0f19d973
```

Matches the CRC `pack` reported. And then:

| Cross-check | Result |
| --- | --- |
| host (`wasmicon run`) vs ESP32-S3 (slot) | **14,352 lines match** |
| Pico 2 W (slot, §9) vs ESP32-S3 (slot) | **14,352 lines match** |

The latter is the key: **two boards that read flash differently** -- the RP2350 passes the XIP slice straight to
`decode`, while the ESP32-S3 reads the header through `esp-storage`, allocates exactly that length from the
arena and copies the body -- produced **the identical host call sequence** from the same `.wasm`. The slot
path does not break the determinism criterion of docs/handoff.md §5 Phase 6.

On top of that, `deploy --monitor` worked in 1 command:

```
$ wasmicon deploy apps/.../lcd_demo_rs.wasm --board esp32s3 \
    --port /dev/cu.usbmodem5C630009931 --monitor -o esp12.log
lcd_demo_rs.bin -> esp32s3 slot (+0x100000)  4104 B (wasm 4088 B, crc32 0f19d973)
-> handing reset and capture to espflash monitor:
silence: 442835 B / trace 14352 lines
```

This also matched host at 14,352 lines. **Unlike the Pico, no button is needed** (DTR/RTS enters the
bootloader, so there is no physical action like BOOTSEL).

### The failure branches were also triggered on hardware (the same 3 points as §9)

Writing directly to the slot (`espflash write-bin --after no-reset` -> `espflash
monitor`), all three entry points were exercised. **This procedure needs no button**, so the BOOTSEL replugging
that the Pico needed disappears:

| Slot content | Serial output |
| --- | --- |
| a valid image | `wasmicon: slot 4088 B crc32=0f19d973` -> lcd-demo runs |
| 1 byte of the body flipped | `wasmicon: slot crc mismatch, running built-in` |
| filled with `0xff` | `wasmicon: slot empty, running built-in` |

The CRC plays its role of separating "a transfer accident" from "an app bug" (the module comment of
`ports/common/src/slot.rs`) on hardware. It does not fail in the shape of passing a corrupted body to `decode`
and reporting an unrelated reason such as "unsupported instruction".

**The ESP32-S3's built-in app is blink** (flashed without `guest-lcd-demo`). `blink start` following the
fallback line is the proof that it fell back to the built-in app.

### What the review (`/code-review #8`, round 3) changed was re-checked on hardware (same day)

The firmware's startup path (`pick_guest`) was rewritten, so it was reflashed and the same 3 points were run
again. **The results do not change**:

| Slot content | Output |
| --- | --- |
| a valid image | `slot 4088 B crc32=0f19d973` -> **14,352 lines match host** |
| 1 byte of the body flipped | `slot crc mismatch, running built-in` -> blink |

What changed is that **the CRC is now checked before allocating from the arena**. `Arena` is a bump allocator
with no way to give memory back, so if it allocated first and then found the image corrupted, it would **run
the built-in app with the arena already reduced**, diverging from the `wasmicon check` estimate. Now it
streams the flash 512 bytes at a time and checks only the CRC first (`crc32_update` was added.
`esp-storage`'s `read` creates a 4 KiB sector buffer on the stack on every call, but the calls are sequential
so the peak is one buffer).

**That `deploy --no-run` had no effect was also reproduced on hardware.** `espflash write-bin` does `--after
hard-reset` by default, so even with `--no-run` **the app was running right after flashing** (capturing in
that state got 11,375 lines) while it printed "written; runs on reset". After the fix it prints
`Staying in bootloader` and **the capture is 0 lines** (it stays stopped).

### Not yet seen

- `Truncated` (the header length points outside the slot) has not been triggered on hardware. There is a
  unit test. With `espflash write-bin`, making an image that lies only about its length takes effort, so it
  was deferred
- **`Overlap` is not checked on the ESP32-S3** (it is an XIP-based check and only exists on the `read_xip`
  side). The ESP32-S3 reads through `esp-storage`, so a firmware / slot overlap has to be prevented on the
  `espflash` side.
  An open item in docs/TODO.md §5

---

## 11. I2C (SHT40) measurement (2026-10-05, ESP32-S3 and Pico 2 W)

The I2C of `ports/esp32s3` / `ports/rp2350` (implemented 2026-10-04) ran on hardware for the first time.
**Both worked on the first try.**

### Conditions

| Item | Value |
|---|---|
| Sensor | SHT40 (address 0x44). The only SDA / SCL pull-ups are the module's own |
| ESP32-S3 wiring | SDA=GPIO8 / SCL=GPIO9 (the defaults of docs/abi-spec.md §8) |
| Pico 2 W wiring | SDA=GP4 / SCL=GP5 (same) |
| Guest | `sensor_display_rs.wasm` (the card-style screen, 6,039 B, SHA-256 `860e00b9...`. `4654221`) |
| Flashing | slot (`wasmicon deploy`). The firmware was reflashed from the same day's main |

At first, only the ESP32-S3 was checked with the guest from before the screen change (4,549 B), to confirm
that I2C works and that the trace up to the read matches host. The numbers below are from the guest after
the screen change, run on two boards.

### Results

On both boards the following appeared, with no `nack` and no `timeout`; the CRC-8 check passed and the
temperature and humidity appeared on the screen:

```
> wasmicon:hal/i2c@0.1.0/[static]bus.open(0, 0)
< 0 [1]
> wasmicon:hal/i2c@0.1.0/[method]bus.write(1, 68, 0xfd)
< 0
> wasmicon:hal/i2c@0.1.0/[method]bus.read(1, 68, 6)
< 0 [len=6]
```

| Comparison | Lines | First difference |
|---|---|---|
| Pico 2 W and ESP32-S3 | 4,182 / 4,182 | line 3,757 |
| host (`verify/sht4x-replay.txt`) and Pico 2 W | 4,182 / 4,182 | line 3,757 |

Line 3,757 is **the first `spi.write` that draws a number after the sensor read**. The two boards read at
different times, and host uses synthetic data, so the values differ and everything from there on diverges, as
expected. **The 3,756 lines up to the sensor read (including all of the frame drawing) are identical across
all three.**

### What this measurement does not say

- **Cross-board agreement of the part that draws the values (the floating point of §4.2).** The trace shows
  **only the length of `i2c.read`, not the bytes read**, so what a board read is unknown, and the same
  response cannot be fed to both boards
- **Replacing `verify/sht4x-replay.txt` (§4.3).** For the same reason, the board's response cannot be
  recorded from the trace
- **The AS version.** Only the Rust version ran on hardware
- **RP2040.** Not evaluated (2026-10-07)

### The ESP32-S3 used separate ports for flashing and the trace

§10 used the single `USB-UART` side (CH343) for both flashing and the trace; this time **flashing went
through the `USB-OTG` side (USB-Serial-JTAG, `/dev/cu.usbmodem*`) and the trace through a CP2102N connected
to UART0 (GPIO43/44)**. In this setup `deploy --monitor` (which delegates to `espflash monitor` and reads the
same port as flashing) cannot be used, so **`wasmicon monitor --port <CP2102N> --idle 0` was opened first,
then `deploy --port <usbmodem>`**. `espflash` resets through USB-Serial-JTAG, so no button is needed.

### The Pico 2 W needs BOOTSEL only once

Flashing the firmware with `picotool load -u -v -t elf` (**without `-x`**) leaves it in BOOTSEL, so `deploy`
can flash the slot right after, and only then does it boot.

---

## 12. Matching the drawn values with the board's own response (2026-10-07, Pico 2 W and ESP32-S3)

In §11 the trace showed only the length of `i2c.read`, so agreement of the part that draws the values could
not be measured. By owner judgment, **the result of read-type calls now includes the bytes received**
(docs/abi-spec.md §9. `< 0 [len=6 data=0x6754c0a4d840]`). Import names, signatures and error codes are
unchanged. The `lcd-demo-rs` trace (14,352 lines), which does not use I2C, stays valid as-is.

### Procedure

```bash
wasmicon monitor --port <CP2102N> --idle 0 -o pico.log    # open first
wasmicon deploy sensor_display_rs.wasm --board rp2350      # after plugging in with BOOTSEL
wasmicon trace replay pico.log -o pico.replay              # extract data=
wasmicon run sensor_display_rs.wasm --trace --i2c-replay pico.replay > host.log
wasmicon trace diff host.log pico.log
```

### Results

| Guest | Response the board read | Board vs host fed with it |
|---|---|---|
| `sensor_display_rs.wasm` (`860e00b9...`) | `67 54 c0 a4 d8 40` (25.63 °C / 74.49 %) | **identical in full, 4,182 lines** |
| `sensor_display_as.wasm` (6,969 B, `53013cbf...`) | `68 38 9f a0 c3 6a` (26.24 °C / 71.94 %) | **identical in full, 4,182 lines** (against both the AS and the Rust version on host) |

ESP32-S3 (same day, reflashed with the new firmware):

| Guest | Response the board read | Board vs host fed with it |
|---|---|---|
| `sensor_display_rs.wasm` | `68 6c 25 ab 15 d2` (26.38 °C / 77.53 %) | **identical in full, 4,182 lines** (against both the Rust and the AS version on host) |
| `sensor_display_as.wasm` | `68 93 89 a4 38 81` (26.48 °C / 74.18 %) | **identical in full, 4,182 lines** (against both the Rust and the AS version on host) |

**Up to the drawing of the numbers and the gauge fill that includes the f32 temperature bar computation, the
RP2350 and the ESP32-S3 each produced the same pixels as host.** All 4 combinations (Rust / AS x 2 boards).
This was also the first time the AS version ran on hardware.

The Rust version's response went into `verify/sht4x-replay.txt` (replacing the synthetic data; §4.3).

### What this measurement does not say

- **A direct comparison of the two boards on the same input.** There is no way to inject a response into the
  hardware, so each board reads a different value. What can be said is "each board = host (for the input that
  board read)", and the two boards' inputs were different values all four times
- **Denormals and out-of-range clamping.** Only 1 point around 25 to 26 °C. The edge values are checked for
  Rust / AS agreement in the host test (`sensor_display_agrees_across_temperatures`), but not on the hardware
  FPU

### Things hit while capturing

- **`monitor --idle` starts counting without waiting for the first output.** If 5 s pass between opening and
  replugging the Pico, it ends with 0 B. In a procedure where a person handles the board, use `--idle 0`
- **Running 2 captures fights over the bytes of the same port** (same as §10). A capture with
  `--idle 0 --timeout 600` was left running while the next was opened, and both logs came up short. Solved by
  stopping when the app's last line (`[resource-drop]pin(1)`) arrives
- **The CP2102N's port name changes with how USB is plugged in** (`usbserial-21420` ->
  `usbserial-1420`). Do not hard-code it; look with `ls /dev/cu.usb*`

### Where the ESP32-S3 got stuck (same day)

**The reset issued over USB stopped working, and after flashing the board would not run.** Symptoms and
causes:

- **After flashing through USB-OTG (USB-Serial-JTAG), the board stopped waiting for download.** The ROM
  output was `rst:0x15 (USB_UART_CHIP_RESET),boot:0x21 (DOWNLOAD(USB/UART0))` /
  `waiting for download`. The LCD stayed dark. In §10 it flashed and ran through the same port, so what
  changed has not been pinned down
- **Using the `USB-UART` side (CH343) with the CP2102N still connected to UART0 puts 2 transmitters on
  GPIO44.** The DevKitC-1's CH343 TX is wired straight to GPIO44 on the board, so it fights the CP2102N's TX,
  the Mac -> chip direction (flashing sync) breaks, and the result is `Failed to connect to the device`. The
  chip -> Mac direction (GPIO43) just has 2 receivers, so the trace is still readable and it is easy to miss.
  **If the CH343 is used, disconnect the CP2102N**
- **The CH343's auto-reset only drove EN; GPIO0 did not work.** Sending the flashing reset sequence booted
  the app instead (a 32 KB trace arrived within 3 s) and it could not enter download mode. Manual
  `BOOT` + `RST` did not get there either (whether the buttons work is unconfirmed)

**The procedure that finally worked**: plug in both USB cables, **flash through the USB-OTG side**
(`espflash write-bin --port <usbmodem...> 0x100000 <app>.bin`), **capture the trace through the USB-UART
side (CH343)**, and boot with the **`RST` (EN) button**. `RST` booted normally with
`rst:0x1 (POWERON),boot:0x8 (SPI_FAST_FLASH_BOOT)`.
Sometimes it boots automatically after flashing, so **only use captures that are complete from the banner
(`wasmicon: slot ...`) to the last line** (a 2,306-line log captured midway was almost taken by mistake).

---

## 13. Firmware without the built-in app (2026-10-07, ESP32-S3)

The `include_bytes!` and the `guest-lcd-demo` feature were removed from all three ports
(docs/app-workflow.md §3.3). 2 points were checked on the ESP32-S3 hardware.

| Slot | Output | Result |
|---|---|---|
| `sensor_display_as.wasm` (6,969 B, crc32 `54778528`) | `wasmicon: slot 6969 B crc32=54778528` -> the app's trace | **identical in full, 4,182 lines**, against host fed with the response the board read (same as before the removal) |
| first 4 KiB filled with `0xff` | `wasmicon: slot empty, idle` | 0 trace lines. The guest does not run |

With the slot empty, `idle::heartbeat` in `ports/common` blinks the pin of the `led` role at 1 Hz. **No
external LED is connected to `led`, so the blinking itself has not been seen** (docs/TODO.md §1.1).
RP2350 / RP2040 call the same `idle::heartbeat` but have not been run on hardware (RP2350: build and clippy;
RP2040: not evaluated).

---

## 14. Firmware with the wiring table moved out (2026-10-07, ESP32-S3)

The fixed vocabulary of role names and the firmware's default table were abolished, and the wiring table was
placed in the config slot (docs/app-workflow.md §3.9). `wasmicon deploy --board esp32s3` was run from the
`apps/sensor-display-rs` directory, and the wiring table from `wasmicon.toml` was written together with the
app as 1 image (displayed as `wiring: lcd-cs=10 lcd-dc=14 lcd-rst=15`).

```
wasmicon esp32s3
wasmicon: roles lcd-cs=10 lcd-dc=14 lcd-rst=15
wasmicon: slot 6039 B crc32=909e18cd
```

All 3 `pin-by-role` lookups were answered from the wiring table, and the trace was normalized to
`role:lcd-cs` and so on. **Identical in full, 4,182 lines**, against host fed with the response the board read
(`67 af a8 a4 42 a2`). **Moving the wiring out of the firmware does not change the trace** (the normalization
of docs/abi-spec.md §9 prints role names). The CLI round trip (reading the app and the wiring table back from
the `deploy` image) is in `tools/wasmicon-cli/tests/pack.rs`; the format and its checks are in
`ports/common/tests/roles.rs`.

### Isolating the ESP32-S3's "reset over USB does not work" (same day)

**While the `USB-OTG` side (USB-Serial-JTAG) is connected to the Mac, booting with the `RST` button also
enters download mode** (`rst:0x1 (POWERON),boot:0x20 (DOWNLOAD(USB/UART0))`). With only the `USB-UART` side
plugged in, the same `RST` boots normally with `boot:0x8 (SPI_FAST_FLASH_BOOT)`. Most of the symptom described
in §12 as "reset over USB does not work" is this; running with the USB-OTG side plugged in is believed to have
been the cause (the mechanism is unconfirmed).

Also, flashing through the `USB-UART` side (CH343) **started working once the CP2102N was disconnected** (the
GPIO44 contention of §12). The only remaining symptom is that the reset of `deploy --monitor`
(`espflash monitor`) does not run the app; the `RST` button substitutes for now.

---

## 15. ESP32-S3 firmware at 240 MHz (2026-10-09)

`Config::default().with_cpu_clock(CpuClock::max())` is passed to `esp_hal::init`, raising the CPU from the
default 80 MHz to 240 MHz (a preset that keeps the PLL at 480 MHz and only changes the divider). Only the
firmware was reflashed, and it ran with the `sensor_display_rs.wasm` and the wiring table that remained in
the slot.

- Boot, reading the wiring table and reading the SHT40 all normal. The UART trace is not garbled either
  (SPI / I2C / UART are derived from APB, so they do not follow the CPU clock)
- **Identical in full, 4,182 lines**, against host fed with the response the board read (`69 0c 6a a9 20 3b`)

**Speed is not measured here.** A build that emits the trace takes about 10 s to push about 120 KB through
the 115200 baud UART, so the run time is capped there. That it is 3x faster was confirmed on 2026-10-05 with
a measurement with the trace off (docs/TODO.md §4).

---

## 16. The firmware now identifies itself (2026-10-09, ESP32-S3)

At boot, the line after the banner now reports the version, git, ABI and configuration in 1 line
(docs/app-workflow.md §3.8). **From here on, records keep the firmware's provenance in this line.**

```
wasmicon esp32s3
wasmicon id esp32s3 fw=0.1.0 git=ba2ce81-dirty abi=wasmicon:hal@0.1.0 trace=on pages=4 if=gpio,i2c,spi,time,log,board slot=v1/65536 roles=v1/4096
wasmicon: roles lcd-cs=10 lcd-dc=14 lcd-rst=15
wasmicon: slot 6039 B crc32=909e18cd
```

The sensor-display trace is **identical in full, 4,182 lines**, against host fed with the board's response.
A `trace diff` against a log with only `git=` edited keeps the verdict as a match but prints a warning.

### The ESP32-S3 serial was cut off at 128 bytes (fixed the same day)

In the first firmware flashed, the identity line was cut off at `slot=v1/65`. `esp-hal`'s `Uart::write`
**writes only as much as fits in the transmit FIFO (128 bytes) and returns the number of bytes written**, but
the port's `SerialPort::write` discarded the return value. The earlier lines happened to fit in 128 bytes so
it never showed, but trace lines are up to 160 bytes, so long hex dumps (a 32-byte `list<u8>` or `data=`)
must have been cut off the same way. Fixed to loop until everything is written. The Pico family uses
`write_full_blocking`, so it is not affected.

---

## 17. Recent changes checked on the Pico 2 W (2026-10-09)

§13 (removal of the built-in app), §14 (wiring table) and §16 (identity) had only been seen on the ESP32-S3.
The firmware at `14713cb` was flashed to the Pico 2 W and the same points were checked. On the Pico, the
single `deploy --monitor` command goes all the way from flash -> boot -> capture (no `RST` as on the
ESP32-S3).

| Checked | Output | Result |
|---|---|---|
| Identity | `wasmicon id rp2350 fw=0.1.0 git=14713cb abi=wasmicon:hal@0.1.0 trace=on pages=4 ...` | printed to the end |
| Reading the wiring table (deploy from `apps/sensor-display-rs`) | `wasmicon: roles lcd-cs=17 lcd-dc=20 lcd-rst=21` | all 3 `pin-by-role` succeed and normalize to `role:lcd-cs` and so on |
| Trace match | the path where the sensor does not respond | **identical in full, 3,734 lines**, against host (no response) |
| deploy without a wiring table (from a place with no toml) | `wasmicon: roles none` -> `[wasm] role not found` | `pin-by-role` is `unsupported` (7). The pins are not driven |
| Erasing the app and the wiring table | `wasmicon: roles empty, no roles` / `wasmicon: slot empty, idle` | 0 trace lines, parked |

### Identical in full down to the sensor values (2026-10-11)

After fixing the bad contact, deployed again from `apps/sensor-display-rs` with the wiring table.
**Identical in full, 4,182 lines**, against host fed with the board's response (`6b 70 36 98 25 88`).
With this, the changes of §13 / §14 / §16 are confirmed down to the sensor values on both the ESP32-S3 and
the Pico 2 W.

### The SHT40 did not respond (cause: contact between the jumper and the SHT40)

On the Pico 2 W between 2026-10-09 and 11, every write to the SHT40 came back `nack`
(`bus.write(1, 68, 0xfd)` -> `< 5`). The order of isolation:

1. **Even the firmware that read it on 10/07 (`7b15465`, Rust 1.97.1) gave `nack`** -> not the firmware
2. **The same SHT40 could be read on the ESP32-S3** -> the sensor and the address (0x44) are fine
3. **A throwaway diagnostic app** read GP4 / GP5 as inputs with pull-down / no pull / pull-up, and wrote to
   every I2C address. A line with an external pull-up should stay high even with the internal pull-down
   applied, but **it was low** (`ack: none`)
4. **With blink, confirmed that the jumpers used for SDA / SCL and the Pico's pins 6 and 7 pass a signal.**
   The SHT40's lamp was on too -> what remains is between the jumpers and the SHT40's SDA / SCL pins
5. Reconnecting the SHT40 with the 2 verified wires gave `pulldown=high` / `ack: 0x44`

**A caution on reading the diagnostic**: "low with pull-down" means "the line is not connected" only when an
external pull-up **is present**. If the module has no pull-up, a connected line reads the same (on this day,
that assumption was dropped once and the reading was misinterpreted).

## 18. The C bindings on the ESP32-S3 and the Pico 2 W (2026-10-20)

`apps/blink-c` is the C version of blink (`bindings/c`). Until now it had run only on the host
(`ports/host/tests/apps.rs`). The same `.wasm` was flashed to both evaluation boards and the traces were
compared with `wasmicon trace diff`.

- Guest: `blink_c.wasm`, 788 B, SHA-256 `fdbfd9fb...`, slot crc32 `6b2467d7`. Built with
  `sh bindings/c/build.sh` and wasi-sdk 33 (clang 22.1.0)
- Firmware: both boards reflashed from `e8443c6` (`trace=on`)
- Wiring: `deploy` from `apps/blink-c` wrote `led=2` (ESP32-S3) and `led=15` (Pico 2 W)

| Board | Capture | Result |
|---|---|---|
| ESP32-S3 DevKitC-1 | `deploy --monitor` through the `USB-UART` port (CH343). Flash, reset and capture in one command; no `RST` press was needed this time | **identical in full, 20 lines**, against host and against blink-rs on host |
| Pico 2 W | `deploy --monitor` from BOOTSEL, trace on UART0 through a CP2102N | **identical in full, 20 lines**, against host and against the ESP32-S3 |

Every host call succeeded (`pin-by-role` -> `role:led`, `pin.open`, 3 x `write` / `toggle`,
`[resource-drop]pin`).

### What this measurement does not say

- blink uses only `board`, `gpio`, `log` and `time`. I2C and SPI from C have run only on the host mock
- blink switches the LED at 1 ms, too fast to see. The LED itself was checked on the Pico 2 W with a
  throwaway variant (500 ms, 10 blinks; 747 B, crc32 `09419121`): it blinked, and its 48-line trace was
  identical in full to the host
- The CI build uses clang 18 from apt, so its `.wasm` bytes differ from the ones flashed here

### Things hit while capturing

- **`deploy` from the repository root flashed an empty wiring table** without a warning. `wasmicon.toml` is
  searched from the current directory, not from the `.wasm`. The board printed `wasmicon: roles none` and
  `pin-by-role("led")` returned `unsupported` (7). Running from `apps/blink-c` fixed it. Tracked in
  `docs/TODO.md §5`
- **The Pico 2 W UART gave 0 bytes** for three captures, then a single NUL at power-on. The firmware was not
  the cause (its code differs from the last verified build only in comments and strings, and the LED
  blinked). The cause was the wiring between GP0 and the CP2102N's RX; after it was redone, the trace came
  through
- `wasmicon monitor --idle 5` opened before `deploy` closed during the flash wait and missed the run. On the
  Pico, `deploy --monitor` from BOOTSEL avoids this

