;; INFO asc module.ts --textFile module.wat --outFile module.wasm --bindings raw --runtime stub
(module
    (func $module/add (param i32 i32) (result i32)
        local.get 0
        local.get 1
        i32.add
        return
    )
    (func $module/get_time (result i32)
        i32.const 3
        i32.const 1
        call $module/add
        return
    )
    (export "get_time" (func $module/get_time))
)
