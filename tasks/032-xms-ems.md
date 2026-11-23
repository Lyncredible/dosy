# 032 — XMS and EMS

- **Status:** todo
- **Milestone:** M2
- **Depends on:** 017
- **Owner:** —

## Goal
Extended/expanded memory for programs that require it.

## Scope
- Memory beyond 1 MiB in `Memory` (configurable, default 16 MiB).
- XMS 3.0 driver (INT 2Fh 4300h/4310h, far-call entry via `Retf` stub):
  version, HMA request/release, A20 control, alloc/free/move/lock EMBs,
  query free.
- EMS 4.0 subset (INT 67h, device name `EMMXXXX0`): page frame at `E000`,
  40h–4Eh, 50h, 51h, 58h basics; mapping via memory remap.
- Config `xms = true`, `ems = true`.

## Acceptance criteria
- [ ] Tests: EMB alloc/move round trip, HMA access with A20 on, EMS map
      page and read through the frame.

## Review follow-ups
- [ ] INT 15h/88h returns 0 KB today. Make it report extended memory
      consistent with what XMS provides (and with the CMOS memory bytes).
- [ ] One source of truth for the extended-memory size: CMOS 17h/18h and
      30h/31h, the CMOS 14h equipment byte, INT 15h/88h and XMS must all agree.
