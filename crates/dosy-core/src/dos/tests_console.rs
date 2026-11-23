use super::*;
use crate::bios::{self, bda, kbd, video};
use crate::bus::FlatBus;
use crate::vfs::MemDrive;

const DATA: u16 = 0x3000;
const STACK_SEG: u16 = 0x9000;
const STACK_SP: u16 = 0xFFF0;
const TEXT: u32 = 0xB8000;

struct T {
    dos: Dos,
    cpu: Cpu,
    bus: FlatBus,
}

impl T {
    fn new() -> Self {
        let mut bus = FlatBus::new();
        bios::install(&mut bus);
        let mut dos = Dos::new();
        dos.install(&mut bus);
        dos.mount(2, Box::new(MemDrive::new()));
        let mut cpu = Cpu::new();
        cpu.sregs.set(Seg::Ds, DATA);
        Self { dos, cpu, bus }
    }

    /// One INT 21h callback; registers set by the caller.
    fn call(&mut self, ax: u16, dx: u16) -> HleResult {
        self.cpu.regs.set_ax(ax);
        self.cpu.regs.set_dx(dx);
        self.cpu.sregs.set(Seg::Ss, STACK_SEG);
        self.cpu.regs.set_sp(STACK_SP);
        self.bus.write16(linear(STACK_SEG, STACK_SP + 4), 0x0202);
        self.cpu.insn_ip = 0x1234;
        self.dos.callback(ID_INT21, &mut self.cpu, &mut self.bus)
    }

    fn cf(&mut self) -> bool {
        self.bus.read16(linear(STACK_SEG, STACK_SP + 4)) & 1 != 0
    }

    fn keys(&mut self, s: &[u8]) {
        for &c in s {
            assert!(kbd::push_key(&mut self.bus, c as u16));
        }
    }

    /// Text on screen row `row`, trailing blanks trimmed.
    fn row(&mut self, row: u32) -> String {
        let s: String = (0..80)
            .map(|c| self.bus.read8(TEXT + row * 160 + c * 2) as char)
            .collect();
        s.trim_end_matches([' ', '\0']).to_string()
    }

    fn put(&mut self, off: u16, s: &[u8]) {
        self.bus.load(linear(DATA, off), s);
    }
}

#[test]
fn ah09_writes_string_to_text_memory() {
    let mut t = T::new();
    t.put(0x100, b"Hello,\tDOS!\r\nline 2$ignored");
    assert_eq!(t.call(0x0900, 0x100), HleResult::Done);
    assert_eq!(t.cpu.regs.al(), b'$');
    assert_eq!(t.row(0), "Hello,  DOS!");
    assert_eq!(t.row(1), "line 2");
    assert_eq!(video::cursor(&mut t.bus, 0), (6, 1));
    // INT 29h and AH=02 also reach the screen.
    t.cpu.regs.set_al(b'X');
    t.dos.callback(ID_INT29, &mut t.cpu, &mut t.bus);
    t.call(0x0200, b'Y' as u16);
    assert_eq!(t.row(1), "line 2XY");
}

#[test]
fn ah0a_line_input_with_backspace_and_retry() {
    let mut t = T::new();
    t.put(0x200, &[10]);
    assert_eq!(t.call(0x0A00, 0x200), HleResult::Retry, "no key yet");
    t.keys(b"ab\x08");
    assert_eq!(t.call(0x0A00, 0x200), HleResult::Retry, "line kept");
    t.keys(b"cdefghijklm\r");
    assert_eq!(t.call(0x0A00, 0x200), HleResult::Done);
    let a = linear(DATA, 0x200) as usize;
    // Max 10 bytes incl. CR: 9 characters fit, the rest is dropped.
    assert_eq!(&t.bus.mem[a + 1..a + 12], b"\x09acdefghij\r");
    assert_eq!(t.row(0), "acdefghij");
    // ESC restarts the line.
    t.keys(b"xy\x1bz\r");
    assert_eq!(t.call(0x0A00, 0x200), HleResult::Done);
    assert_eq!(&t.bus.mem[a + 1..a + 4], b"\x01z\r");
}

#[test]
fn char_input_functions() {
    let mut t = T::new();
    assert_eq!(t.call(0x0100, 0), HleResult::Retry);
    assert_eq!(t.call(0x0800, 0), HleResult::Retry);
    assert_eq!(t.call(0x0B00, 0), HleResult::Done);
    assert_eq!(t.cpu.regs.al(), 0);
    t.call(0x06FF, 0xFF);
    assert_eq!(t.cpu.regs.al(), 0);
    assert!(
        t.bus.read16(linear(STACK_SEG, STACK_SP + 4)) & 0x40 != 0,
        "ZF"
    );

    t.keys(b"q");
    t.call(0x0B00, 0);
    assert_eq!(t.cpu.regs.al(), 0xFF);
    assert_eq!(t.call(0x0100, 0), HleResult::Done);
    assert_eq!(t.cpu.regs.al(), b'q');
    assert_eq!(t.row(0), "q", "01h echoes");
    t.keys(b"w");
    t.call(0x0800, 0);
    assert_eq!(t.cpu.regs.al(), b'w');
    assert_eq!(t.row(0), "q", "08h does not echo");

    // Extended key: 0, then the scan code.
    kbd::push_key(&mut t.bus, 0x4B00); // left arrow
    t.call(0x0700, 0);
    assert_eq!(t.cpu.regs.al(), 0);
    t.call(0x0700, 0);
    assert_eq!(t.cpu.regs.al(), 0x4B);

    // 0Ch flushes, then reads.
    t.keys(b"old");
    assert_eq!(t.call(0x0C01, 0), HleResult::Retry);
    t.keys(b"n");
    assert_eq!(t.call(0x0C01, 0), HleResult::Done);
    assert_eq!(t.cpu.regs.al(), b'n');
}

#[test]
fn handles_and_redirection() {
    let mut t = T::new();
    t.put(0x100, b"via handle");
    t.cpu.regs.set_bx(1);
    t.cpu.regs.set_cx(10);
    t.call(0x4000, 0x100);
    assert!(!t.cf());
    assert_eq!(t.row(0), "via handle");

    // Redirect stdout to a file: AH=09 and AH=02 land in it.
    t.put(0x300, b"C:\\OUT.TXT\0");
    t.cpu.regs.set_cx(0);
    t.call(0x3C00, 0x300);
    let file = t.cpu.regs.ax();
    t.cpu.regs.set_bx(file);
    t.cpu.regs.set_cx(1);
    t.call(0x4600, 0);
    assert!(!t.cf());
    t.put(0x100, b"redirected$");
    t.call(0x0900, 0x100);
    t.call(0x0200, b'!' as u16);
    assert_eq!(t.row(0), "via handle", "screen untouched");
    t.cpu.regs.set_bx(1);
    t.call(0x3E00, 0);
    t.cpu.regs.set_bx(file);
    t.call(0x3E00, 0);
    t.call(0x3D00, 0x300);
    let h = t.cpu.regs.ax();
    t.cpu.regs.set_bx(h);
    t.cpu.regs.set_cx(64);
    t.call(0x3F00, 0x400);
    let n = t.cpu.regs.ax() as usize;
    let a = linear(DATA, 0x400) as usize;
    assert_eq!(&t.bus.mem[a..a + n], b"redirected!");
}

#[test]
fn device_names_open_as_devices() {
    let mut t = T::new();
    for (name, info) in [
        (&b"NUL\0"[..], 0x8084),
        (b"C:\\ANY\\DIR\\nul.txt\0", 0x8084),
        (b"con\0", 0x80D3),
        (b"CLOCK$\0", 0x8088),
        (b"PRN.DAT\0", 0x80C0),
    ] {
        t.put(0x300, name);
        t.call(0x3D02, 0x300);
        assert!(!t.cf(), "{name:?}");
        let h = t.cpu.regs.ax();
        t.cpu.regs.set_bx(h);
        t.call(0x4400, 0);
        assert_eq!(t.cpu.regs.dx(), info);
    }
    // Writes to NUL vanish; reads are EOF. Handles 0-4 are devices.
    t.put(0x300, b"NUL\0");
    t.call(0x3D02, 0x300);
    let h = t.cpu.regs.ax();
    t.cpu.regs.set_bx(h);
    t.cpu.regs.set_cx(5);
    t.call(0x4000, 0x100);
    assert_eq!(t.cpu.regs.ax(), 5);
    t.cpu.regs.set_bx(h);
    t.call(0x3F00, 0x100);
    assert_eq!(t.cpu.regs.ax(), 0);
    for (h, info) in [
        (0, 0x80D3),
        (1, 0x80D3),
        (2, 0x80D3),
        (3, 0x80C0),
        (4, 0x80C0),
    ] {
        t.cpu.regs.set_bx(h);
        t.call(0x4400, 0);
        assert_eq!(t.cpu.regs.dx(), info, "handle {h}");
    }
    // A file has the device bit clear and its drive number.
    t.put(0x300, b"C:\\F.TXT\0");
    t.cpu.regs.set_cx(0);
    t.call(0x3C00, 0x300);
    let h = t.cpu.regs.ax();
    t.cpu.regs.set_bx(h);
    t.call(0x4400, 0);
    assert_eq!(t.cpu.regs.dx(), 2);
}

#[test]
fn con_handle_reads_cooked_and_raw() {
    let mut t = T::new();
    t.cpu.regs.set_bx(0);
    t.cpu.regs.set_cx(3);
    assert_eq!(t.call(0x3F00, 0x100), HleResult::Retry);
    t.keys(b"hi\r");
    t.cpu.regs.set_bx(0);
    t.cpu.regs.set_cx(3);
    assert_eq!(t.call(0x3F00, 0x100), HleResult::Done);
    assert_eq!(t.cpu.regs.ax(), 3);
    t.cpu.regs.set_bx(0);
    t.cpu.regs.set_cx(10);
    t.call(0x3F00, 0x103);
    assert_eq!(t.cpu.regs.ax(), 1, "rest of the line");
    let a = linear(DATA, 0x100) as usize;
    assert_eq!(&t.bus.mem[a..a + 4], b"hi\r\n");

    // Raw mode: returns available keys without waiting for Enter.
    t.cpu.regs.set_bx(0);
    t.call(0x4401, 0x20);
    t.keys(b"ab");
    t.cpu.regs.set_bx(0);
    t.cpu.regs.set_cx(10);
    t.call(0x3F00, 0x100);
    assert_eq!(t.cpu.regs.ax(), 2);
    t.cpu.regs.set_bx(0);
    t.call(0x4400, 0);
    assert_eq!(t.cpu.regs.dx(), 0x80F3);
}

#[test]
fn ctrl_c_issues_int23_and_ends_the_child() {
    let mut t = T::new();
    // Pretend a child process is running.
    let child = mcb::alloc(
        &mut t.bus,
        FIRST_MCB,
        0x20,
        t.dos.root_psp,
        mcb::Strategy::FirstFit,
    )
    .unwrap();
    psp::create_psp(&mut t.bus, child, child + 0x20, t.dos.root_psp, 0);
    t.bus.write32(linear(child, psp::INT22), 0x5000_0100);
    t.dos.processes.push(Process {
        psp: child,
        ss: STACK_SEG,
        sp: 0x8000,
        flags: 0x202,
    });
    t.dos.set_current_psp(&mut t.bus, child);

    t.keys(&[3]);
    assert_eq!(t.call(0x0100, 0), HleResult::Done);
    assert_eq!(t.row(0), "^C");
    let int23 = (bus_vec(&mut t.bus, 0x23).0, bus_vec(&mut t.bus, 0x23).1);
    assert_eq!((t.cpu.sregs.get(Seg::Cs), t.cpu.ip as u16), int23);
    // The pushed return address re-runs the INT 21h callback.
    assert_eq!(t.bus.read16(linear(STACK_SEG, t.cpu.regs.sp())), 0x1234);
    // Default INT 23h: terminate the child.
    t.dos.callback(ID_INT23, &mut t.cpu, &mut t.bus);
    assert!(t.dos.processes.is_empty());
    assert_eq!((t.cpu.sregs.get(Seg::Cs), t.cpu.ip), (0x5000, 0x0100));
    t.call(0x4D00, 0);
    assert_eq!(t.cpu.regs.ax(), 0x0100, "terminated by ^C");
}

fn bus_vec(bus: &mut FlatBus, v: u32) -> (u16, u16) {
    (bus.read16(v * 4 + 2), bus.read16(v * 4))
}

#[test]
fn ctrl_break_goes_through_int1b() {
    let mut t = T::new();
    // INT 1Bh points at DOS's stub.
    assert_eq!(
        bus_vec(&mut t.bus, 0x1B),
        (crate::hle::ROM_SEGMENT, ID_INT1B.stub_offset())
    );
    // Ctrl down, then E0 46 (Break).
    for code in [0x1D, 0xE0, 0x46] {
        t.bus.io_in.insert(0x60, code);
        let brk = kbd::int09(&mut t.bus);
        assert_eq!(brk, code == 0x46);
    }
    assert!(bda::read8(&mut t.bus, kbd::BREAK_FLAG) & 0x80 != 0);
    assert_eq!(kbd::peek_key(&mut t.bus), Some(0));
    // The break code alone (E0 46 / E0 C6) also counts; its release is quiet.
    for (code, brk) in [
        (0x9D, false),
        (0xE0, false),
        (0x46, true),
        (0xE0, false),
        (0xC6, false),
    ] {
        t.bus.io_in.insert(0x60, code);
        assert_eq!(kbd::int09(&mut t.bus), brk, "{code:02X}");
    }
    let scroll = bda::read8(&mut t.bus, bda::KBD_FLAGS1) & 0x10;
    assert_eq!(scroll, 0, "Scroll Lock untouched");
    t.dos.callback(ID_INT1B, &mut t.cpu, &mut t.bus);
    // The next console call reports ^C; at top level INT 23h just returns.
    assert_eq!(t.call(0x0200, b'x' as u16), HleResult::Done);
    assert_eq!(t.row(0), "^C");
    assert_eq!(bda::read8(&mut t.bus, kbd::BREAK_FLAG) & 0x80, 0);
    t.dos.callback(ID_INT23, &mut t.cpu, &mut t.bus);
    assert_eq!(t.dos.exit_code(), None);
}
