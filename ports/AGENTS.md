# ports/

`ports/common` is the shared HAL implementation and trace formatter; each board
directory implements only the `Board` trait and `main`. `ports/host` is the PC
port that CI and the CLI use.

- Trace formatting is identical on every port; `time` is the only host call
  that is not traced (docs/decisions.md D-10, docs/abi-spec.md §9). Never
  add behavior that "happens to succeed" on one board (docs/decisions.md P-13).
- The firmware embeds no app. Apps come from the app slot, wiring from the
  config slot (docs/app-workflow.md §3.3, §3.9). With an empty slot the
  firmware prints the reason and parks; it drives no pins.
- Register access is `unsafe`; every block states the bit-width bound it relies
  on in its `// SAFETY:` comment. Each port crate turns on
  `clippy::undocumented_unsafe_blocks`.
- Each board is its own Cargo workspace with its own toolchain and
  `.cargo/config.toml`:
  `cd ports/<board> && cargo fmt --check && cargo clippy --release -- -D warnings && cargo build --release`.
- `ports/esp32s3` needs the espup Xtensa toolchain and `~/export-esp.sh`
  (docs/decisions.md P-10). Go through the wrapper:
  `sh ports/esp32s3/build.sh clippy --release -- -D warnings` and again with
  `--features hw-probe`. There is no ESP32-S3 CI job, so say in the commit body
  whether it compiled.
- Hardware results belong in `docs/verification-report.md`, not in source
  comments.
