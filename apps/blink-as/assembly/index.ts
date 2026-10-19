// Blink (AssemblyScript). Produces the same host-call sequence as blink-rs.
//
// The LED pin number comes from board.pin-by-role, so the same .wasm runs
// on both ESP32-S3 and RP2040 (docs/abi-spec.md §8).

// asc cannot resolve the scoped npm package (@wasmicon/hal) as ~lib, so the
// bindings are imported by relative path. The layout in README.md is the
// source of truth for where they live.
import {
  Level,
  Pin,
  PinMode,
  board,
  log,
  time,
} from "../../../bindings/assemblyscript/assembly/index";

/// Number of blinks.
const BLINKS: i32 = 3;

/// On and off interval in milliseconds.
const INTERVAL_MS: u32 = 1;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
export function run(): void {
  log.info("blink start");

  const index = board.pinByRole("led");
  if (index < 0) {
    log.error("no pin assigned to role led");
    return;
  }
  const pin = Pin.open(index as u32, PinMode.Output);
  if (pin == null) {
    log.error("cannot open led pin");
    return;
  }

  for (let i = 0; i < BLINKS; i++) {
    if (pin.write(Level.High) != 0) {
      log.error("write failed");
      return;
    }
    time.sleepMs(INTERVAL_MS);
    if (pin.toggle() != 0) {
      log.error("toggle failed");
      return;
    }
    time.sleepMs(INTERVAL_MS);
  }
  pin.close();
}
