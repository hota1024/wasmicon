//! インタプリタ本体。検証済みバイトコードを直接実行する。
//!
//! - 制御スタックは持たない。分岐は `validate` が作った side table を引く
//! - Rust の再帰を使わない。呼び出しは明示的なフレームスタックで表現し、
//!   深さの上限を超えたら `Exhausted` を返す（`assert_exhaustion` はこれ）
//! - オペランドは型なしの 64 ビットスロット。型は検証で保証済み
//! - 算術は全て wrapping。`wrapping_*` を明示的に使う（CLAUDE.md）

use crate::arena::Arena;
use crate::config::Config;
use crate::error::{Error, Result, Trap};
use crate::float;
use crate::instance::{Instance, Memory, NO_FUNC, Resolver};
use crate::validate::FuncInfo;

/// 呼び出しフレーム。
#[derive(Clone, Copy, Default)]
struct Frame {
    func: u32,
    pc: u32,
    locals: u32,
    ret: u32,
}

/// 実行用の作業領域。ポートが arena から確保して使い回す。
pub struct Exec<'a> {
    stack: &'a mut [u64],
    frames: &'a mut [Frame],
}

impl<'a> Exec<'a> {
    /// 設定に従って確保する。
    pub fn new(cfg: &Config, arena: &mut Arena<'a>) -> Result<Self> {
        Ok(Exec {
            stack: arena.alloc(cfg.operand_stack_slots, 0u64)?,
            frames: arena.alloc(cfg.max_call_depth, Frame::default())?,
        })
    }
}

// ---------------------------------------------------------------- 読み出し補助

/// 検証済みなので LEB は必ず正しい。範囲外だけ 0 で受け止める。
#[inline]
fn leb_u32(body: &[u8], pc: &mut usize) -> u32 {
    let mut r = 0u32;
    let mut s = 0u32;
    loop {
        let b = *body.get(*pc).unwrap_or(&0);
        *pc += 1;
        r |= u32::from(b & 0x7f) << s;
        if b & 0x80 == 0 {
            return r;
        }
        s += 7;
        if s >= 32 {
            return r;
        }
    }
}

#[inline]
fn leb_u64(body: &[u8], pc: &mut usize) -> u64 {
    let mut r = 0u64;
    let mut s = 0u32;
    loop {
        let b = *body.get(*pc).unwrap_or(&0);
        *pc += 1;
        r |= u64::from(b & 0x7f) << s;
        if b & 0x80 == 0 {
            return r;
        }
        s += 7;
        if s >= 64 {
            return r;
        }
    }
}

#[inline]
fn leb_i32(body: &[u8], pc: &mut usize) -> i32 {
    let mut r = 0u32;
    let mut s = 0u32;
    loop {
        let b = *body.get(*pc).unwrap_or(&0);
        *pc += 1;
        r |= u32::from(b & 0x7f) << s;
        s += 7;
        if b & 0x80 == 0 {
            if b & 0x40 != 0 && s < 32 {
                r |= u32::MAX << s;
            }
            return r as i32;
        }
        if s >= 32 {
            return r as i32;
        }
    }
}

#[inline]
fn leb_i64(body: &[u8], pc: &mut usize) -> i64 {
    let mut r = 0u64;
    let mut s = 0u32;
    loop {
        let b = *body.get(*pc).unwrap_or(&0);
        *pc += 1;
        r |= u64::from(b & 0x7f) << s;
        s += 7;
        if b & 0x80 == 0 {
            if b & 0x40 != 0 && s < 64 {
                r |= u64::MAX << s;
            }
            return r as i64;
        }
        if s >= 64 {
            return r as i64;
        }
    }
}

/// ブロック型の即値を読み飛ばす。
#[inline]
fn skip_block_type(body: &[u8], pc: &mut usize) {
    let b = *body.get(*pc).unwrap_or(&0);
    if b == 0x40 || (0x7c..=0x7f).contains(&b) {
        *pc += 1;
    } else {
        leb_i64(body, pc);
    }
}

/// `memarg` を読む。align は使わないので捨てる。
#[inline]
fn memarg(body: &[u8], pc: &mut usize) -> u32 {
    leb_u32(body, pc);
    leb_u64(body, pc) as u32
}

// ---------------------------------------------------------------- メモリ

#[inline]
fn load_at<'x>(mem: &'x Memory<'_>, addr: u32, offset: u32, n: usize) -> Result<&'x [u8]> {
    let a = u64::from(addr) + u64::from(offset);
    let end = a + n as u64;
    if end > mem.len() as u64 {
        return Err(Error::Trap(Trap::MemoryOutOfBounds));
    }
    Ok(&mem.bytes()[a as usize..end as usize])
}

#[inline]
fn store_at(mem: &mut Memory<'_>, addr: u32, offset: u32, src: &[u8]) -> Result<()> {
    let a = u64::from(addr) + u64::from(offset);
    let end = a + src.len() as u64;
    if end > mem.len() as u64 {
        return Err(Error::Trap(Trap::MemoryOutOfBounds));
    }
    mem.bytes_mut()[a as usize..end as usize].copy_from_slice(src);
    Ok(())
}

/// 浮動小数から整数へのトラップつき変換。範囲外・NaN はトラップ。
/// `lo` は含み、`hi` は含まない。境界は 2 のべき乗なので f64 で厳密に表せる。
#[inline]
fn trunc_check(x: f64, lo: f64, hi: f64) -> Result<f64> {
    if x.is_nan() {
        return Err(Error::Trap(Trap::InvalidConversionToInteger));
    }
    let t = float::trunc_f64(x);
    if !(t >= lo && t < hi) {
        return Err(Error::Trap(Trap::IntegerOverflow));
    }
    Ok(t)
}

// ---------------------------------------------------------------- 実行

/// export された関数を呼ぶ。
///
/// `args` / `results` は型なしのスロット。i32 は下位 32 ビット、
/// f32 はビットパターンを下位 32 ビットに入れる。
pub fn invoke(
    inst: &mut Instance<'_, '_>,
    exec: &mut Exec<'_>,
    resolver: &mut dyn Resolver,
    func: u32,
    args: &[u64],
    results: &mut [u64],
) -> Result<()> {
    let ty = inst
        .module
        .func_type(func)
        .ok_or(Error::Invalid("unknown function"))?;
    if args.len() != ty.param_count() || results.len() < ty.result_count() {
        return Err(Error::Invalid("wrong arity"));
    }

    let stack = &mut *exec.stack;
    let frames = &mut *exec.frames;
    if args.len() > stack.len() {
        return Err(Error::Exhausted("operand stack overflow"));
    }
    stack[..args.len()].copy_from_slice(args);
    let mut sp = args.len();

    // ホスト関数を直接呼ぶ場合。
    if func < inst.module.imported_funcs {
        let host = inst.host_funcs[func as usize];
        let nres = ty.result_count();
        let (a, rest) = stack.split_at_mut(sp);
        let mem: &mut [u8] = match inst.memory.as_mut() {
            Some(m) => m.bytes_mut(),
            None => &mut [],
        };
        resolver.call(host, &a[sp - args.len()..], &mut rest[..nres], mem)?;
        results[..nres].copy_from_slice(&rest[..nres]);
        return Ok(());
    }

    let mut nframes = 0usize;
    let mut cur = enter(inst, stack, &mut sp, func, args.len(), &mut nframes, frames)?;

    run(
        inst,
        resolver,
        stack,
        frames,
        &mut sp,
        &mut nframes,
        &mut cur,
    )?;

    let n = ty.result_count();
    results[..n].copy_from_slice(&stack[sp - n..sp]);
    Ok(())
}

/// 実行中の関数の位置。
#[derive(Clone, Copy)]
struct Cursor<'m> {
    func: u32,
    body: &'m [u8],
    pc: usize,
    info: FuncInfo,
    locals: usize,
    ret: usize,
}

/// 関数に入る。引数は既にスタック上位に積まれている。
fn enter<'m>(
    inst: &Instance<'m, '_>,
    stack: &mut [u64],
    sp: &mut usize,
    func: u32,
    nparams: usize,
    nframes: &mut usize,
    frames: &mut [Frame],
) -> Result<Cursor<'m>> {
    if *nframes >= frames.len() {
        return Err(Error::Exhausted("call stack exhausted"));
    }
    let defined = (func - inst.module.imported_funcs) as usize;
    let info = *inst
        .validated
        .funcs
        .get(defined)
        .ok_or(Error::Invalid("unknown function"))?;
    let code = *inst
        .module
        .code
        .get(defined)
        .ok_or(Error::Invalid("unknown function"))?;
    let ty = inst
        .module
        .func_type(func)
        .ok_or(Error::Invalid("unknown function"))?;

    let locals = *sp - nparams;
    let slots = info.frame_slots as usize;
    // このフレームで積める最大量を先に確保できているか確かめる。
    // ここを通れば以降の push とローカル書き込みは範囲内に収まる。
    if locals + slots + info.max_stack as usize > stack.len() {
        return Err(Error::Exhausted("operand stack overflow"));
    }
    // 宣言されたローカルは 0 で初期化する。
    for i in nparams..slots {
        stack[locals + i] = 0;
    }
    *sp = locals + slots;

    Ok(Cursor {
        func,
        body: code.body,
        pc: 0,
        info,
        locals,
        ret: ty.result_count(),
    })
}

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
fn run<'m>(
    inst: &mut Instance<'m, '_>,
    resolver: &mut dyn Resolver,
    stack: &mut [u64],
    frames: &mut [Frame],
    sp_out: &mut usize,
    nframes: &mut usize,
    cur_out: &mut Cursor<'m>,
) -> Result<()> {
    let mut sp = *sp_out;
    let mut cur = *cur_out;

    loop {
        let pc0 = cur.pc;
        let op = *cur.body.get(cur.pc).unwrap_or(&0x0b);
        cur.pc += 1;

        macro_rules! pop {
            () => {{
                sp -= 1;
                stack[sp]
            }};
        }
        macro_rules! push {
            ($v:expr) => {{
                stack[sp] = $v;
                sp += 1;
            }};
        }
        macro_rules! mem {
            () => {
                inst.memory
                    .as_mut()
                    .ok_or(Error::Invalid("unknown memory"))?
            };
        }

        match op {
            // --- 制御 ---
            0x00 => return Err(Error::Trap(Trap::Unreachable)),
            0x01 => {}
            0x02 | 0x03 => skip_block_type(cur.body, &mut cur.pc),
            0x04 => {
                skip_block_type(cur.body, &mut cur.pc);
                let c = pop!() as u32;
                if c == 0 {
                    let b = inst
                        .validated
                        .branch(&cur.info, pc0 as u32, 0)
                        .ok_or(Error::Invalid("missing side table entry"))?;
                    cur.pc = b.target as usize;
                }
            }
            0x05 => {
                let b = inst
                    .validated
                    .branch(&cur.info, pc0 as u32, 0)
                    .ok_or(Error::Invalid("missing side table entry"))?;
                cur.pc = b.target as usize;
            }
            0x0b => {
                // 関数末尾の end だけが復帰。ブロックの end は何もしない。
                if cur.pc >= cur.body.len()
                    && finish_frame(inst, &mut cur, stack, &mut sp, frames, nframes)
                {
                    break;
                }
            }
            0x0c..=0x0e => {
                let n = match op {
                    0x0c => {
                        leb_u32(cur.body, &mut cur.pc);
                        0
                    }
                    0x0d => {
                        leb_u32(cur.body, &mut cur.pc);
                        let c = pop!() as u32;
                        if c == 0 {
                            continue;
                        }
                        0
                    }
                    _ => {
                        let count = leb_u32(cur.body, &mut cur.pc);
                        let idx = pop!() as u32;
                        if idx < count {
                            idx as usize
                        } else {
                            count as usize
                        }
                    }
                };
                let b = *inst
                    .validated
                    .branch(&cur.info, pc0 as u32, n)
                    .ok_or(Error::Invalid("missing side table entry"))?;
                do_branch(&mut cur, stack, &mut sp, b.target, b.keep, b.drop);
                if cur.pc >= cur.body.len()
                    && finish_frame(inst, &mut cur, stack, &mut sp, frames, nframes)
                {
                    break;
                }
            }
            0x0f => {
                // return: 戻り値だけ残してフレームを閉じる。
                let n = cur.ret;
                stack.copy_within(sp - n..sp, cur.locals);
                sp = cur.locals + n;
                if finish_return(inst, &mut cur, frames, nframes) {
                    break;
                }
            }
            0x10 => {
                let f = leb_u32(cur.body, &mut cur.pc);
                call(inst, resolver, stack, &mut sp, frames, nframes, &mut cur, f)?;
            }
            0x11 => {
                let t = leb_u32(cur.body, &mut cur.pc);
                leb_u32(cur.body, &mut cur.pc); // table index（検証済みで 0）
                let i = pop!() as u32;
                let slot = *inst
                    .table
                    .get(i as usize)
                    .ok_or(Error::Trap(Trap::TableOutOfBounds))?;
                if slot == NO_FUNC {
                    return Err(Error::Trap(Trap::UninitializedElement));
                }
                let want = inst
                    .module
                    .types
                    .get(t as usize)
                    .ok_or(Error::Invalid("unknown type"))?;
                let got = inst
                    .module
                    .func_type(slot)
                    .ok_or(Error::Trap(Trap::UndefinedElement))?;
                if !want.matches(&got) {
                    return Err(Error::Trap(Trap::IndirectCallTypeMismatch));
                }
                call(
                    inst, resolver, stack, &mut sp, frames, nframes, &mut cur, slot,
                )?;
            }

            // --- パラメトリック ---
            0x1a => {
                sp -= 1;
            }
            0x1b => {
                let c = pop!() as u32;
                let b = pop!();
                let a = pop!();
                push!(if c != 0 { a } else { b });
            }

            // --- 変数 ---
            0x20 => {
                let i = leb_u32(cur.body, &mut cur.pc) as usize;
                push!(stack[cur.locals + i]);
            }
            0x21 => {
                let i = leb_u32(cur.body, &mut cur.pc) as usize;
                let v = pop!();
                stack[cur.locals + i] = v;
            }
            0x22 => {
                let i = leb_u32(cur.body, &mut cur.pc) as usize;
                let v = stack[sp - 1];
                stack[cur.locals + i] = v;
            }
            0x23 => {
                let i = leb_u32(cur.body, &mut cur.pc) as usize;
                push!(inst.globals[i]);
            }
            0x24 => {
                let i = leb_u32(cur.body, &mut cur.pc) as usize;
                let v = pop!();
                inst.globals[i] = v;
            }

            // --- ロード ---
            0x28 => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 4)?;
                push!(u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            }
            0x29 => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 8)?;
                push!(u64::from_le_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]
                ]));
            }
            0x2a => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 4)?;
                push!(u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            }
            0x2b => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 8)?;
                push!(u64::from_le_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]
                ]));
            }
            0x2c => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 1)?;
                push!(u64::from(i32::from(b[0] as i8) as u32));
            }
            0x2d => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 1)?;
                push!(u64::from(b[0]));
            }
            0x2e => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 2)?;
                push!(u64::from(i32::from(i16::from_le_bytes([b[0], b[1]])) as u32));
            }
            0x2f => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 2)?;
                push!(u64::from(u16::from_le_bytes([b[0], b[1]])));
            }
            0x30 => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 1)?;
                push!(i64::from(b[0] as i8) as u64);
            }
            0x31 => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 1)?;
                push!(u64::from(b[0]));
            }
            0x32 => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 2)?;
                push!(i64::from(i16::from_le_bytes([b[0], b[1]])) as u64);
            }
            0x33 => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 2)?;
                push!(u64::from(u16::from_le_bytes([b[0], b[1]])));
            }
            0x34 => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 4)?;
                push!(i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) as u64);
            }
            0x35 => {
                let o = memarg(cur.body, &mut cur.pc);
                let a = pop!() as u32;
                let b = load_at(mem!(), a, o, 4)?;
                push!(u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            }

            // --- ストア ---
            0x36 | 0x38 => {
                let o = memarg(cur.body, &mut cur.pc);
                let v = pop!() as u32;
                let a = pop!() as u32;
                store_at(mem!(), a, o, &v.to_le_bytes())?;
            }
            0x37 | 0x39 => {
                let o = memarg(cur.body, &mut cur.pc);
                let v = pop!();
                let a = pop!() as u32;
                store_at(mem!(), a, o, &v.to_le_bytes())?;
            }
            0x3a | 0x3c => {
                let o = memarg(cur.body, &mut cur.pc);
                let v = pop!() as u8;
                let a = pop!() as u32;
                store_at(mem!(), a, o, &[v])?;
            }
            0x3b | 0x3d => {
                let o = memarg(cur.body, &mut cur.pc);
                let v = pop!() as u16;
                let a = pop!() as u32;
                store_at(mem!(), a, o, &v.to_le_bytes())?;
            }
            0x3e => {
                let o = memarg(cur.body, &mut cur.pc);
                let v = pop!() as u32;
                let a = pop!() as u32;
                store_at(mem!(), a, o, &v.to_le_bytes())?;
            }

            0x3f => {
                cur.pc += 1; // 0x00
                let p = mem!().pages();
                push!(u64::from(p));
            }
            0x40 => {
                cur.pc += 1; // 0x00
                let d = pop!() as u32;
                let r = mem!().grow(d).map_or(-1i32 as u32, |old| old);
                push!(u64::from(r));
            }

            // --- 定数 ---
            0x41 => {
                let v = leb_i32(cur.body, &mut cur.pc);
                push!(u64::from(v as u32));
            }
            0x42 => {
                let v = leb_i64(cur.body, &mut cur.pc);
                push!(v as u64);
            }
            0x43 => {
                let b = cur.body.get(cur.pc..cur.pc + 4).unwrap_or(&[0; 4]);
                cur.pc += 4;
                push!(u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            }
            0x44 => {
                let b = cur.body.get(cur.pc..cur.pc + 8).unwrap_or(&[0; 8]);
                cur.pc += 8;
                push!(u64::from_le_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]
                ]));
            }

            0xfc => {
                let sub = leb_u32(cur.body, &mut cur.pc);
                match sub {
                    // trunc_sat: Rust の `as` が仕様どおり飽和し NaN を 0 にする。
                    0 => {
                        let v = f32::from_bits(pop!() as u32);
                        push!(u64::from(v as i32 as u32));
                    }
                    1 => {
                        let v = f32::from_bits(pop!() as u32);
                        push!(u64::from(v as u32));
                    }
                    2 => {
                        let v = f64::from_bits(pop!());
                        push!(u64::from(v as i32 as u32));
                    }
                    3 => {
                        let v = f64::from_bits(pop!());
                        push!(u64::from(v as u32));
                    }
                    4 => {
                        let v = f32::from_bits(pop!() as u32);
                        push!(v as i64 as u64);
                    }
                    5 => {
                        let v = f32::from_bits(pop!() as u32);
                        push!(v as u64);
                    }
                    6 => {
                        let v = f64::from_bits(pop!());
                        push!(v as i64 as u64);
                    }
                    7 => {
                        let v = f64::from_bits(pop!());
                        push!(v as u64);
                    }
                    8 => {
                        // memory.init
                        let d = leb_u32(cur.body, &mut cur.pc) as usize;
                        cur.pc += 1; // 0x00
                        let n = pop!() as u32 as usize;
                        let src = pop!() as u32 as usize;
                        let dst = pop!() as u32 as usize;
                        let seg = if inst.dropped_data[d] {
                            &[][..]
                        } else {
                            inst.module.datas[d].bytes
                        };
                        if src + n > seg.len() {
                            return Err(Error::Trap(Trap::MemoryOutOfBounds));
                        }
                        let m = mem!();
                        if dst + n > m.len() {
                            return Err(Error::Trap(Trap::MemoryOutOfBounds));
                        }
                        m.bytes_mut()[dst..dst + n].copy_from_slice(&seg[src..src + n]);
                    }
                    9 => {
                        let d = leb_u32(cur.body, &mut cur.pc) as usize;
                        inst.dropped_data[d] = true;
                    }
                    10 => {
                        cur.pc += 2; // 0x00 0x00
                        let n = pop!() as u32 as usize;
                        let src = pop!() as u32 as usize;
                        let dst = pop!() as u32 as usize;
                        let m = mem!();
                        if src + n > m.len() || dst + n > m.len() {
                            return Err(Error::Trap(Trap::MemoryOutOfBounds));
                        }
                        m.bytes_mut().copy_within(src..src + n, dst);
                    }
                    11 => {
                        cur.pc += 1; // 0x00
                        let n = pop!() as u32 as usize;
                        let v = pop!() as u8;
                        let dst = pop!() as u32 as usize;
                        let m = mem!();
                        if dst + n > m.len() {
                            return Err(Error::Trap(Trap::MemoryOutOfBounds));
                        }
                        m.bytes_mut()[dst..dst + n].fill(v);
                    }
                    _ => return Err(Error::Unsupported("unsupported 0xFC opcode")),
                }
            }

            _ => {
                numeric(op, &mut sp, stack)?;
            }
        }
    }

    *sp_out = sp;
    *cur_out = cur;
    Ok(())
}

/// 分岐する。持ち越す値を下へ詰めてから飛ぶ。
#[inline]
fn do_branch(
    cur: &mut Cursor<'_>,
    stack: &mut [u64],
    sp: &mut usize,
    target: u32,
    keep: u32,
    drop: u32,
) {
    let keep = keep as usize;
    let drop = drop as usize;
    if drop > 0 && keep > 0 {
        stack.copy_within(*sp - keep..*sp, *sp - keep - drop);
    }
    *sp -= drop;
    cur.pc = target as usize;
}

/// 関数末尾の `end` に来たときの後始末。呼び出し元が無ければ `true`。
fn finish_frame<'m>(
    inst: &Instance<'m, '_>,
    cur: &mut Cursor<'m>,
    stack: &mut [u64],
    sp: &mut usize,
    frames: &[Frame],
    nframes: &mut usize,
) -> bool {
    let n = cur.ret;
    stack.copy_within(*sp - n..*sp, cur.locals);
    *sp = cur.locals + n;
    finish_return(inst, cur, frames, nframes)
}

/// フレームを 1 つ戻す。呼び出し元が無ければ `true`。
fn finish_return<'m>(
    inst: &Instance<'m, '_>,
    cur: &mut Cursor<'m>,
    frames: &[Frame],
    nframes: &mut usize,
) -> bool {
    if *nframes == 0 {
        return true;
    }
    *nframes -= 1;
    let f = frames[*nframes];
    let defined = (f.func - inst.module.imported_funcs) as usize;
    cur.func = f.func;
    cur.pc = f.pc as usize;
    cur.locals = f.locals as usize;
    cur.ret = f.ret as usize;
    cur.body = inst.module.code[defined].body;
    cur.info = inst.validated.funcs[defined];
    false
}

/// ホスト関数の引数・戻り値の最大個数。
const MAX_HOST_ARITY: usize = 16;

/// 関数を呼ぶ。ホスト関数ならその場で実行し、Wasm 関数ならフレームを積む。
#[allow(clippy::too_many_arguments)]
fn call<'m>(
    inst: &mut Instance<'m, '_>,
    resolver: &mut dyn Resolver,
    stack: &mut [u64],
    sp: &mut usize,
    frames: &mut [Frame],
    nframes: &mut usize,
    cur: &mut Cursor<'m>,
    f: u32,
) -> Result<()> {
    let ty = inst
        .module
        .func_type(f)
        .ok_or(Error::Invalid("unknown function"))?;
    let nparams = ty.param_count();
    let nresults = ty.result_count();

    if f < inst.module.imported_funcs {
        if nparams > MAX_HOST_ARITY || nresults > MAX_HOST_ARITY {
            return Err(Error::Unsupported("host function arity is too large"));
        }
        let mut args = [0u64; MAX_HOST_ARITY];
        let mut res = [0u64; MAX_HOST_ARITY];
        let base = *sp - nparams;
        args[..nparams].copy_from_slice(&stack[base..*sp]);
        let host = inst.host_funcs[f as usize];
        let mem: &mut [u8] = match inst.memory.as_mut() {
            Some(m) => m.bytes_mut(),
            None => &mut [],
        };
        resolver.call(host, &args[..nparams], &mut res[..nresults], mem)?;
        stack[base..base + nresults].copy_from_slice(&res[..nresults]);
        *sp = base + nresults;
        return Ok(());
    }

    if *nframes >= frames.len() {
        return Err(Error::Exhausted("call stack exhausted"));
    }
    frames[*nframes] = Frame {
        func: cur.func,
        pc: cur.pc as u32,
        locals: cur.locals as u32,
        ret: cur.ret as u32,
    };
    *nframes += 1;
    *cur = enter(inst, stack, sp, f, nparams, nframes, frames)?;
    Ok(())
}

/// 数値命令。スタックの上位だけを見るのでインスタンスに触らない。
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
fn numeric(op: u8, sp: &mut usize, stack: &mut [u64]) -> Result<()> {
    macro_rules! pop {
        () => {{
            *sp -= 1;
            stack[*sp]
        }};
    }
    macro_rules! push {
        ($v:expr) => {{
            stack[*sp] = $v;
            *sp += 1;
        }};
    }
    /// i32 の 2 項演算。
    macro_rules! i32_bin {
        (|$a:ident, $b:ident| $e:expr) => {{
            let $b = pop!() as u32;
            let $a = pop!() as u32;
            push!(u64::from($e));
        }};
    }
    macro_rules! i64_bin {
        (|$a:ident, $b:ident| $e:expr) => {{
            let $b = pop!();
            let $a = pop!();
            push!($e);
        }};
    }
    macro_rules! f32_bin {
        (|$a:ident, $b:ident| $e:expr) => {{
            let $b = f32::from_bits(pop!() as u32);
            let $a = f32::from_bits(pop!() as u32);
            let r: f32 = $e;
            push!(u64::from(r.to_bits()));
        }};
    }
    macro_rules! f64_bin {
        (|$a:ident, $b:ident| $e:expr) => {{
            let $b = f64::from_bits(pop!());
            let $a = f64::from_bits(pop!());
            let r: f64 = $e;
            push!(r.to_bits());
        }};
    }
    macro_rules! f32_un {
        (|$a:ident| $e:expr) => {{
            let $a = f32::from_bits(pop!() as u32);
            let r: f32 = $e;
            push!(u64::from(r.to_bits()));
        }};
    }
    macro_rules! f64_un {
        (|$a:ident| $e:expr) => {{
            let $a = f64::from_bits(pop!());
            let r: f64 = $e;
            push!(r.to_bits());
        }};
    }
    macro_rules! cmp {
        ($v:expr) => {{
            push!(u64::from(u32::from($v)));
        }};
    }

    match op {
        // --- i32 比較 ---
        0x45 => {
            let a = pop!() as u32;
            cmp!(a == 0);
        }
        0x46 => {
            let b = pop!() as u32;
            let a = pop!() as u32;
            cmp!(a == b);
        }
        0x47 => {
            let b = pop!() as u32;
            let a = pop!() as u32;
            cmp!(a != b);
        }
        0x48 => {
            let b = pop!() as i32;
            let a = pop!() as i32;
            cmp!(a < b);
        }
        0x49 => {
            let b = pop!() as u32;
            let a = pop!() as u32;
            cmp!(a < b);
        }
        0x4a => {
            let b = pop!() as i32;
            let a = pop!() as i32;
            cmp!(a > b);
        }
        0x4b => {
            let b = pop!() as u32;
            let a = pop!() as u32;
            cmp!(a > b);
        }
        0x4c => {
            let b = pop!() as i32;
            let a = pop!() as i32;
            cmp!(a <= b);
        }
        0x4d => {
            let b = pop!() as u32;
            let a = pop!() as u32;
            cmp!(a <= b);
        }
        0x4e => {
            let b = pop!() as i32;
            let a = pop!() as i32;
            cmp!(a >= b);
        }
        0x4f => {
            let b = pop!() as u32;
            let a = pop!() as u32;
            cmp!(a >= b);
        }

        // --- i64 比較 ---
        0x50 => {
            let a = pop!();
            cmp!(a == 0);
        }
        0x51 => {
            let b = pop!();
            let a = pop!();
            cmp!(a == b);
        }
        0x52 => {
            let b = pop!();
            let a = pop!();
            cmp!(a != b);
        }
        0x53 => {
            let b = pop!() as i64;
            let a = pop!() as i64;
            cmp!(a < b);
        }
        0x54 => {
            let b = pop!();
            let a = pop!();
            cmp!(a < b);
        }
        0x55 => {
            let b = pop!() as i64;
            let a = pop!() as i64;
            cmp!(a > b);
        }
        0x56 => {
            let b = pop!();
            let a = pop!();
            cmp!(a > b);
        }
        0x57 => {
            let b = pop!() as i64;
            let a = pop!() as i64;
            cmp!(a <= b);
        }
        0x58 => {
            let b = pop!();
            let a = pop!();
            cmp!(a <= b);
        }
        0x59 => {
            let b = pop!() as i64;
            let a = pop!() as i64;
            cmp!(a >= b);
        }
        0x5a => {
            let b = pop!();
            let a = pop!();
            cmp!(a >= b);
        }

        // --- f32 / f64 比較 ---
        0x5b => {
            let b = f32::from_bits(pop!() as u32);
            let a = f32::from_bits(pop!() as u32);
            cmp!(a == b);
        }
        0x5c => {
            let b = f32::from_bits(pop!() as u32);
            let a = f32::from_bits(pop!() as u32);
            cmp!(a != b);
        }
        0x5d => {
            let b = f32::from_bits(pop!() as u32);
            let a = f32::from_bits(pop!() as u32);
            cmp!(a < b);
        }
        0x5e => {
            let b = f32::from_bits(pop!() as u32);
            let a = f32::from_bits(pop!() as u32);
            cmp!(a > b);
        }
        0x5f => {
            let b = f32::from_bits(pop!() as u32);
            let a = f32::from_bits(pop!() as u32);
            cmp!(a <= b);
        }
        0x60 => {
            let b = f32::from_bits(pop!() as u32);
            let a = f32::from_bits(pop!() as u32);
            cmp!(a >= b);
        }
        0x61 => {
            let b = f64::from_bits(pop!());
            let a = f64::from_bits(pop!());
            cmp!(a == b);
        }
        0x62 => {
            let b = f64::from_bits(pop!());
            let a = f64::from_bits(pop!());
            cmp!(a != b);
        }
        0x63 => {
            let b = f64::from_bits(pop!());
            let a = f64::from_bits(pop!());
            cmp!(a < b);
        }
        0x64 => {
            let b = f64::from_bits(pop!());
            let a = f64::from_bits(pop!());
            cmp!(a > b);
        }
        0x65 => {
            let b = f64::from_bits(pop!());
            let a = f64::from_bits(pop!());
            cmp!(a <= b);
        }
        0x66 => {
            let b = f64::from_bits(pop!());
            let a = f64::from_bits(pop!());
            cmp!(a >= b);
        }

        // --- i32 算術 ---
        0x67 => {
            let a = pop!() as u32;
            push!(u64::from(a.leading_zeros()));
        }
        0x68 => {
            let a = pop!() as u32;
            push!(u64::from(a.trailing_zeros()));
        }
        0x69 => {
            let a = pop!() as u32;
            push!(u64::from(a.count_ones()));
        }
        0x6a => i32_bin!(|a, b| a.wrapping_add(b)),
        0x6b => i32_bin!(|a, b| a.wrapping_sub(b)),
        0x6c => i32_bin!(|a, b| a.wrapping_mul(b)),
        0x6d => {
            let b = pop!() as i32;
            let a = pop!() as i32;
            if b == 0 {
                return Err(Error::Trap(Trap::IntegerDivideByZero));
            }
            if a == i32::MIN && b == -1 {
                return Err(Error::Trap(Trap::IntegerOverflow));
            }
            push!(u64::from(a.wrapping_div(b) as u32));
        }
        0x6e => {
            let b = pop!() as u32;
            let a = pop!() as u32;
            if b == 0 {
                return Err(Error::Trap(Trap::IntegerDivideByZero));
            }
            push!(u64::from(a / b));
        }
        0x6f => {
            let b = pop!() as i32;
            let a = pop!() as i32;
            if b == 0 {
                return Err(Error::Trap(Trap::IntegerDivideByZero));
            }
            push!(u64::from(a.wrapping_rem(b) as u32));
        }
        0x70 => {
            let b = pop!() as u32;
            let a = pop!() as u32;
            if b == 0 {
                return Err(Error::Trap(Trap::IntegerDivideByZero));
            }
            push!(u64::from(a % b));
        }
        0x71 => i32_bin!(|a, b| a & b),
        0x72 => i32_bin!(|a, b| a | b),
        0x73 => i32_bin!(|a, b| a ^ b),
        0x74 => i32_bin!(|a, b| a.wrapping_shl(b)),
        0x75 => {
            let b = pop!() as u32;
            let a = pop!() as i32;
            push!(u64::from(a.wrapping_shr(b) as u32));
        }
        0x76 => i32_bin!(|a, b| a.wrapping_shr(b)),
        0x77 => i32_bin!(|a, b| a.rotate_left(b % 32)),
        0x78 => i32_bin!(|a, b| a.rotate_right(b % 32)),

        // --- i64 算術 ---
        0x79 => {
            let a = pop!();
            push!(u64::from(a.leading_zeros()));
        }
        0x7a => {
            let a = pop!();
            push!(u64::from(a.trailing_zeros()));
        }
        0x7b => {
            let a = pop!();
            push!(u64::from(a.count_ones()));
        }
        0x7c => i64_bin!(|a, b| a.wrapping_add(b)),
        0x7d => i64_bin!(|a, b| a.wrapping_sub(b)),
        0x7e => i64_bin!(|a, b| a.wrapping_mul(b)),
        0x7f => {
            let b = pop!() as i64;
            let a = pop!() as i64;
            if b == 0 {
                return Err(Error::Trap(Trap::IntegerDivideByZero));
            }
            if a == i64::MIN && b == -1 {
                return Err(Error::Trap(Trap::IntegerOverflow));
            }
            push!(a.wrapping_div(b) as u64);
        }
        0x80 => {
            let b = pop!();
            let a = pop!();
            if b == 0 {
                return Err(Error::Trap(Trap::IntegerDivideByZero));
            }
            push!(a / b);
        }
        0x81 => {
            let b = pop!() as i64;
            let a = pop!() as i64;
            if b == 0 {
                return Err(Error::Trap(Trap::IntegerDivideByZero));
            }
            push!(a.wrapping_rem(b) as u64);
        }
        0x82 => {
            let b = pop!();
            let a = pop!();
            if b == 0 {
                return Err(Error::Trap(Trap::IntegerDivideByZero));
            }
            push!(a % b);
        }
        0x83 => i64_bin!(|a, b| a & b),
        0x84 => i64_bin!(|a, b| a | b),
        0x85 => i64_bin!(|a, b| a ^ b),
        0x86 => i64_bin!(|a, b| a.wrapping_shl(b as u32)),
        0x87 => {
            let b = pop!();
            let a = pop!() as i64;
            push!(a.wrapping_shr(b as u32) as u64);
        }
        0x88 => i64_bin!(|a, b| a.wrapping_shr(b as u32)),
        0x89 => i64_bin!(|a, b| a.rotate_left((b % 64) as u32)),
        0x8a => i64_bin!(|a, b| a.rotate_right((b % 64) as u32)),

        // --- f32 ---
        0x8b => f32_un!(|a| float::abs_f32(a)),
        0x8c => f32_un!(|a| float::neg_f32(a)),
        0x8d => f32_un!(|a| float::ceil_f32(a)),
        0x8e => f32_un!(|a| float::floor_f32(a)),
        0x8f => f32_un!(|a| float::trunc_f32(a)),
        0x90 => f32_un!(|a| float::nearest_f32(a)),
        0x91 => f32_un!(|a| float::sqrt_f32(a)),
        0x92 => f32_bin!(|a, b| a + b),
        0x93 => f32_bin!(|a, b| a - b),
        0x94 => f32_bin!(|a, b| a * b),
        0x95 => f32_bin!(|a, b| a / b),
        0x96 => f32_bin!(|a, b| float::min_f32(a, b)),
        0x97 => f32_bin!(|a, b| float::max_f32(a, b)),
        0x98 => f32_bin!(|a, b| float::copysign_f32(a, b)),

        // --- f64 ---
        0x99 => f64_un!(|a| float::abs_f64(a)),
        0x9a => f64_un!(|a| float::neg_f64(a)),
        0x9b => f64_un!(|a| float::ceil_f64(a)),
        0x9c => f64_un!(|a| float::floor_f64(a)),
        0x9d => f64_un!(|a| float::trunc_f64(a)),
        0x9e => f64_un!(|a| float::nearest_f64(a)),
        0x9f => f64_un!(|a| float::sqrt_f64(a)),
        0xa0 => f64_bin!(|a, b| a + b),
        0xa1 => f64_bin!(|a, b| a - b),
        0xa2 => f64_bin!(|a, b| a * b),
        0xa3 => f64_bin!(|a, b| a / b),
        0xa4 => f64_bin!(|a, b| float::min_f64(a, b)),
        0xa5 => f64_bin!(|a, b| float::max_f64(a, b)),
        0xa6 => f64_bin!(|a, b| float::copysign_f64(a, b)),

        // --- 変換 ---
        0xa7 => {
            let a = pop!();
            push!(u64::from(a as u32));
        }
        0xa8 => {
            let a = f64::from(f32::from_bits(pop!() as u32));
            let t = trunc_check(a, -2_147_483_648.0, 2_147_483_648.0)?;
            push!(u64::from(t as i32 as u32));
        }
        0xa9 => {
            let a = f64::from(f32::from_bits(pop!() as u32));
            let t = trunc_check(a, 0.0, 4_294_967_296.0)?;
            push!(u64::from(t as u32));
        }
        0xaa => {
            let a = f64::from_bits(pop!());
            let t = trunc_check(a, -2_147_483_648.0, 2_147_483_648.0)?;
            push!(u64::from(t as i32 as u32));
        }
        0xab => {
            let a = f64::from_bits(pop!());
            let t = trunc_check(a, 0.0, 4_294_967_296.0)?;
            push!(u64::from(t as u32));
        }
        0xac => {
            let a = pop!() as u32;
            push!(i64::from(a as i32) as u64);
        }
        0xad => {
            let a = pop!() as u32;
            push!(u64::from(a));
        }
        0xae => {
            let a = f64::from(f32::from_bits(pop!() as u32));
            let t = trunc_check(a, -9_223_372_036_854_775_808.0, 9_223_372_036_854_775_808.0)?;
            push!(t as i64 as u64);
        }
        0xaf => {
            let a = f64::from(f32::from_bits(pop!() as u32));
            let t = trunc_check(a, 0.0, 18_446_744_073_709_551_616.0)?;
            push!(t as u64);
        }
        0xb0 => {
            let a = f64::from_bits(pop!());
            let t = trunc_check(a, -9_223_372_036_854_775_808.0, 9_223_372_036_854_775_808.0)?;
            push!(t as i64 as u64);
        }
        0xb1 => {
            let a = f64::from_bits(pop!());
            let t = trunc_check(a, 0.0, 18_446_744_073_709_551_616.0)?;
            push!(t as u64);
        }
        0xb2 => {
            let a = pop!() as u32 as i32;
            push!(u64::from((a as f32).to_bits()));
        }
        0xb3 => {
            let a = pop!() as u32;
            push!(u64::from((a as f32).to_bits()));
        }
        0xb4 => {
            let a = pop!() as i64;
            push!(u64::from((a as f32).to_bits()));
        }
        0xb5 => {
            let a = pop!();
            push!(u64::from((a as f32).to_bits()));
        }
        0xb6 => {
            let a = f64::from_bits(pop!());
            let r = if a.is_nan() {
                f32::from_bits(float::NAN_F32)
            } else {
                a as f32
            };
            push!(u64::from(r.to_bits()));
        }
        0xb7 => {
            let a = pop!() as u32 as i32;
            push!(f64::from(a).to_bits());
        }
        0xb8 => {
            let a = pop!() as u32;
            push!(f64::from(a).to_bits());
        }
        0xb9 => {
            let a = pop!() as i64;
            push!((a as f64).to_bits());
        }
        0xba => {
            let a = pop!();
            push!((a as f64).to_bits());
        }
        0xbb => {
            let a = f32::from_bits(pop!() as u32);
            let r = if a.is_nan() {
                f64::from_bits(float::NAN_F64)
            } else {
                f64::from(a)
            };
            push!(r.to_bits());
        }
        // reinterpret はビットをそのまま通す。
        0xbc..=0xbf => {}

        // --- sign-extension-ops ---
        0xc0 => {
            let a = pop!() as u32;
            push!(u64::from(i32::from(a as u8 as i8) as u32));
        }
        0xc1 => {
            let a = pop!() as u32;
            push!(u64::from(i32::from(a as u16 as i16) as u32));
        }
        0xc2 => {
            let a = pop!();
            push!(i64::from(a as u8 as i8) as u64);
        }
        0xc3 => {
            let a = pop!();
            push!(i64::from(a as u16 as i16) as u64);
        }
        0xc4 => {
            let a = pop!();
            push!(i64::from(a as u32 as i32) as u64);
        }

        _ => return Err(Error::Malformed("illegal opcode")),
    }
    Ok(())
}
