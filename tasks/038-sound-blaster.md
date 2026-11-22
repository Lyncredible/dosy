# 038 — Sound Blaster and 8237 DMA

- **Status:** todo
- **Milestone:** M3
- **Depends on:** 034
- **Owner:** —

## Goal
Digitized sound effects via Sound Blaster 2.0/Pro-compatible DSP.

## Scope
- `hw/dma.rs`: 8237 channels 0–3 (8-bit), page registers, single and
  auto-init, mask/mode/flip-flop, `transfer(ch, buf)` for devices.
- `hw/sb.rs` at 220h, IRQ 7 (config), DMA 1: DSP reset/ready (AAh), version
  (E1h → 2.01 / 3.02), speaker on/off, time constant (40h), sample rate
  (41h), 8-bit single-cycle (14h) and auto-init (1Ch/48h/90h) PCM, IRQ ack
  via 22Eh, direct DAC (10h), mixer chip basics (224h/225h), OPL mirror at
  228h/229h when 036 exists.
- `BLASTER=A220 I7 D1 T3` set in the DOS environment.

## Acceptance criteria
- [ ] Tests: DSP reset handshake, single-cycle transfer raises IRQ after N
      bytes at the right emulated time, auto-init loops, output samples in
      the mixer match the DMA buffer.
