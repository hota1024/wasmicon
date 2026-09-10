;; Lチカの最小ゲスト。手書きの WAT。
;;
;; HANDOFF §5 Phase 2 の完了条件は apps/blink-rs を動かすことだが、
;; それは Phase 3 で作る。ここでは同じ host call 列を出す手書きの
;; モジュールでポートとトレースを先に検証する。
(module
  (import "wasmicon:hal/gpio@0.1.0" "[static]pin.open"
    (func $pin_open (param i32 i32 i32) (result i32)))
  (import "wasmicon:hal/gpio@0.1.0" "[method]pin.write"
    (func $pin_write (param i32 i32) (result i32)))
  (import "wasmicon:hal/gpio@0.1.0" "[method]pin.toggle"
    (func $pin_toggle (param i32) (result i32)))
  (import "wasmicon:hal/gpio@0.1.0" "[resource-drop]pin"
    (func $pin_drop (param i32)))
  (import "wasmicon:hal/time@0.1.0" "sleep-ms" (func $sleep_ms (param i32)))
  (import "wasmicon:hal/log@0.1.0" "log" (func $log (param i32 i32 i32)))

  (memory (export "memory") 1)
  (data (i32.const 0) "blink start")

  (global $handle (mut i32) (i32.const 0))

  (func (export "run")
    (local $i i32)
    ;; log(info, "blink start")
    (call $log (i32.const 2) (i32.const 0) (i32.const 11))

    ;; pin.open(index=2, mode=output, out=0x100)
    (drop (call $pin_open (i32.const 2) (i32.const 3) (i32.const 0x100)))
    (global.set $handle (i32.load (i32.const 0x100)))

    ;; 3 回点滅
    (block $done
      (loop $again
        (br_if $done (i32.ge_u (local.get $i) (i32.const 3)))
        (drop (call $pin_write (global.get $handle) (i32.const 1)))
        (call $sleep_ms (i32.const 1))
        (drop (call $pin_toggle (global.get $handle)))
        (call $sleep_ms (i32.const 1))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $again)
      )
    )

    (call $pin_drop (global.get $handle))
  )
)
