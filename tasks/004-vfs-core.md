# 004 — VFS traits, DOS paths and MemDrive

- **Status:** done
- **Milestone:** M1
- **Depends on:** 000
- **Owner:** worker-3

## Goal
The drive abstraction the DOS kernel talks to, plus an in-memory drive for
tests.

## Scope (`dosy-vfs`)
- `Drive`, `DosFile` traits, `DriveInfo`, `DirEntry` (8.3 name, attr, size,
  `DosDateTime`), `OpenMode`, `Attr` bitflags, `SeekFrom` use, per
  `docs/ARCHITECTURE.md`.
- `DosError` with `code() -> u16` (2 file not found, 3 path not found,
  4 too many open files, 5 access denied, 0x12 no more files, 0x50 exists…).
- `DosName` (validated 8.3, uppercase, DOS-legal chars) and `DosPath`
  (components; parse `\A\B.TXT`, `.`/`..` resolution, no drive letter).
- Wildcards: `fn matches(pattern: &DosName-ish, name) -> bool` with DOS
  semantics (`*` fills field, `?`, `*.*`, `FOO*.?X`).
- `MemDrive`: tree in memory, all ops, optional read-only flag.

## Acceptance criteria
- [x] Tests: name validation, path parsing, wildcard table, MemDrive CRUD,
      read-only errors, seek beyond EOF + write extends, rename across dirs.

## Notes
- No `zip`/host code here (011, 028). Keep `Send`.
