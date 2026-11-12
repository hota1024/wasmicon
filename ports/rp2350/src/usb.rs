//! The USB console (docs/decisions.md A-11, docs/TODO.md §5-11).
//!
//! USB is a composite device with three interfaces: a CDC ACM console that
//! carries plain lines, a CDC ACM control interface for the channel of
//! docs/app-workflow.md §3.6 (it answers nothing yet, but is there so the
//! descriptors do not change when the channel lands), and the picotool reset
//! interface, so `picotool reboot -f -u` and `picotool load -f` reach BOOTSEL
//! without the button.
//!
//! The console carries the guest `log` and the firmware's own lines; the trace
//! stays on UART0. Lines go into a queue that is sent while a host has the
//! port open (DTR up), so the boot lines wait for the host instead of being
//! sent into nothing. The firmware never waits for the host: a line that does
//! not fit is dropped whole and counted, and the count goes out as a line of
//! its own once there is room. The end of the queue is kept for the
//! firmware's lines, so a trap reason still fits after a guest has filled the
//! queue with `log`. UART0 still has every line.
//!
//! USB is serviced from `USBCTRL_IRQ`, not from host calls: a guest can run a
//! long time without one, and the host must still see enumeration.

use core::cell::RefCell;

use critical_section::Mutex;
use rp235x_hal as hal;
use usb_device::LangID;
use usb_device::class_prelude::{
    ControlIn, ControlOut, InterfaceNumber, StringIndex, UsbBus, UsbBusAllocator, UsbClass,
};
use usb_device::control::{Recipient, RequestType};
use usb_device::descriptor::DescriptorWriter;
use usb_device::device::{StringDescriptors, UsbDevice, UsbDeviceBuilder, UsbVidPid};
use usbd_serial::SerialPort;
use wasmicon_port::fmt::Buf;
use wasmicon_port::serial::{LINE_END, LineKind, write_parts};

use hal::pac::interrupt;
use hal::usb::UsbBus as Bus;

/// Raspberry Pi's VID and the PID of pico-sdk's stdio USB on the RP2350.
/// A temporary pair (docs/TODO.md §5-11): picotool finds the reset interface
/// without `--vid` / `--pid` only under this VID and a PID it knows.
const VID_PID: UsbVidPid = UsbVidPid(0x2E8A, 0x0009);

/// Lines waiting for the host. The guest log and the firmware lines are
/// short; this holds the boot lines until a host opens the port.
const QUEUE_LEN: usize = 8 * 1024;

/// Room at the end of the queue that only firmware lines may use. Holds a
/// trap reason and the dropped count.
const FIRMWARE_RESERVE: usize = 256;

/// The allocator outlives every class built on it, so it is a static.
static mut BUS: Option<UsbBusAllocator<Bus>> = None;

/// The serial number string: the chip's id in hex, built once by `init`.
static mut SERIAL: [u8; 16] = [b'0'; 16];

/// Shared by `main` (which queues lines) and `USBCTRL_IRQ` (which drains them).
static STATE: Mutex<RefCell<Option<State>>> = Mutex::new(RefCell::new(None));

struct State {
    device: UsbDevice<'static, Bus>,
    console: SerialPort<'static, Bus>,
    control: SerialPort<'static, Bus>,
    reset: PicotoolReset,
    queue: Queue,
}

/// Brings USB up and starts servicing it from `USBCTRL_IRQ`. Call once.
pub fn init(
    usb: hal::pac::USB,
    dpram: hal::pac::USB_DPRAM,
    clock: hal::clocks::UsbClock,
    resets: &mut hal::pac::RESETS,
) {
    // The Pico 2 W routes VBUS through the CYW43439, so the controller
    // cannot sense it: force the detect bit.
    let bus = UsbBusAllocator::new(Bus::new(usb, dpram, clock, true, resets));
    // SAFETY: `init` runs once, from `main`, before the interrupt is unmasked,
    // so nothing else holds a reference to BUS. It is never written again, so
    // the shared reference handed to the classes stays valid for 'static.
    let bus: &'static UsbBusAllocator<Bus> = unsafe { (*core::ptr::addr_of_mut!(BUS)).insert(bus) };

    let console = SerialPort::new_with_interface_names(bus, Some("console"), Some("console"));
    let control = SerialPort::new_with_interface_names(bus, Some("control"), Some("control"));
    let reset = PicotoolReset::new(bus);
    let device = UsbDeviceBuilder::new(bus, VID_PID)
        .strings(&[StringDescriptors::new(LangID::EN_US)
            .manufacturer("wasmicon")
            .product("wasmicon rp2350")
            .serial_number(serial_number())])
        .ok()
        .unwrap()
        .composite_with_iads()
        .build();

    critical_section::with(|cs| {
        STATE.borrow_ref_mut(cs).replace(State {
            device,
            console,
            control,
            reset,
            queue: Queue::new(),
        });
    });
    // SAFETY: STATE is set above, so the handler finds it. The handler only
    // touches STATE inside a critical section, as `write_line` does.
    unsafe { cortex_m::peripheral::NVIC::unmask(hal::pac::Interrupt::USBCTRL_IRQ) };
}

/// The chip's id as the boot ROM reports it in BOOTSEL. picotool, after it
/// reboots a running firmware through the reset interface, looks for the
/// BOOTSEL device by the serial number it saw, so the two must be equal.
fn serial_number() -> &'static str {
    let (high, low) = match hal::rom_data::sys_info_api::chip_info() {
        Ok(Some(info)) => (info.wafer_id, info.device_id),
        _ => (0, 0),
    };
    // SAFETY: called once, from `init`, before the interrupt is unmasked, so
    // nothing else reads SERIAL; it is never written again.
    let serial = unsafe { &mut *core::ptr::addr_of_mut!(SERIAL) };
    for (i, digit) in serial.iter_mut().enumerate() {
        let word = if i < 8 { high } else { low };
        let nibble = (word >> (28 - 4 * (i % 8))) & 0xF;
        *digit = b"0123456789ABCDEF"[nibble as usize];
    }
    // Only ASCII hex digits were written.
    core::str::from_utf8(serial).unwrap_or("0")
}

/// Queues one line for the console and starts sending it. Never waits: a line
/// that does not fit is dropped whole and counted. Before `init`, does nothing.
pub fn write_line(kind: LineKind, parts: &[&[u8]]) {
    let reserve = match kind {
        LineKind::Firmware => 0,
        LineKind::Log | LineKind::Trace => FIRMWARE_RESERVE,
    };
    critical_section::with(|cs| {
        if let Some(s) = STATE.borrow_ref_mut(cs).as_mut() {
            s.queue.push(parts, reserve);
            // Arm the IN endpoint now: if it is idle, no interrupt would
            // come to drain the queue until some unrelated bus event.
            s.drain();
        }
    });
}

impl State {
    /// Moves queued bytes into the console until it takes no more. Only while
    /// a host has the port open: bytes the console takes before that go out
    /// to nobody, and may end in the middle of a line.
    fn drain(&mut self) {
        if !self.console.dtr() {
            return;
        }
        while let Some(chunk) = self.queue.front() {
            let len = chunk.len();
            match self.console.write(chunk) {
                Ok(n) => {
                    self.queue.consume(n);
                    if n < len {
                        break;
                    }
                }
                // WouldBlock: the endpoint is busy or no host reads. Anything
                // else: nowhere to report it. Either way, try again later.
                Err(_) => break,
            }
        }
        let _ = self.console.flush();
        // Sending made room: the dropped count need not wait for a new line.
        self.queue.note_dropped(FIRMWARE_RESERVE);
    }
}

#[interrupt]
fn USBCTRL_IRQ() {
    critical_section::with(|cs| {
        let mut state = STATE.borrow_ref_mut(cs);
        let Some(s) = state.as_mut() else { return };
        if s.device
            .poll(&mut [&mut s.console, &mut s.control, &mut s.reset])
        {
            // Both ports carry nothing in: drop what the host sends, so a
            // terminal that writes never stalls.
            let mut sink = [0u8; 64];
            while matches!(s.console.read(&mut sink), Ok(n) if n > 0) {}
            while matches!(s.control.read(&mut sink), Ok(n) if n > 0) {}
        }
        if s.reset.reboot {
            // The status stage of the reply goes out on the host's next IN
            // token, without the CPU; 10 ms covers it with room to spare.
            cortex_m::asm::delay(REBOOT_DELAY_CYCLES);
            hal::reboot::reboot(
                hal::reboot::RebootKind::Normal,
                hal::reboot::RebootArch::Normal,
            );
        }
        s.drain();
    });
}

/// 10 ms at the 150 MHz system clock that `init_clocks_and_plls` sets.
const REBOOT_DELAY_CYCLES: u32 = 1_500_000;

/// A ring of whole lines.
struct Queue {
    buf: [u8; QUEUE_LEN],
    head: usize,
    len: usize,
    /// Lines dropped since the last count went out.
    dropped: u32,
}

impl Queue {
    const fn new() -> Self {
        Queue {
            buf: [0; QUEUE_LEN],
            head: 0,
            len: 0,
            dropped: 0,
        }
    }

    /// Adds `parts` and a line end, leaving `reserve` bytes free, or counts the
    /// line as dropped. A line never goes in ahead of a count still waiting,
    /// so the count stands where the gap is.
    fn push(&mut self, parts: &[&[u8]], reserve: usize) {
        let len = parts.iter().map(|p| p.len()).sum::<usize>() + LINE_END.len();
        self.note_dropped(reserve);
        if self.dropped > 0 || len + reserve > QUEUE_LEN - self.len {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        write_parts(parts, |b| self.put(b));
    }

    /// Queues the count of dropped lines, if any and if it fits.
    fn note_dropped(&mut self, reserve: usize) {
        if self.dropped == 0 {
            return;
        }
        let mut raw = [0u8; 48];
        let mut note = Buf::new(&mut raw);
        note.str("wasmicon: usb console dropped ");
        note.u32(self.dropped);
        note.str(" lines");
        if note.as_bytes().len() + LINE_END.len() + reserve > QUEUE_LEN - self.len {
            return;
        }
        write_parts(&[note.as_bytes()], |b| self.put(b));
        self.dropped = 0;
    }

    fn put(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.buf[(self.head + self.len) % QUEUE_LEN] = b;
            self.len += 1;
        }
    }

    /// The queued bytes up to the end of the buffer, if any.
    fn front(&self) -> Option<&[u8]> {
        if self.len == 0 {
            return None;
        }
        let end = (self.head + self.len).min(QUEUE_LEN);
        Some(&self.buf[self.head..end])
    }

    fn consume(&mut self, n: usize) {
        self.head = (self.head + n) % QUEUE_LEN;
        self.len -= n;
    }
}

// The picotool reset interface. The values are pico-sdk's
// (src/common/pico_usb_reset_interface/include/pico/usb_reset_interface.h).
// `usbd-picotool-reset` has the same class but calls the RP2040 ROM, so the
// port has its own (docs/TODO.md §5-11).
const CLASS_VENDOR_SPECIFIC: u8 = 0xFF;
const RESET_INTERFACE_SUBCLASS: u8 = 0x00;
const RESET_INTERFACE_PROTOCOL: u8 = 0x01;
const RESET_REQUEST_BOOTSEL: u8 = 0x01;
const RESET_REQUEST_FLASH: u8 = 0x02;

/// Reboots into BOOTSEL (`picotool reboot -f -u`, `load -f`) or into the
/// firmware (`picotool reboot -f`) when picotool asks.
struct PicotoolReset {
    interface: InterfaceNumber,
    name: StringIndex,
    /// A reboot into the firmware was asked for and answered.
    reboot: bool,
}

impl PicotoolReset {
    fn new<B: UsbBus>(alloc: &UsbBusAllocator<B>) -> Self {
        PicotoolReset {
            interface: alloc.interface(),
            name: alloc.string(),
            reboot: false,
        }
    }

    /// Whether a class request is addressed to this interface.
    fn is_mine(&self, request_type: RequestType, recipient: Recipient, index: u16) -> bool {
        request_type == RequestType::Class
            && recipient == Recipient::Interface
            && index == u16::from(u8::from(self.interface))
    }
}

impl<B: UsbBus> UsbClass<B> for PicotoolReset {
    fn get_configuration_descriptors(
        &self,
        writer: &mut DescriptorWriter,
    ) -> usb_device::Result<()> {
        writer.interface_alt(
            self.interface,
            0,
            CLASS_VENDOR_SPECIFIC,
            RESET_INTERFACE_SUBCLASS,
            RESET_INTERFACE_PROTOCOL,
            Some(self.name),
        )
    }

    fn get_string(&self, index: StringIndex, _lang_id: LangID) -> Option<&str> {
        (index == self.name).then_some("reset")
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = xfer.request();
        if !self.is_mine(req.request_type, req.recipient, req.index) {
            return;
        }
        if req.request == RESET_REQUEST_BOOTSEL {
            // Bits 0..=6 of wValue disable the boot ROM's interfaces as in
            // pico-sdk (bit 0: mass storage, bit 1: PICOBOOT). The activity
            // LED of bits 8.. has no pin to drive here and is ignored.
            hal::reboot::reboot(
                hal::reboot::RebootKind::BootSel {
                    msd_disabled: req.value & 0x01 != 0,
                    picoboot_disabled: req.value & 0x02 != 0,
                },
                hal::reboot::RebootArch::Normal,
            );
        }
        if req.request == RESET_REQUEST_FLASH {
            // Answer first, as pico-sdk does: picotool waits for the reply
            // and never returns if the device resets under it.
            // `USBCTRL_IRQ` reboots once the reply has gone out.
            let _ = xfer.accept();
            self.reboot = true;
            return;
        }
        let _ = xfer.reject();
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = xfer.request();
        if self.is_mine(req.request_type, req.recipient, req.index) {
            let _ = xfer.reject();
        }
    }
}
