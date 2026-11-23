//! `Machine`: CPU + Hardware + HLE dispatch, driven by `run_for`.
//!
//! Loop (see "Timing model" in `docs/ARCHITECTURE.md`): fire due events,
//! take a pending PIC interrupt when IF=1 and no interrupt shadow, then step
//! the CPU until the budget, the next scheduler deadline or a deliverable
//! IRQ. HLT fast-forwards to the next event.

use crate::cpu::flags::IF;
use crate::cpu::{Cpu, CpuFault, Step};
use crate::hle::{HleId, HleResult, Subsystem};
use crate::hw::{DEFAULT_CYCLES_PER_MS, Hardware};
use crate::vfs::Drive;

/// Core-side machine settings (the app maps its `Config` to this).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineConfig {
    pub cycles_per_ms: u32,
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            cycles_per_ms: DEFAULT_CYCLES_PER_MS,
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
    /// Mounted drives A..Z; the DOS kernel (017) takes these over.
    pub drives: [Option<Box<dyn Drive>>; 26],
    /// Host RTC base in Unix seconds, for CMOS (009).
    pub rtc_base: i64,
}

impl Machine {
    /// Power-on state. BIOS/DOS/shell installers (015, 017, 021) add one
    /// line each here.
    pub fn new(cfg: MachineConfig) -> Self {
        let mut hw = Hardware::new();
        hw.set_cycles_per_ms(cfg.cycles_per_ms);
        Self {
            cpu: Cpu::new(),
            hw,
            handlers: Default::default(),
            drives: Default::default(),
            rtc_base: 0,
        }
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
        self.drives[i - b'A' as usize] = Some(drive);
    }

    pub fn set_cycles_per_ms(&mut self, n: u32) {
        self.hw.set_cycles_per_ms(n);
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
                        Some(HleResult::Done) => {}
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
        let h = self.handlers[slot(id.subsystem())?].as_mut()?;
        Some(h(id, &mut self.cpu, &mut self.hw))
    }
}

#[cfg(test)]
mod tests;
