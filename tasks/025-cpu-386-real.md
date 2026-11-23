# 025 — 80186 and 386 real-mode instructions

- **Status:** done
- **Milestone:** M2
- **Depends on:** 007
- **Owner:** worker-2

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
- [x] Tests per group; CPU-detection idioms (PUSHF/POPF bit 12–15, AC flag)
      report the configured model.
- [ ] 80286 SingleStepTests (via 014 harness) pass for real-mode opcodes.

## Progress
Part 1 (186/286): PUSHA/POPA, PUSH imm, IMUL imm, C0/C1, INS/OUTS, ENTER/
LEAVE, BOUND (INT 5), `0F 01` SGDT/SIDT/LGDT/LIDT/SMSW/LMSW stubs, F1
(INT1 on 386, LOCK alias before), `CpuModel` (`MachineConfig.cpu_model`,
config key `cpu`): FLAGS 12..15, PUSH SP, shift-count masking, `0F` =
POP CS / #UD (INT 6). 8086 aliases in 60..6F, C0/C1/C8/C9 are not emulated.
The app still has to map `cpu` to `MachineConfig` (022 part 2).

Part 2 (386): 66h/67h (32-bit operands; 32-bit addressing with SIB,
offsets truncated to 16 bits), 32-bit forms of the one-byte opcodes
(`ops32.rs`), string ops and LOOP/JCXZ with ECX, `0F` Jcc near, SETcc,
PUSH/POP FS/GS, BT/BTS/BTR/BTC, BSF/BSR, SHLD/SHRD, IMUL r,r/m,
MOVZX/MOVSX, LSS/LFS/LGS, MOV CRn stubs (PE=1 unsupported), CLTS; 66h/67h
and FS/GS prefixes raise #UD on the 186. PUSHFD/POPFD keep AC/ID at 0.
Not done: 32-bit ENTER/BOUND (rare). The 286 SingleStepTests run belongs to
014's harness.
