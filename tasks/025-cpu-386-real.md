# 025 — 80186 and 386 real-mode instructions

- **Status:** todo
- **Milestone:** M2
- **Depends on:** 007
- **Owner:** —

## Goal
Run code built by later compilers/assemblers that assume a 186/286/386 in
real mode.

## Scope
- 80186: PUSHA/POPA, PUSH imm, IMUL r,r/m,imm, shift/rotate by imm,
  INS/OUTS, ENTER/LEAVE, BOUND (INT 5).
- 286 real mode: SMSW/LMSW (no PE), `0F 01` stubs as needed.
- 386 real mode: 66h/67h prefixes, 32-bit regs and addressing (SIB), FS/GS,
  MOVZX/MOVSX, SETcc, Jcc near (`0F 8x`), BT/BTS/BTR/BTC, BSF/BSR,
  SHLD/SHRD, IMUL r,r/m, CDQ/CWDE, LFS/LGS/LSS, PUSHFD/POPFD, 32-bit
  string ops and LOOP with 67h.
- CPU model config (`cpu = 8086|186|386`) affecting FLAGS high bits and
  invalid-opcode behavior used by CPU detection code.

## Acceptance criteria
- [ ] Tests per group; CPU-detection idioms (PUSHF/POPF bit 12–15, AC flag)
      report the configured model.
- [ ] 80286 SingleStepTests (via 014 harness) pass for real-mode opcodes.
