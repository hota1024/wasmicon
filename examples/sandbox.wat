;; INFO asc module.ts --textFile module.wat --outFile module.wasm --bindings raw --runtime stub
(module
 (func $module/get_time (result i32)
  i32.const 3
  i32.const 1
  call $module/add
  return
 )
)

