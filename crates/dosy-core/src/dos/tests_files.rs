use super::*;
use crate::bus::FlatBus;
use crate::vfs::{Attr, MemDrive};

const DATA: u16 = 0x3000;
const STACK_SEG: u16 = 0x9000;
const STACK_SP: u16 = 0xFFF0;
const C: u8 = 2;
const D: u8 = 3;

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
        c.add_file("\\README.TXT", b"readme");
        dos.mount(C, Box::new(c));
        dos.mount(D, Box::new(MemDrive::new()));
        let mut cpu = Cpu::new();
        cpu.sregs.set(Seg::Ds, DATA);
        cpu.sregs.set(Seg::Es, DATA);
        Self { dos, cpu, bus }
    }

    /// Place an ASCIIZ string at DATA:off.
    fn s(&mut self, off: u16, s: &str) -> u16 {
        path::write_asciiz(&mut self.bus, DATA, off, s.as_bytes());
        off
    }

    /// INT 21h; Ok(AX) or Err(error code).
    fn call(&mut self, ax: u16, bx: u16, cx: u16, dx: u16) -> Result<u16, u16> {
        let r = &mut self.cpu.regs;
        r.set_ax(ax);
        r.set_bx(bx);
        r.set_cx(cx);
        r.set_dx(dx);
        self.cpu.sregs.set(Seg::Ss, STACK_SEG);
        r.set_sp(STACK_SP);
        let frame = linear(STACK_SEG, STACK_SP);
        self.bus.write16(frame + 4, 0x0202);
        self.dos.callback(ID_INT21, &mut self.cpu, &mut self.bus);
        let ax = self.cpu.regs.ax();
        if self.bus.read16(frame + 4) & 1 != 0 {
            Err(ax)
        } else {
            Ok(ax)
        }
    }

    fn create(&mut self, name: &str) -> Result<u16, u16> {
        let p = self.s(0, name);
        self.call(0x3C00, 0, 0, p)
    }

    fn open(&mut self, name: &str, mode: u8) -> Result<u16, u16> {
        let p = self.s(0, name);
        self.call(0x3D00 | mode as u16, 0, 0, p)
    }

    fn write(&mut self, h: u16, bytes: &[u8]) -> Result<u16, u16> {
        self.bus.load(linear(DATA, 0x400), bytes);
        self.call(0x4000, h, bytes.len() as u16, 0x400)
    }

    fn read(&mut self, h: u16, n: u16) -> Result<Vec<u8>, u16> {
        let got = self.call(0x3F00, h, n, 0x400)?;
        let a = linear(DATA, 0x400) as usize;
        Ok(self.bus.mem[a..a + got as usize].to_vec())
    }

    fn seek(&mut self, h: u16, whence: u8, off: i32) -> Result<u32, u16> {
        let off = off as u32;
        self.call(0x4200 | whence as u16, h, (off >> 16) as u16, off as u16)?;
        Ok((self.cpu.regs.dx() as u32) << 16 | self.cpu.regs.ax() as u32)
    }

    fn path_call(&mut self, ax: u16, name: &str) -> Result<u16, u16> {
        let p = self.s(0, name);
        self.call(ax, 0, 0, p)
    }

    /// Names from FindFirst/FindNext with `attr`.
    fn find(&mut self, pattern: &str, attr: u16) -> Result<Vec<String>, u16> {
        let p = self.s(0, pattern);
        self.call(0x1A00, 0, 0, 0x200)?; // DTA = DATA:0200
        let mut out = Vec::new();
        let mut r = self.call(0x4E00, 0, attr, p);
        while r.is_ok() {
            let a = linear(DATA, 0x200 + 0x1E) as usize;
            let name = self.bus.mem[a..a + 13].split(|&b| b == 0).next().unwrap();
            out.push(String::from_utf8(name.to_vec()).unwrap());
            r = self.call(0x4F00, 0, 0, 0);
        }
        match r {
            Err(0x12) if !out.is_empty() => Ok(out),
            Err(e) => Err(e),
            Ok(_) => unreachable!(),
        }
    }

    fn cwd(&mut self, dl: u8) -> Result<String, u16> {
        self.cpu.regs.set_si(0x300);
        self.call(0x4700, 0, 0, dl as u16)?;
        let a = linear(DATA, 0x300) as usize;
        let s = self.bus.mem[a..a + 64].split(|&b| b == 0).next().unwrap();
        Ok(String::from_utf8(s.to_vec()).unwrap())
    }
}

#[test]
fn create_write_seek_read() {
    let mut t = T::new();
    t.call(0x0E00, 0, 0, C as u16).unwrap();
    let h = t.create("C:\\FOO.TXT").unwrap();
    assert_eq!(h, 5, "handles 0-4 are the devices");
    assert_eq!(t.write(h, b"hello world"), Ok(11));
    assert_eq!(t.seek(h, 0, 0), Ok(0));
    assert_eq!(t.read(h, 5).unwrap(), b"hello");
    assert_eq!(t.seek(h, 1, -2), Ok(3));
    assert_eq!(t.read(h, 3).unwrap(), b"lo ");
    assert_eq!(t.seek(h, 2, 0), Ok(11));
    assert_eq!(t.seek(h, 0, 5), Ok(5));
    assert_eq!(t.write(h, b""), Ok(0), "CX=0 truncates");
    t.call(0x3E00, h, 0, 0).unwrap();

    let h = t.open("foo.txt", 0).unwrap();
    assert_eq!(t.read(h, 100).unwrap(), b"hello");
    assert_eq!(t.read(h, 100).unwrap(), b"", "EOF");
    // File time round trip.
    t.call(0x5701, h, 0x6000, 0x1C2A).unwrap();
    t.call(0x5700, h, 0, 0).unwrap();
    assert_eq!((t.cpu.regs.cx(), t.cpu.regs.dx()), (0x6000, 0x1C2A));
    t.call(0x3E00, h, 0, 0).unwrap();

    // Attributes, rename and delete.
    assert_eq!(t.path_call(0x4300, "FOO.TXT"), Ok(t.cpu.regs.ax()));
    assert_eq!(t.cpu.regs.cx(), Attr::ARCHIVE.0 as u16);
    let from = t.s(0, "FOO.TXT");
    let to = t.s(0x80, "BAR.TXT");
    t.cpu.regs.set_di(to);
    t.call(0x5600, 0, 0, from).unwrap();
    assert_eq!(t.open("FOO.TXT", 0), Err(2));
    t.path_call(0x4100, "BAR.TXT").unwrap();
    assert_eq!(t.open("BAR.TXT", 0), Err(2));
}

#[test]
fn long_names_truncate_like_dos() {
    let mut t = T::new();
    let h = t.create("C:\\LONGFILENAME.TEXT").unwrap();
    t.write(h, b"x").unwrap();
    t.call(0x3E00, h, 0, 0).unwrap();
    assert_eq!(
        t.find("C:\\*.*", 0).unwrap(),
        ["LONGFILE.TEX", "README.TXT"]
    );
    assert!(t.open("C:\\LONGFILEXYZ.TEXTS", 0).is_ok());
    assert_eq!(t.find("C:\\LONGFILENAME.T*", 0).unwrap(), ["LONGFILE.TEX"]);
}

#[test]
fn find_lists_in_order_with_attribute_filter() {
    let mut t = T::new();
    for name in ["C:\\B.TXT", "C:\\A.TXT", "C:\\C.EXE", "C:\\HID.TXT"] {
        let h = t.create(name).unwrap();
        t.call(0x3E00, h, 0, 0).unwrap();
    }
    t.path_call(0x3900, "C:\\SUB").unwrap();
    let p = t.s(0, "C:\\HID.TXT");
    t.call(0x4301, 0, Attr::HIDDEN.0 as u16, p).unwrap();

    assert_eq!(
        t.find("C:\\*.TXT", 0).unwrap(),
        ["A.TXT", "B.TXT", "README.TXT"]
    );
    assert_eq!(
        t.find("C:\\*.*", 0).unwrap(),
        ["A.TXT", "B.TXT", "C.EXE", "README.TXT"]
    );
    assert_eq!(
        t.find("C:\\*.*", 0x12).unwrap(),
        ["A.TXT", "B.TXT", "C.EXE", "HID.TXT", "README.TXT", "SUB"]
    );
    assert_eq!(t.find("C:\\?.*", 0).unwrap(), ["A.TXT", "B.TXT", "C.EXE"]);
    assert_eq!(t.find("C:\\*.COM", 0), Err(0x12));
    assert_eq!(t.find("C:\\*.*", 0x08), Err(0x12), "volume label ignored");
    assert_eq!(t.find("C:\\NONE\\*.*", 0), Err(3));
    // The DTA records attributes and size of the match.
    t.find("C:\\README.TXT", 0).unwrap();
    assert_eq!(t.bus.read32(linear(DATA, 0x200 + 0x1A)), 6);
}

#[test]
fn error_codes() {
    let mut t = T::new();
    assert_eq!(t.open("C:\\MISSING.TXT", 0), Err(2));
    assert_eq!(t.open("C:\\NODIR\\X.TXT", 0), Err(3));
    assert_eq!(t.open("Q:\\X.TXT", 0), Err(3));
    assert_eq!(t.open("C:\\README.TXT", 3), Err(0x0C));
    assert_eq!(t.call(0x3E00, 9, 0, 0), Err(6));
    assert_eq!(t.call(0x3E00, 99, 0, 0), Err(6));
    let h = t.open("C:\\README.TXT", 1).unwrap();
    assert_eq!(t.read(h, 4), Err(5));
    let h = t.open("C:\\README.TXT", 0).unwrap();
    assert_eq!(t.write(h, b"x"), Err(5));
    let p = t.s(0, "C:\\README.TXT");
    assert_eq!(t.call(0x5B00, 0, 0, p), Err(0x50));
    let from = t.s(0, "C:\\README.TXT");
    let to = t.s(0x80, "D:\\README.TXT");
    t.cpu.regs.set_di(to);
    assert_eq!(t.call(0x5600, 0, 0, from), Err(0x11));
    assert_eq!(t.path_call(0x3B00, "C:\\README.TXT"), Err(3));
    assert_eq!(t.path_call(0x3B00, "C:\\.."), Err(3));
    assert_eq!(t.cwd(10), Err(0x0F));
    assert_eq!(t.call(0x7300, 0, 0, 0), Err(1));
}

#[test]
fn cwd_per_drive() {
    let mut t = T::new();
    t.call(0x0E00, 0, 0, C as u16).unwrap();
    assert_eq!(t.cpu.regs.al(), 26);
    t.path_call(0x3900, "C:\\APPS").unwrap();
    t.path_call(0x3900, "D:\\TOOL").unwrap();
    t.path_call(0x3B00, "\\APPS").unwrap();
    t.path_call(0x3B00, "D:TOOL").unwrap();
    assert_eq!(t.cwd(0).unwrap(), "APPS");
    assert_eq!(t.cwd(3).unwrap(), "APPS");
    assert_eq!(t.cwd(4).unwrap(), "TOOL");
    assert_eq!(t.path_call(0x3A00, "C:\\APPS"), Err(0x10));

    // Relative names use the drive's cwd.
    t.call(0x0E00, 0, 0, D as u16).unwrap();
    t.call(0x1900, 0, 0, 0).unwrap();
    assert_eq!(t.cpu.regs.al(), D);
    let h = t.create("FOO.TXT").unwrap();
    t.call(0x3E00, h, 0, 0).unwrap();
    assert_eq!(t.find("D:\\TOOL\\*.*", 0).unwrap(), ["FOO.TXT"]);
    assert_eq!(t.find("C:*.*", 0), Err(0x12));
    t.path_call(0x3B00, "..").unwrap();
    assert_eq!(t.cwd(0).unwrap(), "");
    assert_eq!(t.find("tool/./foo.txt", 0).unwrap(), ["FOO.TXT"]);
    t.path_call(0x3B00, "C:..").unwrap();
    t.path_call(0x3A00, "C:\\APPS").unwrap();

    // Selecting an unmounted drive keeps the current one.
    t.call(0x0E00, 0, 0, 20).unwrap();
    t.call(0x1900, 0, 0, 0).unwrap();
    assert_eq!(t.cpu.regs.al(), D);
}

#[test]
fn handle_exhaustion_and_dup() {
    let mut t = T::new();
    let mut handles = Vec::new();
    for _ in 5..20 {
        handles.push(t.open("C:\\README.TXT", 0).unwrap());
    }
    assert_eq!(t.open("C:\\README.TXT", 0), Err(4));
    assert_eq!(t.create("C:\\NEW.TXT"), Err(4));
    assert_eq!(t.open("C:\\NEW.TXT", 0), Err(4), "nothing was created");
    assert_eq!(t.call(0x4500, handles[0], 0, 0), Err(4));
    for &h in &handles[1..] {
        t.call(0x3E00, h, 0, 0).unwrap();
    }

    // A duplicate shares the file position and survives closing the original.
    let h = handles[0];
    let d = t.call(0x4500, h, 0, 0).unwrap();
    assert_eq!(t.read(h, 2).unwrap(), b"re");
    assert_eq!(t.read(d, 2).unwrap(), b"ad");
    t.call(0x3E00, h, 0, 0).unwrap();
    assert_eq!(t.read(d, 2).unwrap(), b"me");
    // Force-dup onto stdout, then close both.
    t.call(0x4600, d, 1, 0).unwrap();
    assert_eq!(t.seek(1, 0, 0), Ok(0));
    assert_eq!(t.read(1, 3).unwrap(), b"rea");
    t.call(0x3E00, d, 0, 0).unwrap();
    t.call(0x3E00, 1, 0, 0).unwrap();
    assert!(t.dos.sft.iter().skip(3).all(|e| e.is_none()));
}

#[test]
fn free_space() {
    let mut t = T::new();
    t.call(0x3600, 0, 0, 3).unwrap();
    let r = &t.cpu.regs;
    assert_eq!((r.ax(), r.cx(), r.dx()), (4, 512, 0x8000));
    assert!(r.bx() > 0 && r.bx() < 0x8000);
    t.call(0x3600, 0, 0, 9).unwrap();
    assert_eq!(t.cpu.regs.ax(), 0xFFFF);
}

#[test]
fn find_lists_dot_entries_in_subdirectories() {
    let mut t = T::new();
    t.path_call(0x3900, "D:\\TOOL").unwrap();
    let h = t.create("D:\\TOOL\\FOO.TXT").unwrap();
    t.call(0x3E00, h, 0, 0).unwrap();
    assert_eq!(
        t.find("D:\\TOOL\\*.*", 0x10).unwrap(),
        [".", "..", "FOO.TXT"]
    );
    assert_eq!(t.find("D:\\TOOL\\*.*", 0).unwrap(), ["FOO.TXT"]);
    assert_eq!(t.find("D:\\TOOL\\*", 0x10).unwrap(), [".", ".."]);
    assert_eq!(
        t.find("D:\\*.*", 0x10).unwrap(),
        ["TOOL"],
        "no dots in the root"
    );
    let p = t.s(0, "D:\\TOOL\\*.*");
    t.call(0x4E00, 0, 0x10, p).unwrap();
    assert_eq!(t.bus.read8(linear(DATA, 0x200 + 0x15)), 0x10);
}

#[test]
fn find_next_resumes_from_the_snapshot() {
    let mut t = T::new();
    for n in ["D:\\A.TXT", "D:\\B.TXT", "D:\\C.TXT"] {
        let h = t.create(n).unwrap();
        t.call(0x3E00, h, 0, 0).unwrap();
    }
    t.call(0x1A00, 0, 0, 0x200).unwrap();
    let p = t.s(0, "D:\\*.TXT");
    t.call(0x4E00, 0, 0, p).unwrap();
    // Changes after FindFirst don't disturb the running search.
    t.path_call(0x4100, "D:\\B.TXT").unwrap();
    t.create("D:\\0.TXT").unwrap();
    let name = |t: &T| {
        let a = linear(DATA, 0x200 + 0x1E) as usize;
        t.bus.mem[a..a + 5].to_vec()
    };
    t.call(0x4F00, 0, 0, 0).unwrap();
    assert_eq!(name(&t), b"B.TXT");
    t.call(0x4F00, 0, 0, 0).unwrap();
    assert_eq!(name(&t), b"C.TXT");
    assert_eq!(t.call(0x4F00, 0, 0, 0), Err(0x12));
}

#[test]
fn code_page_bytes_in_names() {
    let mut t = T::new();
    path::write_asciiz(&mut t.bus, DATA, 0, b"D:\\CAF\x82");
    t.call(0x3900, 0, 0, 0).unwrap();
    path::write_asciiz(&mut t.bus, DATA, 0, b"D:\\caf\x82\\\x81BER.txt");
    let h = t.call(0x3C00, 0, 0, 0).unwrap();
    t.call(0x3E00, h, 0, 0).unwrap();
    t.call(0x1A00, 0, 0, 0x200).unwrap();
    path::write_asciiz(&mut t.bus, DATA, 0, b"D:\\CAF\x82\\*.*");
    t.call(0x4E00, 0, 0, 0).unwrap();
    let a = linear(DATA, 0x200 + 0x1E) as usize;
    assert_eq!(&t.bus.mem[a..a + 9], b"\x81BER.TXT\0");
    path::write_asciiz(&mut t.bus, DATA, 0, b"D:\\CAF\x82");
    t.call(0x3B00, 0, 0, 0).unwrap();
    t.cpu.regs.set_si(0x300);
    t.call(0x4700, 0, 0, 4).unwrap();
    let a = linear(DATA, 0x300) as usize;
    assert_eq!(&t.bus.mem[a..a + 5], b"CAF\x82\0");
}
