//! ABI のモデル。`docs/abi-spec.md` の lowering 規則を適用した結果を保持する。
//!
//! WIT の型はここで「Core Wasm の引数列」まで落ちきっており、
//! 各エミッタは言語ごとの表記に変換するだけでよい。

/// Core Wasm の値型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmTy {
    I32,
    I64,
    F32,
    F64,
}

impl WasmTy {
    /// `tools/wit2sig.py` と同じシグネチャ表記の 1 文字。
    pub fn sig_char(self) -> char {
        match self {
            WasmTy::I32 => 'i',
            WasmTy::I64 => 'I',
            WasmTy::F32 => 'f',
            WasmTy::F64 => 'F',
        }
    }
}

/// abi-spec §4.1 のスカラー。`enum` / `flags` は WIT 上の型名を持つ。
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

    /// バインディングの extern 宣言で使う Rust 型。enum / flags は生の `u32`。
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

    /// AssemblyScript 型。
    pub fn assemblyscript(&self) -> &'static str {
        self.rust()
    }
}

/// Core Wasm の引数 1 つが何を表すか。abi-spec §4.2 / §4.4。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// resource ハンドル（`self` を含む）。
    Handle,
    /// スカラー・enum・flags。
    Scalar(Scalar),
    /// `list<u8>` / `string` 引数のポインタ。
    DataPtr,
    /// `list<u8>` / `string` 引数の長さ。
    DataLen,
    /// `result<T,_>` の T が スカラー・enum・flags のときの out ポインタ。
    OutScalar(Scalar),
    /// `result<T,_>` の T が resource ハンドルのときの out ポインタ。
    OutHandle,
    /// `result<list<u8>,_>` の書き込み先バッファ。
    OutBuf,
    /// `result<list<u8>,_>` のバッファ容量。
    OutCap,
    /// `result<list<u8>,_>` の書き込みバイト数の out ポインタ。
    OutLen,
}

/// lowering 済みの引数 1 つ。
#[derive(Debug, Clone)]
pub struct Param {
    /// kebab-case の論理名（`self`, `data-ptr`, `len-out` など）。
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

    /// バインディングの extern 宣言で使う Rust 型。
    pub fn rust(&self) -> String {
        match &self.role {
            Role::Handle | Role::DataLen | Role::OutCap => "u32".to_string(),
            Role::Scalar(s) => s.rust().to_string(),
            Role::DataPtr => "*const u8".to_string(),
            Role::OutHandle | Role::OutLen => "*mut u32".to_string(),
            Role::OutBuf => "*mut u8".to_string(),
            Role::OutScalar(s) => format!("*mut {}", s.rust()),
        }
    }

    /// AssemblyScript 型。ポインタは全て `usize`。
    pub fn assemblyscript(&self) -> String {
        match &self.role {
            Role::Handle | Role::DataLen | Role::OutCap => "u32".to_string(),
            Role::Scalar(s) => s.assemblyscript().to_string(),
            Role::DataPtr | Role::OutHandle | Role::OutLen | Role::OutBuf | Role::OutScalar(_) => {
                "usize".to_string()
            }
        }
    }
}

/// 関数の戻り値。
#[derive(Debug, Clone)]
pub enum Ret {
    /// 戻り値なし。
    None,
    /// abi-spec §4.4 の単一 i32 ステータス。
    Status,
    /// `result` を経由しないスカラー戻り値。
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
}

/// Core Wasm の import 1 つ。
#[derive(Debug, Clone)]
pub struct Import {
    /// import フィールド名（`[static]pin.open` など。abi-spec §3.2）。
    pub name: String,
    pub docs: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Ret,
    /// `wasmicon-core` 側のスロット識別子（`GpioPinOpen`）。
    pub slot: String,
    /// Rust バインディングの関数名（`pin_open`）。
    pub rust_name: String,
    /// AssemblyScript バインディングの関数名（`gpio_pin_open`）。
    pub as_name: String,
}

impl Import {
    /// `tools/wit2sig.py` と同じ `params:results` 表記。
    pub fn sig(&self) -> String {
        let mut s: String = self.params.iter().map(|p| p.wasm().sig_char()).collect();
        s.push(':');
        if let Some(r) = self.ret.wasm() {
            s.push(r.sig_char());
        }
        s
    }
}

/// enum のケース。
#[derive(Debug, Clone)]
pub struct EnumCase {
    pub docs: Vec<String>,
    /// PascalCase（Rust / AssemblyScript 共通）。
    pub ident: String,
}

/// enum 定義。
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub docs: Vec<String>,
    /// WIT 上の名前（`pin-mode`）。
    pub wit_name: String,
    /// Rust の型名（`PinMode`）。
    pub rust_name: String,
    /// AssemblyScript の型名（`GpioPinMode`）。インターフェース名で前置する。
    pub as_name: String,
    pub cases: Vec<EnumCase>,
}

/// flags の 1 ビット。
#[derive(Debug, Clone)]
pub struct FlagBit {
    pub docs: Vec<String>,
    /// PascalCase（Rust / AssemblyScript 共通）。
    pub ident: String,
}

/// flags 定義。abi-spec §4.1 により宣言順にビット 0 から割り当てる。
#[derive(Debug, Clone)]
pub struct FlagsDef {
    pub docs: Vec<String>,
    /// WIT 上の名前。
    pub wit_name: String,
    /// Rust の型名。
    pub rust_name: String,
    /// AssemblyScript の定数名の前置。
    pub as_name: String,
    pub bits: Vec<FlagBit>,
}

/// 1 インターフェース。
#[derive(Debug, Clone)]
pub struct Iface {
    /// WIT 上の名前（`gpio`）。
    pub name: String,
    /// Core Wasm の import モジュール名（`wasmicon:hal/gpio@0.1.0`）。
    pub module: String,
    pub docs: Vec<String>,
    pub enums: Vec<EnumDef>,
    pub flags: Vec<FlagsDef>,
    pub imports: Vec<Import>,
}

/// HAL 全体。
#[derive(Debug, Clone)]
pub struct Hal {
    /// `wasmicon:hal@0.1.0`
    pub package: String,
    pub interfaces: Vec<Iface>,
}

impl Hal {
    /// 全インターフェースの import を宣言順に並べたもの。
    pub fn imports(&self) -> impl Iterator<Item = (&Iface, &Import)> {
        self.interfaces
            .iter()
            .flat_map(|i| i.imports.iter().map(move |f| (i, f)))
    }
}
