# 018 — DOS file and directory I/O

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 004, 017
- **Owner:** —

## Goal
Handle-based file and directory services on top of `dosy-vfs` drives.

## Scope
- Drive table, current drive (0Eh/19h), per-drive cwd (3Bh/47h), path
  resolution (drive letters, relative paths, `.`/`..`, `/` as separator).
- Handles: JFT in PSP + system file table; 3Ch create, 3Dh open (modes,
  sharing ignored), 3Eh close, 3Fh read, 40h write (CX=0 truncates), 41h
  delete, 42h seek, 43h attributes, 45h/46h dup, 56h rename, 57h time,
  5Bh create new, 39h/3Ah mkdir/rmdir, 36h free space.
- 4Eh/4Fh find first/next using DTA layout (attributes filter, volume label
  ignored), search state stored in DTA reserved bytes.
- Handles 0–4 reserved for devices (wired to CON in 020).

## Acceptance criteria
- [ ] Tests with `MemDrive`: create/write/seek/read, find-first wildcard
      listing order, error codes, cwd per drive, handle exhaustion.

## Review follow-ups
- [ ] Real DOS silently truncates over-long names
      (`LONGFILENAME.TEXT` → `LONGFILE.TEX`), but `DosName::parse` rejects
      them. Truncate each component before parsing, or add a lenient parse to
      `dosy-vfs`, for open, create, find and rename paths. Add a test.
