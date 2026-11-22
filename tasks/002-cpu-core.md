# 002 — CPU state, decoder and control flow

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 000
- **Owner:** —

## Goal
The CPU skeleton everything else plugs into: state, fetch/decode, ModRM,
`Cpu::step`, interrupts and the HLE callback opcode.

## Scope
- `cpu/mod.rs`, `cpu/regs.rs`, `cpu/decode.rs`, `cpu/exec.rs` per
  `docs/ARCHITECTURE.md` (`Cpu`, `Step`, `Cpu::reset/step/interrupt`).
- Prefixes: segment override, REP/REPNE (recorded only), LOCK (ignored).
- ModRM + 16-bit effective addresses with default segment rules.
- Instructions: MOV (all forms), PUSH/POP (reg, sreg, r/m), PUSHF/POPF,
  XCHG, LEA, LDS, LES, LAHF/SAHF, CLC/STC/CMC/CLD/STD/CLI/STI, NOP, HLT,
  JMP (short/near/far/indirect), CALL/RET/RETF (+imm), Jcc, JCXZ, LOOP*,
  INT n, INT3, INTO, IRET.
- `FE 38 lo hi` → `Step::Callback(HleId)` (see `hle.rs`), IP after the 4 bytes.
- Unknown opcode → `Step::Fault(CpuFault::InvalidOpcode{..})` (no panic).
- Cycle cost per DOSBox semantics (1 per instruction for now).

## Acceptance criteria
- [ ] Tests on `FlatBus` with hand-assembled programs: far jump/call/ret,
      stack wrap at SP=0, segment override, INT/IRET round trip through IVT,
      callback opcode, Jcc table, LOOP.
- [ ] Instruction pointer wraps within the 64 KiB segment.

## Notes
- Leave ALU opcodes for 007; keep the opcode dispatch table easy to extend
  (one `match` arm per opcode or a table of fn pointers).
- Keep `Regs` 32-bit capable (EAX..) for 025 even if only 16-bit is used.
