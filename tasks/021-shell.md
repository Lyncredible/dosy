# 021 — Built-in shell and MOUNT

- **Status:** in-progress
- **Milestone:** M1
- **Depends on:** 011, 013, 019, 020
- **Owner:** worker-3

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

## Progress
- Part 1: done. Shell process + `Machine::start_shell(autoexec)`, Z: drive,
  prompt, line editing from the BIOS keyboard (`Retry` while empty), output
  via the INT 10h teletype; ECHO, CLS, VER, CD/CHDIR, `X:`, DIR (/W, sizes,
  free bytes), TYPE, SET, PATH, EXIT, MOUNT (`X path`, `-u X`, list);
  program lookup (cwd, then PATH; .COM, .EXE; .BAT reports "not supported").
- Part 2: todo. Switch I/O to DOS CON / 0Ah (020), redirection `>`, `>>`,
  `<`, MD/RD, DEL/ERASE, REN, COPY.

## Acceptance criteria
- [x] Headless test: boot `Machine` with C: = `MemDrive`, reach the prompt in
      `TextScreen`, inject keys for `echo hi`, see `hi`; `dir` lists files;
      running a tiny COM that prints and exits returns to the prompt.

## Review follow-ups
- [x] Handle termination of the root process, together with 019. If the
      shell itself exits (`EXIT` at top level) or a program runs without a
      shell, the machine stops with a defined `RunExit`. It must not warn and
      continue.
- [x] The shell gives each child a real resume point: the parent return
      address in the child's PSP points back into the shell stub. Then
      `RunExit::Exited` happens only when the shell itself exits, never when a
      child terminates.
