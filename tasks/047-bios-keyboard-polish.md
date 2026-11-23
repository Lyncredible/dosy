# 047 — BIOS keyboard polish

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 015
- **Owner:** —

## Goal
Finish the less common INT 09h/16h behaviors left out of 015.

## Scope (`bios/`)
- PrtSc → INT 05h.
- Ctrl-Alt-Del → warm reboot (set `0040:0072` = 1234h, jump to reset), or a
  defined `RunExit` if reboot isn't supported.
- Alt + keypad digits enter an ASCII code on Alt release.
- Alt-Tab returns A500h from the enhanced read (INT 16h/10h).
- Shift + keypad 0 with NumLock on acts as Insert.
- INT 09h calls the INT 15h/4Fh keyboard intercept hook (and can drop the
  key). Update the INT 15h/C0h feature byte to advertise it.
- Beep (PC speaker via PIT ch2) when the keyboard buffer is full.

## Acceptance criteria
- [ ] One `FlatBus` / `Hardware` test per item.
