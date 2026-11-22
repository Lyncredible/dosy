# 013 — Machine loop, interrupts and HLE dispatch

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 003, 007, 008
- **Owner:** —

## Goal
`Machine` ties CPU, Hardware and HLE together per the timing model.

## Scope
- `machine.rs`: `Machine::new(MachineConfig)`, `run_for(cycles) -> RunExit`,
  `set_cycles_per_ms`, `set_rtc`, `key_event`, `frame()` (if VGA landed),
  `mount` placeholder.
- Loop: step CPU until budget or next scheduler deadline; fire events;
  accept PIC interrupts when IF=1 (after STI/MOV SS shadow); HLT fast-forwards
  to the next event.
- Dispatch `Step::Callback(id)` on `id.subsystem()` to registered handlers;
  implement `HleResult::Retry` (rewind IP, set IF, halt).
- Wire installers of BIOS/DOS if they already exist; otherwise later tasks
  add one line each to `Machine::new`.

## Acceptance criteria
- [ ] Headless tests: PIT IRQ0 handler counting ticks over 1 emulated second
      (≈18), HLT wakes on IRQ, Retry loop resumes after an IRQ, callback
      dispatch, determinism (same inputs → same memory hash).

## Review follow-ups
- [ ] Honor `cpu.int_shadow`: take no maskable IRQ for the one instruction
      after STI, MOV SS or POP SS. Test it with a pending IRQ0.
- [ ] Wire `Hardware::fire_due`, `advance_cycles` and `cycles_until` into
      the `run_for` loop, so the budget stops at the next scheduler deadline.
