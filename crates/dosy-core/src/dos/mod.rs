//! HLE DOS kernel (tasks 017–020, 030, 032): guest-visible structures (MCB
//! chain, PSP, environment, List of Lists, SDA) live in guest memory; the
//! INT 20h/21h/2Fh handlers run in Rust behind ROM stubs.
//!
//! Low-memory layout (segment `DOS_SEG`):
//! - `0x24`: first MCB segment, `0x26`: List of Lists (INT 21h/52h)
//! - `0x100`: swappable data area (InDOS at `+1`, DTA, current PSP, ...)
//! - `0x180..0x1800`: free for later kernel tables (SFT, CDS)
//!
//! Then the MCB chain from `FIRST_MCB` to the top of conventional memory.

pub mod mcb;
pub mod psp;
pub mod time;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use crate::bus::{Bus, linear};
use crate::cpu::{Cpu, Seg, flags};
use crate::hle::{HleId, HleResult, ROM_SEGMENT, StubKind, install_stub};
use mcb::Strategy;
use time::Date;

/// Version reported by INT 21h/30h: 5.0 (major in the low byte).
pub const VERSION: u16 = 0x0005;

pub const DOS_SEG: u16 = 0x0080;
pub const LOL_OFF: u16 = 0x0026;
pub const SDA_OFF: u16 = 0x0100;
/// Paragraph of the first MCB; everything below belongs to the kernel.
pub const FIRST_MCB: u16 = 0x0200;

// SDA fields (offsets from `SDA_OFF`).
pub const SDA_INDOS: u16 = 0x01;
pub const SDA_DTA: u16 = 0x0C;
pub const SDA_PSP: u16 = 0x10;
pub const SDA_RETURN_CODE: u16 = 0x14;
pub const SDA_DRIVE: u16 = 0x16;
pub const SDA_BREAK: u16 = 0x17;

/// Master environment and root (shell) process block sizes in paragraphs.
pub const ENV_PARAS: u16 = 0x20;
pub const ROOT_PARAS: u16 = 0x40;
pub const DEFAULT_ENV: &[&str] = &["COMSPEC=Z:\\COMMAND.COM", "PATH=Z:\\"];
pub const SHELL_PATH: &str = "Z:\\COMMAND.COM";

/// BDA conventional memory size in KiB.
const BDA_MEM_KIB: u32 = 0x413;

// DOS error codes.
pub const ERR_FUNCTION: u16 = 1;
pub const ERR_MCB_DESTROYED: u16 = 7;
pub const ERR_NO_MEMORY: u16 = 8;
pub const ERR_BAD_BLOCK: u16 = 9;

/// Callback ids (`0x01xx`; low byte = vector).
pub const ID_INT20: HleId = HleId(HleId::DOS_BASE + 0x20);
pub const ID_INT21: HleId = HleId(HleId::DOS_BASE + 0x21);
pub const ID_INT22: HleId = HleId(HleId::DOS_BASE + 0x22);
pub const ID_INT23: HleId = HleId(HleId::DOS_BASE + 0x23);
pub const ID_INT24: HleId = HleId(HleId::DOS_BASE + 0x24);
pub const ID_INT28: HleId = HleId(HleId::DOS_BASE + 0x28);
pub const ID_INT2F: HleId = HleId(HleId::DOS_BASE + 0x2F);

/// A running child process: where to resume its parent on terminate.
/// EXEC (task 019) pushes one per child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Process {
    pub psp: u16,
    /// Parent's SS:SP to restore; execution continues at the child's PSP
    /// INT 22h address.
    pub ss: u16,
    pub sp: u16,
}

/// Host-side kernel state.
#[derive(Debug, Default)]
pub struct Dos {
    pub strategy: Strategy,
    pub date: Date,
    pub processes: Vec<Process>,
    /// The first process (the shell); owns the master environment.
    pub root_psp: u16,
    warned: HashSet<u16>,
}

fn sda(off: u16) -> u32 {
    linear(DOS_SEG, SDA_OFF + off)
}

/// Set or clear a flag both in the CPU and in the FLAGS image of the IRET
/// frame at SS:SP (handlers run inside `callback; IRET` stubs).
fn set_flag<B: Bus>(cpu: &mut Cpu, bus: &mut B, mask: u32, on: bool) {
    let a = linear(cpu.sregs.get(Seg::Ss), cpu.regs.sp().wrapping_add(4));
    let f = bus.read16(a);
    let f = if on {
        f | mask as u16
    } else {
        f & !(mask as u16)
    };
    bus.write16(a, f);
    if on {
        cpu.flags |= mask;
    } else {
        cpu.flags &= !mask;
    }
}

fn ok<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    set_flag(cpu, bus, flags::CF, false);
}

fn fail<B: Bus>(cpu: &mut Cpu, bus: &mut B, code: u16) {
    cpu.regs.set_ax(code);
    set_flag(cpu, bus, flags::CF, true);
}

/// Write an environment block: `VAR=value\0...\0`, word 1, program path.
pub fn write_env<B: Bus>(bus: &mut B, seg: u16, vars: &[&str], program: &str) {
    let mut bytes = Vec::new();
    for v in vars {
        bytes.extend_from_slice(v.as_bytes());
        bytes.push(0);
    }
    if vars.is_empty() {
        bytes.push(0);
    }
    bytes.extend_from_slice(&[0, 1, 0]);
    bytes.extend_from_slice(program.as_bytes());
    bytes.push(0);
    for (i, b) in bytes.into_iter().enumerate() {
        bus.write8(linear(seg, i as u16), b);
    }
}

impl Dos {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install vectors, kernel data, the MCB chain, the master environment
    /// and the root process (current PSP afterwards).
    pub fn install<B: Bus>(&mut self, bus: &mut B) {
        for (id, kind) in [
            (ID_INT20, StubKind::Iret),
            (ID_INT21, StubKind::IretSti),
            (ID_INT22, StubKind::Iret),
            (ID_INT23, StubKind::Iret),
            (ID_INT24, StubKind::Iret),
            (ID_INT28, StubKind::Iret),
            (ID_INT2F, StubKind::Iret),
        ] {
            install_stub(bus, id, kind);
            let v = (id.0 & 0xFF) as u32 * 4;
            bus.write16(v, id.stub_offset());
            bus.write16(v + 2, ROM_SEGMENT);
        }

        for off in 0..(FIRST_MCB - DOS_SEG) * 16 {
            bus.write8(linear(DOS_SEG, off), 0);
        }
        let lol = |off: u16| linear(DOS_SEG, LOL_OFF + off);
        bus.write16(lol(0) - 2, FIRST_MCB);
        for ptr in [0x00, 0x04, 0x08, 0x0C, 0x12, 0x16, 0x1A] {
            bus.write32(lol(ptr), 0xFFFF_FFFF);
        }
        bus.write16(lol(0x10), 512); // max bytes per sector
        bus.write8(lol(0x21), 26); // LASTDRIVE
        // NUL device header.
        bus.write32(lol(0x22), 0xFFFF_FFFF);
        bus.write16(lol(0x26), 0x8004);
        for (i, &b) in b"NUL     ".iter().enumerate() {
            bus.write8(lol(0x2C + i as u16), b);
        }
        bus.write8(sda(SDA_DRIVE), 2); // C:

        let kib = match bus.read16(BDA_MEM_KIB) {
            0 => 640,
            k => k.min(640),
        };
        mcb::init(bus, FIRST_MCB, kib * 64);

        let first = FIRST_MCB;
        let env = mcb::alloc(bus, first, ENV_PARAS, mcb::OWNER_DOS, Strategy::FirstFit)
            .expect("fresh chain");
        let root = mcb::alloc(bus, first, ROOT_PARAS, mcb::OWNER_DOS, Strategy::FirstFit)
            .expect("fresh chain");
        for seg in [env, root] {
            let mut m = mcb::read(bus, seg - 1);
            m.owner = root;
            mcb::write(bus, &m);
            mcb::set_name(bus, seg - 1, b"COMMAND");
        }
        write_env(bus, env, DEFAULT_ENV, SHELL_PATH);
        psp::create_psp(bus, root, root + ROOT_PARAS, root, env);
        self.root_psp = root;
        self.processes.clear();
        self.set_current_psp(bus, root);
        self.set_dta(bus, root, psp::TAIL);
    }

    pub fn current_psp<B: Bus>(&self, bus: &mut B) -> u16 {
        bus.read16(sda(SDA_PSP))
    }
    pub fn set_current_psp<B: Bus>(&self, bus: &mut B, seg: u16) {
        bus.write16(sda(SDA_PSP), seg);
    }
    /// Disk transfer address `(segment, offset)`.
    pub fn dta<B: Bus>(&self, bus: &mut B) -> (u16, u16) {
        (bus.read16(sda(SDA_DTA + 2)), bus.read16(sda(SDA_DTA)))
    }
    pub fn set_dta<B: Bus>(&self, bus: &mut B, seg: u16, off: u16) {
        bus.write16(sda(SDA_DTA), off);
        bus.write16(sda(SDA_DTA + 2), seg);
    }

    /// Dispatch a DOS-range callback.
    pub fn callback<B: Bus>(&mut self, id: HleId, cpu: &mut Cpu, bus: &mut B) -> HleResult {
        match id {
            ID_INT21 => return self.int21(cpu, bus),
            ID_INT20 => self.terminate(cpu, bus, 0, 0),
            ID_INT24 => cpu.regs.set_al(3), // fail
            ID_INT23 | ID_INT28 | ID_INT2F => {}
            _ => self.warn_once(id.0, || format!("unhandled DOS callback {:#06x}", id.0)),
        }
        HleResult::Done
    }

    fn warn_once(&mut self, key: u16, msg: impl FnOnce() -> String) {
        if self.warned.insert(key) {
            log::warn!("{}", msg());
        }
    }

    fn int21<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B) -> HleResult {
        let r = &mut cpu.regs;
        let (ah, al) = (r.ah(), r.al());
        let ds = cpu.sregs.get(Seg::Ds);
        match ah {
            0x00 => self.terminate(cpu, bus, 0, 0),
            0x1A => self.set_dta(bus, ds, cpu.regs.dx()),
            0x25 => {
                bus.write16(al as u32 * 4, cpu.regs.dx());
                bus.write16(al as u32 * 4 + 2, ds);
            }
            0x2A => {
                self.roll_date(bus);
                let d = self.date;
                cpu.regs.set_cx(d.year);
                cpu.regs.set_dh(d.month);
                cpu.regs.set_dl(d.day);
                cpu.regs.set_al(d.weekday());
            }
            0x2B => {
                let r = &mut cpu.regs;
                match Date::new(r.cx(), r.dh(), r.dl()) {
                    Some(d) => {
                        self.date = d;
                        r.set_al(0);
                    }
                    None => r.set_al(0xFF),
                }
            }
            0x2C => {
                self.roll_date(bus);
                let (h, m, s, cs) = time::get_time(bus);
                let r = &mut cpu.regs;
                r.set_ch(h);
                r.set_cl(m);
                r.set_dh(s);
                r.set_dl(cs);
            }
            0x2D => {
                self.roll_date(bus);
                let r = &mut cpu.regs;
                let ok = time::set_time(bus, r.ch(), r.cl(), r.dh(), r.dl());
                r.set_al(if ok { 0 } else { 0xFF });
            }
            0x2F => {
                let (seg, off) = self.dta(bus);
                cpu.sregs.set(Seg::Es, seg);
                cpu.regs.set_bx(off);
            }
            0x30 => {
                let r = &mut cpu.regs;
                // AL=1 asks for the version flag (BH=0: not in ROM/HMA).
                r.set_bh(if al == 1 { 0 } else { 0xFF });
                r.set_ax(VERSION);
                r.set_bl(0);
                r.set_cx(0);
            }
            0x33 => self.break_flag(cpu, bus),
            0x34 => {
                cpu.sregs.set(Seg::Es, DOS_SEG);
                cpu.regs.set_bx(SDA_OFF + SDA_INDOS);
            }
            0x35 => {
                cpu.regs.set_bx(bus.read16(al as u32 * 4));
                cpu.sregs.set(Seg::Es, bus.read16(al as u32 * 4 + 2));
            }
            0x48 => {
                let owner = self.current_psp(bus);
                match mcb::alloc(bus, FIRST_MCB, cpu.regs.bx(), owner, self.strategy) {
                    Ok(seg) => {
                        cpu.regs.set_ax(seg);
                        ok(cpu, bus);
                    }
                    Err((code, largest)) => {
                        cpu.regs.set_bx(largest);
                        fail(cpu, bus, code);
                    }
                }
            }
            0x49 => {
                let es = cpu.sregs.get(Seg::Es);
                match mcb::free(bus, FIRST_MCB, es) {
                    Ok(()) => ok(cpu, bus),
                    Err(code) => fail(cpu, bus, code),
                }
            }
            0x4A => {
                let es = cpu.sregs.get(Seg::Es);
                match mcb::resize(bus, FIRST_MCB, es, cpu.regs.bx()) {
                    Ok(()) => ok(cpu, bus),
                    Err((code, max)) => {
                        if code == ERR_NO_MEMORY {
                            cpu.regs.set_bx(max);
                        }
                        fail(cpu, bus, code);
                    }
                }
            }
            0x4C => self.terminate(cpu, bus, al, 0),
            0x4D => {
                cpu.regs.set_ax(bus.read16(sda(SDA_RETURN_CODE)));
                bus.write16(sda(SDA_RETURN_CODE), 0);
                ok(cpu, bus);
            }
            0x50 => self.set_current_psp(bus, cpu.regs.bx()),
            0x51 | 0x62 => {
                let p = self.current_psp(bus);
                cpu.regs.set_bx(p);
            }
            0x52 => {
                cpu.sregs.set(Seg::Es, DOS_SEG);
                cpu.regs.set_bx(LOL_OFF);
            }
            0x58 => self.strategy_fn(cpu, bus),
            _ => {
                self.warn_once(0x2100 | ah as u16, || {
                    format!("unimplemented INT 21h AH={ah:02X}h")
                });
                fail(cpu, bus, ERR_FUNCTION);
            }
        }
        HleResult::Done
    }

    /// Apply the BIOS midnight flag to the kept date.
    fn roll_date<B: Bus>(&mut self, bus: &mut B) {
        let days = bus.read8(time::BDA_MIDNIGHT);
        if days != 0 {
            bus.write8(time::BDA_MIDNIGHT, 0);
            for _ in 0..days {
                self.date.next_day();
            }
        }
    }

    fn break_flag<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B) {
        let a = sda(SDA_BREAK);
        let r = &mut cpu.regs;
        match r.al() {
            0 => r.set_dl(bus.read8(a)),
            1 => bus.write8(a, (r.dl() != 0) as u8),
            2 => {
                let old = bus.read8(a);
                bus.write8(a, (r.dl() != 0) as u8);
                r.set_dl(old);
            }
            5 => r.set_dl(3), // boot drive C:
            6 => {
                r.set_bx(VERSION);
                r.set_dx(0);
            }
            _ => r.set_al(0xFF),
        }
    }

    fn strategy_fn<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B) {
        match cpu.regs.al() {
            0 => {
                cpu.regs.set_ax(self.strategy as u16);
                ok(cpu, bus);
            }
            1 => {
                // High bits select UMB use; there are no UMBs, so ignore them.
                self.strategy = match cpu.regs.bx() & 0x3F {
                    0 => Strategy::FirstFit,
                    1 => Strategy::BestFit,
                    2 => Strategy::LastFit,
                    _ => return fail(cpu, bus, ERR_FUNCTION),
                };
                ok(cpu, bus);
            }
            2 => {
                cpu.regs.set_al(0); // UMBs not linked
                ok(cpu, bus);
            }
            3 if cpu.regs.bx() == 0 => ok(cpu, bus),
            _ => fail(cpu, bus, ERR_FUNCTION),
        }
    }

    /// End the current process (`kind`: 0 normal, 1 Ctrl-C, 2 critical
    /// error, 3 TSR): free its memory, restore INT 22h–24h, switch to the
    /// parent and resume at its INT 22h address on the parent's stack.
    pub fn terminate<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B, code: u8, kind: u8) {
        bus.write16(sda(SDA_RETURN_CODE), (kind as u16) << 8 | code as u16);
        let child = self.current_psp(bus);
        let Some(frame) = self.processes.pop() else {
            self.warn_once(0xFFFF, || "root process terminated; ignored".into());
            return;
        };
        if frame.psp != child {
            log::warn!(
                "terminate: current PSP {child:04X} is not the exec'd {:04X}",
                frame.psp
            );
        }
        let p = |off: u16| linear(frame.psp, off);
        for (off, vec) in [
            (psp::INT22, 0x22u32),
            (psp::INT23, 0x23),
            (psp::INT24, 0x24),
        ] {
            let v = bus.read32(p(off));
            bus.write32(vec * 4, v);
        }
        let parent = bus.read16(p(psp::PARENT));
        let ret_ip = bus.read16(p(psp::INT22));
        let ret_cs = bus.read16(p(psp::INT22 + 2));
        if kind != 3 {
            if let Err(e) = mcb::free_owned(bus, FIRST_MCB, frame.psp) {
                log::warn!("terminate: MCB chain error {e}");
            }
        }
        self.set_current_psp(bus, parent);
        self.set_dta(bus, parent, psp::TAIL);
        cpu.sregs.set(Seg::Ss, frame.ss);
        cpu.regs.set_sp(frame.sp);
        cpu.far_jump(ret_cs, ret_ip);
    }
}
