# App development workflow design (loader and the `wasmicon` CLI)

This document is the **design**. **Remaining work and open questions are owned by
docs/TODO.md §5** and are not written here (see AGENTS.md: scattered copies always go
stale). The body refers to them by number, as in §5-1.

Note: RP2040 (Pico WH) is build-only since 2026-10; the evaluated boards are ESP32-S3 and RP2350 (Pico 2 W).

What must not change is collected in §6. In short: **the ABI (docs/decisions.md
D-02, D-03, D-05), `wit/`, the rules in docs/abi-spec.md, and the recorded traces
do not move.**

---

## 1. What changes

### 1.1 The current flow (as it is)

> **Added 2026-10-07**: the description below is the state before the work started
> (2026-10-04). Today the app is `deploy`ed to a slot and the firmware contains no
> app (§3.3; the built-in app was removed on 2026-10-07).

The app's `.wasm` is **baked in by the port from a path hardcoded with
`include_bytes!`** (`ports/rp2350/src/main.rs:48`). The only alternative is the
cargo feature `guest-lcd-demo`. So:

| Goal | What you do today |
|---|---|
| Fix one line of the app and try it | `cd apps && cargo build --release` -> rebuild the port -> reflash (Pico: hold BOOTSEL; ESP32-S3: espflash) |
| Run a different app | Edit the port's `main.rs` or add a feature |
| Write an app outside the repository | **Not possible.** The ABI-conformant build flags exist only in `apps/.cargo/config.toml` |
| Flash the firmware | ESP32-S3 requires espup and the Xtensa toolchain |

In other words, **the app and the firmware are fused into one deliverable**. The
reason for using Wasm at all (replace only the app) does not show up in the flow.

### 1.2 Goals

- **The firmware is flashed once per board.** After that, app authors never build firmware
- **The app is an independent deliverable** (one `.wasm`), sent to the firmware over USB or HTTP
- The only surface an app author touches is the `wasmicon` CLI. ABI-conformant build flags are the CLI's responsibility
- One cycle: `wasmicon run` (host, seconds) -> `wasmicon deploy` (hardware, a few seconds)

### 1.3 Relation to the v0.1 verification (docs/TODO.md §1)

**This work does not push docs/TODO.md §1 aside; it shortens it.** The next step in
§1.3 there is "read the SHT40 on hardware", and the procedure is "build the guest ->
rebuild the firmware -> flash -> capture the serial output -> cross-check against the
host trace". Of these, **the firmware rebuild and the capture / cross-check are
replaced outright by stages 0 and 1 of §5** (`monitor` enumerates `/dev/cu.usb*`;
the CH343 note in docs/TODO.md §1.1 matters here).

**The Phase 4 / 5 / 6 completion criteria (docs/TODO.md §1.3; defined over the two
boards ESP32-S3 and Pico WH) do not change.**

---

## 2. The three layers

| Layer | Deliverable | Who produces it | How often it changes |
|---|---|---|---|
| Firmware | `wasmicon-rp2350.uf2` / `wasmicon-esp32s3.bin` | This repository (release deliverable) | Rarely |
| App | `app.wasm` | The app author | Constantly |
| CLI | `wasmicon` | This repository | - |

The firmware is **runtime + HAL + loader / supervisor**. The app's contract is only
docs/abi-spec.md §6 (feature set, the `run` and `memory` exports, import names and
signatures); nothing board-specific is in it (pin numbers come from
`board.pin-by-role`, docs/abi-spec.md §8).

---

## 3. Firmware side: loader and supervisor

### 3.1 The supervisor loop

```
boot
 |- print the banner (e.g. "wasmicon rp2350") on UART0
 |- read the slot
 |   |- valid -> decode -> validate -> Exec -> instantiate -> start -> run -> cleanup -> park
 |   `- empty / corrupt -> print the reason and park (§3.3)
 `- park: wait for a frame on the control channel (§3.6)
        the port layer blinks the LED to show it is alive (does not go through Wasm)
```

Today `main` stays stopped after `run`
(`ports/rp2040/src/main.rs:146` and `ports/rp2350/src/main.rs:173` are
`loop { wfi() }`, `ports/esp32s3/src/main.rs:117` is `loop { spin_loop() }`, and
the host process exits). That follows docs/decisions.md A-04, "log and halt, no
restart", and **this policy does not change** (the final decision is held by open
question #3 in docs/abi-spec.md §10 and remains in docs/TODO.md §2).
"Halt" keeps its meaning (**never re-run the same app on its own**); the loader adds
one state, "accept a new app". docs/abi-spec.md §6.6 leaves behavior after a trap to the
port, so **this is not an ABI change** (to be settled under §5-1).

Rebuilt on every cycle:

- **The arena.** `Arena` is a bump allocator, so the only way to free it is to rebuild it (`runtime/src/arena.rs`)
- **The `Hal`.** `Hal::new(board, trace_on)` takes ownership of the board
  (`ports/common/src/lib.rs:114`), so either `into_board()` or a `reset()` of the
  handle tables is needed
- **The wiring table `roles` / `nroles`.** It remembers the numbers handed out by
  `pin-by-role` and is used for the docs/abi-spec.md §9 normalization (number ->
  `role:led`). **If it carries over, a number handed out by the previous app is
  printed as `role:` and the trace can change**

The app bytes live **outside the arena**. `decode` returns `Module<'m, 'a>`, which
**borrows the wasm bytes `'m` and the arena `'a` separately**
(`runtime/src/decode.rs:40`, `runtime/src/module.rs:92`). The current design of
passing an XIP slice straight in (docs/design-notes.md §4) depends on this.

### 3.2 Prerequisite fix: the docs/abi-spec.md §5.2 handle cleanup (implemented 2026-10-04)

docs/abi-spec.md §5.2 says "**when `run` returns, the host drops every remaining
handle**", but **no port implemented it**. Neither `run_wasm_opts` in
`ports/host/src/lib.rs` nor any port's `main.rs` cleaned up after `run`; it went
unnoticed because the `Drop` impls in the Rust bindings
(`bindings/rust/src/hal.rs:90` / `:176` / `:236`) released handles on the guest side.

Harmless for a single-shot firmware (it runs and stops). **With a loader, the next
app hits `busy`.** If the guest traps, handles are left behind, so the next app is certain to
hit it.

`Hal::release_all()` was added to `ports/common` and is **called in all four places
(host and the three ports) once `run` has returned**. It also runs when `run` left by
a trap. **This implements what the spec already says; it is not an ABI change.**

- **The cleanup emits no trace lines.** It is not a guest host call, and printing
  would change the recorded 14,352 lines (docs/verification-report.md §6, §7). It
  talks to `board` directly, bypassing `Hal::call`, which writes the trace
- **The release order is fixed: gpio -> i2c -> spi, ascending handle.** If it differed
  per port, boards would differ in a way the trace does not show
- Locked in by `ports/common/tests/release_all.rs` (release order, idempotence,
  **no trace lines**, re-`open` works after cleanup, **the role table survives**).
  Emptying `release_all` fails 3 of them
- **The role table is not cleared.** It is not a handle; it is state for the
  docs/abi-spec.md §9 normalization. If the loader reuses one `Hal`, it has to be
  cleared separately (§3.1)
- **The cost: the screen no longer survives a trap.** The cleanup returns pins to
  input (docs/abi-spec.md §5.2), so `lcd-rst` floats and the panel goes back to white
  (the same effect as "it goes white when drawing finishes" in §3.3). You can no
  longer look at what was on screen at the moment of the trap. The cleanup went in
  anyway because **docs/abi-spec.md §6.6 explicitly requires a drop on trap: "the
  host destroys the instance, drops all handles and logs the reason"**. **To look at
  the screen for diagnosis, use the `probe` command (§3.6)**
- **The order, however, is "log the reason, then clean up".** docs/abi-spec.md §6.6
  reads drop -> log, but cleanup **can wait indefinitely** (the `BSY` wait in
  `spi_close`, for example; docs/TODO.md §2.1). Doing it first means that when a
  peripheral hangs, the firmware stops silently with no reason printed, which breaks
  docs/decisions.md A-04, "log and halt". How to treat the §6.6 order is placed in
  docs/TODO.md §2

### 3.3 Where the app lives

Two places, and **both are needed**.

**(S1) RAM slot** - receive and run immediately. Lost on reboot. For the dev loop.

**(S2) Flash slot** - runs at power-on. Persistent.

| | RP2040 | RP2350 | ESP32-S3 |
|---|---|---|---|
| SRAM | 264 KB (`ARENA` 160 KB) | 520 KB (`ARENA` 320 KB) | 512 KB (`ARENA` 300 KB; almost no DRAM left. docs/TODO.md §1.4) |
| How the RAM slot is taken | Cut off the head of `ARENA` with `split_at_mut` | Cut off the head of `ARENA` with `split_at_mut` | Cut off the head of `ARENA` with `split_at_mut` |
| Flash write | `rom_data` (same family as RP2350) | `rom_data::connect_internal_flash` / `flash_exit_xip` / `flash_range_erase` / `flash_range_program` / `flash_flush_cache` (rp235x-hal 0.4.0) | `FlashStorage` from `esp-storage` 0.10 (`SECTOR_SIZE` = 4096) |
| Running from flash | **Implemented 2026-10-04** (the XIP slice is passed as is; no RAM copy) | **Implemented 2026-10-04**, the XIP slice passed as is (64 KiB from `0x10100000`; verified on hardware on 2026-10-04, docs/verification-report.md §9; RP2040 never flashed) | **Copied to RAM** (1 MiB slot from `0x100000`, only the app's length is copied). No assumption that an arbitrary offset is XIP-mapped (MMU / DROM) |
| Assets slot (§3.10) | `0x111000` to the end of flash (978,944 B), read in place | `0x111000` to the end of flash (3,076,096 B), read in place | `0x201000` to the end of flash (6,287,360 B), read by range through `esp-storage` |

- Apps are currently 775 B to 5.3 KB (`blink_rs` 775 B / `lcd_demo_rs` 4.0 KB /
  `sensor_display_as` 5.3 KB), so even a 64 KiB slot is under 1/5 of `ARENA` on
  RP2350. **RP2040 is tight on SRAM, so keep it small** (16 KiB or so)
- The ESP32-S3 slot is 1 MiB for the PSRAM firmware's large apps (docs/TODO.md
  §5-10). Only the app's length is copied, so the plain firmware's arena does not
  shrink, but an app over roughly 250 KB does not fit that arena
- On RP2350, **code executing from XIP cannot run while erasing or programming**.
  The write routine has to be placed in RAM (`#[link_section = ".data"]`) and called
  with interrupts off (**to be confirmed** -> §5-6)
#### No app in the firmware (owner decision 2026-10-04)

> **Done on 2026-10-07.** The `include_bytes!` and the `guest-lcd-demo` feature were
> dropped from the three ports, and the CI rp2040 / rp2350 jobs no longer build `apps`
> first. An empty slot prints `wasmicon: slot empty, idle` and enters
> `idle::park` in `ports/common`, which drives no pins (§3.9).

**The built-in `include_bytes!` app is removed.** On an empty slot the firmware
prints the reason on the UART and parks; **no pin is driven** (the LED heartbeat this
design first proposed was removed on 2026-10-07, §3.9).

The reason is not only consistency ("no app in the firmware", without exception).
**The current `include_bytes!` makes the firmware build depend on the `apps/`
build** - that is why the CI rp2040 / rp2350 jobs build `apps` first - and
**the firmware cannot be produced as a standalone release deliverable** (a head-on
conflict with the distribution in §4.5).

- **Remove it at the same time as the slot goes in** (stage 1 of §5). Removing it
  first would leave the firmware running nothing
- **Fix CI at the same time**: the port jobs no longer need the `apps` pre-build, and
  the `cargo clippy --features guest-lcd-demo` line disappears (the feature itself is gone)
- **What is lost**: the quick way to confirm on hardware, right after flashing, that
  "the runtime gets from decode through run". The host tests and the first `deploy`
  replace it. The banner only shows "the firmware is alive", not that the
  runtime works
- **The recorded traces stay valid.** `deploy` `lcd_demo_rs.wasm` (`fc470947...`) to
  the slot and run it: **the bytes are identical, so the host call sequence is
  identical** (the guest does not know how it arrived). Which XIP address it sits at,
  or whether it was copied to RAM, does not show in the trace

Where the slot goes in flash is **a separate problem per board**:

- **RP2350 / RP2040**: 4 MB flash. The firmware ends at `__flash_binary_end`
  (`ports/rp2350/memory.x`). Put the slot at a fixed offset (e.g. 1 MB from the
  start = `0x1010_0000`) and **check at boot that it does not overlap the end of the
  firmware, printing the result on the UART**. An overlap destroys the firmware itself
- **ESP32-S3**: **do not hardcode a fixed offset.** espflash's default partition
  table extends `factory` to the end of flash, so a choice like "the 3 MB mark" lands
  inside factory. Add a dedicated `wasmicon, data, ...` entry to `partitions.csv`,
  flash with `espflash --partition-table`, and `write-bin` to that offset. Today
  `esp-bootloader-esp-idf` is built with `default-features = false`, which turns the
  partition-table check off (`ports/esp32s3/Cargo.toml`), so **consistency is our
  own job**
- Flash size and whether PSRAM is present depend on the exact board variant at hand -> §5-2

### 3.4 Slot format

```
offset  size  content
0       4     magic "WMCA" (Wasmicon App)
4       2     format version (= 1)
6       2     reserved (0)
8       4     wasm length (u32 LE)
12      4     wasm CRC-32 (u32 LE)
16      n     wasm body
```

The format is owned by `slot` in `ports/common` (**not written twice, once for the
writer and once for the reader**). On the CLI side `wasmicon pack` builds the image.

- The wasm starts on a 16-byte boundary, so the slice can be passed straight to `decode`
- The CRC distinguishes "transfer accident" from "app bug". One fewer "stops silently"
  case from docs/TODO.md §1.4
- **Empty (erased `0xff` / unused `0x00`) is not a failure.** The firmware sees it and
  parks (§3.1). It is distinguished from a magic mismatch (something else was
  written) - the log means something different
- The **whole** slot region may be passed in (the header carries the length, so the
  rest is ignored). The intended use is reading an entire 4 KiB sector and passing it in
- **The ABI version is not in the header.** The import module name
  (`wasmicon:hal/gpio@0.1.0`) already carries the version and must match exactly
  (docs/abi-spec.md §3.1, §6.4), so a mismatch is rejected by `instantiate` with
  `unknown import`. The CLI rejects it **before sending** using the ABI version that
  `info` (§3.6) returns

### 3.5 Transport

**Control goes over USB, the trace over UART0.** Mixing them on one UART makes frame
sync and log lines collide. And **both boards have a free USB port** (the ESP32-S3
`USB-OTG` side outputs nothing with the current firmware,
`apps/lcd-demo-rs/README.md`; the Pico's USB is unused).

| Board | Control channel | Basis |
|---|---|---|
| ESP32-S3 | USB-Serial-JTAG (the `USB-OTG` port) | `usb::usb_serial_jtag` in `esp-hal`. **No extra crate** |
| RP2350 / RP2040 | USB CDC | `rp235x-hal::usb` (the `UsbBus` impl for `usb-device`) + `usbd-serial` |

**Stage 0, before any USB stack** (write only the slot with the existing flasher):

- ESP32-S3: `espflash write-bin <offset> app.bin`. DTR/RTS drops it into the ROM
  bootloader, so **no button press is needed**
- RP2350 / RP2040: BOOTSEL + `picotool load -o <offset>`, or **the CLI builds a UF2
  that points at the slot address and copies it to the `RPI-RP2` drive** (no
  picotool). Whether RP2350 needs the absolute family ID (0xe48bff57) for an
  arbitrary address is **to be confirmed** (-> §5-6)
- Once the firmware has a USB control channel it can drop itself into BOOTSEL with
  `reboot::RebootKind::BootSel` (rp235x-hal 0.4.0 `src/reboot.rs`), which removes the
  Pico's button press too

**HTTP:**

- **Start with ESP32-S3 + Wi-Fi (`esp-radio`). The RAM budget is the real issue.** DRAM
  is nearly exhausted today (`.bss` 315 KB / `ARENA` 300 KB / native stack 17.4 KiB;
  docs/TODO.md §1.4). Linking esp-radio costs about 54 KiB of internal SRAM even
  when Wi-Fi is never started, and starting it needs about 49 KiB of heap
  (docs/verification-report.md §22), so it is **either a separate Wi-Fi firmware with
  3 pages instead of 4, or linear memory in PSRAM**. The former is within what
  docs/abi-spec.md §6.2 allows (the port decides the limit) but **an app that uses the
  fourth page does not run on the Wi-Fi firmware**. The latter works on the board on hand (a
  Freenove ESP32-S3-WROOM board with 8 MiB of octal PSRAM; docs/verification-report.md
  §21), but **PSRAM is slower** (reads 2.9x sequential, 8.5x on cache misses) and a
  PSRAM-less module cannot do it -> docs/TODO.md §5-3
- **Pico 2 W / Pico WH have a CYW43439.** The usable driver `cyw43` assumes embassy
  (async), which means rewriting the port away from the current blocking setup.
  The recommendation is **HTTP in v1 on ESP32-S3 only** -> §5-4
- Direction (device as server, or pull from a URL) is §5-5. For the dev loop
  `POST /app` + mDNS is easiest; for OTA, pull is easiest

### 3.6 Protocol

**The agreement for the CLI and the firmware to talk over USB / HTTP.** It is a
different thing, with a different counterpart and purpose, from the ABI between
guest and runtime (`wasmicon:hal@0.1.0`); **the app is not involved at all.**

```
frame: "WMCN" | ver u8 | cmd u8 | len u32 LE | payload | crc32 LE
```

Resync on the magic. Commands:

| cmd | name | payload | meaning |
|---|---|---|---|
| 0x01 | `info` | none | return the board's self-description |
| 0x02 | `load` | wasm | to the RAM slot (S1) |
| 0x03 | `store` | wasm | to the flash slot (S2) |
| 0x04 | `run` | none | run the app in the slot |
| 0x05 | `stop` | none | end the run and return to idle |
| 0x06 | `reset` | none | reset |
| 0x07 | `probe` | none | drive the port layer directly for diagnosis (below) |
| 0x08 | `config` | config text (empty = erase) | write the role map (§3.9). `info` returns the effective values |

What `info` returns -> **§3.8** (board name, firmware version, ABI version, `Config`,
slot capacity and format version, protocol version, role names, configuration).
**This is what stops the CLI from hardcoding board limits** (§4.3).

`probe` is the current `hw-probe` feature (`ports/esp32s3/src/probe.rs`) turned into
a command. **It drives `Board` directly without running a guest**, so it is the only
way to look at "the trace matches the host exactly but the screen is white" - whether
the register settings reach the pads - and it is what actually caught `out_sel` on
ESP32-S3 (docs/verification-report.md §7). **As a command it diagnoses without
reflashing, and there is one build configuration fewer** (§3.8).

**Interrupting a running app is done plainly by reset.** There is no knowing when the
guest yields, and polling USB inside a host call changes how long host calls take
(the trace content does not change, but `time` values do). For the dev loop,
"reset -> idle -> send" is enough.

HTTP is the same commands in a different shell (`GET /info`, `POST /app`). **The
protocol version applies to both paths equally** (the version constrains the command
set and payload shapes, not the shell).

#### Why there is a version, and what is frozen

**The CLI and the firmware are updated separately.** Firmware is flashed once and
left alone; the CLI gets newer through `cargo install`, so "**a new CLI talks to an
old firmware**" happens routinely. If `info` answers `protocol 1`, the CLI can decide
"this firmware only speaks v1 -> do not use v2 features / ask for a reflash".

For that, **the frame envelope (magic + `ver` + `cmd` + `len` + `crc`) must be frozen
once decided**. Otherwise the very frame that asks for the version becomes
unreadable. The version applies not to the envelope but to the contents (command set
and payload shapes), and **a firmware that receives an unknown `ver` reads only the
envelope and answers "I do not speak that version".**

#### The four "versions" in this design

Easy to confuse, so here they are side by side. **The top three move independently,
so they are held separately.**

| Name | Between whom | Where it lives | On mismatch |
|---|---|---|---|
| **ABI version** `wasmicon:hal@0.1.0` | guest (`.wasm`) <-> runtime | Enforced by the import module name (exact match, docs/abi-spec.md §3.1). `info` declares it too (§3.8, to reject before `deploy`) | Every app fails with `unknown import` |
| **Protocol version** | CLI <-> firmware | The frame's `ver u8` (this section) | The CLI cannot talk |
| **Slot format version** | writer <-> the bytes in flash | The slot header (§3.4) | An old firmware reads a new slot |
| **Firmware version** | - (not involved in compatibility) | `info` and the banner (§3.8) | Nothing breaks. Provenance only |

Adding one command without touching the ABI bumps only the protocol version, and
**the app is not affected at all**. This separation is what §3.8 means by "a single
version compared as a range would lie"; how firmware versions are assigned is §5-8.

### 3.7 Relation to determinism

- **The loader adds no host calls**, so the trace lines of docs/abi-spec.md §9 do not
  change. The recorded 14,352 lines (docs/verification-report.md §6, §7) stay valid
- `verify/diff-traces.sh` discards lines that do not start with `>` / `<`, so loader
  log lines (the banner, `loaded 4096 bytes crc=...`, and so on) may be mixed in
- **There are only two paths that break it**, both avoided in §3.1 / §3.2: the handle
  cleanup emitting trace lines, and the role table carrying over
- **But "same `.wasm`, different trace" can now be caused by a firmware difference**,
  so it has to be possible to record which firmware produced a trace -> §3.8

---

### 3.8 Firmware identity and the axes of compatibility

Firmware is **different per board** and **different per version**. What is needed is
not a "firmware package manager" but **the firmware stating who it is** and **a
cross-check before sending**.

#### Compatibility is not one version number

Seen from the app there are five independent axes.

| Axis | Where it is today | What happens on mismatch |
|---|---|---|
| ABI version `wasmicon:hal@0.1.0` | The import module name (exact match, docs/abi-spec.md §3.1) | Every app fails with `unknown import` |
| Board profile (`Config` + **implemented interfaces**) | Each `main.rs` (RP2040 = 2 pages, others = 4; `esp32s3-psram` = 112 since 2026-10-23) and each `board.rs` (RP2040 has no SPI / I2C) | A large app fails validate / `unsupported` on hardware |
| Role names (free-form since 2026-10-07) | The app's `wasmicon.toml` -> the device's config slot (§3.9) | `pin-by-role` returns `unsupported` |
| Protocol version | The frame's `ver u8` (§3.6) | The CLI cannot talk |
| Slot format version | The slot header (§3.4, = 1) | An old firmware reads a new slot |

**A design that keeps one firmware version and compares it as a range will lie.**
`info` **declares each axis** and the CLI judges each axis. The firmware version is
**provenance only**.

#### The firmware states who it is in the banner (implemented 2026-10-09)

At boot, the line after the banner is a one-line identity (`identity` in
`ports/common`):

```
wasmicon id esp32s3 fw=0.1.0 git=ba2ce81-dirty abi=wasmicon:hal@0.1.0 trace=on pages=4 if=gpio,i2c,spi,time,log,board,assets slot=v1/1048576 roles=v1/4096 assets=v1/6287360
```

- `git` is `git describe --always --dirty` at build time (each port's `build.rs`
  includes `ports/common/build/git.rs` to fill it in). The ABI version is
  `generated::PACKAGE`, generated from `wit/`. The role list goes on the next line
  (`wasmicon: roles ...`)
- `monitor` captures the raw serial output, so the identity stays at the head of the
  log. `trace diff` warns if the identities of the two logs differ (the verdict does
  not change)
- **There is no protocol version yet** (the protocol itself arrives in stage 2).
  `info` is stage 2 as well

Below is the state before implementation (2026-10-04):

- The banner was the single line `wasmicon rp2350`, **with neither version nor git**
  (`ports/rp2350/src/main.rs:140`, `ports/esp32s3/src/main.rs:87`,
  `ports/rp2040/src/main.rs:124`)
- docs/verification-report.md **records the guest's SHA-256 (`fc470947...`) but
  nothing about the firmware**. Which build produced the 14,352 lines can only be
  guessed from the report's date and the git history
- For a single-shot firmware "the person who flashed it remembers" is enough, but
  **once a loader is in, "same `.wasm`, different trace" can be caused by a firmware
  difference** (§3.7)

#### What it carries

`info` and **the one-line banner** (useful where only serial is available) print the
same thing:

board name / firmware version + `git describe` + dirty / ABI version / configuration
(whether `trace` is on) / `Config` / **implemented interfaces** / slot capacity and
format version / protocol version / list of role names.

- `git describe` is filled in at build time (`build.rs`). **Do not drop dirty.**
  If a local build cannot say so, the verification record becomes ambiguous
- **A variant is configuration, not a version.** Point `monitor` at a board flashed
  without `trace` and nothing comes out (today you would start by suspecting the
  wiring). `trace: off` says it at once
- **There are only two variants: `trace` on / off.** `guest-lcd-demo` disappeared
  with the built-in app (§3.3, 2026-10-07). `hw-probe` becomes the `probe` command
  rather than a build configuration (§3.6). The manifest and cache key is
  **board x version x variant** (the version alone is not unique)

#### Uses

- **Cross-check before `deploy`**: ABI version, initial pages <= `max_memory_pages`,
  **whether the interfaces the app imports are implemented on that port**, size <=
  slot capacity, slot format version. Just swap the table in `check --board` (§4.3)
  for the device's self-description
  - **Role names cannot be enumerated statically** (§4.3). Only names found in the
    data segments can be cross-checked against the role list `info` holds
  - Without the implementation status, `check` for `ports/rp2040` **passes
    statically** (SPI / I2C returning `unsupported` happens at run time). The §4.6
    example shows this
- **A stamp on the trace**: `monitor` records the identity at the head of the captured
  log. `trace diff` excludes the identity line from the comparison but **warns if it
  differs** (same treatment as excluding `time`; docs/decisions.md D-10)
- **Distribution**: a manifest per board x version deliverable (file + sha256 + the
  five axes above). `wasmicon flash --board rp2350 [--fw 0.2.0]` with a local cache,
  and `wasmicon fw list` (versions on hand, and the version on the device). The
  counterpart of §4.5

#### Not built (for now)

A/B slot rollback, signature verification, firmware self-update over HTTP, a device
inventory. Overkill for one person, three boards and a cable on the desk; A/B in
particular **complicates the flash budget and the app slot placement at the same
time** (§3.3).

**The slot format version and the protocol version are kept from the start, though.**
They cost a few bytes and cannot be added later. How version numbers are assigned
(one shared across the three ports, or per port) is §5-8.

---

### 3.9 The wiring table as device-side configuration

> **The owner settled this on 2026-10-07 and the stage 1 minimum was implemented**
> (docs/TODO.md §5-9). Decided: **role names are not firmware vocabulary** (the fixed
> `ROLE_NAMES` vocabulary is gone) / **the firmware has no default table** /
> **parking on an empty slot drives no pins** (the LED blink was removed).

Originally the role -> GPIO mapping lived **inside the firmware**
(`profile::<board>.roles`). **Changing the wiring meant reflashing the firmware**,
which conflicted with "flash the firmware once" (§1.2). Waiting for a firmware
release just to move the LCD's CS to the next pin is wrong.

#### Role names are "an agreement between the app and the wiring"

The fixed vocabulary was a shortcut: with no `alloc`, the trace normalization
(docs/abi-spec.md §9) wanted `'static` names. It was not a design requirement. A
role name is:

- **declared** by the app in `[requirements] pin-roles` of `wasmicon.toml`
- wired **by number** in `[board.<name>.roles]`
- written by `deploy` into the config slot, and the firmware answers **exactly as the
  table it read says**

A name starts with a lowercase letter, is at most 16 characters of `a-z` / `0-9` /
`-`, and there are at most 8 (`roles::valid_name` / `MAX_ROLES` in `ports/common`;
the limit exists because there is no `alloc`). `led` is no longer special to the
firmware either (it is just one of the names blink looks up).

**Typos are caught** not by a vocabulary but by "the declaration and the wiring table
disagree". `check` / `deploy` fail **before flashing** if a declared role is missing
from that board's wiring table (no device connection needed).

#### What is configurable (roles only, to begin with)

| Target | Configurable? | Why |
|---|---|---|
| Role -> GPIO | **Yes** | Pins the guest drives directly. This is what people want to rewire |
| Bus pins (SPI SCK/MOSI/MISO, I2C SDA/SCL) | **No** (for now) | Hardware constraints differ per chip. On RP2350 the PL022 SPI0 is fixed to specific pins by FUNCSEL; on ESP32-S3 the GPIO matrix allows almost anything. **It needs a table of allowed pins per port**, a separate topic |
| Bus `index` | No | The guest sees only `index`; what is behind it has always been the port's job (docs/abi-spec.md §8) |

#### Where it lives

**4 KiB right after the app slot** (`profile::<board>.role_slot`; currently
`0x110000`, and `0x200000` on the ESP32-S3). The lifetimes differ, but **`deploy`
writes the app slot (64 KiB; 1 MiB on the ESP32-S3) and the config slot as one image
in one go** (`pack::with_roles`), so
flashing takes no extra step. The space after the app is filled with `0xff` to the
end of the slot.

**The wiring table is written every time.** Even an empty table is written, so the
result of a `deploy` is determined by that moment's `wasmicon.toml` alone (no
previously flashed table survives).

#### The format is text

The header is built like the app slot's (magic `"WMCR"` + format version + length +
CRC-32); the body is one entry per line:

```
lcd-cs=17
lcd-dc=20
lcd-rst=21
```

Binary (`[role_id u8, gpio u8]`) would be smaller, but text was chosen.

- **No new "role ID" contract.** Assigning IDs creates a fifth compatibility axis
  (§3.6), "IDs shift between versions"
- `info` can read it out as is. Dump the flash and a human can read it
- Parsing works in `no_std` (split on `=`, read a decimal). The body is at most 256 bytes
- `"none"` in `wasmicon.toml` means "do not hand out" and is not written to the table

#### Checked twice (CLI and firmware)

The same check (`roles::RoleMap::insert`) runs **in the CLI before flashing** (when
`wasmicon.toml` is read) and **again in the firmware at boot**. The constraints live
in the profile's `gpio_count` / `reserved` / `bus_pins`, and each port's `board.rs`
confirms at compile time that its own values agree.

| Check | Why it rejects |
|---|---|
| Name format | Must not collide with `=` or newlines in the body, or with `role:` in the trace |
| `number >= gpio_count` | Out of range |
| **Reserved pins** | GP0/GP1 are **the trace UART0**. Assigning them kills the trace. Same for the CYW43439 side (GP23/24/25/29) |
| Two roles on one number | At run time it is only `busy`, but the cause is hard to see |
| **Numbers the port uses for a bus** (rp2350: GP4/5, 16/18/19; esp32s3: GPIO8/9, 11/12/13) | Assigning them silently breaks SPI / I2C |

The last row needs care. For the reasons in docs/abi-spec.md §8, **bus pins are
deliberately not in `reserved`** (putting them there makes "can this GPIO be opened"
differ from the host mock, and the trace cross-check becomes a different thing per
board). **Rejecting them in the wiring-table check does not change run-time behavior,
so it does not conflict with that decision.**

**A corrupt table never drives pins on its own.** The firmware has no default table,
so for empty, CRC mismatch or a constraint violation alike it **boots handing out no
roles** and prints the reason in the banner (`wasmicon: roles crc mismatch, no
roles`). One bad entry discards the whole table. When it reads fine it prints
`wasmicon: roles lcd-cs=17 lcd-dc=20 lcd-rst=21`. Neither is a trace line, so
`trace diff` does not compare them.

**Applied after reboot** (read once at boot). `deploy` resets after writing, so the
flashed table takes effect immediately.

#### The host keeps its table in the profile

The host (mock) has no flash, so its table is in `profile::HOST.roles`, used by
`wasmicon run` and the tests (`led` / `lcd-cs` / `lcd-dc` / `lcd-rst`). The real
boards' `roles` is empty.

#### Why determinism is not broken

docs/abi-spec.md §9 **normalizes numbers handed out through a role to
`role:lcd-cs`**, so **changing the mapping does not change a trace line.** The role
indirection had this property from the start; stripping wiring freedom out of the
firmware keeps the determinism criterion (docs/decisions.md D-10) intact.

**One trap, though.** docs/abi-spec.md §9 also says the host can **only judge from
the numeric argument**: if a guest hardcodes a pin number that happens to match a
role assignment, that too is replaced by `role:`. So **changing the wiring table changes
the trace of an app that hardcodes pins** (a line that was `role:lcd-cs` becomes a
raw number). §9 already excludes such apps from determinism verification, but the
symptom reads as "I changed the configuration and the trace changed", so the CLI
should be able to explain it.

#### Reflected in the identity (an extension of §3.8)

**The same firmware version with different wiring is a different setup.** So:

- `info` declares the **effective values** (default + override) and whether an
  override is in place
- the identity that `monitor` stamps at the head of the log **includes the CRC-32 of
  the effective wiring table**

Without that, "same `.wasm`, different trace", which §3.8 set out to solve, just gets
one more candidate cause.

#### What this still does not solve

- **Bus pins** -> needs a per-port table of allowed pins. Separate topic
- **Electrical facts** - external pull-ups, `lcd-rst` floating and the screen going
  white (docs/TODO.md §2) -> not a configuration matter
- **Rewriting over the protocol** (`config apply`) is stage 2 (USB control channel).
  Today `deploy` writes it from outside together with the app (stage 1)

### 3.10 The assets slot (read-only data the app ships)

Large read-only data (DOOM's IWAD, fonts, images) does not belong in the `.wasm`:
it would not fit the app slot, and every app update would resend it. It goes into a
third slot, right after the config slot, that runs to the end of flash
(`Profile::assets`; the `assets` module in `ports/common`). The guest opens an asset
by name through `wasmicon:hal/assets@0.1.0` (docs/abi-spec.md §7) and reads it by
range; a body is never copied to RAM as a whole.

| | RP2040 | RP2350 | ESP32-S3 | ESP32-C5 |
|---|---|---|---|---|
| Offset | `0x111000` | `0x111000` | `0x201000` | `0x111000` |
| Size | 978,944 B | 3,076,096 B | 6,287,360 B | 15,659,008 B |
| Read | XIP slice, in place | XIP slice, in place | `esp-storage`, by range | `esp-storage`, by range |

The shareware IWAD (4,196,020 B) fits only the ESP boards; `check --board` says so
before anything is flashed.

```
offset  size  content
0       4     magic "WMCB" (Wasmicon Blobs)
4       2     format version (= 1)
6       2     number of entries (at most 8)
8       4     CRC-32 of the entry table (u32 LE)
12      4     reserved (0)
16      44*n  entries: name (32 bytes, NUL-padded), body offset (u32 LE, from the
              start of the slot, a multiple of 4096), body length (u32 LE),
              body CRC-32 (u32 LE)
4096    ...   bodies, each on a 4096-byte boundary, in table order
```

- The format is owned by `assets` in `ports/common` (writer and reader share it, as
  for the app slot in §3.4). `wasmicon assets pack` builds the image from the
  `[assets]` table of `wasmicon.toml` (§4.7) and `wasmicon assets write` flashes it
- **Only the CLI writes the slot.** The firmware never writes flash, and `deploy` does
  not touch this slot, so the data survives app updates. The slot is per device, not
  per app: the last `assets write` wins
- Names: 1..32 bytes of `a-z` / `0-9` / `-` / `_` / `.`, starting with a letter or a
  digit. Flat: no `/`, no directories. At most 8 entries
- Each body starts on an erase-unit boundary, so a body is one contiguous range (on
  XIP boards the guest's `read` is a `memcpy` out of it) and can be rewritten alone later
- **Empty (erased `0xff` / unused `0x00`) is "no assets", not a failure.** The firmware
  prints `wasmicon: assets empty, no assets` and runs; `open` answers `not-found`. A
  directory that was written with no entries prints `wasmicon: assets none`
- **Two CRCs, two moments.** The table's CRC is checked at boot; one bad entry discards
  the whole directory (`wasmicon: <reason>, no assets`). A body's CRC is checked once,
  at the guest's first successful `open` of it, and the verdict is kept for the run: a
  mismatch makes that `open` fail with `io`. Checking every body at boot would delay
  every boot by the size of the slot
- At boot the firmware prints the directory: `wasmicon: assets doom1.wad=4196020
  font.psf=3092` (names and lengths). The identity line (§3.8) carries
  `assets=v1/<slot size>`

---

## 4. The `wasmicon` CLI

### 4.1 Location and style

`tools/wasmicon-cli` (root workspace, bin name `wasmicon`). Dependency crates are
allowed (precedent: anyhow / wit-parser in `wasmicon-gen`). **"Zero dependencies, no
`alloc`" is a constraint on `runtime/` only**, not on the CLI. Argument parsing
follows `wasmicon-gen`: hand-written plus a `USAGE` constant. The flashers (picotool /
espflash / probe-rs) are **invoked, never reimplemented**.

### 4.2 Commands

**How far each is implemented is owned by docs/TODO.md §5** (not written here;
see AGENTS.md: remaining work lives in docs/TODO.md only). "Replaces" names the
existing thing that does the job today.

| Command | What it does | Replaces |
|---|---|---|
| `new <name> --lang rust\|as` | Scaffold. Fills in the ABI-conformant build flags (§4.4) | Nothing |
| ~~`build`~~ | **Not built.** It would be a thin wrapper that picks `cargo build --release` / `npm run build` by language; the flags live in the files `new` wrote (§4.4), so there is nothing to add | - |
| `check <app.wasm> --board X` | Runs decode / validate / `Exec` / instantiate **on the real runtime**, **with that board's actual arena size** (§4.3) | Nothing (`wasm-tools validate` sees neither the import table nor the board limits) |
| `run <app.wasm> [--trace] [--i2c-replay f] [--asset n=f]` | Runs on the host port. `--i2c-replay` corresponds to the existing env var `WASMICON_I2C_REPLAY`. The `[assets]` table of `wasmicon.toml` is served from its files; `--asset` adds or replaces one (§3.10) | `cargo run -p wasmicon-host` (the crate stays, used as a lib) |
| `deploy <app.wasm>` | Sends the app over USB / HTTP. `--persist` for the flash slot. Never touches the assets slot | Nothing |
| `assets pack [--board X] [-o f]` | Builds the assets slot image from `[assets]` (§3.10). With a board, checks that it fits the slot and prints the flash command | Nothing |
| `assets write --board X` | `assets pack`, then flashes the image to the board's assets slot through the same flasher as `deploy`. Per device, not per app: the last write wins | Nothing |
| `flash --board X [--fw v]` | Flashes **the firmware** (once). The version comes from the manifest (§3.8) | The written procedure (`apps/lcd-demo-rs/README.md`) |
| `fw list` | Lists the versions on hand and the version on the device (§3.8) | Nothing |
| `pack <app.wasm>` | Builds the slot image (§3.4). In stage 1, the input for an external flasher | Nothing |
| `config show` / `set` / `apply` / `reset` | View / write / push from `wasmicon.toml` / erase the role map (§3.9 / §4.7) | Nothing (today: reflash the firmware) |
| `monitor` | Opens the serial port and captures the trace. Enumerates `/dev/cu.usb*` and **records the identity at the head** (§3.8) | `cat /dev/cu.usbmodemXXXX \| tee` |
| `trace diff a.log b.log` | Normalizes and cross-checks. **Prints the first mismatching line** (traces are append-only, so that is closest to the cause) | `verify/diff-traces.sh` (**kept**; a test cross-checks that the CLI gives the same verdict) |
| `size [target]` | Core code size | `tools/measure-size.sh` |
| `doctor` | Toolchain check | Nothing |

`monitor` **does not assume `usbserial`**. The bridge on the ESP32-S3 at hand is a
CH343 and shows up as `/dev/cu.usbmodem*` (docs/TODO.md §1.1).

### 4.3 What `check` looks at

**The point is checking on the real runtime.** Stronger than
`wasm-tools validate --features=...`:

- **Exact match of import names and signatures** (docs/abi-spec.md §6.4, §7). The
  table is `runtime/src/generated.rs` (= derived from `wit/`)
- The `run` and `memory` exports (docs/abi-spec.md §3.3, §6.2). **`run` is checked
  down to its type** (anything but `func()` fails at run time with `wrong arity`, so
  checking only for the export's presence lets it through). A missing memory
  **definition** fails too (§6.2 requires one)
- **validate with that board's `Config`** (`max_memory_pages` and so on)
- **Whether the interfaces the app imports are implemented on that port** (§3.8).
  `ports/rp2040` returns `unsupported` for SPI / I2C, so without this the static
  check passes and the failure appears only on hardware

**What it cannot do: enumerate role names.** The argument of `pin-by-role` is a
`string` passed at run time as `(ptr, len)` (`wit/board.wit`); the vocabulary is not
in the WIT types. So "which roles does this app look up" **cannot be enumerated
reliably from the `.wasm`**. `check` only looks for known names in the bytes, which is
**informational**, not a guarantee. Two measured limits:

- **It also picks up names inside log strings.** `sensor-display` does not use `led`,
  yet `led` shows up in `roles` (it is inside `sensor crc failed`)
- **It does not work on AssemblyScript guests.** AS stores string literals as UTF-16
  and converts to UTF-8 at the call, so the UTF-8 role name never appears in the
  binary (`sensor_display_as.wasm` reports "not found"). What is certain is only
  known at run time, where a `pin-by-role` that returned `unsupported` leaves a line
  in the trace (§3.1).
**With `requirements.pin-roles` in `wasmicon.toml` (§4.7), the enumeration becomes a
declaration and turns into a guarantee.** It drops back to informational only when a
bare `.wasm` arrives from outside a project.

**The check is assembled on the CLI side.** `wasmicon_host::run_wasm_opts` calls
all the way to `run`, so it cannot be used for a check. The CLI wires
`wasmicon-core` + `wasmicon_port::Hal` + `wasmicon_host::hal::HostBoard` directly
and lines up decode -> validate -> `Exec` -> instantiate (the host port is not
fattened for the CLI). `instantiate` does not execute the guest, so the result is the
same whatever the board implementation.

**`Config` alone is not enough.** Linear memory takes all of the remaining arena
(`Arena::alloc_rest`), so even within `max_memory_pages` it can happen that
"`min_pages * 64 KiB` does not fit in what is left after decode / validate / `Exec`
took theirs". On hardware, `instantiate` fails right there. That is why `Profile`
carries the actual `arena` / `scratch` sizes and `check` uses them (`ports/rp2040`
has only 160 KiB; with 2 pages only about 20 KiB is left). **The port's
`static ARENA` uses the same value**, so it is not split across two places.

The failing stage (decode / validate / instantiate) is **reported distinctly**. Arena
shortage shows up in instantiate; saying "validate failed" would steer people toward
shrinking `max_memory_pages` (the right fix is a bigger arena).

**Host execution is looser than every board.** `ports/host` runs with
`Config::default()` (`max_memory_pages` = 65536), while hardware is RP2040 = 2 /
RP2350 = 4 / ESP32-S3 = 4 / ESP32-S3 with PSRAM = 112. **You can write an app that passes `wasmicon run` and
fails validate on hardware.** That is why `check --board` exists.

**Board profiles were gathered into `profile` in `ports/common`** (2026-10-04). The
scattered `Config` (three `main.rs`), the role assignments (`ROLES` in each
`board.rs`) and the implemented interfaces are now in one place. Each port pulls from
there, so **the `cargo build` in the CI rp2040 / rp2350 jobs checks that the values
agree**. The values were locked in to the numbers from before the move, in
`ports/common/tests/profiles.rs`.

```
profile::RP2040   2 pages / I2C and SPI not implemented / GPIO constraints (reserved and bus pins)
profile::RP2350   4 pages / everything implemented   / same constraints (same header)
profile::ESP32S3  4 pages / everything implemented   / Freenove board constraints
profile::HOST     Config::DEFAULT (loose) / the mock role table (host only)
```

`Config::DEFAULT` is a const added to `runtime` for this, and `Default` returns it
(written twice, it would inevitably diverge).

**Real boards have no role table** (2026-10-07; §3.9). What the profile holds is the
GPIO constraints used to check the wiring table (`gpio_count` / `reserved` /
`bus_pins`); the CLI checks the wiring table in `wasmicon.toml` against them, and the
firmware runs the same check again at boot. (The fixed vocabulary `ROLE_NAMES` and
`assert_role_names`, which checked it at compile time, were removed.)

### 4.4 A single source of truth for build flags (silent breakage when it rots)

The app's contract is embodied in these two:

- `apps/.cargo/config.toml` - `-Ctarget-feature=-reference-types` /
  `-Clink-arg=--initial-memory=65536` / `-Clink-arg=-zstack-size=8192`
- `apps/*/asconfig.json` - `runtime: stub` / `disable: [..., reference-types]` /
  `enable: [sign-extension, nontrapping-f2i, bulk-memory, mutable-globals]`

If the `wasmicon new` scaffold copied these, there would be **three copies**. When
they diverge, things break silently: `reference-types` slipping in gets rejected at
the `call_indirect` table index (the note in docs/abi-spec.md §6.1), and a larger
`--initial-memory` exceeds RP2040's 2 pages.

**Done the same way as `wasmicon-gen --check`** (implemented 2026-10-04). The source
of truth is **the file contents themselves** held in `tools/wasmicon-cli/src/flags.rs`
(not a table of values), and `wasmicon new` only writes them out. Agreement with the
real files in `apps/` is checked **byte for byte** by
`tools/wasmicon-cli/tests/flags.rs`.

- **A one-character difference in a comment fails it.** Loosening that goes back to
  "the values agree but nobody knows which is the truth"
- The check rides on the `cargo test` already in CI; no new job
- `[profile.release]` is not included. If it diverges the app only **gets bigger**; it
  does not break silently
- It also checks that **the Rust and AS initial memory agree at page granularity** and
  **fits the tightest board (2 pages on `ports/rp2040`)** (taken from
  `profile::PROFILES`)

### 4.5 Firmware distribution

**App authors do not build firmware.** Otherwise flashing an ESP32-S3 alone requires
espup and the Xtensa toolchain (`ports/esp32s3/build.sh`).

- Firmware is a release deliverable (`wasmicon-rp2350.uf2` / `wasmicon-esp32s3.bin`).
  `wasmicon flash --board X` only flashes it
- `doctor` requires only rustc + the wasm32 target (or `asc`) and a flasher
- The deliverables come with a **manifest** (board -> file + sha256 + the five axes of
  §3.8). `flash --fw` and `fw list` read it
- **CI does not build ESP32-S3 today** (because of Xtensa; see the comment in
  `ci.yml`). Producing release deliverables needs a job that installs espup -> §5-7

---

### 4.6 End to end (CLI usage example)

**This is the picture once the whole design is in.** Which command appears at which
stage is in the table at the end. The output shows the format; versions and SHAs are
not real. **The app size, import layout, trace lines and counts (1,214 lines /
277 `spi.write` calls) are real, though**, measured by running
`apps/sensor-display-rs` on the host port.

The task: "run an app that reads the SHT4x and draws on the ILI9341 on both the
Pico 2 W and the ESP32-S3, and cross-check the traces". Exactly the next step in
docs/TODO.md §1.3.

#### 0. Check the tools

```
$ wasmicon doctor
ok  rustc     1.97.1 (8bab26f4f 2026-07-14) + wasm32-unknown-unknown
ok  asc       Version 0.28.20
ok  picotool  v2.3.1 (Darwin, ...)
ok  espflash  4.5.0
--  probe-rs  not found (optional; for a debugger)

-> boards you can flash firmware to: rp2040 / rp2350 / esp32s3
   Rust apps can be built
   AssemblyScript apps can be built
```

Firmware is a release deliverable, so **espup and the Xtensa toolchain are not
needed** (§4.5).

#### 1. Flash the firmware once

```
$ wasmicon fw list
local    rp2350  0.1.0  (g6b1a2c3)  sha256 9c1f...
         esp32s3 0.1.0  (g6b1a2c3)  sha256 4ab7...
device   not connected

$ wasmicon flash --board rp2350
Hold BOOTSEL while plugging in USB, then press Enter...
Wrote wasmicon-rp2350 0.1.0 (g6b1a2c3) to RPI-RP2
(right after flashing the slot is empty: the UART prints "slot empty" and the LED blinks)

$ wasmicon flash --board esp32s3
Wrote wasmicon-esp32s3 0.1.0 (g6b1a2c3) to /dev/cu.usbmodem1101 (CH343)
```

**From here on, the firmware is not touched.**

#### 2. Let the board state who it is (§3.8)

```
$ wasmicon info
board      rp2350 (Pico 2 W)
firmware   0.1.0 (g6b1a2c3, clean)
abi        wasmicon:hal@0.1.0
protocol   1
slot       65536 B / format 1 / empty
config     memory 4 pages, call depth 32, table 256 elems
interfaces gpio, i2c, spi, time, log, board
features   trace on
roles      led, lcd-cs, lcd-dc, lcd-rst
```

#### 3. Create an app

```
$ wasmicon new room-monitor --lang rust --board rp2350
room-monitor/
  wasmicon.toml        [requirements] pin-roles is empty (add the roles you use)
  Cargo.toml           standalone workspace + cdylib + profile.release
  .cargo/config.toml   -reference-types / --initial-memory=65536 / -zstack-size=8192
  rust-toolchain.toml
  src/lib.rs           run() scaffold (uses only log; role usage is in a comment)
  .gitignore

Next:
    cd room-monitor && cargo build --release
    wasmicon check <the .wasm it produced> --board <board>
```

The `.cargo/config.toml` is **byte-identical** to `apps/.cargo/config.toml`;
`cargo test` checks that (§4.4). With `--lang as` you get `asconfig.json`,
`package.json` and `assembly/index.ts`.

**The `wasmicon-hal` dependency is written as a path.** It is not published to
crates.io / npm (see "distributing `new` and the bindings" in docs/TODO.md §5), so
pass `--hal <where the bindings are>` or, by default, **it searches upward for
`bindings/`**. If none is found it writes a version requirement and **says so** (not
pre-empting the distribution decision).

`Cargo.toml` contains `[workspace]` so that cargo does not stop with "not in the
workspace" when the project sits inside an existing workspace. In that position the
`rustflags` from `.cargo/config.toml` are **concatenated** and the same value applies
twice; `new` says that too (it is harmless).

#### 4. The inner loop - no hardware (host, 1-2 seconds)

**`wasmicon build` is not built.** It would be a thin wrapper that picks `cargo` /
`npm` by language, and the build flags are already in `.cargo/config.toml` and
`asconfig.json` (§4.4), so there is nothing to add.

```
$ cargo build --release
    Finished `release` profile [optimized] target(s) in 0.4s

$ wasmicon check target/wasm32-unknown-unknown/release/room_monitor.wasm --board rp2350
room_monitor.wasm  4,549 B

abi      wasmicon:hal@0.1.0                          enforced by the import names
imports  13, all match                               (board 1 / gpio 3 / i2c 4 / log 1 / spi 3 / time 1)
exports  run, memory                                 present
memory   initial 1 page
table    none

[rp2350]
validate initial 1 page <= 4                         ok
roles    led, lcd-cs, lcd-dc, lcd-rst                present on rp2350 (informational)
-> pass

$ wasmicon run target/wasm32-unknown-unknown/release/room_monitor.wasm --trace --i2c-replay sht4x-replay.txt > host.log
$ head -6 host.log
> wasmicon:hal/log@0.1.0/log(2, "sensor-display start")
<
> wasmicon:hal/board@0.1.0/pin-by-role("lcd-cs")
< 0 [role:lcd-cs]
> wasmicon:hal/gpio@0.1.0/[static]pin.open(role:lcd-cs, 3)
< 0 [1]
```

`check` gives a different result for a different board. **Host execution cannot tell
you this** (§4.3). `ports/rp2040` has no SPI / I2C (`spi_open` / `i2c_open` return
`unsupported`; docs/TODO.md §1.2), so **a static check alone would pass**:

```
$ wasmicon check target/wasm32-unknown-unknown/release/room_monitor.wasm --board rp2040
[rp2040]
validate initial 1 page <= 2                         ok
i2c      not implemented on this port (unsupported on hardware)  <- fails
spi      not implemented on this port (unsupported on hardware)  <- fails
roles    lcd-cs, lcd-dc, lcd-rst                     in the rp2040 wiring table
-> fail (2 on this board)
```

The `roles` line checks that `requirements.pin-roles` in `wasmicon.toml` (the
declaration) agrees with that board's wiring table (`[board.rp2040.roles]`) (§4.7).
Without a declaration, roles are not cross-checked (they cannot be enumerated
reliably from the `.wasm`; §4.3).

**This can be said only because the board profile holds "implemented interfaces"**
(§3.8). Without it, `check` passes and you learn on hardware when `i2c.bus.open`
returns `unsupported`. The initial pages fit and the wiring table agrees, so **nothing
else fails.**

#### 5. The outer loop - send to hardware (seconds)

```
$ wasmicon deploy target/wasm32-unknown-unknown/release/room_monitor.wasm --monitor > pico.log
cross-checked against rp2350 0.1.0 (g6b1a2c3)
  abi ok / 1 page <= 4 / roles ok / 4,549 B <= 65536 B / slot format 1
sent 4,549 B to the RAM slot (crc32 1f3a9c21)
run -> capturing the trace from UART0 115200 (Ctrl-C to stop)
```

**Control over USB, the trace over UART0** (§3.5), so the CLI holds both. Without
`--monitor` it only sends and runs. The head of the log carries the identity:

```
$ head -2 pico.log
# wasmicon rp2350 0.1.0 (g6b1a2c3) abi=wasmicon:hal@0.1.0 trace=on app=1f3a9c21
> wasmicon:hal/log@0.1.0/log(2, "sensor-display start")
```

#### 6. Cross-check on two boards (the point of this project)

Swap in the ESP32-S3 and send **the same `.wasm`**:

```
$ wasmicon deploy target/wasm32-unknown-unknown/release/room_monitor.wasm --board esp32s3 --monitor > esp32s3.log
$ wasmicon trace diff pico.log esp32s3.log
app       1f3a9c21 / 1f3a9c21                     same binary
identity  rp2350 0.1.0 (g6b1a2c3) / esp32s3 0.1.0 (g6b1a2c3)
1,214 lines excluding time                        match
277 spi.write CRC-32 values                       match
-> match
```

If the firmware versions differ, it warns without failing (§3.8):

```
identity  rp2350 0.1.0 (g6b1a2c3) / esp32s3 0.1.0 (g7f09de1)   versions differ
```

#### 7. Persist (runs at power-on)

```
$ wasmicon deploy target/wasm32-unknown-unknown/release/room_monitor.wasm --persist
wrote 4,549 B to the flash slot at 0x1010_0000 (crc32 1f3a9c21, format 1)
runs from the next power-on
```

#### The two loops

| Loop | What it cycles | How often |
|---|---|---|
| Inner | `cargo build` -> `check` -> `run` (host, mock HAL and replay) | Constantly |
| Outer | `deploy --monitor` -> `trace diff` | Once the inner loop passes |
| Firmware | `flash` | Rarely (only on a version bump) |

#### Which command at which stage (§5)

| Stage | What becomes available |
|---|---|
| 0 | `doctor` / `new` / `check` / `run` / `trace diff` / `monitor` (without identity) |
| 1 | `flash` (firmware) / `deploy --persist` (through the existing flasher; **the Pico still needs BOOTSEL**) |
| 2 | `info` / `deploy` (USB, no button) / `deploy --monitor` / identity in `monitor` / `fw list` |
| 3 | `deploy --via http` (ESP32-S3) |

---

### 4.7 `wasmicon.toml`

One per project. **What goes here is "properties of the app" and "wiring".**
`deploy` writes the wiring table to the config slot, and the firmware answers roles
from that table alone (§3.9).

```toml
# wasmicon.toml - one per project
version = 1

[requirements]
# The role names this app looks up with `board.pin-by-role` (any names). Numbers go below
pin-roles = ["lcd-cs", "lcd-dc", "lcd-rst"]

[defaults]
board = "rp2350"
i2c-replay = "fixtures/sht4x.txt"

# Wiring table per board. deploy writes it to the config slot (§3.9)
[board.rp2350.roles]
lcd-cs = 17
lcd-dc = 20
lcd-rst = 21

[board.esp32s3.roles]
lcd-cs = 10
lcd-dc = 14
lcd-rst = 15

# Read-only data the app opens through assets.blob (§3.10). Asset name -> file.
# run serves the files; assets write puts them in the device's assets slot
[assets]
"doom1.wad" = "wad/doom1.wad"
"font.psf" = "assets/terminus-12.psf"
```

#### Sources of truth

| Information | Source of truth | Role of the toml |
|---|---|---|
| Required role names (`requirements.pin-roles`) | **The toml.** A property of the app | The source itself |
| Role -> GPIO | **The toml** (`deploy` writes it to the config slot; §3.9) | The source itself. The device only reads it |
| Asset name -> file (`[assets]`) | **The toml** (`assets write` writes the slot from it; §3.10). What the app can open on a device is exactly this table, so there is no separate declaration | The source itself. `check` reports a missing file and a slot it does not fit |
| ABI-conformant build flags | **`.cargo/config.toml` / `asconfig.json`** | **Not allowed here** (below) |

#### Cross-checking `requirements.pin-roles` against the wiring table

As in §4.3, role names are passed as strings at run time, so they cannot be
enumerated reliably from the `.wasm`. With a declaration the enumeration is settled,
so **before sending** we can say "this board's wiring table has no `lcd-rst`, so this
app will not work".

**A second item of the same kind is also undeclared.** The `index` of `i2c.bus.open` / `spi.bus.open` is also a run-time integer
argument that cannot be enumerated statically from the `.wasm` (send an app that
opens index 1 to a board with one bus and you get `unsupported` on hardware). v0.1
does not write it, but when needed it is just `i2c-buses = [0]` under
`[requirements]`. **Interface implementation status is visible from the imports**, so
it needs no declaration (§4.3).

The key is named `requirements.pin-roles`. `requires` does not say what is required,
and a flat key like `required-pin-roles` gets longer with every item.

**A typo stops you without a device connected.** There is no vocabulary, so only the
name format is checked, but a typo in either the declaration (`pin-roles`) or the
wiring table makes the two disagree, and `check` / `deploy` say "not in the wiring
table".

#### Schema rules

| Rule | Content |
|---|---|
| **Do not write what can be derived** | The app name and language come from `package.name` in `Cargo.toml` / the presence of `asconfig.json`. The toml only overrides. Same reason as §4.4: two copies always drift |
| **One per app** | Even in a workspace, **one per app**. This repository's `apps/` has nine. Matches the one that `wasmicon new` writes |
| **Required only** | `pin-roles` lists only what the app "cannot run without". Degradation such as "works without `led`" is not expressed in v0.1 (`optional-pin-roles` waits for a real case) |
| **"This role does not exist" is `"none"`** | `lcd-rst = "none"`. Same as omitting the key: not handed out (the firmware has no default table). Harder to misread than `false` or `0` |
| **Paths are relative to the toml's directory** | `i2c-replay = "fixtures/sht4x.txt"`. Relative to the CLI's cwd, it would break depending on where you call from |
| **Unknown keys are errors** | Silently ignoring them hides "I set it but it has no effect". Typos stop here |
| **`version`** | Newer than the CLI supports: error. Older: accepted |
| **The toml is optional (but required to use roles on hardware)** | `check` / `run` **work without a toml**. Real boards have no default table, though, so an app `deploy`ed without a toml gets no roles at all |
| **One wiring table per circuit board** | A firmware variant of a board reads that board's table: `--board esp32s3-psram` uses `[board.esp32s3.roles]` (below). A `[board.esp32s3-psram]` table is an error, since it would never be read |

- **`check` / `deploy` inside a project** -> declaration and wiring table are cross-checked
- **A bare `.wasm` received on its own** -> roles are not cross-checked (they cannot be enumerated)

#### One board, two firmwares: `esp32s3` and `esp32s3-psram`

The Freenove ESP32-S3-WROOM board carries 8 MiB of octal PSRAM
(docs/verification-report.md §21). `ports/esp32s3` builds two firmwares for it, and
each has its own profile, because `check --board` needs a static page limit and a
module without PSRAM has nothing to map:

| `--board` | Firmware | Arena | Page limit |
|---|---|---|---|
| `esp32s3` | `build.sh build --release` | 300 KiB of SRAM | 4 |
| `esp32s3-psram` | `build.sh build --release --features psram` | 8 MiB of PSRAM | 112 (provisional; docs/TODO.md §5-10) |

`esp32s3-psram` also allows deeper code than the other MCUs: control depth 256
(64 elsewhere), call depth 128 (32) and 4096 operand stack slots (1024), for compiled C
of the size the profile exists for (docs/TODO.md §5-10).

Everything else is the board's: pins, reserved pins, bus pins, the app, config and
assets slots, and the wiring table (`Profile::wiring` names `esp32s3`).
`ports/common/tests/profiles.rs` fails if the two profiles ever disagree on a pin or
a slot. So one `.wasm` and one `wasmicon.toml` serve both:

- `check` without `--board` prints one row more. An app meant for both stays within
  the 4 pages of `esp32s3`; a larger one passes only `esp32s3-psram`
- `deploy` takes the board from `--board` (or `[defaults] board`, which names one).
  It does not ask the device which firmware it runs yet (docs/TODO.md §5): a large app
  deployed with `--board esp32s3-psram` to the plain firmware passes `check` and fails
  in `instantiate` on the board. The identity line (`wasmicon id esp32s3-psram ...
  pages=112`) says which firmware is there
- The PSRAM firmware prints `psram missing or smaller than the arena, halt` after the
  identity line on a module without PSRAM

`wasmicon.toml` is **searched for upward from the current directory** (like cargo).
If the declaration and the wiring table disagree, **it stops before flashing, with no
device connected**:

```
$ wasmicon check app.wasm --board rp2350
...
roles    lcd-cs, lcd-dc, lcd-rst    not in the rp2350 wiring table: lcd-rst  <- fails (add it to [board.rp2350.roles])
```

Only when it needs to travel with the binary does a custom section
(`wasmicon.roles`) or bundling with `wasmicon pack` come up. The runtime skips custom
sections, so execution is unaffected (`runtime/src/decode.rs:79`), but **the `.wasm`
bytes change**, so the recorded hash (`fc470947...`) would have to be retaken. Not
until it is needed.

#### `deploy` writes the wiring table every time and shows it before writing

> The policy changed on 2026-10-07. Previously, "cross-check the device's table
> against the toml, stop on mismatch, and push explicitly with `config apply`";
> now **the firmware has no default table and the toml is the table's truth**, so
> `deploy` writes it every time (§3.9).

```
$ wasmicon deploy app.wasm --board rp2350
app.bin -> rp2350 slot (+0x100000)  4565 B (wasm 4549 B, crc32 c1cbe3c1)
  wiring: lcd-cs=17 lcd-dc=20 lcd-rst=21
```

**Rewriting the wiring table changes which pins are actually driven**, which can be
physically dangerous depending on what is connected; that concern remains. Current
handling:

- the table comes **from the app's own `wasmicon.toml`** (nobody else's configuration gets mixed in)
- `deploy` **shows the table on one line before writing**
- reserved pins, bus pins and duplicates are rejected by both the CLI and the firmware
- a corrupt table fails safe: **no pins are driven**

Showing a diff against the table currently on the device and asking for confirmation
(the earlier "cross-check") needs a way to read the table back from the device
(`info` over the stage 2 USB control channel). Until then, the display above stands in.

#### Not allowed in the toml

- **ABI-conformant build flags.** §4.4 called `apps/.cargo/config.toml`,
  `asconfig.json` and the scaffold "three copies that break silently when they rot";
  this would make it **four**. If `initial-memory` could go in the toml, you could
  build "an app that fails validate on RP2040" with the cause spread out
- **Machine-specific values** (`/dev/cu.usbmodem1101` and the like). The name changes
  with the bridge chip (the CH343 in docs/TODO.md §1.1), so committing it breaks on
  someone else's machine

```
wasmicon.toml        committed to git
wasmicon.local.toml  gitignored. Machine-specific overrides such as the serial port
```

Precedence is **CLI flags > `local.toml` > `wasmicon.toml` > device defaults**.
`info` always returns the device's effective values; the toml does not override them
(§3.9).

`wasmicon new` writes this file as part of the scaffold too. The CLI gains one toml
parser dependency (the same scope as `anyhow` / `wit-parser` in `wasmicon-gen`; §4.1).

---

## 5. Stages

**Stages 0 and 1 reach "no firmware rebuild needed".** The USB stack and Wi-Fi come
after.

| Stage | Content | Firmware change | Hardware |
|---|---|---|---|
| 0 | CLI skeleton + `new` / `check` / `run` / `monitor` / `trace diff` / `doctor` | **None** | Not needed |
| 1 | The §3.2 cleanup -> slot format -> a firmware that "reads the slot and runs" -> **removal of the built-in app** (§3.3) -> `deploy` through the existing flasher | The core of the loader | Needed |
| 2 | USB control channel + the §3.6 protocol. `info`, `probe` and the role map configuration (§3.9) arrive | USB stack | Needed |
| 3 | HTTP (ESP32-S3) | Wi-Fi + the RAM budget decision | Needed |

- **Stage 0 helps the SHT40 work in docs/TODO.md §1.3 directly** (capture and cross-check)
- **At stage 1 the ESP32-S3 already needs no button** (espflash resets through
  DTR/RTS). The Pico still needs BOOTSEL; that goes away at stage 2
- **Distributing the bindings** (crates.io / npm) is independent of every stage and is
  not decided here. `new` landed in stage 0 but writes the dependency as a path
  (§4.6, step 3)

---

## 6. What does not change / what breaks if touched

- **The ABI**: import names, signatures, error codes (discriminant + 1), handle 0 =
  invalid (docs/decisions.md D-02, D-03, D-05). If it becomes necessary, do not
  implement; ask the owner
- **`wit/` is the source of truth**; generated files are generator output only (`AGENTS.md`)
- **The recorded `.wasm` hashes and traces** (docs/verification-report.md §6, §7;
  docs/TODO.md §3). The `new` scaffold carries the same flags as `apps/`, so it
  produces the same bytes (§4.4). The loader adds no trace lines
  (§3.7)
- ~~The built-in app and the `guest-lcd-demo` feature are not touched until stage 1.~~
  -> removed on 2026-10-07 once the three ports could read the slot, and CI was
  fixed at the same time (§3.3)
- **The Phase 4 / 5 / 6 completion criteria** (docs/TODO.md §1.3; defined over the two
  boards ESP32-S3 and Pico WH)
- **No automatic re-run of the same app after a trap** (docs/decisions.md A-04). The
  loader only adds "accept a new app" (§3.1)

---

## 7. Open questions

**Collected in docs/TODO.md §5.** Decisions needed: settling the return to the parked state after
a trap, the ESP32-S3 RAM budget (lower `max_memory_pages` or PSRAM), the HTTP
direction, Pico W Wi-Fi (the embassy move), the board variant (flash size and whether
PSRAM is present), and how firmware versions are assigned.
