;; Exercises wasmicon:device/touch@0.1.0 on the mock (tests/touch.rs).
;; Memory: the handle at 0, len-out at 8, the point buffer at 64.
(module
  (import "wasmicon:device/touch@0.1.0" "[static]panel.open"
    (func $open (param i32 i32) (result i32)))
  (import "wasmicon:device/touch@0.1.0" "[method]panel.read"
    (func $read (param i32 i32 i32 i32 i32) (result i32)))
  (import "wasmicon:device/touch@0.1.0" "[resource-drop]panel"
    (func $drop (param i32)))
  (memory (export "memory") 1)
  (func (export "run")
    ;; Display 1 has no touch panel; display 0 opens once.
    (drop (call $open (i32.const 1) (i32.const 0)))
    (drop (call $open (i32.const 0) (i32.const 0)))
    (drop (call $open (i32.const 0) (i32.const 0)))
    ;; max-points 0, and 3 points that do not fit an 8-byte buffer.
    (drop (call $read (i32.const 1) (i32.const 0) (i32.const 64) (i32.const 32) (i32.const 8)))
    (drop (call $read (i32.const 1) (i32.const 3) (i32.const 64) (i32.const 8) (i32.const 8)))
    ;; Script entry 0 has 3 points; 2 are asked for.
    (drop (call $read (i32.const 1) (i32.const 2) (i32.const 64) (i32.const 8) (i32.const 8)))
    ;; Entry 1 is no touch, and there is no entry 2.
    (drop (call $read (i32.const 1) (i32.const 2) (i32.const 64) (i32.const 8) (i32.const 8)))
    (drop (call $read (i32.const 1) (i32.const 2) (i32.const 64) (i32.const 8) (i32.const 8)))
    ;; A handle that was never handed out, then the real one after drop.
    (drop (call $read (i32.const 9) (i32.const 1) (i32.const 64) (i32.const 4) (i32.const 8)))
    (call $drop (i32.const 1))
    (drop (call $read (i32.const 1) (i32.const 1) (i32.const 64) (i32.const 4) (i32.const 8)))))
