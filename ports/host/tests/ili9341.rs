//! Runs display apps through the ILI9341 backend over the mock SPI and GPIO,
//! and decodes what it sent.
//!
//! Locks in docs/device-layer.md §7: the guest's trace is the same whether the
//! backend is the mock display or an ILI9341 (backend traffic is not traced),
//! and the panel receives the same picture the mock shows. Also that the
//! backend's bus and pins are not the guest's (docs/device-layer.md §5.2).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::wat_to_wasm;
use wasmicon_host::hal::BusEvent;
use wasmicon_port::roles::DisplayConfig;

/// Pins clear of the host mock's roles (2, 10, 11, 12).
const LCD: DisplayConfig = DisplayConfig::Ili9341 {
    spi: 0,
    cs: 20,
    dc: 21,
    rst: 22,
    rotation: 90,
};

fn build_demo() -> Vec<u8> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = Command::new("cargo")
        .current_dir(root.join("apps"))
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(["build", "--release", "-p", "display-demo-rs"])
        .status()
        .expect("cannot run cargo");
    assert!(status.success(), "build of display-demo-rs failed");
    let wasm: PathBuf =
        root.join("apps/target/wasm32-unknown-unknown/release/display_demo_rs.wasm");
    std::fs::read(&wasm).unwrap_or_else(|e| panic!("cannot read {}: {e}", wasm.display()))
}

fn run(wasm: &[u8], display: Option<DisplayConfig>) -> wasmicon_host::Outcome {
    let opts = wasmicon_host::Options {
        trace: true,
        display,
        bus_log: display.is_some(),
        ..Default::default()
    };
    wasmicon_host::run_wasm_opts(wasm, opts)
        .unwrap_or_else(|e| panic!("execution failed: {} [{}]", e.reason(), e.kind().name()))
}

/// What the panel received: commands in order and the frame it holds.
struct Panel {
    commands: Vec<u8>,
    pixels: Vec<u16>,
}

/// Decodes ILI9341 traffic (CASET / PASET / RAMWR; apps/README.md §2) into a
/// 320x240 frame. DC low marks a command byte; bytes with CS high are dropped.
fn decode(log: &[BusEvent], cs: u32, dc: u32) -> Panel {
    let (w, h) = (320usize, 240usize);
    let mut p = Panel {
        commands: Vec::new(),
        pixels: vec![0; w * h],
    };
    let (mut cs_low, mut dc_high) = (false, false);
    let mut cmd = 0u8;
    let mut args: Vec<u8> = Vec::new();
    let (mut x0, mut x1, mut y0, mut y1) = (0usize, 0usize, 0usize, 0usize);
    let (mut x, mut y) = (0usize, 0usize);
    let mut half: Option<u8> = None;
    for e in log {
        match e {
            BusEvent::Gpio(pin, high) if *pin == cs => cs_low = !high,
            BusEvent::Gpio(pin, high) if *pin == dc => dc_high = *high,
            BusEvent::Gpio(..) => {}
            BusEvent::Spi(_, bytes) if !cs_low => assert!(bytes.is_empty(), "SPI with CS high"),
            BusEvent::Spi(_, bytes) if !dc_high => {
                assert_eq!(bytes.len(), 1, "one command byte at a time");
                cmd = bytes[0];
                p.commands.push(cmd);
                args.clear();
                (x, y, half) = (x0, y0, None);
            }
            BusEvent::Spi(_, bytes) => {
                for &b in bytes {
                    if cmd != 0x2C {
                        args.push(b);
                        if args.len() == 4 {
                            let a = usize::from(u16::from_be_bytes([args[0], args[1]]));
                            let z = usize::from(u16::from_be_bytes([args[2], args[3]]));
                            match cmd {
                                0x2A => (x0, x1) = (a, z),
                                0x2B => (y0, y1) = (a, z),
                                _ => {}
                            }
                        }
                        continue;
                    }
                    let Some(hi) = half.take() else {
                        half = Some(b);
                        continue;
                    };
                    p.pixels[y * w + x] = u16::from_be_bytes([hi, b]);
                    x += 1;
                    if x > x1 {
                        x = x0;
                        y += 1;
                    }
                    assert!(y <= y1 + 1 && y1 < h, "RAMWR past the window");
                }
            }
        }
    }
    p
}

#[test]
fn the_demo_traces_the_same_and_draws_the_same_picture_on_an_ili9341() {
    let wasm = build_demo();
    let mock = run(&wasm, None);
    let lcd = run(&wasm, Some(LCD));

    assert_eq!(
        lcd.trace, mock.trace,
        "the ILI9341 backend changed the guest's trace"
    );

    let panel = decode(&lcd.bus_log, 20, 21);
    // Initialized once, at the first open, as apps/README.md §2 orders it.
    let init: Vec<u8> = panel
        .commands
        .iter()
        .copied()
        .filter(|c| ![0x2A, 0x2B, 0x2C].contains(c))
        .collect();
    assert_eq!(init, [0x01, 0x11, 0x3A, 0x36, 0x29]);
    let frame = mock.frame.expect("the mock display");
    assert!(
        panel.pixels == frame.pixels,
        "the panel received a different picture from the one the mock shows"
    );
    // The reset line stays high after the run: release_all leaves the
    // backend's pins alone, so the panel keeps the picture.
    let last_rst = lcd.bus_log.iter().rev().find_map(|e| match e {
        BusEvent::Gpio(22, high) => Some(*high),
        _ => None,
    });
    assert_eq!(last_rst, Some(true));
}

#[test]
fn the_backend_bus_and_pins_are_not_the_guests() {
    let wasm = wat_to_wasm("display_owns.wat");
    let out = run(&wasm, Some(LCD));
    assert_eq!(
        out.trace,
        "> wasmicon:hal/gpio@0.1.0/[static]pin.open(20, 3)\n\
         < 1\n\
         > wasmicon:hal/gpio@0.1.0/[static]pin.open(23, 3)\n\
         < 0 [1]\n\
         > wasmicon:hal/spi@0.1.0/[static]bus.open(0, 1000000, 0)\n\
         < 3\n\
         > wasmicon:hal/spi@0.1.0/[static]bus.open(1, 1000000, 0)\n\
         < 0 [2]\n"
    );
}
