# 035 — Host audio output (cpal)

- **Status:** todo
- **Milestone:** M3
- **Depends on:** 022, 034
- **Owner:** —

## Goal
Hear the emulator.

## Scope (`dosy`)
- cpal default output device, f32 stereo at device rate; `AudioSink` impl
  over an SPSC ring (`rtrb`), silence on underrun.
- Latency target ~40 ms (config `audio-latency`); pacing nudge from ring
  fill level; `audio = false` disables; `audio-volume`.
- Device loss/reopen handled without crashing.
- Linux: `libasound2-dev` in CI apt step.

## Acceptance criteria
- [ ] PC speaker beeps audibly (e.g. `echo` of BEL) on Windows.
- [ ] Unit tests for ring/pacing logic; builds on all three OSes.
