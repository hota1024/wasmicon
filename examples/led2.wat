;; INFO asc module.ts --textFile module.wat --outFile module.wasm --bindings raw -O3 --runtime stub
(module
 (type $0 (func (param i32 i32)))
 (type $1 (func (param i32) (result i32)))
 (type $2 (func))
 (import "wasmarch" "register32_write" (func $module/register32_write (param i32 i32)))
 (import "wasmarch" "register32_read" (func $module/register32_read (param i32) (result i32)))
 (memory $0 0)
 (export "main" (func $module/main))
 (export "memory" (memory $0))
 (func $module/main
  i32.const 1072973104
  i32.const 256
  call $module/register32_write
  i32.const 1072971808
  i32.const 1072971808
  call $module/register32_read
  i32.const -33
  i32.and
  call $module/register32_write
  i32.const 1072971808
  i32.const 1072971808
  call $module/register32_read
  i32.const 32
  i32.or
  call $module/register32_write
  i32.const 1072971780
  i32.const 1072971780
  call $module/register32_read
  i32.const -33
  i32.and
  call $module/register32_write
  i32.const 1072971780
  i32.const 1072971780
  call $module/register32_read
  call $module/register32_write
 )
)
