;; Minimal guest for checking the escaping of strings embedded in a trace.
;; Logs "a\nb\"c"; emitted raw it would break the one-record-per-line form of
;; docs/abi-spec.md §9.
(module
  (import "wasmicon:hal/log@0.1.0" "log" (func $log (param i32 i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "a\0Ab\22c")
  (func (export "run")
    (call $log (i32.const 2) (i32.const 0) (i32.const 5))
  )
)
