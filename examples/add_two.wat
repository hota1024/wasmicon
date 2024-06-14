(module
    (export "add_two" (func $add_two))
    (func $add_two (param $0 i32) (param $1 i32) (result i32)
        local.get $0
        local.get $1
        i32.add
    )
)
