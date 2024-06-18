;; INFO asc module.ts --textFile module.wat --outFile module.wasm --bindings raw --runtime stub
(module
 (type $0 (func (param i32 i32)))
 (type $1 (func (param i32) (result i32)))
 (type $2 (func (param i32)))
 (type $3 (func))
 (import "wasmarch" "register32_write" (func $module/register32_write (param i32 i32)))
 (import "wasmarch" "register32_read" (func $module/register32_read (param i32) (result i32)))
 (global $module/GPIO_ENABLE_REG i32 (i32.const 1072971808))
 (global $module/GPIO_ENABLE1_REG i32 (i32.const 1072971820))
 (global $module/GPIO_FUNC_OUT_SEL_CFG_REG i32 (i32.const 1072973104))
 (global $module/GPIO_OUT_REG i32 (i32.const 1072971780))
 (global $module/GPIO_OUT1_REG i32 (i32.const 1072971792))
 (memory $0 0)
 (table $0 1 1 funcref)
 (elem $0 (i32.const 1))
 (export "main" (func $module/main))
 (export "memory" (memory $0))
 (func $module/bit (param $0 i32) (result i32)
  i32.const 1
  local.get $0
  i32.shl
  return
 )
 (func $module/gpio_output_enable (param $0 i32) (param $1 i32)
  global.get $module/GPIO_ENABLE_REG
  global.get $module/GPIO_ENABLE_REG
  call $module/register32_read
  local.get $0
  call $module/bit
  i32.const -1
  i32.xor
  i32.and
  call $module/register32_write
  global.get $module/GPIO_ENABLE_REG
  global.get $module/GPIO_ENABLE_REG
  call $module/register32_read
  local.get $1
  local.get $0
  i32.shl
  i32.or
  call $module/register32_write
 )
 (func $module/gpio_output (param $0 i32)
  global.get $module/GPIO_FUNC_OUT_SEL_CFG_REG
  i32.const 256
  call $module/register32_write
  local.get $0
  i32.const 1
  call $module/gpio_output_enable
 )
 (func $module/gpio_write (param $0 i32) (param $1 i32)
  global.get $module/GPIO_OUT_REG
  global.get $module/GPIO_OUT_REG
  call $module/register32_read
  local.get $0
  call $module/bit
  i32.const -1
  i32.xor
  i32.and
  call $module/register32_write
  global.get $module/GPIO_OUT_REG
  global.get $module/GPIO_OUT_REG
  call $module/register32_read
  local.get $1
  local.get $0
  i32.shl
  i32.or
  call $module/register32_write
 )
 (func $module/main
  i32.const 4
  call $module/gpio_output
  i32.const 4
  i32.const 1
  call $module/gpio_write
 )
)
