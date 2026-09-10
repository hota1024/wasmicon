// Lチカ（AssemblyScript）。blink-rs と同じ host call 列を出す。
//
// LED のピン番号は board.pin-by-role で引くので、同一の .wasm が
// ESP32-S3 と RP2040 の両方で動く（abi-spec §8）。

// asc はスコープ付き npm パッケージ（@wasmicon/hal）を ~lib として解決できないので
// 相対パスで参照する。配置は README.md の「構成」を正とする。
import {
  Level,
  Pin,
  PinMode,
  board,
  log,
  time,
} from "../../../bindings/assemblyscript/assembly/index";

/// 点滅回数。
const BLINKS: i32 = 3;

/// 点灯・消灯の間隔（ミリ秒）。
const INTERVAL_MS: u32 = 1;

/// エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。
export function run(): void {
  log.info("blink start");

  const index = board.pinByRole("led");
  if (index < 0) {
    log.error("led の割り当てが無い");
    return;
  }
  const pin = Pin.open(index as u32, PinMode.Output);
  if (pin == null) {
    log.error("led のピンを開けない");
    return;
  }

  for (let i = 0; i < BLINKS; i++) {
    if (pin.write(Level.High) != 0) {
      log.error("write に失敗");
      return;
    }
    time.sleepMs(INTERVAL_MS);
    if (pin.toggle() != 0) {
      log.error("toggle に失敗");
      return;
    }
    time.sleepMs(INTERVAL_MS);
  }
  pin.close();
}
