# 003 — Memory, Hardware bus and scheduler

- **Status:** done
- **Milestone:** M1
- **Depends on:** 000
- **Owner:** worker-3

## Goal
The real machine's `Bus`: RAM, A20, ROM, MMIO window, port routing and the
event scheduler that all devices use.

## Scope
- `memory.rs`: 1 MiB + HMA RAM, A20 gate (wrap at 1 MiB when off),
  write-protected ROM ranges (`F0000–FFFFF`, writable by host code via a
  separate `load_rom`), MMIO hook for `A0000–BFFFF` (a trait or enum the VGA
  implements later; default: plain RAM).
- `hw/mod.rs`: `Hardware` implementing `Bus`; `hw/ports.rs` routing `match`
  (empty for now; unclaimed ports read `0xFF`, warn once per port).
- `IoDevice`, `IoCtx`, `IrqLines` as in `docs/ARCHITECTURE.md`.
- `hw/sched.rs`: `Scheduler` min-heap keyed by emulated ns with
  `schedule(at, DeviceId, kind)`, `cancel(DeviceId, kind)`, `next_due()`,
  `pop_due(now)`. Deterministic tie-break (insertion order).
- `Hardware::now()` = emulated ns; `advance_cycles(n)` using `cycles_per_ms`.

## Acceptance criteria
- [x] Tests: A20 wrap on/off, ROM write ignored, MMIO routing, scheduler
      ordering/cancel/ties, cycles→ns conversion when `cycles_per_ms` changes.
- [x] Port dispatch has a one-line-per-device pattern later tasks extend.

## Notes
- Hot path: `read8/write8` must be cheap (bounds-checked slice index, range
  check for MMIO first).
