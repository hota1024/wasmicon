;; Test guest of docs/verification-report.md §42: logs `usb console test line`
;; 400 times (level info), then traps on `unreachable`. 400 lines overflow the
;; 8 KiB queue of the rp2350 USB console (docs/decisions.md A-12), so a console
;; opened after the run shows the dropped count and then the trap reason.
;;
;; Requires: wat2wasm (wabt), picotool, a Pico 2 W running ports/rp2350
;; Usage: cd verify/usb-console && wat2wasm flood.wat -o flood.wasm
;;        wasmicon deploy flood.wasm --board rp2350
;;        then open the console port (interface 0 of `wasmicon rp2350`)
(module
  (import "wasmicon:hal/log@0.1.0" "log" (func $log (param i32 i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 16) "usb console test line")
  (func (export "run") (local $i i32)
    (loop $l
      (call $log (i32.const 2) (i32.const 16) (i32.const 21))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 400))))
    unreachable))
