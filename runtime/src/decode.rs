//! セクション解析。バイナリを読んで `Module` を組み立てる。
//!
//! ここで返すエラーは全て `Error::Malformed`（構文の不正）か
//! `Error::Invalid`（形だけで分かる意味の不正）。型検査は `validate` の仕事。

use crate::arena::Arena;
use crate::error::{Error, Result};
use crate::module::{Code, Data, Elem, Export, ExportDesc, Global, Import, ImportDesc, Module};
use crate::reader::Reader;
use crate::types::{FuncType, GlobalType, Limits, ValType};

const MAGIC: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];
const VERSION: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

/// メモリの最大ページ数（仕様上の上限）。
pub const MAX_MEMORY_PAGES: u32 = 65536;
/// テーブルの最大要素数（仕様上の上限）。
pub const MAX_TABLE_ELEMS: u32 = 0xffff_ffff;

/// セクション ID から並び順の順位へ。custom (0) は任意位置。
const fn section_rank(id: u8) -> Option<u8> {
    match id {
        1 => Some(1),   // type
        2 => Some(2),   // import
        3 => Some(3),   // function
        4 => Some(4),   // table
        5 => Some(5),   // memory
        6 => Some(6),   // global
        7 => Some(7),   // export
        8 => Some(8),   // start
        9 => Some(9),   // element
        12 => Some(10), // data count
        10 => Some(11), // code
        11 => Some(12), // data
        _ => None,
    }
}

/// モジュールバイナリをデコードする。
pub fn decode<'m, 'a>(bytes: &'m [u8], arena: &mut Arena<'a>) -> Result<Module<'m, 'a>> {
    let mut r = Reader::new(bytes);

    if r.remaining() < 4 || r.bytes(4)? != MAGIC {
        return Err(Error::Malformed("magic header not detected"));
    }
    if r.remaining() < 4 || r.bytes(4)? != VERSION {
        return Err(Error::Malformed("unknown binary version"));
    }

    let mut m = Module {
        types: &[],
        imports: &[],
        funcs: &[],
        tables: &[],
        mems: &[],
        globals: &[],
        exports: &[],
        start: None,
        elems: &[],
        datas: &[],
        code: &[],
        imported_funcs: 0,
        imported_tables: 0,
        imported_mems: 0,
        imported_globals: 0,
        imported_global_types: &[],
        data_count: None,
    };
    let mut last_rank = 0u8;
    let mut data_count: Option<u32> = None;

    while !r.is_empty() {
        let id = r.u8()?;
        let size = r.u32_leb()? as usize;
        let body = r.bytes(size)?;
        let mut s = Reader::new(body);

        if id == 0 {
            // custom セクション。名前が UTF-8 であることだけ確かめて読み飛ばす。
            let _ = s.name()?;
            continue;
        }

        if id == 13 {
            return Err(Error::Unsupported("exception handling is not supported"));
        }
        let rank = section_rank(id).ok_or(Error::Malformed("malformed section id"))?;
        if rank <= last_rank {
            return Err(Error::Malformed("unexpected content after last section"));
        }
        last_rank = rank;

        match id {
            1 => m.types = decode_types(&mut s, arena)?,
            2 => {
                let (imports, counts) = decode_imports(&mut s, arena)?;
                m.imports = imports;
                m.imported_funcs = counts.0;
                m.imported_tables = counts.1;
                m.imported_mems = counts.2;
                m.imported_globals = counts.3;
                m.imported_global_types = decode_imported_global_types(imports, arena)?;
            }
            3 => m.funcs = decode_u32_vec(&mut s, arena)?,
            4 => m.tables = decode_tables(&mut s, arena)?,
            5 => m.mems = decode_mems(&mut s, arena)?,
            6 => m.globals = decode_globals(&mut s, arena)?,
            7 => m.exports = decode_exports(&mut s, arena)?,
            8 => m.start = Some(s.u32_leb()?),
            9 => m.elems = decode_elems(&mut s, arena)?,
            12 => data_count = Some(s.u32_leb()?),
            10 => {
                let off = body.as_ptr() as usize - bytes.as_ptr() as usize;
                m.code = decode_code(&mut s, arena, off)?;
            }
            11 => m.datas = decode_datas(&mut s, arena)?,
            _ => unreachable!(),
        }

        if !s.is_empty() {
            return Err(Error::Malformed("section size mismatch"));
        }
    }

    if m.code.len() != m.funcs.len() {
        return Err(Error::Malformed(
            "function and code section have inconsistent lengths",
        ));
    }
    if let Some(n) = data_count
        && n as usize != m.datas.len()
    {
        return Err(Error::Malformed(
            "data count and data section have inconsistent lengths",
        ));
    }
    m.data_count = data_count;

    Ok(m)
}

fn decode_types<'m, 'a>(r: &mut Reader<'m>, arena: &mut Arena<'a>) -> Result<&'a [FuncType<'m>]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, FuncType::default())?;
    for slot in out.iter_mut() {
        if r.u8()? != 0x60 {
            return Err(Error::Malformed("integer representation too long"));
        }
        let params = decode_valtypes(r)?;
        let results = decode_valtypes(r)?;
        *slot = FuncType::new(params, results);
    }
    Ok(out)
}

/// 値型のベクタ。対応機能セットでは 1 型 = 1 バイトなので、
/// 検査したうえでバイト列をそのまま返す。
fn decode_valtypes<'m>(r: &mut Reader<'m>) -> Result<&'m [u8]> {
    let n = r.u32_leb()? as usize;
    let bytes = r.bytes(n)?;
    for b in bytes {
        ValType::from_byte(*b)?;
    }
    Ok(bytes)
}

type ImportCounts = (u32, u32, u32, u32);

fn decode_imports<'m, 'a>(
    r: &mut Reader<'m>,
    arena: &mut Arena<'a>,
) -> Result<(&'a [Import<'m>], ImportCounts)> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, Import::default())?;
    let mut counts = (0u32, 0u32, 0u32, 0u32);
    for slot in out.iter_mut() {
        let module = r.name()?;
        let name = r.name()?;
        let desc = match r.u8()? {
            0x00 => {
                counts.0 += 1;
                ImportDesc::Func(r.u32_leb()?)
            }
            0x01 => {
                counts.1 += 1;
                ImportDesc::Table(decode_table_type(r)?)
            }
            0x02 => {
                counts.2 += 1;
                ImportDesc::Memory(Limits::decode(
                    r,
                    MAX_MEMORY_PAGES,
                    "memory size must be at most 65536 pages (4GiB)",
                )?)
            }
            0x03 => {
                counts.3 += 1;
                ImportDesc::Global(GlobalType::decode(r)?)
            }
            _ => return Err(Error::Malformed("malformed import kind")),
        };
        *slot = Import { module, name, desc };
    }
    Ok((out, counts))
}

fn decode_imported_global_types<'a>(
    imports: &[Import<'_>],
    arena: &mut Arena<'a>,
) -> Result<&'a [GlobalType]> {
    let n = imports
        .iter()
        .filter(|i| matches!(i.desc, ImportDesc::Global(_)))
        .count();
    let out = arena.alloc(n, GlobalType::default())?;
    let mut i = 0;
    for im in imports {
        if let ImportDesc::Global(g) = im.desc {
            out[i] = g;
            i += 1;
        }
    }
    Ok(out)
}

fn decode_u32_vec<'a>(r: &mut Reader<'_>, arena: &mut Arena<'a>) -> Result<&'a [u32]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, 0u32)?;
    for slot in out.iter_mut() {
        *slot = r.u32_leb()?;
    }
    Ok(out)
}

fn decode_table_type(r: &mut Reader<'_>) -> Result<Limits> {
    // 要素型は funcref のみ（reference-types 非対応）。
    match r.u8()? {
        0x70 => {}
        0x40 => {
            return Err(Error::Unsupported(
                "table with initializer is not supported",
            ));
        }
        0x63..=0x6f | 0x71..=0x7a => {
            return Err(Error::Unsupported("reference types are not supported"));
        }
        _ => return Err(Error::Malformed("malformed reference type")),
    }
    Limits::decode(r, MAX_TABLE_ELEMS, "table size")
}

fn decode_tables<'a>(r: &mut Reader<'_>, arena: &mut Arena<'a>) -> Result<&'a [Limits]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, Limits::default())?;
    for slot in out.iter_mut() {
        *slot = decode_table_type(r)?;
    }
    Ok(out)
}

fn decode_mems<'a>(r: &mut Reader<'_>, arena: &mut Arena<'a>) -> Result<&'a [Limits]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, Limits::default())?;
    for slot in out.iter_mut() {
        *slot = Limits::decode(
            r,
            MAX_MEMORY_PAGES,
            "memory size must be at most 65536 pages (4GiB)",
        )?;
    }
    Ok(out)
}

fn decode_globals<'m, 'a>(r: &mut Reader<'m>, arena: &mut Arena<'a>) -> Result<&'a [Global<'m>]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, Global::default())?;
    for slot in out.iter_mut() {
        let ty = GlobalType::decode(r)?;
        let init = const_expr(r)?;
        *slot = Global { ty, init };
    }
    Ok(out)
}

fn decode_exports<'m, 'a>(r: &mut Reader<'m>, arena: &mut Arena<'a>) -> Result<&'a [Export<'m>]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, Export::default())?;
    for slot in out.iter_mut() {
        let name = r.name()?;
        let idx_kind = r.u8()?;
        let idx = r.u32_leb()?;
        let desc = match idx_kind {
            0x00 => ExportDesc::Func(idx),
            0x01 => ExportDesc::Table(idx),
            0x02 => ExportDesc::Memory(idx),
            0x03 => ExportDesc::Global(idx),
            _ => return Err(Error::Malformed("malformed export kind")),
        };
        *slot = Export { name, desc };
    }
    Ok(out)
}

fn decode_elems<'m, 'a>(r: &mut Reader<'m>, arena: &mut Arena<'a>) -> Result<&'a [Elem<'m, 'a>]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, Elem::default())?;
    for slot in out.iter_mut() {
        let flags = r.u32_leb()?;
        let (table, offset) = match flags {
            0 => (0u32, const_expr(r)?),
            2 => {
                let t = r.u32_leb()?;
                (t, const_expr(r)?)
            }
            // passive / declarative は table.init / elem.drop が非対応なので使い道が無い。
            1 | 3 => {
                return Err(Error::Unsupported(
                    "passive element segments are not supported",
                ));
            }
            _ => {
                return Err(Error::Unsupported(
                    "element segments with expressions are not supported",
                ));
            }
        };
        if flags == 2 {
            // elemkind。0x00 = funcref のみ。
            if r.u8()? != 0x00 {
                return Err(Error::Malformed("malformed element kind"));
            }
        }
        let funcs = decode_u32_vec(r, arena)?;
        *slot = Elem {
            table,
            offset,
            funcs,
        };
    }
    Ok(out)
}

fn decode_datas<'m, 'a>(r: &mut Reader<'m>, arena: &mut Arena<'a>) -> Result<&'a [Data<'m>]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, Data::default())?;
    for slot in out.iter_mut() {
        let flags = r.u32_leb()?;
        let active = match flags {
            0 => Some((0u32, const_expr(r)?)),
            1 => None,
            2 => {
                let mem = r.u32_leb()?;
                Some((mem, const_expr(r)?))
            }
            _ => return Err(Error::Malformed("malformed data segment kind")),
        };
        let len = r.u32_leb()? as usize;
        let bytes = r.bytes(len)?;
        *slot = Data { active, bytes };
    }
    Ok(out)
}

fn decode_code<'m, 'a>(
    r: &mut Reader<'m>,
    arena: &mut Arena<'a>,
    section_offset: usize,
) -> Result<&'a [Code<'m>]> {
    let n = r.u32_leb()? as usize;
    let out = arena.alloc(n, Code::default())?;
    for slot in out.iter_mut() {
        let size = r.u32_leb()? as usize;
        let start = r.pos();
        let body_all = r.bytes(size)?;
        let mut b = Reader::new(body_all);

        let groups = b.u32_leb()?;
        let locals_start = b.pos();
        let mut local_count: u32 = 0;
        for _ in 0..groups {
            let cnt = b.u32_leb()?;
            ValType::from_byte(b.u8()?)?;
            local_count = local_count
                .checked_add(cnt)
                .ok_or(Error::Malformed("too many locals"))?;
        }
        let locals = &body_all[locals_start..b.pos()];
        let body = b.rest();
        *slot = Code {
            locals,
            local_count,
            body,
            body_offset: section_offset + start + b.pos(),
        };
    }
    Ok(out)
}

/// 定数式のバイト列を切り出す（末尾の `end` は含めない）。
/// 型の整合は `validate` が見る。
fn const_expr<'m>(r: &mut Reader<'m>) -> Result<&'m [u8]> {
    let start = r.pos();
    let mut end_at = start;
    loop {
        let op = r.u8()?;
        match op {
            0x0b => break,
            0x41 => {
                r.i32_leb()?;
            }
            0x42 => {
                r.i64_leb()?;
            }
            0x43 => {
                r.f32_bits()?;
            }
            0x44 => {
                r.f64_bits()?;
            }
            0x23 => {
                r.u32_leb()?;
            }
            // extended-const 提案の算術。Wasm としては正しいが対応機能セット外。
            0x6a..=0x6c | 0x7c..=0x7e => {
                return Err(Error::Unsupported(
                    "extended constant expressions are not supported",
                ));
            }
            _ => return Err(Error::Invalid("constant expression required")),
        }
        end_at = r.pos();
    }
    // start..end_at が `end` を除いた本体。
    Ok(r.slice(start, end_at))
}
