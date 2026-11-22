# 019 — COM/EXE loader and EXEC

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 018
- **Owner:** —

## Goal
Load and run programs: INT 21h/4Bh and process lifecycle.

## Scope
- COM: allocate largest block, PSP, load at 100h, SP=FFFEh, push 0.
- MZ: header parse, min/max alloc, load module, relocations, SS:SP/CS:IP,
  overlay data ignored; `4B03` load overlay with relocation factor.
- `4B00`: environment copy (+ program path after the env), command tail at
  PSP 80h, FCBs 5Ch/6Ch parsed, parent PSP/SS:SP saved, INT 22h/23h/24h
  saved in PSP; terminate restores parent, frees memory, closes handles,
  sets return code (4Dh).
- `4B01` optional. 31h TSR (keep memory).
- Public Rust `exec(dos, cpu, bus, path, tail)` for the shell.

## Acceptance criteria
- [ ] Tests: hand-built MZ with relocations loads with correct segments;
      COM register state; nested exec returns to parent with code in 4Dh;
      memory fully freed after child exit.
