# 022 — Emulation thread, pacing and CLI

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 012, 013
- **Owner:** —

## Goal
Run `Machine` on its own thread at real-time speed and show its frames.

## Scope (`dosy`)
- Threading per `docs/ARCHITECTURE.md`: `HostEvent` mpsc to the emu thread,
  latest-wins frame mailbox, `EventLoopProxy` redraw wake.
- Pacing: ~1 ms ticks, `cycles_per_ms × elapsed`, cap catch-up at e.g.
  50 ms; `cycles = max` runs flat out with frame-rate-limited presents.
- Host RTC injected at startup (`set_rtc`).
- CLI: `dosy [PATH]` mounts PATH as C: (dir/zip/file) and runs an optional
  `-c "command"`; config `mount`/`autoexec` keys applied.
- Clean shutdown on window close; fault → message in title + pause.

## Acceptance criteria
- [ ] Boots to the shell prompt in a window (once 021 has landed; before
      that, shows the BIOS text screen).
- [ ] Unit tests for the pacing calculator and mailbox.
