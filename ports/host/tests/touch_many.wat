;; Asks for 9 points from the mock (tests/touch.rs). The handle is at 0,
;; len-out at 8, the point buffer at 64.
(module
  (import "wasmicon:device/touch@0.1.0" "[static]panel.open"
    (func $open (param i32 i32) (result i32)))
  (import "wasmicon:device/touch@0.1.0" "[method]panel.read"
    (func $read (param i32 i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "run")
    (drop (call $open (i32.const 0) (i32.const 0)))
    (drop (call $read (i32.const 1) (i32.const 9) (i32.const 64) (i32.const 36) (i32.const 8)))))
