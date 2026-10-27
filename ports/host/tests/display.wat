;; Display guest, hand-written in WAT (tests/display.rs).
;;
;; Walks wasmicon:device/display@0.1.0 through its success and error paths.
;; Memory: 0 = out-handle, 16.. = four pixels, high byte first.
(module
  (import "wasmicon:device/display@0.1.0" "[static]screen.open"
    (func $open (param i32 i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.width"
    (func $width (param i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.height"
    (func $height (param i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.format"
    (func $format (param i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.fill-rect"
    (func $fill (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.write-rect"
    (func $write (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.present"
    (func $present (param i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[resource-drop]screen"
    (func $drop (param i32)))

  (memory (export "memory") 1)
  ;; red, green, blue, white
  (data (i32.const 16) "\f8\00\07\e0\00\1f\ff\ff")

  (func (export "run")
    (local $s i32)
    (drop (call $open (i32.const 0) (i32.const 0)))
    (local.set $s (i32.load (i32.const 0)))
    (drop (call $width (local.get $s)))
    (drop (call $height (local.get $s)))
    (drop (call $format (local.get $s)))
    ;; Whole screen white, then a blue square at the bottom-right corner.
    (drop (call $fill (local.get $s) (i32.const 0) (i32.const 0) (i32.const 320) (i32.const 240) (i32.const 0xffff)))
    (drop (call $fill (local.get $s) (i32.const 318) (i32.const 238) (i32.const 2) (i32.const 2) (i32.const 0x001f)))
    ;; invalid-argument: one pixel past the right edge, empty, color too wide.
    (drop (call $fill (local.get $s) (i32.const 319) (i32.const 0) (i32.const 2) (i32.const 1) (i32.const 0)))
    (drop (call $fill (local.get $s) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)))
    (drop (call $fill (local.get $s) (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1) (i32.const 0x10000)))
    ;; Four pixels as 2x2 at the origin; then the same bytes as 3x1 (wrong length).
    (drop (call $write (local.get $s) (i32.const 0) (i32.const 0) (i32.const 2) (i32.const 2) (i32.const 16) (i32.const 8)))
    (drop (call $write (local.get $s) (i32.const 0) (i32.const 0) (i32.const 3) (i32.const 1) (i32.const 16) (i32.const 8)))
    (drop (call $present (local.get $s)))
    ;; Drawn after the last present: never visible.
    (drop (call $fill (local.get $s) (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1) (i32.const 0)))
    ;; Already open; then bad handles.
    (drop (call $open (i32.const 0) (i32.const 4)))
    (drop (call $open (i32.const 1) (i32.const 4)))
    (drop (call $width (i32.const 7)))
    (drop (call $present (i32.const 7)))
    (call $drop (local.get $s))
    (drop (call $height (local.get $s)))))
