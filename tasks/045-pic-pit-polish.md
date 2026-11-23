# 045 — Chipset accuracy polish (PIC, PIT, 8042, RTC)

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 008, 009
- **Owner:** —

## Goal
Close the 8259/8254/8042/RTC accuracy gaps left after 008 and 009.

## Scope
PIC (`hw/pic.rs`):
- A pending slave request shows as master IRR bit 2.
- OCW3 `0x48` (special mask mode off) is accepted without a warning.
- The master poll command takes the cascade into account.

PIT (`hw/pit.rs`):
- With count 1 in mode 2/3, an IRQ is scheduled every tick. Either clamp it
  or match hardware, and don't flood the scheduler.
- In mode 3, a count written mid-run takes effect at the end of the current
  half-cycle.
- One-tick load delay after a count write.
- In mode 0, while counting is stopped after the first byte of a two-byte
  write, reads return the frozen count.
- The latch command does not reset the lobyte/hibyte read toggle.
- BCD counting (control word bit 0).

Machine tests (only once 013 / PR #14 has merged):
- Replace the fake IRQ0 tick test with one driven by the real PIT.

BIOS timing:
- INT 15h/86h (wait CX:DX µs) returns immediately today. Implement it with
  `HleResult::Retry` plus a deadline in emulated time.

8042 / CMOS (left over from 009):
- 8042 aux (PS/2 mouse) port: commands A7h/A8h/D4h and IRQ12.
- RTC alarm and periodic interrupts on IRQ8 (status regs A/B/C).
- CPU reset through the 8042 (command FEh pulse), resuming via the BDA
  shutdown byte / `0040:0067`.
- Keyboard FEh (resend) response.
- Port 70h bit 7 NMI mask.

## Acceptance criteria
- [ ] One unit test per item above; existing PIC/PIT/8042/CMOS tests still pass.
