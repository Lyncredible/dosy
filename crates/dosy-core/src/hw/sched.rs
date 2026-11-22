//! Event scheduler keyed by emulated nanoseconds.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Identifies the device an event belongs to. Constants are reserved up
/// front so device tasks don't edit shared code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub u8);

impl DeviceId {
    pub const PIC: Self = Self(0);
    pub const PIT: Self = Self(1);
    pub const KBD: Self = Self(2);
    pub const CMOS: Self = Self(3);
    pub const VGA: Self = Self(4);
    pub const SPEAKER: Self = Self(5);
    pub const DMA: Self = Self(6);
    pub const OPL: Self = Self(7);
    pub const SB: Self = Self(8);
    pub const MPU: Self = Self(9);
}

/// A scheduled event: device-defined `kind` due at `at` ns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub at: u64,
    pub dev: DeviceId,
    pub kind: u32,
}

/// Min-heap of events. Events due at the same time pop in insertion order.
#[derive(Clone, Debug, Default)]
pub struct Scheduler {
    heap: BinaryHeap<Reverse<(u64, u64, DeviceId, u32)>>,
    seq: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an event. Existing events for the same `(dev, kind)` are kept;
    /// call [`cancel`](Self::cancel) first to reschedule.
    pub fn schedule(&mut self, at: u64, dev: DeviceId, kind: u32) {
        self.heap.push(Reverse((at, self.seq, dev, kind)));
        self.seq += 1;
    }

    /// Remove every pending event for `(dev, kind)`.
    pub fn cancel(&mut self, dev: DeviceId, kind: u32) {
        self.heap
            .retain(|Reverse((_, _, d, k))| !(*d == dev && *k == kind));
    }

    /// Due time of the earliest event.
    pub fn next_due(&self) -> Option<u64> {
        self.heap.peek().map(|Reverse((at, ..))| *at)
    }

    /// Pop the earliest event if it is due at or before `now`.
    pub fn pop_due(&mut self, now: u64) -> Option<Event> {
        if self.next_due()? > now {
            return None;
        }
        let Reverse((at, _, dev, kind)) = self.heap.pop()?;
        Some(Event { at, dev, kind })
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(s: &mut Scheduler, now: u64) -> Vec<(u64, u8, u32)> {
        std::iter::from_fn(|| s.pop_due(now))
            .map(|e| (e.at, e.dev.0, e.kind))
            .collect()
    }

    #[test]
    fn pops_in_time_order_up_to_now() {
        let mut s = Scheduler::new();
        s.schedule(300, DeviceId::PIT, 0);
        s.schedule(100, DeviceId::VGA, 1);
        s.schedule(200, DeviceId::KBD, 2);
        assert_eq!(s.next_due(), Some(100));
        assert_eq!(s.pop_due(99), None);
        assert_eq!(drain(&mut s, 200), vec![(100, 4, 1), (200, 2, 2)]);
        assert_eq!(s.next_due(), Some(300));
        assert_eq!(drain(&mut s, u64::MAX), vec![(300, 1, 0)]);
        assert_eq!(s.next_due(), None);
    }

    #[test]
    fn ties_pop_in_insertion_order() {
        let mut s = Scheduler::new();
        s.schedule(50, DeviceId::VGA, 9);
        s.schedule(50, DeviceId::PIC, 3);
        s.schedule(50, DeviceId::VGA, 1);
        s.schedule(10, DeviceId::SB, 0);
        assert_eq!(
            drain(&mut s, 50),
            vec![(10, 8, 0), (50, 4, 9), (50, 0, 3), (50, 4, 1)]
        );
    }

    #[test]
    fn cancel_removes_only_matching_events() {
        let mut s = Scheduler::new();
        s.schedule(10, DeviceId::PIT, 0);
        s.schedule(20, DeviceId::PIT, 1);
        s.schedule(30, DeviceId::PIT, 0);
        s.schedule(40, DeviceId::VGA, 0);
        s.cancel(DeviceId::PIT, 0);
        assert_eq!(s.len(), 2);
        assert_eq!(drain(&mut s, 100), vec![(20, 1, 1), (40, 4, 0)]);
        assert!(s.is_empty());
    }
}
