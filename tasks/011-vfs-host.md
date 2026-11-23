# 011 — Host directory and single-file drives

- **Status:** done
- **Milestone:** M1
- **Depends on:** 004
- **Owner:** worker-4

## Goal
Mount a host directory (or one file) as a DOS drive with 8.3 names.

## Scope (`dosy-vfs`)
- `shortname.rs`: deterministic long→8.3 mapping per directory
  (`NAME~N.EXT`, sorted host names, illegal chars → `_`), reusable by 028.
- `HostDrive`: case-insensitive lookup, create/write/delete/rename/mkdir,
  timestamps → `DosDateTime`, free space (fixed plausible value is fine).
  Read-only option. Never escape the mount root (reject `..` past root,
  symlinks out of root).
- Single-file mount: drive exposing one file at its root.
- `open_mount(path, opts)` for dir/file (zip arm added by 028).

## Acceptance criteria
- [x] Tests using a temp dir: mapping of long/mixed-case/duplicate names,
      round-trip writes, read-only enforcement, root-escape rejection.
