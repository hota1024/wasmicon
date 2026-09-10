//! デコード済みモジュールの表現。
//!
//! 命令列・名前・型のバイト列はモジュールのバイナリ（フラッシュ上）を
//! そのまま指す。索引が要るもの（型表、関数表など）だけを arena に置く。

use crate::types::{FuncType, GlobalType, Limits, ValType};

/// import の対象。
#[derive(Clone, Copy)]
pub enum ImportDesc {
    Func(u32),
    Table(Limits),
    Memory(Limits),
    Global(GlobalType),
}

impl Default for ImportDesc {
    fn default() -> Self {
        ImportDesc::Func(0)
    }
}

/// import 1 つ。
#[derive(Clone, Copy, Default)]
pub struct Import<'m> {
    pub module: &'m str,
    pub name: &'m str,
    pub desc: ImportDesc,
}

/// export の対象。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExportDesc {
    Func(u32),
    Table(u32),
    Memory(u32),
    Global(u32),
}

impl Default for ExportDesc {
    fn default() -> Self {
        ExportDesc::Func(0)
    }
}

/// export 1 つ。
#[derive(Clone, Copy, Default)]
pub struct Export<'m> {
    pub name: &'m str,
    pub desc: ExportDesc,
}

/// グローバル定義。初期化式はバイト列のまま持つ。
#[derive(Clone, Copy, Default)]
pub struct Global<'m> {
    pub ty: GlobalType,
    /// 定数式のバイト列（末尾の `end` を含まない）。
    pub init: &'m [u8],
}

/// 関数本体。
#[derive(Clone, Copy, Default)]
pub struct Code<'m> {
    /// ローカル宣言のバイト列（個数 LEB の後ろから、本体の直前まで）。
    pub locals: &'m [u8],
    /// ローカルの合計個数（引数を含まない）。
    pub local_count: u32,
    /// 命令列。末尾の `end` (0x0B) を含む。
    pub body: &'m [u8],
    /// モジュール先頭からの body のオフセット（トレース用）。
    pub body_offset: usize,
}

/// 要素セグメント。v0.1 は active のみ（table.* 命令が非対応のため）。
#[derive(Clone, Copy, Default)]
pub struct Elem<'m, 'a> {
    pub table: u32,
    /// オフセットを与える定数式。
    pub offset: &'m [u8],
    pub funcs: &'a [u32],
}

/// データセグメント。
#[derive(Clone, Copy, Default)]
pub struct Data<'m> {
    /// active なら (メモリ番号, オフセット定数式)。passive なら `None`。
    pub active: Option<(u32, &'m [u8])>,
    pub bytes: &'m [u8],
}

/// デコード済みモジュール。
pub struct Module<'m, 'a> {
    pub types: &'a [FuncType<'m>],
    pub imports: &'a [Import<'m>],
    /// 定義済み関数の型インデックス。import 由来は含まない。
    pub funcs: &'a [u32],
    pub tables: &'a [Limits],
    pub mems: &'a [Limits],
    pub globals: &'a [Global<'m>],
    pub exports: &'a [Export<'m>],
    pub start: Option<u32>,
    pub elems: &'a [Elem<'m, 'a>],
    pub datas: &'a [Data<'m>],
    pub code: &'a [Code<'m>],
    pub imported_funcs: u32,
    pub imported_tables: u32,
    pub imported_mems: u32,
    pub imported_globals: u32,
    /// import されたグローバルの型（定数式の検証で使う）。
    pub imported_global_types: &'a [GlobalType],
    /// DataCount セクションの値。`memory.init` / `data.drop` に必要。
    pub data_count: Option<u32>,
}

impl<'m> Module<'m, '_> {
    /// import を含む関数の総数。
    #[must_use]
    pub fn func_count(&self) -> u32 {
        self.imported_funcs + self.funcs.len() as u32
    }

    /// import を含むグローバルの総数。
    #[must_use]
    pub fn global_count(&self) -> u32 {
        self.imported_globals + self.globals.len() as u32
    }

    /// import を含むテーブルの総数。
    #[must_use]
    pub fn table_count(&self) -> u32 {
        self.imported_tables + self.tables.len() as u32
    }

    /// import を含むメモリの総数。
    #[must_use]
    pub fn mem_count(&self) -> u32 {
        self.imported_mems + self.mems.len() as u32
    }

    /// 関数インデックスから型を引く。
    #[must_use]
    pub fn func_type(&self, idx: u32) -> Option<FuncType<'m>> {
        let type_idx = if idx < self.imported_funcs {
            let mut seen = 0;
            let mut found = None;
            for im in self.imports {
                if let ImportDesc::Func(t) = im.desc {
                    if seen == idx {
                        found = Some(t);
                        break;
                    }
                    seen += 1;
                }
            }
            found?
        } else {
            *self.funcs.get((idx - self.imported_funcs) as usize)?
        };
        self.types.get(type_idx as usize).copied()
    }

    /// グローバルインデックスから型を引く。
    #[must_use]
    pub fn global_type(&self, idx: u32) -> Option<GlobalType> {
        if idx < self.imported_globals {
            self.imported_global_types.get(idx as usize).copied()
        } else {
            self.globals
                .get((idx - self.imported_globals) as usize)
                .map(|g| g.ty)
        }
    }

    /// 名前から export を引く。
    #[must_use]
    pub fn export(&self, name: &str) -> Option<ExportDesc> {
        self.exports.iter().find(|e| e.name == name).map(|e| e.desc)
    }

    /// ローカルの型を順に返す（検証で使う）。
    pub fn locals_of(&self, code: &Code<'m>) -> LocalIter<'m> {
        LocalIter {
            bytes: code.locals,
            pos: 0,
            remaining: 0,
            ty: ValType::I32,
        }
    }
}

/// ローカル宣言 `(count, type)*` を 1 個ずつ展開する。
pub struct LocalIter<'m> {
    bytes: &'m [u8],
    pos: usize,
    remaining: u32,
    ty: ValType,
}

impl Iterator for LocalIter<'_> {
    type Item = ValType;

    fn next(&mut self) -> Option<ValType> {
        while self.remaining == 0 {
            // 次のグループ。`decode` が検証済みなのでここでは失敗しない前提。
            let mut r = crate::reader::Reader::new(&self.bytes[self.pos..]);
            let n = r.u32_leb().ok()?;
            let t = ValType::from_byte(r.u8().ok()?).ok()?;
            self.pos += r.pos();
            if n == 0 {
                continue;
            }
            self.remaining = n;
            self.ty = t;
        }
        self.remaining -= 1;
        Some(self.ty)
    }
}
