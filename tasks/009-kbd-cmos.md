# 009 — 8042 keyboard controller, port 61h, CMOS/RTC

- **Status:** done
- **Milestone:** M1
- **Depends on:** 003
- **Owner:** worker-3

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

## Notes (done)
- Port 61h stays as 008 left it (bits 0/1/4/5); nothing keyboard-specific
  is needed on an AT. Port 92h landed with 015.
- Next byte reaches 60h 1 ms after a read, so chained INT 9 handlers that
  re-read 60h see the same byte. `Machine::key_event` → `push_scancodes`.
- `Machine::set_rtc` sets the CMOS clock and resyncs the BIOS tick count.
- Not modelled: aux (mouse) port, RTC alarm/periodic IRQ8, CPU reset.

## Acceptance criteria
- [x] Tests: scancode queue ordering and IRQ1 pacing, 8042 commands,
      A20 toggles memory wrap, CMOS time advances with emulated time.

## Review follow-ups
- [ ] Port 92h (fast A20) already lives in `Hardware`. The 8042 output-port
      path (command D1h) must go through the same `mem.set_a20`, so both
      paths share one A20 state. Test that each path is visible to the other.
