//! Audio output interface. The core mixes all sound devices into stereo `f32`
//! frames at the sink's sample rate and pushes them as emulated time advances.

/// One stereo sample, `[left, right]`, nominally in `-1.0..=1.0`.
pub type StereoFrame = [f32; 2];

/// Destination for mixed audio. The app implements this over a lock-free ring
/// buffer feeding the audio device; tests use a `Vec`.
pub trait AudioSink: Send {
    /// Output sample rate in Hz. Must stay constant for the sink's lifetime.
    fn sample_rate(&self) -> u32;
    /// Append frames. Must not block.
    fn push(&mut self, frames: &[StereoFrame]);
}

/// Discards everything. Default sink when audio is disabled.
#[derive(Debug, Default)]
pub struct NullSink;

impl AudioSink for NullSink {
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn push(&mut self, _frames: &[StereoFrame]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_accepts_frames() {
        let mut s = NullSink;
        s.push(&[[0.0, 0.0]; 4]);
        assert_eq!(s.sample_rate(), 48_000);
    }
}
