// Guest-side bindings of the Wasmicon HAL (AssemblyScript).
//
// generated.ts is the output of wasmicon-gen; do not edit it by hand.
// This file adds thin wrappers over the generated @external declarations.
//
// Unlike the Rust binding, AssemblyScript has no Drop (and no finalizer runs
// under --runtime stub). Release a handle by calling close() explicitly.

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

/// Scratch space for out-pointers. The guest is single-threaded, so it can be reused.
const out32 = new Uint32Array(1);

/// An open GPIO pin.
export class Pin {
  constructor(public handle: u32) {}

  /// Opens the pin in the given mode. Returns null on failure.
  static open(index: u32, mode: GpioPinMode): Pin | null {
    const status = gpio_pin_open(index, mode as u32, out32.dataStart);
    if (status != 0) return null;
    return new Pin(out32[0]);
  }

  /// Changes the pin mode. Returns 0 on success.
  setMode(mode: GpioPinMode): u32 {
    return gpio_pin_set_mode(this.handle, mode as u32);
  }

  /// Reads the current input level. Returns -1 on failure.
  read(): i32 {
    const status = gpio_pin_read(this.handle, out32.dataStart);
    if (status != 0) return -1;
    return out32[0] as i32;
  }

  /// Sets the output level. Returns 0 on success.
  write(level: GpioLevel): u32 {
    return gpio_pin_write(this.handle, level as u32);
  }

  /// Inverts the output level. Returns 0 on success.
  toggle(): u32 {
    return gpio_pin_toggle(this.handle);
  }

  /// Releases the pin. Call it explicitly; AssemblyScript has no Drop.
  close(): void {
    gpio_pin_drop(this.handle);
  }
}

/// An open I2C bus.
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

  /// Reads buf.length bytes. Returns the number of bytes read, or -1 on failure.
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

/// An open SPI bus. The guest drives CS and DC through gpio.
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

/// Monotonic time and blocking delays.
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

/// Debug output. AssemblyScript strings are UTF-16, so they are re-encoded as UTF-8.
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

/// Board-specific pin assignment.
export namespace board {
  /// Looks up the GPIO number of a role name. Returns -1 if there is no assignment.
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
