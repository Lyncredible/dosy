//! Debugging aids: instruction trace ring buffer, optional full trace sink,
//! interrupt call log, warn-once registry and fault reports.
//!
//! All of it is opt-in: `Machine::tracer` is `None` by default, which costs
//! one branch per instruction.

pub mod disasm;

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::{LazyLock, Mutex};

use crate::bus::{Bus, linear};
use crate::cpu::{Cpu, Seg, flags};

/// Bytes captured per traced instruction (prefixes + longest common form).
pub const INSN_BYTES: usize = 8;

/// One executed instruction: where, its bytes and the registers before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraceEntry {
    pub cs: u16,
    pub ip: u16,
    pub bytes: [u8; INSN_BYTES],
    /// AX CX DX BX SP BP SI DI (encoding order).
    pub regs: [u16; 8],
    pub ds: u16,
    pub es: u16,
    pub ss: u16,
    pub flags: u16,
}

impl TraceEntry {
    pub fn capture<B: Bus>(cpu: &Cpu, bus: &mut B) -> Self {
        let cs = cpu.sregs.get(Seg::Cs);
        let ip = cpu.ip as u16;
        let mut bytes = [0; INSN_BYTES];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = bus.read8(linear(cs, ip.wrapping_add(i as u16)));
        }
        Self {
            cs,
            ip,
            bytes,
            regs: std::array::from_fn(|i| cpu.regs.get16(i)),
            ds: cpu.sregs.get(Seg::Ds),
            es: cpu.sregs.get(Seg::Es),
            ss: cpu.sregs.get(Seg::Ss),
            flags: cpu.flags as u16,
        }
    }
}

impl std::fmt::Display for TraceEntry {
    /// `CS:IP  bytes  mnemonic  registers`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (text, len) = disasm::disasm(&self.bytes, self.ip);
        let mut hex = String::new();
        for b in &self.bytes[..len.min(INSN_BYTES)] {
            let _ = write!(hex, "{b:02X} ");
        }
        let r = &self.regs;
        write!(
            f,
            "{:04X}:{:04X}  {hex:<18} {text:<28} AX={:04X} BX={:04X} CX={:04X} DX={:04X} \
             SI={:04X} DI={:04X} BP={:04X} SP={:04X} DS={:04X} ES={:04X} SS={:04X} F={:04X}",
            self.cs,
            self.ip,
            r[0],
            r[3],
            r[1],
            r[2],
            r[6],
            r[7],
            r[5],
            r[4],
            self.ds,
            self.es,
            self.ss,
            self.flags
        )
    }
}

/// Receives the full instruction trace (one line per instruction). The app
/// writes it to a file; the core never does I/O itself.
pub trait TraceSink: Send {
    fn line(&mut self, line: &str);
}

/// Instruction trace: a ring of the last N instructions, an optional sink
/// for every instruction, and `INT n` call logging for selected vectors.
pub struct Tracer {
    ring: Vec<TraceEntry>,
    /// Next slot to write.
    next: usize,
    len: usize,
    sink: Option<Box<dyn TraceSink>>,
    log_ints: [bool; 256],
}

impl Tracer {
    /// Ring of `depth` entries (0 = no ring, e.g. only INT logging).
    pub fn new(depth: usize) -> Self {
        Self {
            ring: vec![TraceEntry::default(); depth],
            next: 0,
            len: 0,
            sink: None,
            log_ints: [false; 256],
        }
    }

    /// Also send every instruction to `sink`.
    pub fn with_sink(mut self, sink: Box<dyn TraceSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Log `INT vector` instructions (registers at the call) via `log`
    /// with target `dosy::int`.
    pub fn log_int(&mut self, vector: u8) {
        self.log_ints[vector as usize] = true;
    }

    /// Called by the machine before each instruction.
    pub fn record<B: Bus>(&mut self, cpu: &Cpu, bus: &mut B) {
        let e = TraceEntry::capture(cpu, bus);
        if e.bytes[0] == 0xCD && self.log_ints[e.bytes[1] as usize] {
            log::info!(target: "dosy::int", "{}", int_call(&e));
        }
        if let Some(s) = &mut self.sink {
            s.line(&e.to_string());
        }
        if !self.ring.is_empty() {
            self.ring[self.next] = e;
            self.next = (self.next + 1) % self.ring.len();
            self.len = (self.len + 1).min(self.ring.len());
        }
    }

    /// Recorded entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &TraceEntry> {
        let cap = self.ring.len();
        let start = (self.next + cap - self.len) % cap.max(1);
        (0..self.len).map(move |i| &self.ring[(start + i) % cap])
    }
}

/// One-line `INT n` call description with the registers that matter.
fn int_call(e: &TraceEntry) -> String {
    let r = &e.regs;
    format!(
        "INT {:02X}h AX={:04X} BX={:04X} CX={:04X} DX={:04X} SI={:04X} DI={:04X} DS={:04X} ES={:04X} from {:04X}:{:04X}",
        e.bytes[1], r[0], r[3], r[1], r[2], r[6], r[7], e.ds, e.es, e.cs, e.ip
    )
}

/// Deduplicates warnings by `(category, key)`.
#[derive(Debug, Default)]
pub struct WarnOnce {
    seen: HashSet<(&'static str, u64)>,
}

impl WarnOnce {
    /// True the first time `(category, key)` is seen.
    pub fn first(&mut self, category: &'static str, key: u64) -> bool {
        self.seen.insert((category, key))
    }
}

static WARNED: LazyLock<Mutex<WarnOnce>> = LazyLock::new(Default::default);

/// Log `msg` at warn level once per `(category, key)` for the process,
/// e.g. `warn_once("int10", ax as u64, || format!(..))`. Returns whether
/// this was the first time; `msg` only runs if warnings are enabled.
pub fn warn_once(category: &'static str, key: u64, msg: impl FnOnce() -> String) -> bool {
    let first = WARNED
        .lock()
        .map(|mut w| w.first(category, key))
        .unwrap_or(true);
    if first {
        log::warn!("{}", msg());
    }
    first
}

fn flag_letters(f: u32) -> String {
    [
        (flags::OF, 'O'),
        (flags::DF, 'D'),
        (flags::IF, 'I'),
        (flags::TF, 'T'),
        (flags::SF, 'S'),
        (flags::ZF, 'Z'),
        (flags::AF, 'A'),
        (flags::PF, 'P'),
        (flags::CF, 'C'),
    ]
    .iter()
    .map(|&(m, c)| if f & m != 0 { c } else { '-' })
    .collect()
}

/// Multi-line report for a stopped machine: `why`, registers, the bytes and
/// disassembly at CS:IP, and the trace history if one was recorded.
pub fn fault_report<B: Bus>(why: &str, cpu: &Cpu, bus: &mut B, tracer: Option<&Tracer>) -> String {
    let r = &cpu.regs;
    let s = &cpu.sregs;
    let (cs, ip) = (s.get(Seg::Cs), cpu.ip as u16);
    let mut out = String::new();
    let _ = writeln!(out, "{why}");
    let _ = writeln!(
        out,
        "AX={:04X} BX={:04X} CX={:04X} DX={:04X} SI={:04X} DI={:04X} BP={:04X} SP={:04X}",
        r.ax(),
        r.bx(),
        r.cx(),
        r.dx(),
        r.si(),
        r.di(),
        r.bp(),
        r.sp()
    );
    let _ = writeln!(
        out,
        "CS={cs:04X} DS={:04X} ES={:04X} SS={:04X} IP={ip:04X} FLAGS={:04X} [{}]",
        s.get(Seg::Ds),
        s.get(Seg::Es),
        s.get(Seg::Ss),
        cpu.flags as u16,
        flag_letters(cpu.flags)
    );
    let bytes: Vec<u8> = (0..16)
        .map(|i| bus.read8(linear(cs, ip.wrapping_add(i))))
        .collect();
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
    let _ = writeln!(out, "{cs:04X}:{ip:04X}: {}", hex.join(" "));
    let _ = writeln!(out, "  {}", disasm::disasm(&bytes, ip).0);
    match tracer {
        Some(t) if t.entries().next().is_some() => {
            let _ = writeln!(out, "last {} instructions:", t.entries().count());
            for e in t.entries() {
                let _ = writeln!(out, "  {e}");
            }
        }
        _ => {
            let _ = writeln!(out, "(no instruction history; run with --trace)");
        }
    }
    out
}

#[cfg(test)]
mod tests;
