# 024 — Debug tracing and fault reports

- **Status:** done
- **Milestone:** M1
- **Depends on:** 013
- **Owner:** worker-1

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
- [x] Tests: ring buffer wrap, warn-once dedupe, fault dump format.
- [x] Tracing off costs no measurable time (branch on a flag).

## Notes
- `Machine::tracer: Option<Box<Tracer>>` (None = one branch per
  instruction); `Machine::fault_report(exit)`; `debug::warn_once`.
- App: `--trace` (64-entry ring), `--trace=FILE` (+ full trace),
  `--log-int=21,10`, `DOSY_LOG` level; reports print on fault/halt.
- Per-instance warn-once sets in `Hardware` (ports) and `Dos` predate the
  shared registry and were left as is.
