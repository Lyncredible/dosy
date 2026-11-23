# 015 — BIOS HLE core

- **Status:** in-progress
- **Milestone:** M1
- **Depends on:** 002
- **Owner:** worker-3

## Goal
Enough PC BIOS for DOS and programs: BDA, IVT, and the core interrupts.

## Scope (`bios/`, ids `0x00xx`)
- `install<B: Bus>(bus)`: BDA (`0040:0000`: equipment word, memory size 640,
  keyboard buffer head/tail, video fields, tick count), IVT defaults (unused
  vectors → dummy IRET), ROM stubs via `hle::install_stub`, reset vector,
  model byte `F000:FFFE`, PIC/PIT init via port writes.
- INT 08 (tick count, midnight flag, chain INT 1Ch, EOI), INT 09 (read port
  60h, set-1 translation to ASCII incl. shift/ctrl/alt/caps, E0 keys, BDA
  ring buffer, EOI), INT 11, INT 12, INT 15 (C0 config, 86 wait, 88 ext mem
  = 0, 24xx A20, others CF=1), INT 16 (00/01/02/10/11/12, `Retry` on empty),
  INT 1A (00/01 ticks, 02/04 RTC via CMOS ports).
- Handlers generic over `Bus`; INT 10 lives in 016.

## Progress
- Part 1: `install` (ROM, IVT, BDA, PIC/PIT init), INT 08/11/12/15/19/1A,
  default IRQ EOI handlers, port 92h A20, robust `load_rom`. Done.
  INT 15h/86h returns at once (no wait yet). INT 08h sends EOI before
  chaining INT 1Ch (stub slots are 8 bytes).
- Part 2: INT 09h and INT 16h (keyboard). Todo.

## Acceptance criteria
- [ ] `FlatBus` tests per handler: tick rollover, key translation table
      (letters, shifted, ctrl, extended arrows → 00/E0 scan), buffer full,
      INT 16/00 Retry then Done.

## Review follow-ups
- [x] The PIC's power-on state is already post-BIOS (vectors 08h/70h, IMR
      all unmasked). BIOS init should program the IMR the way a real BIOS
      leaves it: only the IRQs it services unmasked (timer, keyboard,
      cascade).
- [x] The PIT is unprogrammed at power-on, so BIOS init must program
      channel 0 for the 18.2 Hz tick: `43h ← 36h`, then `40h ← 00h, 00h`.
- [x] (Low priority) Make `Memory::load_rom` / `Hardware::load_rom` robust. Today they apply
      the A20 mask to the start address only and panic when the copy runs past
      the end of RAM. Mask or wrap per byte (or clip) and never panic. Add a
      test that loads across the 1 MiB and HMA end boundaries.
