# 046 — DOS kernel polish

- **Status:** todo
- **Milestone:** M2
- **Depends on:** 018, 019
- **Owner:** —

## Goal
Close the DOS-compatibility gaps that the 017/018/019 reviews found.

## Scope (`dos/`, `dosy-vfs` where noted)
File I/O:
- Seeking before the start of a file (42h with a negative result) wraps to
  the 32-bit unsigned position as DOS does, instead of failing. `MemDrive`
  currently returns `InvalidFunction`; 011's `HostDrive` must match.
- FindFirst/FindNext return `.` and `..` in subdirectories. 4Fh resumes
  from saved state instead of re-reading the whole directory each call.
- Code-page (≥ 80h) bytes are allowed in 8.3 names.
- The no-inherit bit (3Dh open mode bit 7) is honored on EXEC.

EXEC / processes:
- An MZ image with min = max = 0 loads high (top of the block).
- 4B01 (load, don't run) pushes AX onto the child stack as DOS does.
- Child AL/AH = FFh when the FCB 1 / FCB 2 drive is invalid.
- A bad MZ header returns error 0Bh (invalid format).
- Environment and program blocks are allocated with the current 58h
  allocation strategy.
- A failed `Machine::exec` restores the CPU state, or the behavior is
  documented on the function.

## Acceptance criteria
- [ ] One test per item, using `MemDrive` and hand-built COM/MZ images.
