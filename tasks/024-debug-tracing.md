# 024 — Debug tracing and fault reports

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 013
- **Owner:** —

## Goal
Make "why did this program break" fast to answer.

## Scope
- `log` facade in core; app installs a logger (`env_logger` or minimal).
- Warn-once registry for unimplemented opcodes, ports, INT functions.
- Instruction trace ring buffer (last N `CS:IP opcode regs`), dumped on
  `RunExit::Fault`; opt-in full trace to file (`--trace`).
- Interrupt call log filter (`--log-int=21,10`).
- Disassembler-lite for the dump (mnemonic + operands for common opcodes).

## Acceptance criteria
- [ ] Tests: ring buffer wrap, warn-once dedupe, fault dump format.
- [ ] Tracing off costs no measurable time (branch on a flag).
