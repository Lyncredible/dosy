# 036 — AdLib ports, OPL timers, MPU-401 stub

- **Status:** todo
- **Milestone:** M3
- **Depends on:** 003
- **Owner:** —

## Goal
Sound-card detection succeeds before synthesis exists.

## Scope
- `hw/opl.rs`: ports 388h/389h (and 228h/229h mirror later via SB),
  register file write/read, status register with timer 1/2 (80 µs /
  320 µs) via the scheduler, IRQ-less flags, reset bits — exactly what
  AdLib detection routines check.
- Register-write event stream (`(time_ns, reg, val)`) for the synth (037).
- `hw/mpu.rs`: MPU-401 at 330h/331h, UART mode (3Fh) ack FEh, status
  bits, bytes collected into a MIDI out queue (consumed later or dropped).

## Acceptance criteria
- [ ] Test running the standard AdLib detection sequence (timer 1 start,
      wait, status = C0h) on `Hardware`.
- [ ] MPU reset/UART handshake test.
