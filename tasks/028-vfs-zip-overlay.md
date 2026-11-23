# 028 — Zip drives and writable overlay

- **Status:** done
- **Milestone:** M2
- **Depends on:** 011
- **Owner:** worker-4

## Goal
Mount a `.zip` as a read-only DOS drive, optionally with writes going to an
overlay.

## Scope (`dosy-vfs`)
- `zip` crate, `default-features = false`, deflate (pure Rust backend).
- `ZipDrive`: index entries at mount (implicit directories, CP437/UTF-8
  names, 8.3 mapping via `shortname.rs`), inflate to memory on open,
  timestamps from entries.
- `OverlayDrive<Lower, Upper>`: reads fall through, writes copy-up to upper
  (`MemDrive` or `HostDrive`), deletes recorded as whiteouts, merged
  `read_dir`.
- `open_mount` handles `.zip` (`overlay` option: none / memory / dir).

## Acceptance criteria
- [x] Tests build zips at runtime in a temp dir (no committed archives):
      listing, nested dirs, long names, read content, write rejected
      without overlay, overlay copy-up/delete/rename.
