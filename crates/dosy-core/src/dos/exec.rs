//! COM/MZ loader and INT 21h/4Bh EXEC (modes 00 load+run, 01 load, 03
//! overlay). The parent's CS:IP, SS:SP and FLAGS at the time of `exec` are
//! its resume point: the child's PSP INT 22h holds CS:IP and the pushed
//! [`Process`] holds the stack and flags.

use super::path::read_asciiz;
use super::{Dos, Process, mcb, psp};
use crate::bus::{Bus, linear};
use crate::cpu::{Cpu, Seg, flags};
use crate::vfs::{DosError, OpenMode, SeekFrom};

/// Largest program file read (conventional memory can't hold more).
const MAX_FILE: u64 = 0xA0000;

/// MZ header fields we use.
struct Mz {
    image_off: usize,
    image_len: usize,
    min_alloc: u16,
    max_alloc: u16,
    ss: u16,
    sp: u16,
    ip: u16,
    cs: u16,
    relocs: Vec<(u16, u16)>,
}

fn word(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn parse_mz(b: &[u8]) -> Option<Mz> {
    if b.len() < 0x1C || !(b.starts_with(b"MZ") || b.starts_with(b"ZM")) {
        return None;
    }
    let (last, pages) = (word(b, 2) as usize, word(b, 4) as usize);
    let mut size = pages * 512;
    if last != 0 {
        size = size.saturating_sub(512 - last);
    }
    let image_off = word(b, 8) as usize * 16;
    let size = size.min(b.len());
    let image_len = size.saturating_sub(image_off);
    let (count, table) = (word(b, 6) as usize, word(b, 0x18) as usize);
    let relocs = (0..count)
        .map(|i| table + 4 * i)
        .take_while(|&o| o + 4 <= b.len())
        .map(|o| (word(b, o), word(b, o + 2)))
        .collect();
    Some(Mz {
        image_off: image_off.min(b.len()),
        image_len,
        min_alloc: word(b, 0x0A),
        max_alloc: word(b, 0x0C),
        ss: word(b, 0x0E),
        sp: word(b, 0x10),
        ip: word(b, 0x14),
        cs: word(b, 0x16),
        relocs,
    })
}

/// Parse a command-line argument into an FCB name (`drive` + 11 bytes),
/// like INT 21h/29h: `*` expands to `?`, missing fields are spaces.
pub fn parse_fcb(arg: &str) -> [u8; 12] {
    let mut fcb = [b' '; 12];
    fcb[0] = 0;
    let mut s = arg.as_bytes();
    if let [l, b':', rest @ ..] = s {
        if l.is_ascii_alphabetic() {
            fcb[0] = l.to_ascii_uppercase() - b'A' + 1;
            s = rest;
        }
    }
    let end = s
        .iter()
        .position(|c| b" \t,;=+/\\\"[]<>|".contains(c))
        .unwrap_or(s.len());
    let s = &s[..end];
    let (base, ext) = match s.iter().position(|&c| c == b'.') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, &[][..]),
    };
    let (name, ext_field) = fcb[1..].split_at_mut(8);
    for (field, src) in [(name, base), (ext_field, ext)] {
        for (i, &c) in src.iter().take(field.len()).enumerate() {
            if c == b'*' {
                field[i..].fill(b'?');
                break;
            }
            field[i] = c.to_ascii_uppercase();
        }
    }
    fcb
}

/// What `load` produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub psp: u16,
    pub cs: u16,
    pub ip: u16,
    pub ss: u16,
    pub sp: u16,
}

/// Inputs for a process load.
pub struct ExecParams<'a> {
    /// Environment to copy; 0 = the parent's.
    pub env: u16,
    /// Command tail without the count byte or CR (max 126 bytes).
    pub tail: &'a [u8],
    pub fcb1: [u8; 16],
    pub fcb2: [u8; 16],
}

impl<'a> ExecParams<'a> {
    /// Parameters as the shell builds them: FCBs parsed from the tail.
    pub fn from_tail(tail: &'a str) -> Self {
        let mut args = tail.split_ascii_whitespace();
        let f = |a: Option<&str>| {
            let mut fcb = [0u8; 16];
            fcb[..12].copy_from_slice(&parse_fcb(a.unwrap_or("")));
            fcb
        };
        let (fcb1, fcb2) = (f(args.next()), f(args.next()));
        Self {
            env: 0,
            tail: tail.as_bytes(),
            fcb1,
            fcb2,
        }
    }
}

impl Dos {
    /// Read a whole program file.
    fn read_program<B: Bus>(
        &mut self,
        bus: &mut B,
        raw: &[u8],
    ) -> Result<(Vec<u8>, String), DosError> {
        let (drive, path) = self.resolve_path(bus, raw)?;
        let mut f = self.drive_mut(drive)?.open(&path, OpenMode::Read)?;
        let len = f.seek(SeekFrom::End(0))?;
        if len > MAX_FILE {
            return Err(DosError::InsufficientMemory);
        }
        f.seek(SeekFrom::Start(0))?;
        let mut bytes = vec![0; len as usize];
        let mut n = 0;
        while n < bytes.len() {
            match f.read(&mut bytes[n..])? {
                0 => break,
                k => n += k,
            }
        }
        bytes.truncate(n);
        Ok((bytes, format!("{}:{}", (b'A' + drive) as char, path)))
    }

    /// Copy environment `src` (variables only) plus `program` into a new
    /// block; returns its segment (owner set later).
    fn copy_env<B: Bus>(&mut self, bus: &mut B, src: u16, program: &str) -> Result<u16, DosError> {
        let mut vars = Vec::new();
        if src != 0 {
            for i in 0..0x8000u16 {
                let b = bus.read8(linear(src, i));
                vars.push(b);
                if b == 0 && (vars.len() == 1 || vars[vars.len() - 2] == 0) {
                    break;
                }
            }
        }
        if vars.is_empty() {
            vars.push(0);
        }
        if vars.len() == 1 {
            vars.push(0); // empty environment: two NULs
        }
        vars.extend_from_slice(&[1, 0]);
        vars.extend_from_slice(program.as_bytes());
        vars.push(0);
        let paras = vars.len().div_ceil(16) as u16;
        let owner = self.current_psp(bus);
        let seg = mcb::alloc(bus, super::FIRST_MCB, paras, owner, mcb::Strategy::FirstFit)
            .map_err(|_| DosError::InsufficientMemory)?;
        for (i, b) in vars.into_iter().enumerate() {
            bus.write8(linear(seg, i as u16), b);
        }
        Ok(seg)
    }

    fn set_owner<B: Bus>(bus: &mut B, seg: u16, owner: u16, name: &[u8]) {
        let mut m = mcb::read(bus, seg - 1);
        m.owner = owner;
        mcb::write(bus, &m);
        mcb::set_name(bus, seg - 1, name);
    }

    /// Load a program as a child of the current process: environment,
    /// PSP (inheriting handles), image, relocations. Does not switch to it.
    pub fn load<B: Bus>(
        &mut self,
        bus: &mut B,
        raw: &[u8],
        p: &ExecParams,
    ) -> Result<Loaded, DosError> {
        let (bytes, full_path) = self.read_program(bus, raw)?;
        let parent = self.current_psp(bus);
        let env_src = match p.env {
            0 => bus.read16(linear(parent, psp::ENV)),
            e => e,
        };
        let mz = parse_mz(&bytes);
        let image_paras = match &mz {
            Some(h) => h.image_len.div_ceil(16) as u32,
            None => (bytes.len() as u32 + 0x100).div_ceil(16),
        };
        let env = self.copy_env(bus, env_src, &full_path)?;
        let fail = |bus: &mut B, e| {
            let _ = mcb::free(bus, super::FIRST_MCB, env);
            Err(e)
        };

        let blocks = match mcb::walk(bus, super::FIRST_MCB) {
            Ok(b) => b,
            Err(_) => return fail(bus, DosError::InsufficientMemory),
        };
        let largest = mcb::largest_free(&blocks) as u32;
        let (need, want) = match &mz {
            Some(h) => {
                let base = psp::PARAS as u32 + image_paras;
                (base + h.min_alloc as u32, base + h.max_alloc as u32)
            }
            // COM: the image plus at least a stack word.
            None => (image_paras + 1, 0xFFFF),
        };
        if need > largest || need > 0xFFFF {
            return fail(bus, DosError::InsufficientMemory);
        }
        let size = want.min(largest) as u16;
        let seg = match mcb::alloc(bus, super::FIRST_MCB, size, parent, mcb::Strategy::FirstFit) {
            Ok(s) => s,
            Err(_) => return fail(bus, DosError::InsufficientMemory),
        };
        let name: Vec<u8> = full_path
            .rsplit('\\')
            .next()
            .unwrap_or("")
            .bytes()
            .take_while(|&c| c != b'.')
            .collect();
        Self::set_owner(bus, seg, seg, &name);
        Self::set_owner(bus, env, seg, &name);

        psp::create_psp(bus, seg, seg + size, parent, env);
        self.inherit_handles(bus, parent, seg);
        let at = |off: u16| linear(seg, off);
        for i in 0..16 {
            bus.write8(at(psp::FCB1 + i), p.fcb1[i as usize]);
            bus.write8(at(psp::FCB2 + i), p.fcb2[i as usize]);
        }
        let tail = &p.tail[..p.tail.len().min(126)];
        bus.write8(at(psp::TAIL), tail.len() as u8);
        for (i, &b) in tail.iter().chain(&[0x0D]).enumerate() {
            bus.write8(at(psp::TAIL + 1 + i as u16), b);
        }

        Ok(match mz {
            Some(h) => {
                let start = seg + psp::PARAS;
                load_image(bus, start, &bytes[h.image_off..h.image_off + h.image_len]);
                relocate(bus, start, start, &h.relocs);
                Loaded {
                    psp: seg,
                    cs: start.wrapping_add(h.cs),
                    ip: h.ip,
                    ss: start.wrapping_add(h.ss),
                    sp: h.sp,
                }
            }
            None => {
                load_image(bus, seg + psp::PARAS, &bytes);
                // SP at the top of the segment (or block), on a zero word.
                let top = (size as u32 * 16).min(0x10000) as u16;
                let sp = top.wrapping_sub(2) & !1;
                bus.write16(linear(seg, sp), 0);
                Loaded {
                    psp: seg,
                    cs: seg,
                    ip: 0x100,
                    ss: seg,
                    sp,
                }
            }
        })
    }

    /// Copy the parent's JFT into the child's, adding SFT references.
    fn inherit_handles<B: Bus>(&mut self, bus: &mut B, parent: u16, child: u16) {
        let (pjft, plen) = jft_of(bus, parent);
        let (cjft, clen) = jft_of(bus, child);
        for h in 0..clen.min(plen) as u32 {
            let i = bus.read8(pjft + h);
            let open = self.sft.get_mut(i as usize).and_then(|e| e.as_mut());
            let v = match open {
                Some(e) => {
                    e.refs += 1;
                    i
                }
                None => super::files::JFT_FREE,
            };
            bus.write8(cjft + h, v);
        }
    }

    /// Close every handle of `psp` (process exit).
    pub fn close_all<B: Bus>(&mut self, bus: &mut B, psp_seg: u16) {
        let saved = self.current_psp(bus);
        self.set_current_psp(bus, psp_seg);
        let (_, len) = jft_of(bus, psp_seg);
        for h in 0..len {
            let _ = self.close_handle(bus, h);
        }
        self.set_current_psp(bus, saved);
    }

    /// Load and start a program (INT 21h/4B00). The CPU's current CS:IP,
    /// SS:SP and FLAGS become the parent's resume point; on success the CPU
    /// is at the child's entry with DS=ES=PSP.
    pub fn exec<B: Bus>(
        &mut self,
        cpu: &mut Cpu,
        bus: &mut B,
        raw: &[u8],
        p: &ExecParams,
    ) -> Result<(), DosError> {
        let l = self.load(bus, raw, p)?;
        self.start(cpu, bus, l);
        Ok(())
    }

    /// Record the parent's resume point and make `l` the current process.
    fn enter<B: Bus>(&mut self, cpu: &Cpu, bus: &mut B, l: Loaded) {
        let parent = self.current_psp(bus);
        let (ss, sp) = (cpu.sregs.get(Seg::Ss), cpu.regs.sp());
        bus.write16(linear(parent, psp::SAVED_SS_SP), sp);
        bus.write16(linear(parent, psp::SAVED_SS_SP + 2), ss);
        bus.write16(linear(l.psp, psp::INT22), cpu.ip as u16);
        bus.write16(linear(l.psp, psp::INT22 + 2), cpu.sregs.get(Seg::Cs));
        self.processes.push(Process {
            psp: l.psp,
            ss,
            sp,
            flags: cpu.flags as u16,
        });
        self.set_current_psp(bus, l.psp);
        self.set_dta(bus, l.psp, psp::TAIL);
    }

    fn start<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B, l: Loaded) {
        self.enter(cpu, bus, l);
        for s in [Seg::Ds, Seg::Es] {
            cpu.sregs.set(s, l.psp);
        }
        cpu.sregs.set(Seg::Ss, l.ss);
        cpu.regs.set_sp(l.sp);
        cpu.regs.set_ax(0);
        cpu.far_jump(l.cs, l.ip);
        cpu.set_flags16(flags::IF as u16);
    }

    /// INT 21h/4Bh. Returns `Err` for the caller to report via CF/AX.
    /// Mode 00 pops the INT's IRET frame itself first, so on success the
    /// child runs and on failure the CPU is already back in the caller.
    pub(super) fn int21_exec<B: Bus>(
        &mut self,
        cpu: &mut Cpu,
        bus: &mut B,
    ) -> Result<(), DosError> {
        let ds = cpu.sregs.get(Seg::Ds);
        let raw = read_asciiz(bus, ds, cpu.regs.dx());
        let pb = linear(cpu.sregs.get(Seg::Es), cpu.regs.bx());
        let far = |bus: &mut B, off: u32| {
            let o = bus.read16(pb + off);
            let s = bus.read16(pb + off + 2);
            linear(s, o)
        };
        match cpu.regs.al() {
            0 | 1 => {
                let tail_at = far(bus, 2);
                let n = (bus.read8(tail_at) as u32).min(126);
                let tail: Vec<u8> = (1..=n).map(|i| bus.read8(tail_at + i)).collect();
                let mut fcbs = [[0u8; 16]; 2];
                for (k, fcb) in fcbs.iter_mut().enumerate() {
                    let a = far(bus, 6 + 4 * k as u32);
                    for (i, b) in fcb.iter_mut().enumerate() {
                        *b = bus.read8(a + i as u32);
                    }
                }
                let p = ExecParams {
                    env: bus.read16(pb),
                    tail: &tail,
                    fcb1: fcbs[0],
                    fcb2: fcbs[1],
                };
                if cpu.regs.al() == 0 {
                    // IRET by hand: the parent resumes after its INT 21h.
                    cpu.ip = cpu.pop16(bus) as u32;
                    let cs = cpu.pop16(bus);
                    cpu.sregs.set(Seg::Cs, cs);
                    let f = cpu.pop16(bus);
                    cpu.set_flags16(f);
                    return self.exec(cpu, bus, &raw, &p);
                }
                let l = self.load(bus, &raw, &p)?;
                // The parent resumes past this INT (after its IRET frame).
                let mut parent = cpu.clone();
                parent.regs.set_sp(cpu.regs.sp().wrapping_add(6));
                parent.ip = bus.read16(linear(cpu.sregs.get(Seg::Ss), cpu.regs.sp())) as u32;
                parent.sregs.set(
                    Seg::Cs,
                    bus.read16(linear(
                        cpu.sregs.get(Seg::Ss),
                        cpu.regs.sp().wrapping_add(2),
                    )),
                );
                self.enter(&parent, bus, l);
                bus.write16(pb + 0x0E, l.sp);
                bus.write16(pb + 0x10, l.ss);
                bus.write16(pb + 0x12, l.ip);
                bus.write16(pb + 0x14, l.cs);
                Ok(())
            }
            3 => {
                let (bytes, _) = self.read_program(bus, &raw)?;
                let seg = bus.read16(pb);
                let factor = bus.read16(pb + 2);
                match parse_mz(&bytes) {
                    Some(h) => {
                        load_image(bus, seg, &bytes[h.image_off..h.image_off + h.image_len]);
                        relocate(bus, seg, factor, &h.relocs);
                    }
                    None => load_image(bus, seg, &bytes),
                }
                Ok(())
            }
            _ => Err(DosError::InvalidFunction),
        }
    }
}

fn jft_of<B: Bus>(bus: &mut B, p: u16) -> (u32, u16) {
    let off = bus.read16(linear(p, psp::JFT_PTR));
    let seg = bus.read16(linear(p, psp::JFT_PTR + 2));
    (linear(seg, off), bus.read16(linear(p, psp::JFT_SIZE)))
}

fn load_image<B: Bus>(bus: &mut B, seg: u16, bytes: &[u8]) {
    let base = linear(seg, 0);
    for (i, &b) in bytes.iter().enumerate() {
        bus.write8(base + i as u32, b);
    }
}

/// Add `factor` to each relocated word; entries are relative to `start`.
fn relocate<B: Bus>(bus: &mut B, start: u16, factor: u16, relocs: &[(u16, u16)]) {
    for &(off, seg) in relocs {
        let a = linear(start.wrapping_add(seg), off);
        let v = bus.read16(a);
        bus.write16(a, v.wrapping_add(factor));
    }
}
