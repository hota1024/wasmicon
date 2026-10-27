# Local copy of esp-sync 0.3.0 for ports/esp32p4

This is esp-sync 0.3.0 from crates.io with one change, made so that the
firmware runs on ESP32-P4 silicon before v3.0 (the M5Stack Tab5 has v1.0).
`ports/esp32p4/Cargo.toml` swaps it in with `[patch.crates-io]`.

The change: `SingleCoreInterruptLock::enter` / `exit` in `src/raw.rs` had an
`esp32p4` arm that raises `mintthresh` (CSR 0x347) while interrupts are off.
That arm works around a Zcmp hardware bug of P4 v3.2 (ECO7), but it is chosen
by `cfg(esp32p4)` alone, and on v1.0 the CSR does not exist: the first
critical section inside `esp_hal::init` takes an illegal-instruction exception
and the watchdog resets the chip, forever. Both arms are removed (each is
marked `WASMICON LOCAL PATCH`), so the generic `riscv` arm runs. The firmware
targets `riscv32imafc`, which has no Zcmp, so the workaround has nothing to
guard against there.

esp-hal 1.2.2 touches the same CSR in its CLIC interrupt code
(`interrupt/riscv/clic.rs`), which this copy cannot change. The firmware
enables no interrupt, so that code never runs; docs/TODO.md §1.3.2 records
the constraint.

`ports/esp32p4/build.rs` fails the build when `Cargo.lock` takes esp-sync
from crates.io instead of this copy.

Delete this directory and the `[patch.crates-io]` entry once an esp-sync
release picks the workaround by silicon revision (docs/TODO.md §1.3.2).

The upstream README follows.

# esp-sync

[![Crates.io](https://img.shields.io/crates/v/esp-sync?labelColor=1C2C2E&color=C96329&logo=Rust&style=flat-square)](https://crates.io/crates/esp-sync)
[![docs.rs](https://img.shields.io/docsrs/esp-sync?labelColor=1C2C2E&color=C96329&logo=rust&style=flat-square)](https://docs.espressif.com/projects/rust/esp-sync/latest/)
![MSRV](https://img.shields.io/badge/MSRV-1.95.0-blue?labelColor=1C2C2E&style=flat-square)
![Crates.io](https://img.shields.io/crates/l/esp-sync?labelColor=1C2C2E&style=flat-square)
[![Matrix](https://img.shields.io/matrix/esp-rs:matrix.org?label=join%20matrix&labelColor=1C2C2E&color=BEC5C9&logo=matrix&style=flat-square)](https://matrix.to/#/#esp-rs:matrix.org)

This crate provides an optimized raw mutex (a lock type that doesn't wrap any data) for ESP32 devices.

## Minimum Supported Rust Version (MSRV)

This crate is guaranteed to compile when using the latest stable Rust version at the time of the crate's release. It _might_ compile with older versions, but that may change in any new release, including patches.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the
work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
