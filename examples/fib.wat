;; INFO asc module.ts --textFile module.wat --outFile module.wasm --bindings raw --runtime stub
(module
 (type $0 (func (param i32) (result i32)))
 (memory $0 0)
 (table $0 1 1 funcref)
 (elem $0 (i32.const 1))
 (export "fib" (func $module/fib))
 (export "memory" (memory $0))
 (func $module/fib (param $0 i32) (result i32)
  local.get $0
  i32.const 2
  i32.lt_s
  if
   local.get $0
   return
  end
  local.get $0
  i32.const 1
  i32.sub
  call $module/fib
  local.get $0
  i32.const 2
  i32.sub
  call $module/fib
  i32.add
  return
 )
)
