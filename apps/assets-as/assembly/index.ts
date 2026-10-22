// Assets (AssemblyScript). Produces the same host-call sequence as assets-rs.
//
// hello.txt is read whole and logged as text. pattern.bin is read in chunks
// and checked against the formula that generated it, so the trace and the
// log together show that every byte arrives intact (docs/app-workflow.md §3.10).

// asc cannot resolve the scoped npm package (@wasmicon/hal) as ~lib, so the
// bindings are imported by relative path. The layout in README.md is the
// source of truth for where they live.
import { Blob, log } from "../../../bindings/assemblyscript/assembly/index";

/// Bytes of pattern.bin read per host call. Smaller than the asset, so the
/// reads cross a 4 KiB boundary of the slot.
const CHUNK: u32 = 1024;

/// pattern.bin is PATTERN_LEN bytes of (i * 7) as a byte (apps/assets-rs/assets/).
const PATTERN_LEN: u32 = 4100;

/// Longest hello.txt the text buffer holds (the same bound as assets-rs).
const TEXT_CAP: i64 = 64;

/// Entry point. The host calls it once, right after instantiation
/// (docs/abi-spec.md §3.3).
export function run(): void {
  log.info("assets start");

  const hello = Blob.open("hello.txt");
  if (hello == null) {
    log.error("no asset hello.txt");
    return;
  }
  const helloLen = hello.len();
  if (helloLen < 0) {
    log.error("len failed");
    return;
  }
  const text = new Uint8Array(helloLen > TEXT_CAP ? 0 : (helloLen as i32));
  if (helloLen > TEXT_CAP || hello.read(0, text) < 0) {
    log.error("cannot read hello.txt");
    return;
  }
  // assets-rs rejects a text that is not UTF-8; String.UTF8.decode replaces
  // bad sequences instead, which the committed fixture never has.
  log.info(String.UTF8.decode(text.buffer));
  // Released here, so pattern.bin gets the same handle.
  hello.close();

  const pattern = Blob.open("pattern.bin");
  if (pattern == null) {
    log.error("no asset pattern.bin");
    return;
  }
  if (pattern.len() != (PATTERN_LEN as i64)) {
    log.error("pattern.bin has the wrong length");
    return;
  }
  const buf = new Uint8Array(CHUNK as i32);
  let at: u32 = 0;
  let ok = true;
  while (at < PATTERN_LEN) {
    const n: u32 = PATTERN_LEN - at < CHUNK ? PATTERN_LEN - at : CHUNK;
    const chunk = n == CHUNK ? buf : buf.subarray(0, n as i32);
    if (pattern.read(at, chunk) < 0) {
      log.error("cannot read pattern.bin");
      return;
    }
    for (let i: u32 = 0; i < n; i++) {
      if (chunk[i] != (((at + i) * 7) & 0xff) as u8) {
        ok = false;
      }
    }
    at += n;
  }
  log.info(ok ? "pattern ok" : "pattern mismatch");
  pattern.close();
}
