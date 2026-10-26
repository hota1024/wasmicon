//! `Exec::peak` and `Validated::max_control`: what a run needed, in the units
//! of `Config`, so a port can be sized from a host run.

use wasmicon_core::interp::Peak;
use wasmicon_core::{
    Arena, Config, Error, Exec, Extern, ExternType, Resolver, decode, instantiate, invoke, validate,
};

/// `(func (export "rec") (param i32) (result i32) (local i32)`
/// `  local.get 0  i32.eqz`
/// `  if (result i32)  i32.const 0`
/// `  else  local.get 0  i32.const 1  i32.sub  call 0  end)`
///
/// `rec n` is n + 1 calls deep. Each frame reserves 2 slots of locals and a
/// validated height of 2, and its locals start where the caller's argument
/// was (2 slots above the caller's), so frame k reserves up to 2k + 2.
const REC: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // header
    0x01, 0x06, 0x01, 0x60, 0x01, 0x7f, 0x01, 0x7f, // type: (i32) -> i32
    0x03, 0x02, 0x01, 0x00, // func 0: type 0
    0x07, 0x07, 0x01, 0x03, b'r', b'e', b'c', 0x00, 0x00, // export "rec"
    0x0a, 0x16, 0x01, 0x14, 0x01, 0x01, 0x7f, // code: 20 bytes, one i32 local
    0x20, 0x00, 0x45, 0x04, 0x7f, 0x41, 0x00, 0x05, // get 0, eqz, if, const 0, else
    0x20, 0x00, 0x41, 0x01, 0x6b, 0x10, 0x00, // get 0, const 1, sub, call 0
    0x0b, 0x0b, // end, end
];

struct NoImports;

impl Resolver for NoImports {
    fn resolve(&mut self, _: &str, _: &str, _: &ExternType<'_>) -> Option<Extern> {
        None
    }

    fn call(
        &mut self,
        _: u32,
        _: &[u64],
        _: &mut [u64],
        _: &mut [u8],
    ) -> wasmicon_core::Result<()> {
        Err(Error::Invalid("no host functions"))
    }
}

/// Runs `rec n` under `cfg`. Returns the outcome, the peak and `max_control`.
fn run_rec(cfg: &Config, n: u64) -> (Result<(), Error>, Peak, u32) {
    let mut buf = vec![0u8; 1 << 20];
    let mut scratch_buf = vec![0u8; 1 << 20];
    let mut arena = Arena::new(&mut buf);
    let mut scratch = Arena::new(&mut scratch_buf);
    let m = decode::decode(REC, &mut arena).ok().expect("decodes");
    let v = validate::validate(&m, cfg, &mut arena, &mut scratch)
        .ok()
        .expect("validates");
    let max_control = v.max_control;
    let mut exec = Exec::new(cfg, &mut arena).ok().expect("fits");
    let mut inst = instantiate(m, v, cfg, &mut arena, &mut NoImports)
        .ok()
        .expect("instantiates");
    let rec = inst.export_func("rec").expect("exported");
    let mut out = [0u64];
    let r = invoke(&mut inst, &mut exec, &mut NoImports, rec, &[n], &mut out);
    (r, exec.peak(), max_control)
}

#[test]
fn a_run_reports_its_call_depth_and_operand_slots() {
    let (r, peak, max_control) = run_rec(&Config::DEFAULT, 10);
    assert!(r.is_ok());
    assert_eq!(
        peak,
        Peak {
            call_depth: 11,
            operand_slots: 2 * 11 + 2
        }
    );
    // The function's own block and the `if`.
    assert_eq!(max_control, 2);
}

#[test]
fn the_peak_is_exactly_the_limit_a_run_needs() {
    let need = run_rec(&Config::DEFAULT, 10).1;
    let fits = Config {
        max_call_depth: need.call_depth as usize,
        operand_stack_slots: need.operand_slots as usize,
        ..Config::DEFAULT
    };
    assert!(run_rec(&fits, 10).0.is_ok());

    let shallow = Config {
        max_call_depth: need.call_depth as usize - 1,
        ..fits
    };
    let (r, peak, _) = run_rec(&shallow, 10);
    assert!(matches!(r, Err(Error::Exhausted("call stack exhausted"))));
    assert_eq!(peak.call_depth, 10, "stays at the limit");

    let narrow = Config {
        operand_stack_slots: need.operand_slots as usize - 1,
        ..fits
    };
    let (r, peak, _) = run_rec(&narrow, 10);
    assert!(matches!(r, Err(Error::Exhausted("operand stack overflow"))));
    assert_eq!(peak, need, "the failing frame's reservation is recorded");
}
