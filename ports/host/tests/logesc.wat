;; トレースに埋め込む文字列のエスケープを確かめるための最小ゲスト。
;; "a\nb\"c" を log に渡す。生のまま出すと abi-spec §9 の 1 行 1 レコードが崩れる。
(module
  (import "wasmicon:hal/log@0.1.0" "log" (func $log (param i32 i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "a\0Ab\22c")
  (func (export "run")
    (call $log (i32.const 2) (i32.const 0) (i32.const 5))
  )
)
