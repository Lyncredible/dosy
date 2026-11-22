# 030 — Remaining DOS services (FCB, IOCTL, INT 2Fh)

- **Status:** todo
- **Milestone:** M2
- **Depends on:** 019
- **Owner:** —

## Goal
Fill the long tail of INT 21h used by older programs and C runtimes.

## Scope
- FCB: 0Fh/10h open/close, 11h/12h find, 13h delete, 14h/15h seq
  read/write, 16h create, 17h rename, 21h/22h/27h/28h random, 23h size,
  29h parse filename.
- 44h IOCTL 08/09 (removable/remote), 0Eh/0Fh; 38h country info; 65h/66h;
  59h extended error; 5Ah temp file; 60h truename; 67h handle count.
- INT 2Fh multiplex: 1680h idle, 4300h XMS check (no XMS until 032),
  1600h Windows check → none; INT 28h idle.
- INT 24h default handler (fail).

## Acceptance criteria
- [ ] `MemDrive` tests for FCB sequential/random I/O and 29h parsing table;
      unit tests for the other functions.
