# 009 — 8042 keyboard controller, port 61h, CMOS/RTC

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 003
- **Owner:** —

## Goal
Keyboard scancodes reach the guest via IRQ1; CMOS gives the BIOS a clock.

## Scope
- `hw/kbd.rs`: ports 60h/64h, output buffer + status bits, scancode set 1
  queue (`push_scancodes(&[u8])`), raise IRQ1 when data is available and
  the previous byte was read, commands used by real programs (AAh self test,
  ADh/AEh, D1h output port → A20, EDh LEDs ack FAh, F4h enable).
- Port 61h shared bits with the PIT/speaker (coordinate with 008: one
  `SystemControl` handler in `Hardware`).
- Fast A20 via port 92h.
- `hw/cmos.rs`: ports 70h/71h, 128 bytes, RTC time/date in BCD from an
  injected base time advanced by emulated `now`, status regs A–D, equipment
  and memory size bytes. No host clock reads.

## Acceptance criteria
- [ ] Tests: scancode queue ordering and IRQ1 pacing, 8042 commands,
      A20 toggles memory wrap, CMOS time advances with emulated time.

## Review follow-ups
- [ ] Port 92h (fast A20) already lives in `Hardware`. The 8042 output-port
      path (command D1h) must go through the same `mem.set_a20`, so both
      paths share one A20 state. Test that each path is visible to the other.
