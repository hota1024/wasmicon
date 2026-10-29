# Firmware manager design (`wasmicon fw`)

Created: 2026-10-29
Status: **proposal; waiting for the owner** (docs/TODO.md §7-1 ... §7-10). Nothing here is
implemented. The remaining work and the stages live in docs/TODO.md §7 only. If §7-1 is
adopted, docs/app-workflow.md changes in the places listed in §11.
Related: docs/app-workflow.md §3.8, §4.3, §4.5, docs/device-layer.md §5.1,
docs/verification-report.md §10, docs/TODO.md §5-7, §5-8

---

## 1. Why

Today every user builds the firmware by hand in each port's workspace
(`cargo build --release`, or `ports/esp32s3/build.sh` with espup for the Xtensa
target). The CLI writes the app, config and assets slots, but it neither builds nor
flashes firmware. docs/app-workflow.md §4.5 already says that app authors should not
build firmware and that firmware is a release deliverable with a manifest, but no
part of that exists.

Two newer needs make it larger than a download:

- **Backends.** The device layer puts device drivers in the firmware
  (docs/device-layer.md §5.3). Every backend of a port is compiled in, whether an
  app uses it or not, because the backend is chosen at run time from the config
  slot. That costs only flash today (one or two display backends), but it grows with
  each device interface. Users who need a smaller firmware, or a backend that is
  still experimental, need their own build.
- **Versions.** A project that compares traces across boards (docs/decisions.md D-10)
  needs the same firmware on every board, and a way to go back to an older one.

The proposal: a firmware manager in the CLI, in the style of `mise` or `nvm`.
Common firmware is downloaded from GitHub Releases; a firmware with a chosen set of
backends is built locally and managed the same way.

## 2. What differs from a toolchain manager

`nvm use` changes which binary the shell runs. Here **the active firmware is the
one in the board's flash**, not anything on the host. So:

- The source of truth for "what runs" is the board, and reading it is a problem of
  its own (§6).
- Switching a version means flashing, which takes time and can break another
  project's setup on the same board. The CLI never flashes firmware as a side
  effect; only `fw flash` does.
- Several boards can be connected, and the flashers address them differently: an
  ESP chip by its serial port (`espflash --port`), a Pico in BOOTSEL by its USB
  serial number (`picotool --ser`). `deploy` today passes no selector to `picotool`
  at all, so with two Picos in BOOTSEL it cannot say which one it writes. `fw flash`
  needs a selector for both.

## 3. Compatibility and identity are separate questions

docs/app-workflow.md §3.8 says compatibility is not one version number: the ABI, the
board profile, the role names, the protocol and the slot format are separate axes,
and the firmware version is provenance only. **This design keeps that.**

- **Can this app run on this firmware?** Judged per axis, by `check` and `deploy`,
  as today. The version is never an input to this question.
- **Is this the firmware the project asked for?** Asked only when the project pins a
  build (§5). It compares build IDs (§4), not versions, and it is a reproducibility
  guard the project opted into, not a compatibility verdict. A pin never makes an
  incompatible firmware pass the axes, and an unpinned project is never stopped by a
  version.

What changes is where `check` reads the axes from (§7), and the sentence of
docs/app-workflow.md §3.8 that says no firmware package manager is needed. The
reasoning behind that sentence stays; the manager adds installation and pinning on
top (docs/TODO.md §7-1).

## 4. Build ID

The version alone does not name a firmware. docs/app-workflow.md §3.8 already keys
the manifest and the cache by **board x version x variant**, where the variant is
`trace` on or off. Local builds add the backend set and the source. A **build ID**
names all of it:

| Part | Release | Local build |
|---|---|---|
| Board profile | `esp32s3`, `esp32s3-psram`, ... | the same |
| Version | the release tag, `0.3.1` | the version of the source, `0.3.1` |
| Variant | `trace` or `notrace` | the same |
| Backends | every backend of the port | the chosen set |
| Source | the tag | a commit; a dirty tree has no build ID (§8) |

- **Written form.** `0.3.1/esp32s3/trace` for a release; `0.3.1+local.<hash>/esp32s3/trace`
  for a local build, where `<hash>` covers the commit and the backend set.
- **The firmware reports its build name.** Today `fw=` is the port's
  `CARGO_PKG_VERSION`, so a local build of 0.3.1 also says `fw=0.3.1`. The build
  name (`0.3.1` or `0.3.1+local.<hash>`) has to be embedded at build time, from an
  environment variable that the release job and `fw build` set, with
  `CARGO_PKG_VERSION` as the default for a hand build. Board and variant are already
  in the identity line.
- **One version for every port** (docs/TODO.md §5-8). Each port is its own workspace
  (docs/decisions.md A-09) and carries its own `version = "0.1.0"`. The release job
  has to fail when any port's version differs from the tag; nothing enforces it
  today (docs/TODO.md §7-3).

## 5. Pinning a build in a project

```toml
[firmware]
version = "0.3.1"
variant = "trace"     # default
```

- Optional. Without it, `deploy` behaves as today.
- The board profile is not written here; it is the board `deploy` targets.
- With a pin, `deploy` reads the board's build ID (§6) before writing anything. If
  it differs, `deploy` stops and names `wasmicon fw flash` as the fix
  (docs/TODO.md §7-5 asks whether to stop or to warn).
- **Pins name releases.** A local build exists only on the machine that built it, so
  a pin on it cannot be reproduced elsewhere. Whether a project may pin a local build,
  and how a teammate gets the same firmware, is docs/TODO.md §7-4.
- A global default (`~/.wasmicon/config.toml`) is the build that `fw flash` uses
  outside a project.

## 6. Reading the board's build ID

`deploy` and `fw current` need the build ID of the firmware on the board. The
identity line cannot give it on every board today:

- It is printed once, at boot, on the trace UART. `info` over USB is stage 2
  (docs/app-workflow.md §3.8) and does not exist yet.
- On the Pico family the trace UART is UART0 behind a separate USB-serial adapter,
  and the firmware does not run while the board is in BOOTSEL for `picotool`.
- On the ESP32-S3 reading it means a reset, which runs the old app once.

The alternative is to read the firmware image in the flasher session that writes
the slot, with no firmware running:

- ESP32-S3, -C5, -P4: the ports embed `esp_app_desc!()`, whose version field can
  carry the build name; `espflash read-flash` reads it.
- Pico family: picotool binary info (`picotool info`), with the build name added as
  an entry.

Which source to use, and what `deploy` does on a board where it cannot be read, is
docs/TODO.md §7-6. Recommended: the flasher session, because it works in the state
`deploy` already puts the board in.

## 7. `check` reads the manifest, not a constant

`check --board` judges the board profile and the device backends against `Profile`
constants compiled into the CLI (`ports/common/src/profile.rs`). That holds while every
firmware of a board is built the same way. A local build without the `ili9341`
backend would pass `check` and fail on the board. The firmware cannot fix this by
itself either: `identity::describe` builds the identity line from the same constant.

So:

- **The manifest is the source.** It carries, per build ID, every axis of
  docs/app-workflow.md §3.8, the whole `Config` that docs/app-workflow.md §4.3 checks
  (memory pages, control, call and operand stack limits, arena, scratch), the slot
  formats, and the backends compiled in. The release job writes it; `fw build`
  writes the same for a local build.
- **The firmware's own profile follows its features.** `Interfaces::display_drivers`
  is derived from the backend features with `cfg(feature)`, so the profile in the
  firmware and the one in its manifest agree.
- **The identity line does not grow.** It names the build ID, and the CLI looks the
  manifest up by that. The line is assembled in a 256-byte buffer that truncates
  silently (`ports/common/src/identity.rs` says it stays under 232 bytes). The
  rp2350 example in that file is 206 bytes, and the `esp32s3-psram` line is longer;
  a tagged `git describe` and a `+local.<hash>` build name use most of the rest. A test should hold the longest
  possible line under the buffer (docs/TODO.md §7).
- **Unknown build ID** (a hand build, or a manifest not on this machine): `check`
  falls back to the `Profile` constant, as today, and says so.
- **The CLI and the firmware are separate versions.** CLI 0.4.0 can flash firmware
  0.3.1. Fetching the right tag (§8) does not keep them in step; the manifest's slot
  and config formats do. `deploy` refuses a firmware whose manifest does not read the
  formats this CLI writes.

## 8. Local builds and backend features

- **Source.** The CLI is installed without the repository, so `fw build` fetches the
  source at the release tag of the requested version. `--source <dir>` builds a
  working tree instead, for port developers.
- **Dirty trees.** A dirty tree has no revision to hash, so two different dirty builds
  would get the same name. `fw build --source` of a dirty tree can be flashed, but it
  gets no build ID, is not stored under `~/.wasmicon` and cannot be pinned. The
  identity line still says `-dirty`, as it does today.
- **Backend features.** Each port gets one feature per backend (`backend-ili9341`,
  `backend-tab5`), default on, next to `trace`. Cargo features are additive: a plain
  `--features` list cannot remove a default. So `fw build` always passes
  `--no-default-features` and lists what it keeps: the variant (`trace` or nothing) and
  the chosen backends.
- **Features that are not backends** stay out of `fw build --features`:
  - `psram` selects a board profile (`esp32s3-psram`, 112 pages). `fw build --board
    esp32s3-psram` sets it; the result is stored under that profile, never under
    `esp32s3`.
  - `psram-direct-read`, `read-stats` and `hw-probe` are diagnostic builds. They are
    not managed by `fw`.
- **Tools.** `doctor` reports the tools for local builds (the Rust targets, espup for
  the ESP32-S3) as optional; they are not needed to flash a release.

## 9. Release deliverables

- **One version for every port** (§4), one release with all boards.
- **One firmware per board profile and variant, with every backend.** A backend costs
  flash only, so the release does not split by backend set.
- **Built in CI from the tag**, which needs the espup job of docs/TODO.md §5-7 for the
  ESP32-S3.
- **The manifest** carries, per file: build ID, sha256, size, `git describe`, the axes
  and `Config` (§7), and the backends.
- **What the file holds decides whether the slots survive** (§10). For the ESP chips
  the deliverable must carry the bootloader and the partition table, or a new board
  does not boot, and must not pad to the end of flash (docs/TODO.md §7-8).

## 10. Flash layout constraints

- **The firmware must end below the app slot**, which starts at `0x100000` on the
  Pico family and on the ESP32-S3 (`ports/common/src/profile.rs`).
  - On the ESP chips the app is placed at `0x10000`, the start of the `factory`
    partition (docs/verification-report.md §10), so the app image may be at most
    960 KiB, not 1 MiB.
  - The rule is on the end offset of the image, not its size.
  - The Pico ports already refuse at boot when `__flash_binary_end` reaches the slot
    (`slot::SlotError::Overlap`), but by then the slot is overwritten. The release
    job and `fw build` have to refuse before flashing; no such check exists.
- **Flashing must not erase the slots.** On the ESP32-S3, `espflash flash` of the ELF
  erases only the app's sectors: a marker at `0x700000` survived it
  (docs/verification-report.md §10). That result does not cover a `.bin`: an image
  from `espflash save-image --merge` is padded with `0xFF` to the end of flash
  unless `--skip-padding` is given, and writing it erases the app, config and assets
  slots. The Pico family (UF2 and `picotool load`) has not been checked.
- **Going back across a config slot format.** An older firmware may not read a newer
  config slot (docs/device-layer.md §5.2 keeps version 1 for that reason).
  `fw flash` of an older build warns when the board's config slot is newer than that
  build's manifest reads.

## 11. Commands and layout

| Command | Does |
|---|---|
| `fw list` | The builds on this machine: build ID, origin (release or local) |
| `fw list --remote` | The releases on GitHub |
| `fw install <version> [--board <name>] [--variant <v>]` | Downloads release files and checks their sha256 against the manifest |
| `fw build <version> --board <name> [--backends <list>] [--source <dir>]` | Builds a local firmware (§8) |
| `fw flash [<build>] [--board <name>] [--port <path> \| --ser <serial>]` | Flashes a firmware. Without a build, the project's pin (§5) |
| `fw current [--port <path> \| --ser <serial>]` | The project's pin and the board's build ID (§6), side by side |
| `fw uninstall <build>` | Removes a build from this machine |

Installed firmware lives in the user's data directory, keyed by build ID
(docs/TODO.md §7-7):

```
~/.wasmicon/firmware/
  0.3.1/
    manifest.toml
    esp32s3/trace/wasmicon-esp32s3.<ext>
    esp32s3/notrace/wasmicon-esp32s3.<ext>
    rp2350/trace/wasmicon-rp2350.uf2
  0.3.1+local.5c1e0a/
    manifest.toml
    esp32s3/trace/wasmicon-esp32s3.<ext>
```

`<ext>` depends on §7-8 (an ELF for `espflash flash`, or a merged image).

**What adopting this changes in docs/app-workflow.md** (docs/TODO.md §7-1):

- §3.8: the sentence "What is needed is not a firmware package manager", and under
  Uses, `flash --fw` and `fw list` showing the version on the device (here that is
  `fw current`).
- §4.2: the `flash --board X [--fw v]` and `fw list` rows of the command table.
- §4.5: `wasmicon flash --board X` and `flash --fw`.
- §4.6: the example under "Flash the firmware once".
- §5: the stage table's `flash` rows and `fw list` in stage 2.
