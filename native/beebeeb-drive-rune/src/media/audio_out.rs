//! The sound device and the clock everything else follows: `cpal` (WASAPI in shared mode on Windows, CoreAudio on
//! macOS), f32 at the device's own rate and channels (the helper resamples to them), a ring of at most 2 s, and the
//! clock as the frames the device played. Linux has no `cpal` (it would need ALSA on the test machine): a null sink
//! with the same API and a monotonic clock, for the player's tests.
//!
//! Slice M4 (`media-audio-out`). The ring and the clock ([`Ring`]) are plain code and are tested without a device.

use super::player::{AudioFactory, AudioSink};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(test)]
#[path = "audio_out_tests.rs"]
mod tests;

/// The most the ring holds, in seconds of sound.
const RING_SECONDS: u32 = 2;

/// Why there is no sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// This PC has no output device.
    NoDevice,
    /// The device failed, and failed again on the new default device.
    Failed,
}

/// The queued sound and the clock: frames the device took since the last flush, counted from the pts of the first
/// sample queued after it. Silence the device asked for while the ring was empty is not counted.
struct Ring {
    buf: VecDeque<f32>,
    cap: usize,
    rate: u32,
    channels: u16,
    base_pts_ms: Option<u64>,
    played_frames: u64,
    underruns: u64,
}

impl Ring {
    fn new(rate: u32, channels: u16) -> Ring {
        let channels = channels.max(1);
        let rate = rate.max(1);
        let cap = rate as usize * channels as usize * RING_SECONDS as usize;
        Ring {
            buf: VecDeque::with_capacity(cap),
            cap,
            rate,
            channels,
            base_pts_ms: None,
            played_frames: 0,
            underruns: 0,
        }
    }

    /// Takes whole frames only, as many as fit; the count of samples taken.
    fn push(&mut self, pts_ms: u64, samples: &[f32]) -> usize {
        let ch = self.channels as usize;
        let free = self.cap - self.buf.len();
        let mut take = samples.len().min(free);
        take -= take % ch;
        if take == 0 {
            return 0;
        }
        if self.base_pts_ms.is_none() {
            self.base_pts_ms = Some(pts_ms);
        }
        self.buf.extend(samples[..take].iter().copied());
        take
    }

    /// Fills `out` (the device's buffer): the queued sound, then silence. An underrun is counted when it ran short.
    /// Returns the samples that were real sound.
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let ch = self.channels as usize;
        let mut n = out.len().min(self.buf.len());
        n -= n % ch;
        for (o, s) in out[..n].iter_mut().zip(self.buf.drain(..n)) {
            *o = s;
        }
        for o in out[n..].iter_mut() {
            *o = 0.0;
        }
        if n < out.len() {
            self.underruns += 1;
        }
        self.played_frames += (n / ch) as u64;
        n
    }

    /// Drops `samples` queued samples as if played (the Linux null sink's device).
    #[cfg(not(any(windows, target_os = "macos")))]
    fn consume(&mut self, samples: usize) {
        let ch = self.channels as usize;
        let mut n = samples.min(self.buf.len());
        n -= n % ch;
        self.buf.drain(..n);
        self.played_frames += (n / ch) as u64;
    }

    /// A seek: nothing queued, the clock starts over at the next sample.
    fn flush(&mut self) {
        self.buf.clear();
        self.base_pts_ms = None;
        self.played_frames = 0;
    }

    /// Where playback has got to, in the file's time; `None` before the first sample played.
    fn clock_ms(&self, latency_frames: u64) -> Option<u64> {
        let base = self.base_pts_ms?;
        if self.played_frames == 0 {
            return None;
        }
        let heard = self.played_frames.saturating_sub(latency_frames);
        Some(base + heard * 1000 / self.rate as u64)
    }
}

/// What the stream's callback and the player's thread share.
struct Shared {
    ring: Mutex<Ring>,
    /// Device latency in frames, from the last callback's timestamps.
    latency_frames: AtomicU64,
    /// The error callback saw the device go.
    lost: AtomicBool,
}

impl Shared {
    fn new(rate: u32, channels: u16) -> Arc<Shared> {
        Arc::new(Shared {
            ring: Mutex::new(Ring::new(rate, channels)),
            latency_frames: AtomicU64::new(0),
            lost: AtomicBool::new(false),
        })
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The open output stream.
pub struct AudioOut {
    #[cfg(any(windows, target_os = "macos"))]
    stream: Option<cpal::Stream>,
    shared: Arc<Shared>,
    rate: u32,
    channels: u16,
    paused: bool,
    failed: bool,
    #[cfg(not(any(windows, target_os = "macos")))]
    null: Mutex<NullClock>,
}

/// The Linux null sink's device: a monotonic timer that consumes the ring at the sample rate.
#[cfg(not(any(windows, target_os = "macos")))]
struct NullClock {
    last: std::time::Instant,
    carry_ns: u128,
}

impl AudioOut {
    /// The default output device at its default format. `rate_hint`: the file's rate; the device's own rate wins
    /// and the helper resamples to it, so the hint is not used.
    pub fn start(rate_hint: u32) -> Result<AudioOut, AudioError> {
        let _ = rate_hint;
        #[cfg(any(windows, target_os = "macos"))]
        {
            let (stream, shared, rate, channels) = device::open()?;
            Ok(AudioOut {
                stream: Some(stream),
                shared,
                rate,
                channels,
                paused: false,
                failed: false,
            })
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let (rate, channels) = (48_000, 2);
            Ok(AudioOut {
                shared: Shared::new(rate, channels),
                rate,
                channels,
                paused: false,
                failed: false,
                null: Mutex::new(NullClock {
                    last: std::time::Instant::now(),
                    carry_ns: 0,
                }),
            })
        }
    }

    pub fn format(&self) -> (u32, u16) {
        (self.rate, self.channels)
    }

    /// True once the device was lost and the new default device could not take over (the player then shows
    /// "This file could not be played."; the `AudioSink` trait has no error return, so the player asks this).
    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn push(&mut self, pts_ms: u64, samples: &[f32]) -> usize {
        self.keep_alive();
        if self.failed {
            return 0;
        }
        self.advance_null();
        self.shared.ring().push(pts_ms, samples)
    }

    pub fn clock_ms(&self) -> Option<u64> {
        if self.failed {
            return None;
        }
        self.advance_null();
        self.clock_now()
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn clock_now(&self) -> Option<u64> {
        let latency = self.shared.latency_frames.load(Ordering::Relaxed);
        self.shared.ring().clock_ms(latency)
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn clock_now(&self) -> Option<u64> {
        self.shared.ring().clock_ms(0)
    }

    pub fn pause(&mut self) {
        self.advance_null();
        self.paused = true;
        #[cfg(any(windows, target_os = "macos"))]
        if let Some(stream) = &self.stream {
            use cpal::traits::StreamTrait;
            let _ = stream.pause();
        }
    }

    pub fn resume(&mut self) {
        self.paused = false;
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            self.null.lock().unwrap_or_else(|e| e.into_inner()).last = std::time::Instant::now();
        }
        #[cfg(any(windows, target_os = "macos"))]
        if let Some(stream) = &self.stream {
            use cpal::traits::StreamTrait;
            let _ = stream.play();
        }
    }

    pub fn flush(&mut self) {
        self.advance_null();
        self.shared.ring().flush();
    }

    /// Rebuilds the stream once when the device was lost; a failed rebuild (or a new device with another format,
    /// which the helper would have to resample for) is final.
    fn keep_alive(&mut self) {
        #[cfg(any(windows, target_os = "macos"))]
        if !self.failed && self.shared.lost.swap(false, Ordering::AcqRel) {
            match device::open() {
                Ok((stream, shared, rate, channels)) if (rate, channels) == (self.rate, self.channels) => {
                    use cpal::traits::StreamTrait;
                    if self.paused {
                        let _ = stream.pause();
                    }
                    self.stream = Some(stream);
                    self.shared = shared;
                }
                _ => {
                    self.stream = None;
                    self.failed = true;
                }
            }
        }
    }

    /// The null sink's timer drains the ring at the sample rate; the real device's callback does it elsewhere.
    fn advance_null(&self) {
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let mut null = self.null.lock().unwrap_or_else(|e| e.into_inner());
            let now = std::time::Instant::now();
            if !self.paused {
                let ns = null.carry_ns + now.duration_since(null.last).as_nanos();
                let frames = ns * self.rate as u128 / 1_000_000_000;
                null.carry_ns = ns - frames * 1_000_000_000 / self.rate as u128;
                self.shared.ring().consume(frames as usize * self.channels as usize);
            }
            null.last = now;
        }
    }
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

/// `cpal`: WASAPI in shared mode on Windows, CoreAudio on macOS.
#[cfg(any(windows, target_os = "macos"))]
mod device {
    use super::{AudioError, Ordering, Shared};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::Arc;

    /// The audio host the device is opened on.
    fn host() -> cpal::Host {
        cpal::default_host()
    }

    /// Opens the default output device at its default config, playing (silence until the first push).
    pub(super) fn open() -> Result<(cpal::Stream, Arc<Shared>, u32, u16), AudioError> {
        let device = host().default_output_device().ok_or(AudioError::NoDevice)?;
        let supported = device.default_output_config().map_err(|_| AudioError::NoDevice)?;
        let format = supported.sample_format();
        let config = supported.config();
        let (rate, channels) = (config.sample_rate, config.channels);
        let shared = Shared::new(rate, channels);
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, config, &shared, rate),
            cpal::SampleFormat::I16 => build::<i16>(&device, config, &shared, rate),
            cpal::SampleFormat::U16 => build::<u16>(&device, config, &shared, rate),
            cpal::SampleFormat::I32 => build::<i32>(&device, config, &shared, rate),
            _ => return Err(AudioError::Failed),
        }
        .map_err(|_| AudioError::Failed)?;
        stream.play().map_err(|_| AudioError::Failed)?;
        Ok((stream, shared, rate, channels))
    }

    fn build<T>(
        device: &cpal::Device,
        config: cpal::StreamConfig,
        shared: &Arc<Shared>,
        rate: u32,
    ) -> Result<cpal::Stream, cpal::Error>
    where
        T: cpal::SizedSample,
    {
        let data_shared = Arc::clone(shared);
        let err_shared = Arc::clone(shared);
        // One buffer for the device's own format; sized on the first callbacks, then reused.
        let mut scratch: Vec<f32> = Vec::with_capacity(16_384);
        device.build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                scratch.resize(data.len(), 0.0);
                let ts = info.timestamp();
                let latency = ts.playback.saturating_duration_since(ts.callback);
                data_shared
                    .latency_frames
                    .store((latency.as_secs_f64() * rate as f64) as u64, Ordering::Relaxed);
                // Never wait on the player's thread: a busy ring is silence for this buffer.
                match data_shared.ring.try_lock() {
                    Ok(mut ring) => {
                        ring.fill(&mut scratch);
                    }
                    Err(_) => scratch.iter_mut().for_each(|s| *s = 0.0),
                }
                for (out, s) in data.iter_mut().zip(scratch.iter()) {
                    *out = <T as cpal::Sample>::from_sample(*s);
                }
            },
            move |err: cpal::Error| {
                // The backend rerouted to the new default device by itself: nothing to do.
                if err.kind() != cpal::ErrorKind::DeviceChanged {
                    err_shared.lost.store(true, Ordering::Release);
                }
            },
            None,
        )
    }
}
