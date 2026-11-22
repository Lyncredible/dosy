# 034 — Audio mixer and PC speaker

- **Status:** todo
- **Milestone:** M3
- **Depends on:** 008, 013
- **Owner:** —

## Goal
Core audio pipeline and the first sound source.

## Scope
- `audio/mixer.rs`: channels with own sample rate, lazy render to `now`,
  resampling (linear or windowed sinc), per-channel volume, soft clip,
  push to `AudioSink` at slice end.
- `hw/speaker.rs`: PIT ch2 output + port 61h bits 0/1 → band-limited square
  wave; direct bit-banging of bit 1 (PWM tricks) also produces output.
- `Machine::set_audio_sink`; `VecSink` for tests.

## Acceptance criteria
- [ ] Tests: 440 Hz PIT tone produces the expected zero-crossing rate;
      silence when gated off; sample count matches emulated time exactly
      over 1 s (no drift); determinism.
