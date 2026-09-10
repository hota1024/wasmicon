// Wasmicon HAL のゲスト向けバインディング（AssemblyScript）。
//
// generated.ts は wasmicon-gen の出力。手で編集しない。
// ここは生成された @external 宣言に薄いラッパをかぶせたもの。
//
// Rust 版と違い、AssemblyScript には Drop が無い（--runtime stub では
// ファイナライザも動かない）。ハンドルの解放は明示的に close() を呼ぶ。

import {
  ErrorCode,
  GpioLevel,
  GpioPinMode,
  I2cSpeed,
  LogLevel,
  SpiMode,
  board_pin_by_role,
  gpio_pin_drop,
  gpio_pin_open,
  gpio_pin_read,
  gpio_pin_set_mode,
  gpio_pin_toggle,
  gpio_pin_write,
  i2c_bus_drop,
  i2c_bus_open,
  i2c_bus_read,
  i2c_bus_write,
  i2c_bus_write_read,
  log_log,
  spi_bus_drop,
  spi_bus_open,
  spi_bus_transfer,
  spi_bus_write,
  time_now_us,
  time_sleep_ms,
  time_sleep_us,
} from "./generated";

export { ErrorCode, GpioLevel as Level, GpioPinMode as PinMode, I2cSpeed as Speed, LogLevel, SpiMode };

/// out ポインタ用の作業領域。ゲストは単一スレッドなので使い回してよい。
const out32 = new Uint32Array(1);

/// 確保済みの GPIO ピン。
export class Pin {
  constructor(public handle: u32) {}

  /// ピンを確保しモードを設定する。失敗したら null。
  static open(index: u32, mode: GpioPinMode): Pin | null {
    const status = gpio_pin_open(index, mode as u32, out32.dataStart);
    if (status != 0) return null;
    return new Pin(out32[0]);
  }

  /// モードを変更する。成功したら 0。
  setMode(mode: GpioPinMode): u32 {
    return gpio_pin_set_mode(this.handle, mode as u32);
  }

  /// 現在の入力レベルを読む。失敗したら -1。
  read(): i32 {
    const status = gpio_pin_read(this.handle, out32.dataStart);
    if (status != 0) return -1;
    return out32[0] as i32;
  }

  /// 出力レベルを設定する。成功したら 0。
  write(level: GpioLevel): u32 {
    return gpio_pin_write(this.handle, level as u32);
  }

  /// 出力レベルを反転する。成功したら 0。
  toggle(): u32 {
    return gpio_pin_toggle(this.handle);
  }

  /// 解放する。AssemblyScript には Drop が無いので明示的に呼ぶ。
  close(): void {
    gpio_pin_drop(this.handle);
  }
}

/// 確保済みの I2C バス。
export class I2cBus {
  constructor(public handle: u32) {}

  static open(index: u32, speed: I2cSpeed): I2cBus | null {
    const status = i2c_bus_open(index, speed as u32, out32.dataStart);
    if (status != 0) return null;
    return new I2cBus(out32[0]);
  }

  write(address: u16, data: Uint8Array): u32 {
    return i2c_bus_write(this.handle, address as u32, data.dataStart, data.length);
  }

  /// buf の長さだけ読む。成功したら読めたバイト数、失敗したら -1。
  read(address: u16, buf: Uint8Array): i32 {
    const cap = buf.length as u32;
    const status = i2c_bus_read(
      this.handle,
      address as u32,
      cap,
      buf.dataStart,
      cap,
      out32.dataStart
    );
    if (status != 0) return -1;
    return out32[0] as i32;
  }

  writeRead(address: u16, data: Uint8Array, buf: Uint8Array): i32 {
    const cap = buf.length as u32;
    const status = i2c_bus_write_read(
      this.handle,
      address as u32,
      data.dataStart,
      data.length,
      cap,
      buf.dataStart,
      cap,
      out32.dataStart
    );
    if (status != 0) return -1;
    return out32[0] as i32;
  }

  close(): void {
    i2c_bus_drop(this.handle);
  }
}

/// 確保済みの SPI バス。CS / DC はゲストが gpio で制御する。
export class SpiBus {
  constructor(public handle: u32) {}

  static open(index: u32, frequencyHz: u32, mode: SpiMode): SpiBus | null {
    const status = spi_bus_open(index, frequencyHz, mode as u32, out32.dataStart);
    if (status != 0) return null;
    return new SpiBus(out32[0]);
  }

  write(data: Uint8Array): u32 {
    return spi_bus_write(this.handle, data.dataStart, data.length);
  }

  transfer(data: Uint8Array, buf: Uint8Array): i32 {
    const status = spi_bus_transfer(
      this.handle,
      data.dataStart,
      data.length,
      buf.dataStart,
      buf.length,
      out32.dataStart
    );
    if (status != 0) return -1;
    return out32[0] as i32;
  }

  close(): void {
    spi_bus_drop(this.handle);
  }
}

/// 時間。
export namespace time {
  export function nowUs(): u64 {
    return time_now_us();
  }
  export function sleepMs(ms: u32): void {
    time_sleep_ms(ms);
  }
  export function sleepUs(us: u32): void {
    time_sleep_us(us);
  }
}

/// デバッグ出力。AssemblyScript の string は UTF-16 なので UTF-8 に直して渡す。
export namespace log {
  export function write(level: LogLevel, message: string): void {
    const bytes = String.UTF8.encode(message);
    log_log(level as u32, changetype<usize>(bytes), bytes.byteLength);
  }
  export function info(message: string): void {
    write(LogLevel.Info, message);
  }
  export function error(message: string): void {
    write(LogLevel.Error, message);
  }
}

/// ボード固有のピン割り当て。
export namespace board {
  /// 役割名から GPIO 番号を引く。割り当てが無ければ -1。
  export function pinByRole(role: string): i32 {
    const bytes = String.UTF8.encode(role);
    const status = board_pin_by_role(
      changetype<usize>(bytes),
      bytes.byteLength,
      out32.dataStart
    );
    if (status != 0) return -1;
    return out32[0] as i32;
  }
}
