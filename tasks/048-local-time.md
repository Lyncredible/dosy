# 048 — Local time

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 009, 011
- **Owner:** —

## Goal
DOS sees the host's local time consistently, as on a real PC.

## Scope
- Today the RTC base (`Machine::set_rtc`) and `HostDrive` file timestamps are
  UTC. Switch both to local time. Use one small cross-platform crate (e.g.
  `time` with `local-offset`, or `chrono`) in `dosy-vfs` and in the app.
  `dosy-core` stays clock-free and receives the converted value.
- Apply the same conversion to timestamps written back to host files (5701h,
  close after write).
- Document the convention in `docs/ARCHITECTURE.md`.

## Acceptance criteria
- [ ] Tests with a fixed offset: RTC date/time and DIR timestamps round-trip
      through local time. DST edges are handled by the crate.
