//! The sound device and the clock everything else follows: `cpal` (WASAPI in shared mode on Windows, CoreAudio on
//! macOS), f32 at the device's own rate and channels (the helper resamples to them), a ring of at most 2 s, and the
//! clock as the frames the device played. Linux has no `cpal` (it would need ALSA on the test machine): a null sink
//! with the same API and a monotonic clock, for the player's tests.
//!
//! Slice M4 (`media-audio-out`) fills it. The scaffold opens no device: [`AudioOut::start`] answers
//! [`AudioError::NoDevice`].

use super::player::{AudioFactory, AudioSink};

/// Why there is no sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// This PC has no output device.
    NoDevice,
    /// The device failed, and failed again on the new default device.
    Failed,
}

/// The open output stream.
pub struct AudioOut {
    #[cfg(any(windows, target_os = "macos"))]
    stream: Option<cpal::Stream>,
    rate: u32,
    channels: u16,
}

impl AudioOut {
    /// The default output device at its default format. `rate_hint`: the file's rate, when the device offers it.
    pub fn start(rate_hint: u32) -> Result<AudioOut, AudioError> {
        let _ = rate_hint;
        Err(AudioError::NoDevice)
    }

    pub fn format(&self) -> (u32, u16) {
        (self.rate, self.channels)
    }

    pub fn push(&mut self, pts_ms: u64, samples: &[f32]) -> usize {
        let _ = (pts_ms, samples);
        0
    }

    pub fn clock_ms(&self) -> Option<u64> {
        None
    }

    pub fn pause(&mut self) {}

    pub fn resume(&mut self) {}

    pub fn flush(&mut self) {}
}

impl AudioSink for AudioOut {
    fn format(&self) -> (u32, u16) {
        AudioOut::format(self)
    }

    fn push(&mut self, pts_ms: u64, samples: &[f32]) -> usize {
        AudioOut::push(self, pts_ms, samples)
    }

    fn clock_ms(&self) -> Option<u64> {
        AudioOut::clock_ms(self)
    }

    fn pause(&mut self) {
        AudioOut::pause(self)
    }

    fn resume(&mut self) {
        AudioOut::resume(self)
    }

    fn flush(&mut self) {
        AudioOut::flush(self)
    }
}

/// The real device: [`AudioOut::start`]. The player's tests hand their own factory.
pub struct DefaultAudio;

impl AudioFactory for DefaultAudio {
    fn start(&self, rate_hint: u32) -> Result<Box<dyn AudioSink>, AudioError> {
        Ok(Box::new(AudioOut::start(rate_hint)?))
    }
}

/// The audio host M4 opens the device on (named here so the scaffold's one check proves `cpal` builds).
#[cfg(any(windows, target_os = "macos"))]
fn host() -> cpal::Host {
    cpal::default_host()
}
