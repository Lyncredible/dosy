//! `Machine`: CPU + Hardware + HLE dispatch, driven by `run_for`.
//!
//! Loop (see "Timing model" in `docs/ARCHITECTURE.md`): fire due events,
//! take a pending PIC interrupt when IF=1 and no interrupt shadow, then step
//! the CPU until the budget, the next scheduler deadline or a deliverable
//! IRQ. HLT fast-forwards to the next event.

use crate::bios;
use crate::bus::{Bus, linear};
use crate::cpu::flags::IF;
use crate::cpu::{Cpu, CpuFault, CpuModel, Seg, Step};
use crate::dos::{self, Dos};
use crate::hle::{HleId, HleResult, ROM_SEGMENT, Subsystem};
use crate::hw::{DEFAULT_CYCLES_PER_MS, Hardware};
use crate::vfs::{DosError, Drive};
use crate::video::VideoFrame;

/// Core-side machine settings (the app maps its `Config` to this).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineConfig {
    pub cycles_per_ms: u32,
    pub cpu_model: CpuModel,
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            cycles_per_ms: DEFAULT_CYCLES_PER_MS,
            cpu_model: CpuModel::default(),
        }
    }
}

/// Why `run_for` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunExit {
    /// The cycle budget is used up.
    Budget,
    /// HLT with IF=0: nothing but a reset can wake the CPU.
    Halted,
    /// CPU fault; IP points at the faulting instruction.
    Fault(CpuFault),
    /// Callback with no handler for its subsystem; IP points at it.
    UnhandledCallback(HleId),
    /// The top-level DOS process terminated with this return code (no
    /// shell to return to). Sticky: later calls return it again.
    Exited(u8),
}

/// HLE entry point for one subsystem.
pub type HleHandler = Box<dyn FnMut(HleId, &mut Cpu, &mut Hardware) -> HleResult + Send>;

const SUBSYSTEMS: usize = 4;

fn slot(s: Subsystem) -> Option<usize> {
    match s {
        Subsystem::Bios => Some(0),
        Subsystem::Dos => Some(1),
        Subsystem::Shell => Some(2),
        Subsystem::Driver => Some(3),
        Subsystem::Unknown => None,
    }
}

pub struct Machine {
    pub cpu: Cpu,
    pub hw: Hardware,
    handlers: [Option<HleHandler>; SUBSYSTEMS],
    /// HLE DOS kernel; handles `Subsystem::Dos` callbacks unless a
    /// handler is registered for them.
    pub dos: Dos,
    /// Host RTC base in Unix seconds, for CMOS (009).
    pub rtc_base: i64,
}

impl Machine {
    /// Power-on state. BIOS/DOS/shell installers (015, 017, 021) add one
    /// line each here.
    pub fn new(cfg: MachineConfig) -> Self {
        let mut hw = Hardware::new();
        hw.set_cycles_per_ms(cfg.cycles_per_ms);
        bios::install(&mut hw);
        // After the BIOS: DOS sizes memory from the BDA.
        let mut dos = Dos::new();
        dos.install(&mut hw);
        hw.start_video();
        let mut m = Self {
            cpu: Cpu::with_model(cfg.cpu_model),
            hw,
            handlers: Default::default(),
            dos,
            rtc_base: 0,
        };
        m.set_handler(Subsystem::Bios, Box::new(bios::handle));
        m
    }

    /// Route callbacks of `subsystem` to `handler` (replacing any previous).
    pub fn set_handler(&mut self, subsystem: Subsystem, handler: HleHandler) {
        let i = slot(subsystem).expect("cannot register Subsystem::Unknown");
        self.handlers[i] = Some(handler);
    }

    pub fn mount(&mut self, letter: char, drive: Box<dyn Drive>) {
        let i = letter.to_ascii_uppercase() as usize;
        assert!(
            (b'A' as usize..=b'Z' as usize).contains(&i),
            "bad drive {letter:?}"
        );
        self.dos.mount((i - b'A' as usize) as u8, drive);
    }

    /// Run a program as the top-level process (no shell): when it
    /// terminates, `run_for` returns `RunExit::Exited(code)`.
    pub fn exec(&mut self, path: &str, tail: &str) -> Result<(), DosError> {
        // Resume point: the INT 22h stub, on a stack in the root block.
        let root = self.dos.root_psp;
        self.cpu.far_jump(ROM_SEGMENT, dos::ID_INT22.stub_offset());
        self.cpu.sregs.set(Seg::Ss, root);
        self.cpu.regs.set_sp(dos::ROOT_PARAS * 16 - 2);
        self.hw.write16(linear(root, dos::ROOT_PARAS * 16 - 2), 0);
        let params = dos::exec::ExecParams::from_tail(tail);
        self.dos
            .exec(&mut self.cpu, &mut self.hw, path.as_bytes(), &params)
    }

    pub fn set_cycles_per_ms(&mut self, n: u32) {
        self.hw.set_cycles_per_ms(n);
    }

    /// The frame completed at the latest vretrace (kept until the next one).
    pub fn frame(&self) -> Option<&VideoFrame> {
        self.hw.vga.frame()
    }

    /// Inject host time; the core never reads the wall clock.
    pub fn set_rtc(&mut self, unix_secs: i64) {
        self.rtc_base = unix_secs;
    }

    fn irq_deliverable(&self) -> bool {
        self.cpu.flags & IF != 0 && !self.cpu.int_shadow && self.hw.pic.pending_vector().is_some()
    }

    /// Run about `cycles` CPU cycles of emulated time.
    pub fn run_for(&mut self, cycles: u64) -> RunExit {
        if let Some(code) = self.dos.exit_code() {
            return RunExit::Exited(code);
        }
        let mut left = cycles;
        loop {
            self.hw.fire_due();
            if left == 0 {
                return RunExit::Budget;
            }
            if self.irq_deliverable() {
                // pending_vector() was Some, so acknowledge() yields a vector.
                if let Some(v) = self.hw.pic.acknowledge() {
                    self.cpu.interrupt(&mut self.hw, v);
                }
            }

            let next = self.hw.sched.next_due();
            if self.cpu.halted {
                if self.cpu.flags & IF == 0 {
                    return RunExit::Halted;
                }
                // Skip to the next event if it falls inside the budget.
                match next.map(|t| (t, self.hw.cycles_until(t))) {
                    Some((_, c)) if c <= left => {
                        // Charge whole cycles so time and budget stay in step.
                        self.hw.advance_cycles(c);
                        left -= c;
                    }
                    _ => {
                        self.hw.advance_cycles(left);
                        left = 0;
                    }
                }
                continue;
            }

            // Step until the budget, the next deadline or a deliverable IRQ.
            let mut batch = left;
            if let Some(t) = next {
                batch = batch.min(self.hw.cycles_until(t).max(1));
            }
            let mut used = 0u64;
            let exit = loop {
                let (step, cost) = self.cpu.step(&mut self.hw);
                used += cost as u64;
                self.hw.advance_cycles(cost as u64);
                match step {
                    Step::Ok => {}
                    Step::Halt => break None,
                    Step::Callback(id) => match self.dispatch(id) {
                        Some(HleResult::Done) => {
                            if let Some(code) = self.dos.exit_code() {
                                break Some(RunExit::Exited(code));
                            }
                        }
                        Some(HleResult::Retry) => {
                            // Re-run the callback after the next interrupt.
                            self.cpu.ip = self.cpu.insn_ip;
                            self.cpu.flags |= IF;
                            self.cpu.halted = true;
                            break None;
                        }
                        None => {
                            self.cpu.ip = self.cpu.insn_ip;
                            break Some(RunExit::UnhandledCallback(id));
                        }
                    },
                    Step::Fault(f) => break Some(RunExit::Fault(f)),
                }
                if used >= batch || self.irq_deliverable() {
                    break None;
                }
                // A port write may have scheduled an earlier event.
                if self.hw.sched.next_due().is_some_and(|t| t <= self.hw.now()) {
                    break None;
                }
            };
            left = left.saturating_sub(used);
            if let Some(exit) = exit {
                return exit;
            }
        }
    }

    fn dispatch(&mut self, id: HleId) -> Option<HleResult> {
        let sub = id.subsystem();
        match self.handlers[slot(sub)?].as_mut() {
            Some(h) => Some(h(id, &mut self.cpu, &mut self.hw)),
            None if sub == Subsystem::Dos => {
                Some(self.dos.callback(id, &mut self.cpu, &mut self.hw))
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests;
