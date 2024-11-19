;; INFO asc module.ts --textFile module.wat --outFile module.wasm --bindings raw --runtime stub
(module
 (type $0 (func (param i32) (result i32)))
 (type $1 (func (param i32 i32)))
 (type $2 (func))
 (import "wasmicon" "reg32_read" (func $module/reg32_read (param i32) (result i32)))
 (import "wasmicon" "reg32_write" (func $module/reg32_write (param i32 i32)))
 (global $module/GPIO_OUT_REG i32 (i32.const 1072971780))
 (memory $0 0)
 (table $0 1 1 funcref)
 (elem $0 (i32.const 1))
 (export "wasm_main" (func $module/wasm_main))
 (export "memory" (memory $0))
 (func $module/bit (param $0 i32) (result i32)
  i32.const 1
  local.get $0
  i32.shl
  return
 )
 (func $module/gpio_write (param $0 i32) (param $1 i32)
  (local $2 i32)
  global.get $module/GPIO_OUT_REG
  call $module/reg32_read
  local.set $2
  local.get $2
  local.get $0
  call $module/bit
  i32.const -1
  i32.xor
  i32.and
  local.set $2
  local.get $2
  local.get $1
  if (result i32)
   i32.const 1
  else
   i32.const 0
  end
  i32.const 5
  i32.shl
  i32.or
  local.set $2
  global.get $module/GPIO_OUT_REG
  local.get $2
  call $module/reg32_write
 )
 (func $module/wasm_main
  i32.const 5
  i32.const 1
  call $module/gpio_write
 )
)
