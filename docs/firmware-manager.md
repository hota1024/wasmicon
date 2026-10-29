# Firmware manager design (`wasmicon fw`)

Created: 2026-10-29
Status: **proposal; waiting for the owner** (docs/TODO.md §7-1 ... §7-7). Nothing here is
implemented. It extends docs/app-workflow.md §4.5 and, if §7-1 is adopted, changes one
sentence of docs/app-workflow.md §3.8.
Related: docs/app-workflow.md §3.8, §4.5, §4.7, docs/device-layer.md §5.1, §6,
docs/TODO.md §5-7, §5-8

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

- The source of truth for "what runs" is the board's identity line
  (docs/app-workflow.md §3.8), not a file on the host.
- Switching a version means flashing, which takes time and can break another
  project's setup on the same board. The CLI never flashes firmware as a side
  effect; only `fw flash` does.
- Several boards can be connected. Each is addressed by its serial port, as
  `deploy` does today.

## 3. Compatibility stays per axis

docs/app-workflow.md §3.8 says compatibility is not one version number: the ABI, the
board profile, the role names, the protocol and the slot format are separate axes,
and the firmware version is provenance only. **This design keeps that.**

- `check` and `deploy` judge each axis, as today. The firmware version never
  decides whether an app can run.
- A version pin (§5) asks for an **exact** firmware, for reproducibility. There are
  no version ranges, because a range would claim a compatibility that only the axes
  can establish.

Two things change. `check` reads the axes from the firmware instead of a constant
(§6). And docs/app-workflow.md §3.8 says "What is needed is not a firmware package
manager"; the reasoning behind that sentence (judge per axis, do not trust one
number) stays, and the manager adds installation and pinning on top
(docs/TODO.md §7-1).

## 4. Commands and layout

| Command | Does |
|---|---|
| `fw list` | The installed firmware: version, boards, origin (release or local), features |
| `fw list --remote` | The versions on GitHub Releases |
| `fw install <version> [--board <name>]` | Downloads the deliverables and checks their sha256 against the manifest |
| `fw build <version> --board <name> --features <list>` | Builds a local firmware from the source of that release (§7) |
| `fw flash [<version>] [--board <name>] [--port <path>]` | Flashes a firmware. Without a version, the project's pin (§5) |
| `fw current [--port <path>]` | The project's pin and the identity of the connected board, side by side |
| `fw uninstall <version>` | Removes an installed version |

docs/app-workflow.md §4.5 names `flash --fw` and `fw list`. This design puts every
firmware command under `fw` (`fw flash` replaces `flash --fw`).

Installed firmware lives in the user's data directory, one directory per version and
board, next to its manifest:

```
~/.wasmicon/firmware/
  0.3.1/
    manifest.toml
    esp32s3/wasmicon-esp32s3.bin
    rp2350/wasmicon-rp2350.uf2
  0.3.1+local.5c1e0a/
    manifest.toml
    esp32s3/wasmicon-esp32s3.bin
```

## 5. Pinning a version in a project

```toml
[firmware]
version = "0.3.1"
```

- Optional. Without it, `deploy` behaves as today.
- With it, `deploy` reads the identity line first. If `fw=` differs, `deploy`
  stops before writing anything and names `wasmicon fw flash` as the fix
  (docs/TODO.md §7-5 asks whether to stop or to warn).
- A local build is pinned by its full name (`0.3.1+local.5c1e0a`), so a project that
  needs a custom firmware says so.
- A global default (`~/.wasmicon/config.toml`) is the version that `fw flash` uses
  outside a project.

## 6. `check` reads the firmware, not a constant

`check --board` judges the board profile and the device backends against
`Profile` constants compiled into the CLI (`ports/common/src/profile.rs`). That holds
while every firmware of a board is built the same way. With local builds
(§7) it does not: an `esp32s3` firmware without the `ili9341` backend would pass
`check` and fail on the board.

So the axes come from the firmware itself:

- The release manifest lists, per board, every axis of docs/app-workflow.md §3.8 and
  the device backends compiled in (`Interfaces::display_drivers`,
  docs/device-layer.md §5.1).
- A local build writes the same manifest.
- `check --board <name> --fw <version>` reads the manifest. `deploy` reads the
  identity line of the connected board, which already names the interfaces (`if=`)
  and the device package. It gains the backend list (docs/TODO.md §7-6).
- Without a manifest or a board, `check` falls back to the `Profile` constant, as
  today, and says so.

## 7. Local builds and backend features

- **Source.** The CLI is installed without the repository, so `fw build` fetches
  the source at the release tag of the requested version. Building from another
  revision would let the ABI or the config slot format drift from the CLI. A
  `--source <dir>` option builds a working tree instead, for port developers; its
  version carries `-dirty` when the tree is dirty, as the identity line does.
- **Backends become Cargo features** of each port, default on, so a plain build is
  the release build. `fw build --features` turns some of them off or adds
  experimental ones.
- **Name.** A local build is `<version>+local.<hash>`, where the hash covers the
  feature set and the source revision. The manifest records both.
- **Tools.** `doctor` reports the tools for local builds (the Rust targets, espup
  for the ESP32-S3) as optional; they are not needed to flash a release.

## 8. Release deliverables

- **One firmware version for every port** (docs/TODO.md §5-8): one release carries all
  boards. Per-port versions would make `version = "0.3.1"` mean a different thing per
  board.
- **One firmware per board profile, with every backend.** A backend costs flash
  only, so the release does not split by feature. Separate profiles stay separate
  files (`esp32s3` and `esp32s3-psram`).
- **Built in CI from the release tag**, which needs the espup job of
  docs/TODO.md §5-7 for the ESP32-S3.
- The manifest carries, per file: board, sha256, size, `git describe`, the axes of
  docs/app-workflow.md §3.8 and the backends.

## 9. To check before implementing

- **Does flashing the firmware keep the slots?** The app slot starts at 1 MiB on the
  Pico family and on the ESP32-S3 (`ports/common/src/profile.rs`), above the firmware.
  `espflash` and UF2 should write only the image's range, but this has not been
  checked on hardware for each board. If a slot is lost, `fw flash` has to say that
  the app, config and assets must be written again.
- **The firmware has to stay below the app slot.** A firmware over 1 MiB overlaps the
  app slot. `fw build` and the release job should refuse such an image; no check
  exists today.
- **Rollback across a config slot format.** An older firmware may not read a newer
  config slot (docs/device-layer.md §5.2 keeps version 1 for that reason). `fw flash`
  of an older version should warn when the board's config slot is newer than that
  firmware reads.

## 10. Stages

1. Owner decisions docs/TODO.md §5-7, §5-8 and §7-1 ... §7-7.
2. Release job and manifest; `fw list`, `fw install`, `fw flash`, `fw current`.
3. `check` and `deploy` read the axes from the manifest or the board (§6); the
   `[firmware]` pin (§5).
4. Backends as Cargo features; `fw build` (§7).
