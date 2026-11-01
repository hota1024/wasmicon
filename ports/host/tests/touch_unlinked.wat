;; A guest that imports wasmicon:device/touch@0.1.0 (tests/touch_unlinked.rs).
(module
  (import "wasmicon:device/touch@0.1.0" "[static]panel.open"
    (func $open (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "run")
    (drop (call $open (i32.const 0) (i32.const 0)))))
