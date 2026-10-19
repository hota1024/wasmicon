//! The ABI model: the result of applying the lowering rules of `docs/abi-spec.md`.
//!
//! WIT types are already lowered to Core Wasm parameter lists here, so each
//! emitter only translates them into the notation of its language.

/// Core Wasm value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmTy {
    I32,
    I64,
    F32,
    F64,
}

impl WasmTy {
    /// One character of the signature notation shared with `tools/wit2sig.py`.
    pub fn sig_char(self) -> char {
        match self {
            WasmTy::I32 => 'i',
            WasmTy::I64 => 'I',
            WasmTy::F32 => 'f',
            WasmTy::F64 => 'F',
        }
    }
}

/// Scalar of docs/abi-spec.md §4.1. `Enum` and `Flags` carry the WIT type name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scalar {
    Bool,
    U8,
    U16,
    U32,
    S32,
    U64,
    S64,
    F32,
    F64,
    Enum(String),
    Flags(String),
}

impl Scalar {
    pub fn wasm(&self) -> WasmTy {
        match self {
            Scalar::U64 | Scalar::S64 => WasmTy::I64,
            Scalar::F32 => WasmTy::F32,
            Scalar::F64 => WasmTy::F64,
            _ => WasmTy::I32,
        }
    }

    /// Rust type in the binding's extern declaration. Enums and flags are a raw `u32`.
    pub fn rust(&self) -> &'static str {
        match self {
            Scalar::S32 => "i32",
            Scalar::U64 => "u64",
            Scalar::S64 => "i64",
            Scalar::F32 => "f32",
            Scalar::F64 => "f64",
            _ => "u32",
        }
    }

    /// AssemblyScript type.
    pub fn assemblyscript(&self) -> &'static str {
        self.rust()
    }

    /// C type. Enums and flags are a raw `uint32_t`, as in Rust.
    pub fn c(&self) -> &'static str {
        match self {
            Scalar::S32 => "int32_t",
            Scalar::U64 => "uint64_t",
            Scalar::S64 => "int64_t",
            Scalar::F32 => "float",
            Scalar::F64 => "double",
            _ => "uint32_t",
        }
    }
}

/// What one Core Wasm parameter stands for (docs/abi-spec.md §4.2, §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// Resource handle, including `self`.
    Handle,
    /// Scalar, enum or flags.
    Scalar(Scalar),
    /// Pointer of a `list<u8>` parameter.
    DataPtr,
    /// Pointer of a `string` parameter. Identical to `DataPtr` in Core Wasm; kept apart
    /// only so that the C binding can declare it as `const char *`.
    StrPtr,
    /// Length of a `list<u8>` or `string` parameter.
    DataLen,
    /// Out-pointer for the `T` of `result<T, _>` when `T` is a scalar, enum or flags.
    OutScalar(Scalar),
    /// Out-pointer for the `T` of `result<T, _>` when `T` is a resource handle.
    OutHandle,
    /// Destination buffer of a `result<list<u8>, _>`.
    OutBuf,
    /// Buffer capacity of a `result<list<u8>, _>`.
    OutCap,
    /// Out-pointer for the number of bytes written of a `result<list<u8>, _>`.
    OutLen,
}

/// One lowered parameter.
#[derive(Debug, Clone)]
pub struct Param {
    /// Logical name in kebab-case (`self`, `data-ptr`, `len-out`, ...).
    pub name: String,
    pub role: Role,
}

impl Param {
    pub fn wasm(&self) -> WasmTy {
        match &self.role {
            Role::Scalar(s) => s.wasm(),
            _ => WasmTy::I32,
        }
    }

    /// Rust type in the binding's extern declaration.
    pub fn rust(&self) -> String {
        match &self.role {
            Role::Handle | Role::DataLen | Role::OutCap => "u32".to_string(),
            Role::Scalar(s) => s.rust().to_string(),
            Role::DataPtr | Role::StrPtr => "*const u8".to_string(),
            Role::OutHandle | Role::OutLen => "*mut u32".to_string(),
            Role::OutBuf => "*mut u8".to_string(),
            Role::OutScalar(s) => format!("*mut {}", s.rust()),
        }
    }

    /// AssemblyScript type. Every pointer is a `usize`.
    pub fn assemblyscript(&self) -> String {
        match &self.role {
            Role::Handle | Role::DataLen | Role::OutCap => "u32".to_string(),
            Role::Scalar(s) => s.assemblyscript().to_string(),
            Role::DataPtr
            | Role::StrPtr
            | Role::OutHandle
            | Role::OutLen
            | Role::OutBuf
            | Role::OutScalar(_) => "usize".to_string(),
        }
    }

    /// C type. Pointers lower to i32 because the target is wasm32.
    pub fn c(&self) -> String {
        match &self.role {
            Role::Handle | Role::DataLen | Role::OutCap => "uint32_t".to_string(),
            Role::Scalar(s) => s.c().to_string(),
            Role::DataPtr => "const uint8_t *".to_string(),
            Role::StrPtr => "const char *".to_string(),
            Role::OutHandle | Role::OutLen => "uint32_t *".to_string(),
            Role::OutBuf => "uint8_t *".to_string(),
            Role::OutScalar(s) => format!("{} *", s.c()),
        }
    }
}

/// Return value of a function.
#[derive(Debug, Clone)]
pub enum Ret {
    /// No return value.
    None,
    /// The single i32 status of docs/abi-spec.md §4.4.
    Status,
    /// A scalar returned without a `result`.
    Scalar(Scalar),
}

impl Ret {
    pub fn wasm(&self) -> Option<WasmTy> {
        match self {
            Ret::None => None,
            Ret::Status => Some(WasmTy::I32),
            Ret::Scalar(s) => Some(s.wasm()),
        }
    }

    pub fn rust(&self) -> Option<&'static str> {
        match self {
            Ret::None => None,
            Ret::Status => Some("u32"),
            Ret::Scalar(s) => Some(s.rust()),
        }
    }

    pub fn c(&self) -> &'static str {
        match self {
            Ret::None => "void",
            Ret::Status => "uint32_t",
            Ret::Scalar(s) => s.c(),
        }
    }
}

/// One Core Wasm import.
#[derive(Debug, Clone)]
pub struct Import {
    /// Import field name, such as `[static]pin.open` (docs/abi-spec.md §3.2).
    pub name: String,
    pub docs: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Ret,
    /// Slot identifier on the `wasmicon-core` side (`GpioPinOpen`).
    pub slot: String,
    /// Function name in the Rust binding (`pin_open`).
    pub rust_name: String,
    /// Function name in the AssemblyScript binding (`gpio_pin_open`).
    pub as_name: String,
}

impl Import {
    /// The `params:results` notation shared with `tools/wit2sig.py`.
    pub fn sig(&self) -> String {
        let mut s: String = self.params.iter().map(|p| p.wasm().sig_char()).collect();
        s.push(':');
        if let Some(r) = self.ret.wasm() {
            s.push(r.sig_char());
        }
        s
    }
}

/// One enum case.
#[derive(Debug, Clone)]
pub struct EnumCase {
    pub docs: Vec<String>,
    /// WIT name (`input-pull-up`), used for the C constant name.
    pub wit_name: String,
    /// PascalCase identifier, shared by Rust and AssemblyScript.
    pub ident: String,
}

/// One enum definition.
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub docs: Vec<String>,
    /// WIT name (`pin-mode`).
    pub wit_name: String,
    /// Rust type name (`PinMode`).
    pub rust_name: String,
    /// AssemblyScript type name (`GpioPinMode`), prefixed with the interface name.
    pub as_name: String,
    pub cases: Vec<EnumCase>,
}

/// One bit of a flags type.
#[derive(Debug, Clone)]
pub struct FlagBit {
    pub docs: Vec<String>,
    /// WIT name, used for the C constant name.
    pub wit_name: String,
    /// PascalCase identifier, shared by Rust and AssemblyScript.
    pub ident: String,
}

/// One flags definition. Bits are assigned from bit 0 in declaration order
/// (docs/abi-spec.md §4.1).
#[derive(Debug, Clone)]
pub struct FlagsDef {
    pub docs: Vec<String>,
    /// WIT name.
    pub wit_name: String,
    /// Rust type name.
    pub rust_name: String,
    /// Prefix of the AssemblyScript constant names.
    pub as_name: String,
    pub bits: Vec<FlagBit>,
}

/// One interface.
#[derive(Debug, Clone)]
pub struct Iface {
    /// WIT name (`gpio`).
    pub name: String,
    /// Core Wasm import module name (`wasmicon:hal/gpio@0.1.0`).
    pub module: String,
    pub docs: Vec<String>,
    pub enums: Vec<EnumDef>,
    pub flags: Vec<FlagsDef>,
    pub imports: Vec<Import>,
}

/// The whole HAL.
#[derive(Debug, Clone)]
pub struct Hal {
    /// `wasmicon:hal@0.1.0`
    pub package: String,
    pub interfaces: Vec<Iface>,
}

impl Hal {
    /// All imports of all interfaces, in declaration order.
    pub fn imports(&self) -> impl Iterator<Item = (&Iface, &Import)> {
        self.interfaces
            .iter()
            .flat_map(|i| i.imports.iter().map(move |f| (i, f)))
    }
}
