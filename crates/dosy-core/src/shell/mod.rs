//! Built-in COMMAND.COM: a native state machine running as the root DOS
//! process. Its code is a ROM stub `L: FE 38 <ID_SHELL>; JMP L`; each
//! callback advances the state machine. Programs run via `Dos::exec` with
//! the stub's `JMP` as the resume point, so a child's exit lands back in the
//! shell and the next callback shows the prompt again.
//!
//! Output goes through the INT 10h teletype (`bios::video`); line input
//! reads the BIOS keyboard buffer and returns `HleResult::Retry` while it
//! is empty. Both move to DOS CON (020) later.

mod builtins;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;

use crate::bios::{bda, kbd, video};
use crate::bus::{Bus, linear};
use crate::cpu::{Cpu, Seg, flags};
use crate::dos::{self, Dos, psp};
use crate::hle::{HleId, HleResult, ROM_SEGMENT, StubKind, install_stub};
use crate::vfs::{DosError, DosPath, MemDrive};

/// The shell's main loop callback.
pub const ID_SHELL: HleId = HleId(HleId::SHELL_BASE);
/// Z: holds the shell itself.
pub const Z: u8 = 25;
const MAX_LINE: usize = 126;

/// Install the shell's ROM loop stub.
pub fn install<B: Bus>(bus: &mut B) {
    install_stub(bus, ID_SHELL, StubKind::Loop);
}

/// The built-in Z: drive.
pub fn z_drive() -> MemDrive {
    let mut z = MemDrive::with_label("DOSY");
    // Placeholder so COMSPEC points at a real file.
    z.add_file("\\COMMAND.COM", &[0xCD, 0x20]);
    z
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Not started; the first callback picks the drive and runs autoexec.
    Start,
    /// Show the prompt (or run the next queued line).
    Prompt,
    /// Reading a line from the keyboard.
    Input,
}

/// What running one command line did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    /// A child process is now running on the CPU.
    Started,
    /// The shell itself terminated.
    Exit,
}

pub struct Shell {
    state: State,
    line: Vec<u8>,
    queued: VecDeque<String>,
    /// MOUNT descriptions per drive.
    mounts: [Option<String>; 26],
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    pub fn new() -> Self {
        Self {
            state: State::Start,
            line: Vec::new(),
            queued: VecDeque::new(),
            mounts: Default::default(),
        }
    }

    /// Lines to run after start-up, as if typed at the prompt.
    pub fn queue(&mut self, lines: &[String]) {
        self.queued.extend(lines.iter().cloned());
    }

    /// Point the CPU at the shell loop on the root process's stack.
    pub fn enter(cpu: &mut Cpu, root_psp: u16) {
        cpu.far_jump(ROM_SEGMENT, ID_SHELL.stub_offset());
        cpu.sregs.set(Seg::Ss, root_psp);
        cpu.regs.set_sp(dos::ROOT_PARAS * 16 - 2);
        for s in [Seg::Ds, Seg::Es] {
            cpu.sregs.set(s, root_psp);
        }
        cpu.halted = false;
        cpu.flags |= flags::IF;
    }

    /// Record a mount description for `MOUNT` listings.
    pub fn note_mount(&mut self, drive: u8, what: String) {
        if let Some(m) = self.mounts.get_mut(drive as usize) {
            *m = Some(what);
        }
    }

    pub fn callback<B: Bus>(
        &mut self,
        id: HleId,
        dos: &mut Dos,
        cpu: &mut Cpu,
        bus: &mut B,
    ) -> HleResult {
        if id != ID_SHELL {
            log::warn!("unknown shell callback {:#06x}", id.0);
            return HleResult::Done;
        }
        loop {
            match self.state {
                State::Start => {
                    if !dos.is_mounted(dos.current_drive(bus)) {
                        dos.set_current_drive(bus, Z);
                    }
                    self.state = State::Prompt;
                }
                State::Prompt => match self.queued.pop_front() {
                    Some(line) => {
                        prompt(dos, bus);
                        out(bus, line.as_bytes());
                        out(bus, b"\n");
                        if let Some(r) = self.finish(dos, cpu, bus, line.as_bytes()) {
                            return r;
                        }
                    }
                    None => {
                        prompt(dos, bus);
                        self.line.clear();
                        self.state = State::Input;
                    }
                },
                State::Input => {
                    let Some(key) = kbd::read_key(bus) else {
                        return HleResult::Retry;
                    };
                    if let Some(line) = self.edit(bus, key) {
                        self.state = State::Prompt;
                        if let Some(r) = self.finish(dos, cpu, bus, &line) {
                            return r;
                        }
                    }
                }
            }
        }
    }

    /// Run a line; `Some` when control must go back to the CPU.
    fn finish<B: Bus>(
        &mut self,
        dos: &mut Dos,
        cpu: &mut Cpu,
        bus: &mut B,
        line: &[u8],
    ) -> Option<HleResult> {
        match self.run(dos, cpu, bus, line) {
            Flow::Continue => None,
            Flow::Started | Flow::Exit => Some(HleResult::Done),
        }
    }

    /// Line editing: returns the finished line on Enter.
    fn edit<B: Bus>(&mut self, bus: &mut B, key: u16) -> Option<Vec<u8>> {
        match key as u8 {
            b'\r' => {
                out(bus, b"\n");
                return Some(std::mem::take(&mut self.line));
            }
            0x08 => {
                if self.line.pop().is_some() {
                    out(bus, b"\x08 \x08");
                }
            }
            0x1B => {
                for _ in 0..self.line.len() {
                    out(bus, b"\x08 \x08");
                }
                self.line.clear();
            }
            c if c >= 0x20 && self.line.len() < MAX_LINE => {
                self.line.push(c);
                out(bus, &[c]);
            }
            _ => {} // extended keys, control codes: ignored for now
        }
        None
    }
}

/// Write bytes to the screen; `\n` becomes CR LF.
pub(crate) fn out<B: Bus>(bus: &mut B, s: &[u8]) {
    let page = bda::read8(bus, bda::VIDEO_PAGE);
    for &b in s {
        if b == b'\n' {
            video::teletype(bus, b'\r', page, 7);
        }
        video::teletype(bus, b, page, 7);
    }
}

pub(crate) fn outs<B: Bus>(bus: &mut B, s: &str) {
    out(bus, s.as_bytes());
}

fn drive_letter(d: u8) -> u8 {
    b'A' + d
}

/// `C:\DIR` for a drive's current directory.
pub(crate) fn cwd_string(dos: &Dos, drive: u8) -> Vec<u8> {
    let mut s = vec![drive_letter(drive), b':'];
    s.extend(dos.cwd[drive as usize].to_bytes());
    s
}

fn prompt<B: Bus>(dos: &Dos, bus: &mut B) {
    let d = dos.current_drive(bus);
    let mut p = cwd_string(dos, d);
    p.push(b'>');
    out(bus, &p);
}

/// Message for a DOS error, as COMMAND.COM words it.
pub(crate) fn error_text(e: DosError) -> &'static str {
    match e {
        DosError::FileNotFound => "File not found",
        DosError::PathNotFound => "Path not found",
        DosError::AccessDenied => "Access denied",
        DosError::InvalidDrive => "Invalid drive specification",
        DosError::InsufficientMemory => "Not enough memory",
        DosError::Exists => "File exists",
        DosError::TooManyOpenFiles => "Too many open files",
        _ => "General failure",
    }
}

// --- Environment (the root process's block) ---

fn env_seg<B: Bus>(dos: &Dos, bus: &mut B) -> u16 {
    bus.read16(linear(dos.root_psp, psp::ENV))
}

/// `VAR=value` strings of the master environment.
pub(crate) fn env_vars<B: Bus>(dos: &Dos, bus: &mut B) -> Vec<String> {
    let seg = env_seg(dos, bus);
    let mut vars = Vec::new();
    let mut cur = Vec::new();
    for off in 0..dos::ENV_PARAS * 16 {
        match bus.read8(linear(seg, off)) {
            0 if cur.is_empty() => break,
            0 => vars.push(std::mem::take(&mut cur)),
            b => cur.push(b),
        }
    }
    vars.into_iter()
        .map(|v| v.iter().map(|&b| b as char).collect())
        .collect()
}

pub(crate) fn get_env<B: Bus>(dos: &Dos, bus: &mut B, name: &str) -> Option<String> {
    env_vars(dos, bus).into_iter().find_map(|v| {
        let (k, val) = v.split_once('=')?;
        k.eq_ignore_ascii_case(name).then(|| val.to_string())
    })
}

/// Set (or with an empty value, remove) a variable. Fails when the block is
/// full.
pub(crate) fn set_env<B: Bus>(
    dos: &Dos,
    bus: &mut B,
    name: &str,
    value: &str,
) -> Result<(), ()> {
    let name = name.to_ascii_uppercase();
    let mut vars: Vec<String> = env_vars(dos, bus)
        .into_iter()
        .filter(|v| !v.split_once('=').is_some_and(|(k, _)| k.eq_ignore_ascii_case(&name)))
        .collect();
    if !value.is_empty() {
        vars.push(format!("{name}={value}"));
    }
    let size: usize = vars.iter().map(|v| v.len() + 1).sum::<usize>()
        + 4
        + dos::SHELL_PATH.len()
        + 1;
    if size > dos::ENV_PARAS as usize * 16 {
        return Err(());
    }
    let refs: Vec<&str> = vars.iter().map(String::as_str).collect();
    dos::write_env(bus, env_seg(dos, bus), &refs, dos::SHELL_PATH);
    Ok(())
}

/// Resolve a guest path to (drive, path) and check it is a file.
pub(crate) fn find_file<B: Bus>(dos: &mut Dos, bus: &mut B, raw: &[u8]) -> Option<(u8, DosPath)> {
    let (d, p) = dos.resolve_path(bus, raw).ok()?;
    let e = dos.drive_mut(d).ok()?.stat(&p).ok()?;
    (!e.is_dir()).then_some((d, p))
}
