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
//! The queue is the only buffer: packets go from it straight to the endpoint,
//! and a line leaves it only once the host has taken all of it. A bus reset,
//! or a port closed between two packets, sends the line again from its start
//! in the next session, so a session never opens in the middle of a line. One
//! case is left: a port closed while a packet sits on the endpoint gets that
//! packet and the rest of its line when it opens again, because the endpoint
//! cannot take a packet back.
//!
//! USB is serviced from `USBCTRL_IRQ`, not from host calls: a guest can run a
//! long time without one, and the host must still see enumeration.

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, Ordering};

use critical_section::Mutex;
use rp235x_hal as hal;
use usb_device::LangID;
use usb_device::class_prelude::{
    ControlIn, ControlOut, EndpointAddress, InterfaceNumber, StringIndex, UsbBus, UsbBusAllocator,
    UsbClass,
};
use usb_device::control::{Recipient, RequestType};
use usb_device::descriptor::DescriptorWriter;
use usb_device::device::{StringDescriptors, UsbDevice, UsbDeviceBuilder, UsbVidPid};
use usbd_serial::{CdcAcmClass, SerialPort};
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

/// The console's packet size, the most a full-speed bulk endpoint takes.
const PACKET: usize = 64;

/// Set by the first `init`.
static INIT: AtomicBool = AtomicBool::new(false);

/// The allocator outlives every class built on it, so it is a static.
static mut BUS: Option<UsbBusAllocator<Bus>> = None;

/// The serial number string: the chip's id in hex, built once by `init`.
static mut SERIAL: [u8; 16] = [b'0'; 16];

/// Lines waiting for the host. Apart from `STATE` so it is built at compile
/// time: it sits in .bss instead of passing through the stack in `init`.
static QUEUE: Mutex<RefCell<Queue>> = Mutex::new(RefCell::new(Queue::new()));

/// Shared by `main` (which queues lines) and `USBCTRL_IRQ` (which drains them).
static STATE: Mutex<RefCell<Option<State>>> = Mutex::new(RefCell::new(None));

struct State {
    device: UsbDevice<'static, Bus>,
    console: Console,
    control: SerialPort<'static, Bus>,
    reset: PicotoolReset,
}

/// Brings USB up and starts servicing it from `USBCTRL_IRQ`. Call once; a
/// second call panics.
pub fn init(
    usb: hal::pac::USB,
    dpram: hal::pac::USB_DPRAM,
    clock: hal::clocks::UsbClock,
    resets: &mut hal::pac::RESETS,
) {
    // A second call would replace BUS under the classes that borrow it.
    assert!(
        !INIT.swap(true, Ordering::Relaxed),
        "usb::init called twice"
    );
    // The Pico 2 W routes VBUS through the CYW43439, so the controller
    // cannot sense it: force the detect bit.
    let bus = UsbBusAllocator::new(Bus::new(usb, dpram, clock, true, resets));
    // SAFETY: INIT lets this run once, from `main`, before the interrupt is
    // unmasked, so nothing else holds a reference to BUS. It is never written
    // again, so the shared reference handed to the classes stays valid for
    // 'static.
    let bus: &'static UsbBusAllocator<Bus> = unsafe { (*core::ptr::addr_of_mut!(BUS)).insert(bus) };

    let console = Console::new(bus);
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
    // SAFETY: called once, from `init` (which INIT keeps to one call), before
    // the interrupt is unmasked, so nothing else reads SERIAL; it is never
    // written again.
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
/// that does not fit is dropped whole and counted. The trace is not queued;
/// it stays on UART0 (docs/TODO.md §5-11).
pub fn write_line(kind: LineKind, parts: &[&[u8]]) {
    let reserve = match kind {
        LineKind::Trace => return,
        LineKind::Firmware => 0,
        LineKind::Log => FIRMWARE_RESERVE,
    };
    critical_section::with(|cs| {
        let mut queue = QUEUE.borrow_ref_mut(cs);
        queue.push(parts, reserve);
        if let Some(s) = STATE.borrow_ref_mut(cs).as_mut() {
            // Arm the IN endpoint now: if it is idle, no interrupt would
            // come to drain the queue until some unrelated bus event.
            s.console.drain(&mut queue);
        }
    });
}

#[interrupt]
fn USBCTRL_IRQ() {
    critical_section::with(|cs| {
        let mut state = STATE.borrow_ref_mut(cs);
        let Some(s) = state.as_mut() else { return };
        if s.device
            .poll(&mut [&mut s.console, &mut s.control, &mut s.reset])
        {
            // The control port carries nothing in yet: drop what the host
            // sends, so a terminal that writes never stalls.
            let mut sink = [0u8; 64];
            while matches!(s.control.read(&mut sink), Ok(n) if n > 0) {}
        }
        s.console.drain(&mut QUEUE.borrow_ref_mut(cs));
    });
}

/// The console: a CDC ACM class that sends packets straight from the queue.
/// `UsbDevice::poll` reports to it through `UsbClass`; `drain` applies what
/// it saw to the queue, which lives behind its own lock.
struct Console {
    cdc: CdcAcmClass<'static, Bus>,
    /// Bytes of the packet on the endpoint, until the host takes it.
    /// `Some(0)` is a zero-length packet.
    in_flight: Option<usize>,
    /// Bytes the host took since the last `drain`.
    taken: Option<usize>,
    /// The last packet was full, so a short one (empty if nothing is left)
    /// must follow to end the transfer, or the host holds the bytes back.
    full: bool,
    /// DTR as `drain` last saw it.
    open: bool,
    /// A bus reset cleared the endpoint: the line in progress starts over.
    lost: bool,
}

impl Console {
    fn new(bus: &'static UsbBusAllocator<Bus>) -> Self {
        Console {
            cdc: CdcAcmClass::new_with_interface_names(
                bus,
                PACKET as u16,
                Some("console"),
                Some("console"),
            ),
            in_flight: None,
            taken: None,
            full: false,
            open: false,
            lost: false,
        }
    }

    /// Hands the next packet of the queue to the endpoint, if it is free and
    /// a host has the port open. Only while DTR is up: a packet sent before
    /// that goes out to nobody.
    fn drain(&mut self, queue: &mut Queue) {
        if let Some(n) = self.taken.take() {
            queue.advance(n);
        }
        if core::mem::take(&mut self.lost) {
            queue.restart();
        }
        // Sending made room: the dropped count need not wait for a new line.
        queue.note_dropped(FIRMWARE_RESERVE);
        if !self.cdc.dtr() {
            if self.open && self.in_flight.is_none() {
                // Closed between two packets: the next session gets the
                // whole line.
                queue.restart();
            }
            self.open = false;
            return;
        }
        self.open = true;
        if self.in_flight.is_some() {
            return;
        }
        let mut packet = [0u8; PACKET];
        let n = queue.peek(&mut packet);
        if n == 0 && !self.full {
            return;
        }
        // WouldBlock cannot come with nothing in flight; any other error has
        // nowhere to go. Either way the bytes stay queued for the next try.
        if self.cdc.write_packet(&packet[..n]).is_ok() {
            self.in_flight = Some(n);
            self.full = n == PACKET;
        }
    }
}

impl UsbClass<Bus> for Console {
    fn get_configuration_descriptors(
        &self,
        writer: &mut DescriptorWriter,
    ) -> usb_device::Result<()> {
        self.cdc.get_configuration_descriptors(writer)
    }

    fn get_string(&self, index: StringIndex, lang_id: LangID) -> Option<&str> {
        self.cdc.get_string(index, lang_id)
    }

    fn reset(&mut self) {
        UsbClass::<Bus>::reset(&mut self.cdc);
        self.in_flight = None;
        self.taken = None;
        self.full = false;
        self.open = false;
        self.lost = true;
    }

    fn control_in(&mut self, xfer: ControlIn<Bus>) {
        self.cdc.control_in(xfer);
    }

    fn control_out(&mut self, xfer: ControlOut<Bus>) {
        self.cdc.control_out(xfer);
    }

    fn endpoint_out(&mut self, addr: EndpointAddress) {
        // The console carries nothing in: drop what the host sends.
        if addr == self.cdc.read_ep().address() {
            let mut sink = [0u8; PACKET];
            let _ = self.cdc.read_packet(&mut sink);
        }
    }

    fn endpoint_in_complete(&mut self, addr: EndpointAddress) {
        if addr == self.cdc.write_ep().address() {
            let n = self.in_flight.take().unwrap_or(0);
            self.taken = Some(self.taken.unwrap_or(0) + n);
        }
    }
}

/// Length of a queued line, before its bytes.
const HEADER: usize = 2;

/// The longest dropped count: "wasmicon: usb console dropped " (30 bytes),
/// ten digits, " lines" (6) and the line end (2).
const NOTE_MAX: usize = 48;

/// A ring of whole lines, each a little-endian `u16` length and the bytes,
/// line end included.
struct Queue {
    buf: [u8; QUEUE_LEN],
    head: usize,
    len: usize,
    /// Bytes of the first line the host has taken.
    sent: usize,
    /// Lines dropped since the last count went out.
    dropped: u32,
}

impl Queue {
    const fn new() -> Self {
        Queue {
            buf: [0; QUEUE_LEN],
            head: 0,
            len: 0,
            sent: 0,
            dropped: 0,
        }
    }

    fn free(&self) -> usize {
        QUEUE_LEN - self.len
    }

    /// Adds `parts` and a line end, leaving `reserve` bytes free, or counts the
    /// line as dropped. A count still waiting goes in first, and only with the
    /// line, so the count stands where the gap is and two counts never meet.
    fn push(&mut self, parts: &[&[u8]], reserve: usize) {
        let line = parts.iter().map(|p| p.len()).sum::<usize>() + LINE_END.len();
        let note = if self.dropped > 0 {
            HEADER + NOTE_MAX
        } else {
            0
        };
        if note + HEADER + line + reserve > self.free() {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.note_dropped(0);
        self.put_line(line, parts);
    }

    /// Queues the count of dropped lines, if any and if it fits with `reserve`
    /// bytes left over.
    fn note_dropped(&mut self, reserve: usize) {
        if self.dropped == 0 || HEADER + NOTE_MAX + reserve > self.free() {
            return;
        }
        let mut raw = [0u8; NOTE_MAX];
        let mut note = Buf::new(&mut raw);
        note.str("wasmicon: usb console dropped ");
        note.u32(self.dropped);
        note.str(if self.dropped == 1 { " line" } else { " lines" });
        let len = note.as_bytes().len() + LINE_END.len();
        self.put_line(len, &[note.as_bytes()]);
        self.dropped = 0;
    }

    /// Writes one line of `len` bytes (`parts` and the line end), which fits.
    fn put_line(&mut self, len: usize, parts: &[&[u8]]) {
        // A line that fits is shorter than the queue, so `u16` holds it.
        self.put(&(len as u16).to_le_bytes());
        write_parts(parts, |b| self.put(b));
    }

    fn put(&mut self, bytes: &[u8]) {
        let at = (self.head + self.len) % QUEUE_LEN;
        let first = bytes.len().min(QUEUE_LEN - at);
        self.buf[at..at + first].copy_from_slice(&bytes[..first]);
        self.buf[..bytes.len() - first].copy_from_slice(&bytes[first..]);
        self.len += bytes.len();
    }

    fn get(&self, at: usize, out: &mut [u8]) {
        let at = at % QUEUE_LEN;
        let first = out.len().min(QUEUE_LEN - at);
        let n = out.len();
        out[..first].copy_from_slice(&self.buf[at..at + first]);
        out[first..].copy_from_slice(&self.buf[..n - first]);
    }

    /// Length of the first line, if any.
    fn first_len(&self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let mut raw = [0u8; HEADER];
        self.get(self.head, &mut raw);
        Some(usize::from(u16::from_le_bytes(raw)))
    }

    /// Copies the next bytes of the first line the host has not taken into
    /// `out`, as many as fit. Returns how many; 0 when the queue is empty.
    fn peek(&self, out: &mut [u8]) -> usize {
        let Some(line) = self.first_len() else {
            return 0;
        };
        let n = (line - self.sent).min(out.len());
        self.get(self.head + HEADER + self.sent, &mut out[..n]);
        n
    }

    /// The host took `n` more bytes of the first line; the line leaves the
    /// queue once all of it is taken.
    fn advance(&mut self, n: usize) {
        let Some(line) = self.first_len() else { return };
        self.sent += n;
        if self.sent >= line {
            self.head = (self.head + HEADER + line) % QUEUE_LEN;
            self.len -= HEADER + line;
            self.sent = 0;
        }
    }

    /// Sends the first line again from its start.
    fn restart(&mut self) {
        self.sent = 0;
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
}

impl PicotoolReset {
    fn new<B: UsbBus>(alloc: &UsbBusAllocator<B>) -> Self {
        PicotoolReset {
            interface: alloc.interface(),
            name: alloc.string(),
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
        let kind = match req.request {
            // Bits 0..=6 of wValue disable the boot ROM's interfaces as in
            // pico-sdk (bit 0: mass storage, bit 1: PICOBOOT). The activity
            // LED of bits 8.. has no pin to drive here and is ignored.
            RESET_REQUEST_BOOTSEL => hal::reboot::RebootKind::BootSel {
                msd_disabled: req.value & 0x01 != 0,
                picoboot_disabled: req.value & 0x02 != 0,
            },
            RESET_REQUEST_FLASH => hal::reboot::RebootKind::Normal,
            _ => {
                let _ = xfer.reject();
                return;
            }
        };
        // Answer first: picotool waits for the reply to the FLASH request
        // and never returns if the device resets under it. The reply is
        // armed on the endpoint and goes out on the host's next IN token
        // without the CPU, inside the 500 ms that `reboot` hands the ROM
        // before it resets.
        let _ = xfer.accept();
        hal::reboot::reboot(kind, hal::reboot::RebootArch::Normal);
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = xfer.request();
        if self.is_mine(req.request_type, req.recipient, req.index) {
            let _ = xfer.reject();
        }
    }
}
