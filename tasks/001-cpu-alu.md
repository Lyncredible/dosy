# 001 — CPU ALU and flags

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 000
- **Owner:** —

## Goal
Bit-exact x86 arithmetic/logic as pure functions, independent of the decoder.

## Scope
- `dosy-core/src/cpu/alu.rs`; flag masks already exist in `cpu/flags.rs`.
- 8/16/32-bit: ADD ADC SUB SBB CMP AND OR XOR TEST INC DEC NEG NOT.
- Shifts/rotates: SHL SHR SAR ROL ROR RCL RCR (count masking per 186+,
  OF/CF rules for count 0/1/n).
- MUL IMUL (1/2/3-operand forms' flag rules), DIV IDIV returning
  `Result<_, DivideError>`.
- DAA DAS AAA AAS AAM AAD, CBW CWD CWDE CDQ helpers.
- Signature style: `fn add16(a: u16, b: u16, flags: u32) -> (u16, u32)`.

## Acceptance criteria
- [ ] Unit tests for every op incl. carry/overflow/aux edge cases and
      shift counts 0, 1, 8, 16, 31.
- [ ] No `Bus`/decoder dependency; `cargo test -p dosy-core` passes.

## Notes
- Undefined flags: match a real 8086/386 where known (SingleStepTests in 014
  will check later); document choices in code comments.
