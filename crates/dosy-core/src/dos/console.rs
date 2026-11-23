//! Console I/O: INT 21h 01h–0Ch, CON reads/writes, INT 29h, ^C handling.
//!
//! Output goes through handle 1 (so it follows redirection) and reaches the
//! screen via the BIOS teletype. Input comes from the BIOS keyboard buffer;
//! when it is empty the handler returns `HleResult::Retry` and keeps any
//! partial line in `Dos`, so the callback can simply run again.
//!
//! ^C / Ctrl-Break: a Ctrl-C key or the INT 1Bh flag makes a console call
//! print `^C`, then issue INT 23h with the return address on the INT 21h
//! callback: if the handler IRETs, the call restarts. The default INT 23h
//! terminates the current program (ignored for the top-level process).

use std::collections::VecDeque;

use super::{Dos, set_flag};
use crate::bios::{bda, kbd, video};
use crate::bus::{Bus, linear};
use crate::cpu::{Cpu, Seg, flags};
use crate::hle::HleResult;

/// DOS's internal line buffer size for cooked CON reads.
const CON_LINE: usize = 128;

/// Console state kept across `Retry`s.
#[derive(Default)]
pub struct Console {
    /// Line being edited (0Ah or a cooked CON read).
    line: Option<Vec<u8>>,
    /// Completed cooked line not yet returned by CON reads.
    pending: VecDeque<u8>,
    /// Scan code to return after an extended key's 0.
    ext: Option<u8>,
    /// Set by INT 1Bh (Ctrl-Break).
    pub break_pending: bool,
    /// 0Ch has flushed the buffer for the current call.
    flushed: bool,
}

enum Line {
    Done(Vec<u8>),
    Wait,
    Break,
}

const CTRL_C: u8 = 3;

impl Dos {
    /// Write to the screen (TAB expands to the next multiple of 8).
    pub fn con_write<B: Bus>(&mut self, bus: &mut B, bytes: &[u8]) {
        let page = bda::read8(bus, bda::VIDEO_PAGE);
        for &b in bytes {
            if b == b'\t' {
                loop {
                    video::teletype(bus, b' ', page, 7);
                    if video::cursor(bus, page).0.is_multiple_of(8) {
                        break;
                    }
                }
            } else {
                video::teletype(bus, b, page, 7);
            }
        }
    }

    /// Write to standard output (handle 1), falling back to the screen.
    fn stdout<B: Bus>(&mut self, bus: &mut B, bytes: &[u8]) {
        if self.write_bytes(bus, 1, bytes).is_err() {
            self.con_write(bus, bytes);
        }
    }

    pub(super) fn con_ready<B: Bus>(&mut self, bus: &mut B) -> bool {
        self.console.ext.is_some()
            || !self.console.pending.is_empty()
            || kbd::peek_key(bus).is_some()
    }

    /// Next input byte: extended keys come as 0 then the scan code.
    fn key<B: Bus>(&mut self, bus: &mut B) -> Option<u8> {
        if let Some(s) = self.console.ext.take() {
            return Some(s);
        }
        let k = kbd::read_key(bus)?;
        let ascii = k as u8;
        if (ascii == 0 || ascii == 0xE0) && k != 0 {
            self.console.ext = Some((k >> 8) as u8);
            return Some(0);
        }
        Some(ascii)
    }

    /// Print `^C`, drop pending input and issue INT 23h; on IRET the
    /// interrupted INT 21h call runs again.
    fn do_break<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B) {
        let c = &mut self.console;
        c.line = None;
        c.pending.clear();
        c.ext = None;
        c.break_pending = false;
        c.flushed = false;
        kbd::flush(bus);
        let f = bda::read8(bus, kbd::BREAK_FLAG);
        bda::write8(bus, kbd::BREAK_FLAG, f & 0x7F);
        self.stdout(bus, b"^C\r\n");
        cpu.ip = cpu.insn_ip;
        cpu.interrupt(bus, 0x23);
    }

    /// Check for ^C before a console call: a pending Ctrl-Break or a Ctrl-C
    /// at the head of the keyboard buffer. True if INT 23h was issued.
    fn check_break<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B) -> bool {
        let ctrl_c = kbd::peek_key(bus).is_some_and(|k| k as u8 == CTRL_C);
        if self.console.break_pending || ctrl_c {
            self.do_break(cpu, bus);
            return true;
        }
        false
    }

    /// Line editor: Enter ends, BS erases, ESC restarts, ^C breaks.
    fn edit_line<B: Bus>(&mut self, bus: &mut B, max: usize) -> Line {
        let mut buf = self.console.line.take().unwrap_or_default();
        loop {
            let Some(c) = self.key(bus) else {
                self.console.line = Some(buf);
                return Line::Wait;
            };
            match c {
                b'\r' => {
                    self.stdout(bus, b"\r");
                    return Line::Done(buf);
                }
                0x08 => {
                    if buf.pop().is_some() {
                        self.stdout(bus, b"\x08 \x08");
                    }
                }
                0x1B => {
                    self.stdout(bus, b"\\\r\n");
                    buf.clear();
                }
                CTRL_C => return Line::Break,
                0 => {
                    self.console.ext = None; // extended keys: no editing keys yet
                }
                c if buf.len() + 1 < max => {
                    buf.push(c);
                    self.stdout(bus, &[c]);
                }
                _ => {} // full: DOS beeps
            }
        }
    }

    /// Read from CON for handle reads (3Fh): cooked lines with echo, or raw
    /// keys (at least one).
    fn con_read<B: Bus>(&mut self, bus: &mut B, raw: bool, max: usize) -> Line {
        if raw {
            let mut out = Vec::new();
            while out.len() < max {
                match self.key(bus) {
                    Some(c) => out.push(c),
                    None => break,
                }
            }
            return if out.is_empty() && max > 0 {
                Line::Wait
            } else {
                Line::Done(out)
            };
        }
        if self.console.pending.is_empty() {
            match self.edit_line(bus, CON_LINE) {
                Line::Done(mut l) => {
                    self.stdout(bus, b"\n");
                    l.extend_from_slice(b"\r\n");
                    self.console.pending.extend(l);
                }
                other => return other,
            }
        }
        let n = max.min(self.console.pending.len());
        Line::Done(self.console.pending.drain(..n).collect())
    }

    /// INT 21h 01h–0Ch.
    pub(super) fn console_fn<B: Bus>(&mut self, cpu: &mut Cpu, bus: &mut B, ah: u8) -> HleResult {
        let wait = HleResult::Retry;
        match ah {
            0x01 | 0x08 => {
                if self.check_break(cpu, bus) {
                    return HleResult::Done;
                }
                let Some(c) = self.key(bus) else { return wait };
                if c == CTRL_C {
                    self.do_break(cpu, bus);
                    return HleResult::Done;
                }
                if ah == 0x01 && c != 0 {
                    self.stdout(bus, &[c]);
                }
                cpu.regs.set_al(c);
            }
            0x07 => {
                let Some(c) = self.key(bus) else { return wait };
                cpu.regs.set_al(c);
            }
            0x02 => {
                if self.check_break(cpu, bus) {
                    return HleResult::Done;
                }
                let c = cpu.regs.dl();
                self.stdout(bus, &[c]);
                cpu.regs.set_al(c);
            }
            0x06 => {
                let dl = cpu.regs.dl();
                if dl == 0xFF {
                    let c = self.key(bus);
                    cpu.regs.set_al(c.unwrap_or(0));
                    set_flag(cpu, bus, flags::ZF, c.is_none());
                } else {
                    self.stdout(bus, &[dl]);
                    cpu.regs.set_al(dl);
                }
            }
            0x09 => {
                if self.check_break(cpu, bus) {
                    return HleResult::Done;
                }
                let (ds, dx) = (cpu.sregs.get(Seg::Ds), cpu.regs.dx());
                let mut s = Vec::new();
                for i in 0..=0xFFFFu16 {
                    match bus.read8(linear(ds, dx.wrapping_add(i))) {
                        b'$' => break,
                        b => s.push(b),
                    }
                }
                self.stdout(bus, &s);
                cpu.regs.set_al(b'$');
            }
            0x0A => {
                let (ds, dx) = (cpu.sregs.get(Seg::Ds), cpu.regs.dx());
                let at = |i: u16| linear(ds, dx.wrapping_add(i));
                let max = bus.read8(at(0)) as usize;
                if max == 0 {
                    return HleResult::Done;
                }
                match self.edit_line(bus, max) {
                    Line::Wait => return wait,
                    Line::Break => self.do_break(cpu, bus),
                    Line::Done(l) => {
                        bus.write8(at(1), l.len() as u8);
                        for (i, &b) in l.iter().chain(b"\r").enumerate() {
                            bus.write8(at(2 + i as u16), b);
                        }
                    }
                }
            }
            0x0B => {
                if self.check_break(cpu, bus) {
                    return HleResult::Done;
                }
                let ready = self.con_ready(bus);
                cpu.regs.set_al(if ready { 0xFF } else { 0 });
            }
            0x0C => {
                if !self.console.flushed {
                    kbd::flush(bus);
                    self.console.ext = None;
                    self.console.flushed = true;
                }
                let al = cpu.regs.al();
                let r = match al {
                    0x01 | 0x06 | 0x07 | 0x08 | 0x0A => self.console_fn(cpu, bus, al),
                    _ => HleResult::Done,
                };
                if r == HleResult::Done {
                    self.console.flushed = false;
                }
                return r;
            }
            _ => unreachable!("not a console function: {ah:02X}"),
        }
        HleResult::Done
    }

    /// INT 21h/3Fh on a CON handle.
    pub(super) fn con_read_handle<B: Bus>(
        &mut self,
        cpu: &mut Cpu,
        bus: &mut B,
        raw: bool,
    ) -> HleResult {
        let (ds, dx, cx) = (cpu.sregs.get(Seg::Ds), cpu.regs.dx(), cpu.regs.cx());
        match self.con_read(bus, raw, cx as usize) {
            Line::Wait => HleResult::Retry,
            Line::Break => {
                self.do_break(cpu, bus);
                HleResult::Done
            }
            Line::Done(bytes) => {
                for (i, &b) in bytes.iter().enumerate() {
                    bus.write8(linear(ds, dx.wrapping_add(i as u16)), b);
                }
                cpu.regs.set_ax(bytes.len() as u16);
                set_flag(cpu, bus, flags::CF, false);
                HleResult::Done
            }
        }
    }
}
