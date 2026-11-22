# 021 — Built-in shell and MOUNT

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 011, 013, 019, 020
- **Owner:** —

## Goal
Boot to a `Z:\>` / `C:\>` prompt and run commands and programs.

## Scope (`shell/`, ids `0x02xx`)
- Shell process per `docs/ARCHITECTURE.md` (Loop stub, state machine,
  started by `Machine::new` after BIOS/DOS install). Z: is a built-in
  `MemDrive` holding `AUTOEXEC.BAT`-like config lines.
- Prompt (`PROMPT $P$G`), line input via DOS 0Ah, parse with quoting,
  redirection `>`, `>>`, `<` (pipes later).
- Builtins: ECHO, CLS, DIR (incl. /W, sizes, free bytes), CD/CHDIR, `X:`,
  TYPE, SET, PATH, VER, EXIT, MD/RD, DEL/ERASE, REN, COPY (simple), MOUNT
  (`MOUNT C path`, `-u`, list) using `dosy_vfs::open_mount`.
- Program lookup: cwd then PATH, `.COM` → `.EXE` → `.BAT` (BAT runner
  stubbed until 029).
- `Machine::mount` wired to the DOS drive table.

## Acceptance criteria
- [ ] Headless test: boot `Machine` with C: = `MemDrive`, reach the prompt in
      `TextScreen`, inject keys for `echo hi`, see `hi`; `dir` lists files;
      running a tiny COM that prints and exits returns to the prompt.
