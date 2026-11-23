//! Latest-wins single-slot mailbox between the emulation and main threads.
//! Buffers are swapped, never cloned, so steady state does no allocation.

use std::sync::Mutex;

pub struct Mailbox<T> {
    slot: Mutex<Slot<T>>,
}

struct Slot<T> {
    value: T,
    fresh: bool,
}

impl<T> Mailbox<T> {
    pub fn new(initial: T) -> Self {
        Self {
            slot: Mutex::new(Slot {
                value: initial,
                fresh: false,
            }),
        }
    }

    /// Swaps `value` into the slot, replacing any unread value. `value` gets
    /// the previous slot contents back for reuse.
    pub fn publish(&self, value: &mut T) {
        let mut s = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::swap(&mut s.value, value);
        s.fresh = true;
    }

    /// If a value was published since the last take, swaps it into `out`
    /// and returns true.
    pub fn take(&self, out: &mut T) -> bool {
        let mut s = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if !s.fresh {
            return false;
        }
        std::mem::swap(&mut s.value, out);
        s.fresh = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_wins_and_buffers_recycle() {
        let m = Mailbox::new(0);
        let mut out = -1;
        assert!(!m.take(&mut out));
        assert_eq!(out, -1);

        let (mut a, mut b) = (1, 2);
        m.publish(&mut a);
        assert_eq!(a, 0, "producer gets the old slot buffer back");
        m.publish(&mut b);
        assert!(m.take(&mut out));
        assert_eq!(out, 2, "second publish replaced the unread first");
        assert!(!m.take(&mut out), "consumed");
        assert_eq!(out, 2);
    }

    #[test]
    fn works_across_threads() {
        let m = std::sync::Arc::new(Mailbox::new(Vec::<u64>::new()));
        let p = m.clone();
        std::thread::spawn(move || {
            for i in 0..100 {
                let mut v = vec![i];
                p.publish(&mut v);
            }
        })
        .join()
        .unwrap();
        let mut out = Vec::new();
        assert!(m.take(&mut out));
        assert_eq!(out, [99]);
    }
}
