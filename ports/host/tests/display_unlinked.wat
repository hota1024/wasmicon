;; A guest that imports wasmicon:device/display@0.1.0 (tests/display_unlinked.rs).
(module
  (import "wasmicon:device/display@0.1.0" "[static]screen.open"
    (func $open (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "run")
    (drop (call $open (i32.const 0) (i32.const 0)))))
