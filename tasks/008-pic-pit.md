# 008 — 8259 PIC and 8254 PIT

- **Status:** done
- **Milestone:** M1
- **Depends on:** 003
- **Owner:** worker-3

## Goal
Interrupt controller and timer, the heartbeat of every DOS program.

## Scope
- `hw/pic.rs`: master (20h/21h) + slave (A0h/A1h) cascade on IRQ2, ICW1–4,
  OCW1–3, IMR, IRR/ISR reads, specific/non-specific EOI, edge triggered,
  `pending_vector() -> Option<u8>` / `acknowledge()` for the machine.
- `hw/pit.rs`: 3 channels (40h–43h), modes 0, 2, 3, 4 (1 and 5 basic),
  lobyte/hibyte/both access, latch command, read-back optional. Counters are
  computed lazily from `now`; channel 0 schedules IRQ0 edges via the
  scheduler. Channel 2 gate/output exposed for port 61h and the speaker.
- Port 61h bits 0/1/5 handled in `Hardware` (coordinate with 009).
- Register both in `ports.rs`.

## Progress
- Part 1 (PIC + `Hardware::sync_irqs`): done. Special mask mode,
  level-triggered mode and AEOI rotation are not modelled (warn).
- Part 2 (PIT, port 61h bits 0/1/4/5): done. BCD counting not modelled
  (warn). Power-on PIT is unprogrammed; the BIOS (015) programs ch0
  (`43h←36h`, `40h←00h,00h`).

## Acceptance criteria
- [x] PIC tests: init sequence as the BIOS does it, priority, masking, EOI,
      cascade vector numbers.
- [x] PIT tests: mode 2/3 period at default 65536 → 18.2065 Hz, counter
      readback mid-period, reprogramming mid-count, mode 0 one-shot IRQ.

## Review follow-ups
- [x] Implement `Hardware::sync_irqs` (a no-op since 003): apply each
      device's `IrqLines` to the PIC, otherwise device IRQs are dropped. Test
      that a device raising IRQ0 or IRQ1 shows up in `pending_vector()`.
- PIC nits moved to [045](045-pic-pit-polish.md).
