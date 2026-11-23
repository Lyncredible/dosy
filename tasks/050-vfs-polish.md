# 050 — VFS polish

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 011
- **Owner:** —

## Goal
Address the `HostDrive` follow-ups from the 011 review.

## Scope (`dosy-vfs`)
- `HostDrive` re-lists and canonicalizes a directory on every lookup. Add a
  per-directory cache (listing + 8.3 map) that is invalidated by this drive's
  own writes and refreshed when the directory mtime changes.
- Document that `~N` aliases can change when files are added to a directory
  (DOSBox does the same).
- `single_file` mounted via a symlink exposes the target's name; use the
  link's own name instead.
- Creating a file named like a device (`AUX`, `CON`, `NUL`, `PRN`, `COMn`,
  `LPTn`) must never reach the host, where Windows treats those names
  specially. The DOS device layer (020) resolves these names first; add a
  guard in `HostDrive` that returns an error as a backstop.

## Acceptance criteria
- [ ] Tests: cache invalidation after create/rename/delete and external
      change, symlink name (skipped where symlinks are unavailable), device
      name guard.
- [ ] Benchmark or test showing that repeated lookups no longer re-list.
