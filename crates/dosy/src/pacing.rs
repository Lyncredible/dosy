//! Real-time pacing: converts elapsed wall time into a cycle budget.

use std::time::Duration;

/// Elapsed time beyond this is dropped instead of caught up.
pub const MAX_CATCH_UP: Duration = Duration::from_millis(50);

/// Cycle budget calculator for a fixed `cycles_per_ms` rate. Sub-cycle
/// remainders carry over, so the long-run rate is exact.
#[derive(Debug)]
pub struct Pacer {
    cycles_per_ms: u64,
    /// Leftover nanoseconds × rate not yet turned into a whole cycle.
    carry: u128,
}

impl Pacer {
    pub fn new(cycles_per_ms: u32) -> Self {
        Self {
            cycles_per_ms: cycles_per_ms as u64,
            carry: 0,
        }
    }

    /// Cycles to run for `elapsed` wall time (capped at [`MAX_CATCH_UP`]).
    pub fn budget(&mut self, elapsed: Duration) -> u64 {
        let ns = elapsed.min(MAX_CATCH_UP).as_nanos();
        let total = ns * self.cycles_per_ms as u128 + self.carry;
        self.carry = total % 1_000_000;
        (total / 1_000_000) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn whole_milliseconds() {
        let mut p = Pacer::new(3000);
        assert_eq!(p.budget(MS), 3000);
        assert_eq!(p.budget(MS * 10), 30_000);
        assert_eq!(p.budget(Duration::ZERO), 0);
    }

    #[test]
    fn fractions_carry_over() {
        let mut p = Pacer::new(3);
        let third = Duration::from_nanos(333_334);
        let got: u64 = (0..3).map(|_| p.budget(third)).sum();
        assert_eq!(got, 3);
        let mut p = Pacer::new(3000);
        assert_eq!(p.budget(MS / 2) + p.budget(MS / 2), 3000);
        // 1000 ticks of 1.5 ms at 1 cycle/ms = 1500 cycles exactly.
        let mut p = Pacer::new(1);
        let n: u64 = (0..1000).map(|_| p.budget(MS * 3 / 2)).sum();
        assert_eq!(n, 1500);
    }

    #[test]
    fn catch_up_is_capped() {
        let mut p = Pacer::new(3000);
        assert_eq!(p.budget(Duration::from_secs(5)), 3000 * 50);
        assert_eq!(p.budget(MS), 3000);
    }
}
