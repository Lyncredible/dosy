use super::exec::{ExecParams, parse_fcb};
use super::*;
use crate::bus::FlatBus;
use crate::vfs::MemDrive;

const DATA: u16 = 0x3000;
const STACK_SEG: u16 = 0x9000;
const STACK_SP: u16 = 0xFFF0;
const RET_CS: u16 = 0x5000;
const RET_IP: u16 = 0x1234;

/// MZ: 3-paragraph header, 0x30-byte image, CS:IP = 0001:0000,
/// SS:SP = 0002:0100, relocations at 0000:0004 and 0001:0010.
fn mz() -> Vec<u8> {
    let mut b = vec![0u8; 0x60];
    let hdr: [u16; 14] = [
        0x5A4D, 0x60, 1, 2, 3, 0x10, 0xFFFF, 2, 0x100, 0, 0, 1, 0x1C, 0,
    ];
    for (i, w) in hdr.iter().enumerate() {
        b[2 * i..2 * i + 2].copy_from_slice(&w.to_le_bytes());
    }
    for (i, w) in [4u16, 0, 0x10, 1].iter().enumerate() {
        b[0x1C + 2 * i..0x1E + 2 * i].copy_from_slice(&w.to_le_bytes());
    }
    b[0x30 + 4..0x30 + 6].copy_from_slice(&0u16.to_le_bytes());
    b[0x30 + 0x20..0x30 + 0x22].copy_from_slice(&5u16.to_le_bytes());
    b
}

struct T {
    dos: Dos,
    cpu: Cpu,
    bus: FlatBus,
}

impl T {
    fn new() -> Self {
        let mut bus = FlatBus::new();
        let mut dos = Dos::new();
        dos.install(&mut bus);
        let mut c = MemDrive::new();
        c.add_file("\\APPS\\TOOL.EXE", &mz());
        // mov ax,4C03 ; int 21h
        c.add_file("\\TOOL.COM", &[0xB8, 0x03, 0x4C, 0xCD, 0x21]);
        dos.mount(2, Box::new(c));
        dos.set_current_psp(&mut bus, dos.root_psp);
        bus.write8(linear(DOS_SEG, SDA_OFF + SDA_DRIVE), 2);
        let mut cpu = Cpu::new();
        cpu.sregs.set(Seg::Ds, DATA);
        cpu.sregs.set(Seg::Es, DATA);
        Self { dos, cpu, bus }
    }

    fn free(&mut self) -> u32 {
        mcb::walk(&mut self.bus, FIRST_MCB)
            .unwrap()
            .iter()
            .filter(|m| m.owner == mcb::FREE)
            .map(|m| m.size as u32)
            .sum()
    }

    /// INT 21h with an IRET frame returning to RET_CS:RET_IP. Returns CF
    /// as the caller sees it (frame or CPU, whichever is live).
    fn int21(&mut self, ax: u16, bx: u16, dx: u16) -> bool {
        self.cpu.regs.set_ax(ax);
        self.cpu.regs.set_bx(bx);
        self.cpu.regs.set_dx(dx);
        self.cpu.sregs.set(Seg::Ds, DATA);
        self.cpu.sregs.set(Seg::Ss, STACK_SEG);
        self.cpu.regs.set_sp(STACK_SP);
        let f = linear(STACK_SEG, STACK_SP);
        self.bus.write16(f, RET_IP);
        self.bus.write16(f + 2, RET_CS);
        self.bus.write16(f + 4, 0x0203);
        self.dos.callback(ID_INT21, &mut self.cpu, &mut self.bus);
        if self.cpu.regs.sp() == STACK_SP && self.cpu.sregs.get(Seg::Ss) == STACK_SEG {
            self.bus.read16(f + 4) & 1 != 0
        } else {
            self.cpu.flags & flags::CF != 0
        }
    }

    /// INT 21h/4Bxx for `path` with tail `tail` and FCBs from DATA.
    fn exec(&mut self, al: u8, path: &str, tail: &str) -> bool {
        path::write_asciiz(&mut self.bus, DATA, 0, path.as_bytes());
        let t = linear(DATA, 0x100);
        self.bus.write8(t, tail.len() as u8);
        self.bus.load(t + 1, tail.as_bytes());
        self.bus.write8(t + 1 + tail.len() as u32, 0x0D);
        self.bus.load(linear(DATA, 0x180), &parse_fcb("FOO.TXT"));
        let pb = linear(DATA, 0x200);
        for (i, w) in [0u16, 0x100, DATA, 0x180, DATA, 0x180, DATA]
            .iter()
            .enumerate()
        {
            self.bus.write16(pb + 2 * i as u32, *w);
        }
        self.cpu.sregs.set(Seg::Es, DATA);
        self.int21(0x4B00 | al as u16, 0x200, 0)
    }
}

#[test]
fn mz_loads_with_relocations_and_returns_to_parent() {
    let mut t = T::new();
    let before = t.free();
    assert!(!t.exec(0, "C:\\APPS\\TOOL.EXE", " ARGS"));
    let child = t.dos.current_psp(&mut t.bus);
    assert_ne!(child, t.dos.root_psp);
    let start = child + 0x10;
    let c = &t.cpu;
    assert_eq!((c.sregs.get(Seg::Cs), c.ip), (start + 1, 0));
    assert_eq!((c.sregs.get(Seg::Ss), c.regs.sp()), (start + 2, 0x100));
    assert_eq!((c.sregs.get(Seg::Ds), c.sregs.get(Seg::Es)), (child, child));
    assert_eq!(t.bus.read16(linear(start, 4)), start);
    assert_eq!(t.bus.read16(linear(start + 1, 0x10)), start + 5);

    // PSP: tail, FCB, parent, env with the program path; MCB names.
    let p = |off| linear(child, off);
    assert_eq!(&t.bus.mem[p(0x80) as usize..][..7], b"\x05 ARGS\r");
    assert_eq!(&t.bus.mem[p(0x5C) as usize..][..12], b"\0FOO     TXT");
    assert_eq!(t.bus.read16(p(psp::PARENT)), t.dos.root_psp);
    let env = t.bus.read16(p(psp::ENV));
    let text = &t.bus.mem[linear(env, 0) as usize..][..80];
    let needle = b"\0\0\x01\0C:\\APPS\\TOOL.EXE\0";
    assert!(text.windows(needle.len()).any(|w| w == needle));
    assert!(text.starts_with(b"COMSPEC="));
    assert_eq!(&t.bus.mem[linear(child - 1, 8) as usize..][..5], b"TOOL\0");
    assert_eq!(mcb::read(&mut t.bus, env - 1).owner, child);

    // Child exits: parent resumes after its INT 21h, memory is freed.
    assert!(!t.int21(0x4C05, 0, 0));
    let c = &t.cpu;
    assert_eq!((c.sregs.get(Seg::Cs), c.ip), (RET_CS, RET_IP as u32));
    assert_eq!(
        (c.sregs.get(Seg::Ss), c.regs.sp()),
        (STACK_SEG, STACK_SP + 6)
    );
    assert_eq!(c.flags & 0xFFF, 0x202);
    assert_eq!(t.dos.current_psp(&mut t.bus), t.dos.root_psp);
    assert_eq!(t.free(), before);
    assert!(!t.int21(0x4D00, 0, 0));
    assert_eq!(t.cpu.regs.ax(), 5);
}

#[test]
fn com_register_state_and_fcbs() {
    let mut t = T::new();
    let p = ExecParams::from_tail(" A:FOO.TXT *.BAT");
    t.dos
        .exec(&mut t.cpu, &mut t.bus, b"\\TOOL.COM", &p)
        .unwrap();
    let child = t.dos.current_psp(&mut t.bus);
    let c = &t.cpu;
    for s in [Seg::Cs, Seg::Ds, Seg::Es, Seg::Ss] {
        assert_eq!(c.sregs.get(s), child);
    }
    assert_eq!((c.ip, c.regs.sp(), c.regs.ax()), (0x100, 0xFFFE, 0));
    assert_eq!(t.bus.read16(linear(child, 0xFFFE)), 0);
    assert_eq!(t.bus.read8(linear(child, 0x100)), 0xB8);
    let at = |off| linear(child, off) as usize;
    assert_eq!(&t.bus.mem[at(0x5C)..][..12], b"\x01FOO     TXT");
    assert_eq!(&t.bus.mem[at(0x6C)..][..12], b"\0????????BAT");
    assert_eq!(t.bus.read8(linear(child, 0x80)), 16);
    assert_eq!(t.bus.read8(linear(child, 0x81 + 16)), 0x0D);
    // The child owns the largest block: top of memory in its PSP.
    assert_eq!(t.bus.read16(linear(child, psp::MEM_TOP)), 0xA000);
}

#[test]
fn nested_exec_closes_files_and_frees_memory() {
    let mut t = T::new();
    let before = t.free();
    let open_files = |t: &T| t.dos.sft.iter().filter(|e| e.is_some()).count();
    let files = open_files(&t);

    assert!(!t.exec(0, "C:\\APPS\\TOOL.EXE", ""));
    let a = t.dos.current_psp(&mut t.bus);
    // A frees its excess memory so it can run a child.
    t.cpu.sregs.set(Seg::Es, a);
    assert!(!t.int21(0x4A00, 0x100, 0));
    assert!(!t.exec(0, "C:\\TOOL.COM", ""));
    let b = t.dos.current_psp(&mut t.bus);
    assert_eq!(t.bus.read16(linear(b, psp::PARENT)), a);
    // B opens a file and exits without closing it.
    path::write_asciiz(&mut t.bus, DATA, 0, b"C:\\TOOL.COM");
    assert!(!t.int21(0x3D00, 0, 0));
    assert_eq!(open_files(&t), files + 1);
    assert!(!t.int21(0x4C02, 0, 0));
    assert_eq!(open_files(&t), files);
    assert_eq!(t.dos.current_psp(&mut t.bus), a);
    assert!(!t.int21(0x4D00, 0, 0));
    assert_eq!(t.cpu.regs.ax(), 2);

    assert!(!t.int21(0x4C03, 0, 0));
    assert_eq!(t.dos.current_psp(&mut t.bus), t.dos.root_psp);
    assert_eq!(t.free(), before);
    assert!(t.dos.processes.is_empty());
    assert_eq!(t.dos.exit_code(), None);
    // SFT references of the inherited std handles are back to normal.
    assert_eq!(t.dos.sft[1].as_ref().unwrap().refs, 3);
}

#[test]
fn load_only_and_overlay() {
    let mut t = T::new();
    assert!(!t.exec(1, "C:\\APPS\\TOOL.EXE", ""));
    let child = t.dos.current_psp(&mut t.bus);
    let pb = linear(DATA, 0x200);
    let start = child + 0x10;
    assert_eq!(t.bus.read16(pb + 0x0E), 0x100);
    assert_eq!(t.bus.read16(pb + 0x10), start + 2);
    assert_eq!(t.bus.read16(pb + 0x12), 0);
    assert_eq!(t.bus.read16(pb + 0x14), start + 1);
    assert_eq!(t.cpu.sregs.get(Seg::Cs), 0xF000, "CPU not redirected");
    assert_eq!(t.cpu.sregs.get(Seg::Ss), STACK_SEG);

    // Overlay at 6000h with relocation factor 1234h.
    path::write_asciiz(&mut t.bus, DATA, 0, b"C:\\APPS\\TOOL.EXE");
    t.bus.write16(pb, 0x6000);
    t.bus.write16(pb + 2, 0x1234);
    assert!(!t.int21(0x4B03, 0x200, 0));
    assert_eq!(t.bus.read16(linear(0x6000, 4)), 0x1234);
    assert_eq!(t.bus.read16(linear(0x6001, 0x10)), 0x1239);
}

#[test]
fn exec_errors_return_to_caller() {
    let mut t = T::new();
    assert!(t.exec(0, "C:\\NONE.EXE", ""));
    assert_eq!(t.cpu.regs.ax(), 2);
    assert_eq!(
        (t.cpu.sregs.get(Seg::Cs), t.cpu.ip),
        (RET_CS, RET_IP as u32)
    );
    assert_eq!(t.cpu.regs.sp(), STACK_SP + 6);

    // No memory: grab everything first.
    let before = t.free();
    assert!(t.int21(0x4800, 0xFFFF, 0));
    let largest = t.cpu.regs.bx();
    assert!(!t.int21(0x4800, largest, 0));
    assert!(t.exec(0, "C:\\TOOL.COM", ""));
    assert_eq!(t.cpu.regs.ax(), 8);
    assert!(t.exec(7, "C:\\TOOL.COM", ""));
    assert_eq!(t.cpu.regs.ax(), 1);
    assert_eq!(t.free(), before - largest as u32, "env block released");
}

#[test]
fn tsr_keeps_memory() {
    let mut t = T::new();
    let before = t.free();
    assert!(!t.exec(0, "C:\\TOOL.COM", ""));
    let child = t.dos.current_psp(&mut t.bus);
    assert!(!t.int21(0x3101, 0, 0x20));
    assert_eq!(mcb::read(&mut t.bus, child - 1).size, 0x20);
    assert_eq!(mcb::read(&mut t.bus, child - 1).owner, child);
    assert!(t.free() < before);
    assert!(!t.int21(0x4D00, 0, 0));
    assert_eq!(t.cpu.regs.ax(), 0x0301);
}

#[test]
fn fcb_parsing() {
    assert_eq!(&parse_fcb("c:foo.t*"), b"\x03FOO     T??");
    assert_eq!(&parse_fcb("ab*"), b"\0AB??????   ");
    assert_eq!(&parse_fcb(""), b"\0           ");
    assert_eq!(&parse_fcb("toolong12.exe/x"), b"\0TOOLONG1EXE");
}

#[test]
fn no_inherit_handles_stay_with_the_parent() {
    let mut t = T::new();
    path::write_asciiz(&mut t.bus, DATA, 0, b"C:\\TOOL.COM");
    assert!(!t.int21(0x3D00, 0, 0));
    let shared = t.cpu.regs.ax();
    assert!(!t.int21(0x3D80, 0, 0)); // read, no-inherit
    let private = t.cpu.regs.ax();
    let root = t.dos.root_psp;
    let sft_of = |t: &T, p: u16, h: u16| t.bus.mem[linear(p, psp::JFT + h) as usize];
    let refs = |t: &T, h: u16| {
        let i = sft_of(t, root, h) as usize;
        t.dos.sft[i].as_ref().unwrap().refs
    };
    assert!(!t.exec(0, "C:\\APPS\\TOOL.EXE", ""));
    let child = t.dos.current_psp(&mut t.bus);
    assert_eq!(sft_of(&t, child, shared), sft_of(&t, root, shared));
    assert_eq!(sft_of(&t, child, private), files::JFT_FREE);
    assert_eq!((refs(&t, shared), refs(&t, private)), (2, 1));
}
