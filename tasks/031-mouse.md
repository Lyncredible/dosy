# 031 — INT 33h mouse driver and host mouse

- **Status:** todo
- **Milestone:** M2
- **Depends on:** 015, 022
- **Owner:** —

## Goal
Programs that use a mouse get one.

## Scope
- `bios/mouse.rs` (driver ids `0x03xx`): INT 33h 00–0Ch, 0Fh, 1Ah/1Bh,
  24h; event handler callback (0Ch) invoked via a far-call trampoline;
  software cursor in text mode and mode 13h (drawn into VRAM, saved/restored).
- `Machine::mouse_event`; app forwards relative motion (pointer lock when
  captured, click to capture, release with a keybind).

## Acceptance criteria
- [ ] Tests: position clamping, mickey ratio, button press counters,
      user handler called with correct registers.
