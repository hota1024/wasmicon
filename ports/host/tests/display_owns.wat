;; Opens a pin and a bus that the display backend holds, then free ones
;; (tests/ili9341.rs).
(module
  (import "wasmicon:hal/gpio@0.1.0" "[static]pin.open"
    (func $pin_open (param i32 i32 i32) (result i32)))
  (import "wasmicon:hal/spi@0.1.0" "[static]bus.open"
    (func $bus_open (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "run")
    (drop (call $pin_open (i32.const 20) (i32.const 3) (i32.const 0)))
    (drop (call $pin_open (i32.const 23) (i32.const 3) (i32.const 0)))
    (drop (call $bus_open (i32.const 0) (i32.const 1000000) (i32.const 0) (i32.const 0)))
    (drop (call $bus_open (i32.const 1) (i32.const 1000000) (i32.const 0) (i32.const 0)))))
