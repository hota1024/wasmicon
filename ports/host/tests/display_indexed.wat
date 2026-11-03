;; Indexed display guest, hand-written in WAT (tests/display_indexed.rs).
;;
;; Walks `set-palette` and the `indexed8` forms of fill-rect and write-rect
;; through their success and error paths, at any screen width.
;; Memory: 0 = out-handle, 1024.. = indices (i * 7 mod 256),
;; 8192.. = palette (entry k = k << 8 | (255 - k), high byte first).
(module
  (import "wasmicon:device/display@0.1.0" "[static]screen.open"
    (func $open (param i32 i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.width"
    (func $width (param i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.format"
    (func $format (param i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.fill-rect"
    (func $fill (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.write-rect"
    (func $write (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.set-palette"
    (func $palette (param i32 i32 i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[method]screen.present"
    (func $present (param i32) (result i32)))
  (import "wasmicon:device/display@0.1.0" "[resource-drop]screen"
    (func $drop (param i32)))

  (memory (export "memory") 1)

  (func (export "run")
    (local $s i32)
    (local $w i32)
    (local $i i32)
    ;; Indices for three rows of up to 2,048 pixels.
    (loop $idx
      (i32.store8 (i32.add (i32.const 1024) (local.get $i))
        (i32.mul (local.get $i) (i32.const 7)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $idx (i32.lt_u (local.get $i) (i32.const 6144))))
    (local.set $i (i32.const 0))
    (loop $pal
      (i32.store8 (i32.add (i32.const 8192) (i32.shl (local.get $i) (i32.const 1)))
        (local.get $i))
      (i32.store8 (i32.add (i32.const 8193) (i32.shl (local.get $i) (i32.const 1)))
        (i32.sub (i32.const 255) (local.get $i)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $pal (i32.lt_u (local.get $i) (i32.const 256))))

    (drop (call $open (i32.const 0) (i32.const 0)))
    (local.set $s (i32.load (i32.const 0)))
    (local.set $w (call $width (local.get $s)))
    ;; invalid-argument: one byte short, one byte long; the screen stays rgb565.
    (drop (call $palette (local.get $s) (i32.const 8192) (i32.const 511)))
    (drop (call $palette (local.get $s) (i32.const 8192) (i32.const 513)))
    (drop (call $format (local.get $s)))
    (drop (call $palette (local.get $s) (i32.const 8192) (i32.const 512)))
    (drop (call $format (local.get $s)))
    ;; Whole screen index 3, then an index too wide.
    (drop (call $fill (local.get $s) (i32.const 0) (i32.const 0) (local.get $w) (i32.const 240) (i32.const 3)))
    (drop (call $fill (local.get $s) (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1) (i32.const 256)))
    ;; Three full-width rows at y = 10.
    (drop (call $write (local.get $s) (i32.const 0) (i32.const 10) (local.get $w) (i32.const 3)
      (i32.const 1024) (i32.mul (local.get $w) (i32.const 3))))
    ;; invalid-argument: w * h + 1 bytes, and the rgb565 length.
    (drop (call $write (local.get $s) (i32.const 0) (i32.const 20) (i32.const 3) (i32.const 1) (i32.const 1024) (i32.const 4)))
    (drop (call $write (local.get $s) (i32.const 0) (i32.const 20) (i32.const 3) (i32.const 1) (i32.const 1024) (i32.const 6)))
    ;; An odd width, away from the origin.
    (drop (call $write (local.get $s) (i32.const 5) (i32.const 30) (i32.const 7) (i32.const 2) (i32.const 1024) (i32.const 14)))
    (drop (call $present (local.get $s)))
    (call $drop (local.get $s))
    ;; A new open starts in rgb565 again.
    (drop (call $open (i32.const 0) (i32.const 0)))
    (local.set $s (i32.load (i32.const 0)))
    (drop (call $format (local.get $s)))
    (call $drop (local.get $s))))
