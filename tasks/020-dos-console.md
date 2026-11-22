# 020 — DOS console I/O and character devices

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 016, 017
- **Owner:** —

## Goal
Character I/O through DOS: keyboard in, screen out, devices CON/NUL.

## Scope
- INT 21h 01, 02, 06, 07, 08, 09, 0Ah (buffered line input with editing:
  BS, ESC, F3 optional), 0Bh, 0Ch; Ctrl-C/Ctrl-Break basic; INT 29h fast
  console output.
- Blocking reads use `HleResult::Retry` via the BIOS keyboard buffer.
- Devices CON, NUL, AUX, PRN, CLOCK$ as files: open by name in any dir,
  handles 0/1/2 = CON; 44h IOCTL 00/01 (device info, raw mode), 06/07
  status.
- Output via `bios::video::teletype`.

## Acceptance criteria
- [ ] Tests: `AH=09` string to text memory, `0Ah` line with backspace,
      write to handle 1, redirect-ready (handle 1 can point to a file),
      Retry when no key.
