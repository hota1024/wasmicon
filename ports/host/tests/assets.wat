;; Minimal assets guest, hand-written in WAT.
;;
;; Opens the asset `a.bin`, asks its length, reads eight bytes from offset 4,
;; tries a name that is not there, and drops the handle. Together with
;; tests/assets.rs it locks in the docs/abi-spec.md §7, §9 shape of the
;; `wasmicon:hal/assets@0.1.0` calls without an app build.
(module
  (import "wasmicon:hal/assets@0.1.0" "[static]blob.open"
    (func $blob_open (param i32 i32 i32) (result i32)))
  (import "wasmicon:hal/assets@0.1.0" "[method]blob.len"
    (func $blob_len (param i32 i32) (result i32)))
  (import "wasmicon:hal/assets@0.1.0" "[method]blob.read"
    (func $blob_read (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "wasmicon:hal/assets@0.1.0" "[resource-drop]blob"
    (func $blob_drop (param i32)))

  (memory (export "memory") 1)
  (data (i32.const 0) "a.bin")

  (func (export "run")
    ;; blob.open("a.bin", out=0x100)
    (drop (call $blob_open (i32.const 0) (i32.const 5) (i32.const 0x100)))
    ;; blob.len(handle, out=0x104)
    (drop (call $blob_len (i32.load (i32.const 0x100)) (i32.const 0x104)))
    ;; blob.read(handle, offset=4, len=8, buf=0x200, cap=16, len-out=0x108)
    (drop (call $blob_read (i32.load (i32.const 0x100))
      (i32.const 4) (i32.const 8) (i32.const 0x200) (i32.const 16) (i32.const 0x108)))
    ;; blob.open("a.bi", out=0x10c): a well-formed name that is not in the slot
    (drop (call $blob_open (i32.const 0) (i32.const 4) (i32.const 0x10c)))
    (call $blob_drop (i32.load (i32.const 0x100)))
  )
)
