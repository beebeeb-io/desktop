//! The sound device and the clock everything else follows: `cpal` (WASAPI in shared mode on Windows, CoreAudio on
//! macOS), f32 at the device's own rate and channels (the helper resamples to them), a ring of at most 2 s, and the
//! clock as the frames the device played. Linux has no `cpal` (it would need ALSA on the test machine): a null sink
//! with the same API and a monotonic clock, for the player's tests.
//!
//! Slice M4 (`media-audio-out`). The ring and the clock ([`Ring`]) are plain code and are tested without a device.

use super::player::{AudioFactory, AudioSink};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use zeroize::{Zeroize, Zeroizing};

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

/// The ring's samples: one buffer of fixed size, written round and round. Samples already played stay in it until
/// they are written over, so the whole of it is zeroed on a flush and when it is dropped (the file's decoded sound
/// never stays behind in freed memory).
struct Samples {
    /// Every slot of the ring; its length is the ring's size.
    data: Vec<f32>,
    /// The slot of the oldest queued sample.
    head: usize,
    /// How many samples are queued.
    len: usize,
}

impl Samples {
    fn with_capacity(cap: usize) -> Samples {
        Samples {
            data: vec![0.0; cap],
            head: 0,
            len: 0,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Queues `samples` after the others; the caller keeps to the free room.
    fn extend_from(&mut self, samples: &[f32]) {
        let cap = self.data.len();
        let mut at = (self.head + self.len) % cap.max(1);
        for sample in samples.iter().take(cap - self.len) {
            self.data[at] = *sample;
            at = (at + 1) % cap;
        }
        self.len += samples.len().min(cap - self.len);
    }

    /// Moves the oldest `out.len()` samples (at most what is queued) into `out`.
    fn take_into(&mut self, out: &mut [f32]) {
        let n = out.len().min(self.len);
        let cap = self.data.len();
        for slot in out[..n].iter_mut() {
            *slot = self.data[self.head];
            self.head = (self.head + 1) % cap;
        }
        self.len -= n;
    }

    /// Drops the oldest `n` samples (at most what is queued).
    #[cfg(not(any(windows, target_os = "macos")))]
    fn skip(&mut self, n: usize) {
        let n = n.min(self.len);
        self.head = (self.head + n) % self.data.len().max(1);
        self.len -= n;
    }

    /// Nothing queued, and every slot zeroed (also the ones already played).
    fn wipe(&mut self) {
        self.data.as_mut_slice().zeroize();
        self.head = 0;
        self.len = 0;
    }
}

impl Drop for Samples {
    fn drop(&mut self) {
        self.data.as_mut_slice().zeroize();
    }
}

/// The queued sound and the clock: frames the device took since the last flush, counted from the pts of the first
/// sample queued after it. Silence the device asked for while the ring was empty is not counted as sound; it does
/// move the sound before it through the device's own buffer ([`Ring::clock_ms`]).
struct Ring {
    buf: Samples,
    cap: usize,
    rate: u32,
    channels: u16,
    base_pts_ms: Option<u64>,
    played_frames: u64,
    /// Frames of sound the device took in a row, the latest of them last (silence breaks the run).
    run_frames: u64,
    /// Frames of silence the device took since the last frame of sound.
    silent_after: u64,
    underruns: u64,
}

impl Ring {
    fn new(rate: u32, channels: u16) -> Ring {
        let channels = channels.max(1);
        let rate = rate.max(1);
        let cap = rate as usize * channels as usize * RING_SECONDS as usize;
        Ring {
            buf: Samples::with_capacity(cap),
            cap,
            rate,
            channels,
            base_pts_ms: None,
            played_frames: 0,
            run_frames: 0,
            silent_after: 0,
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
        self.buf.extend_from(&samples[..take]);
        take
    }

    /// Fills `out` (the device's buffer): the queued sound, then silence. An underrun is counted when it ran short.
    /// Returns the samples that were real sound.
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let ch = self.channels as usize;
        let mut n = out.len().min(self.buf.len());
        n -= n % ch;
        self.buf.take_into(&mut out[..n]);
        for o in out[n..].iter_mut() {
            *o = 0.0;
        }
        if n < out.len() {
            self.underruns += 1;
        }
        let sound = (n / ch) as u64;
        let silence = ((out.len() - n) / ch) as u64;
        if sound > 0 {
            // The sound comes first in the buffer, the silence after it.
            self.run_frames = if self.silent_after == 0 {
                self.run_frames.saturating_add(sound)
            } else {
                sound
            };
            self.silent_after = silence;
        } else {
            self.silent_after = self.silent_after.saturating_add(silence);
        }
        self.played_frames = self.played_frames.saturating_add(sound);
        n
    }

    /// Drops `samples` queued samples as if played (the Linux null sink's device).
    #[cfg(not(any(windows, target_os = "macos")))]
    fn consume(&mut self, samples: usize) {
        let ch = self.channels as usize;
        let mut n = samples.min(self.buf.len());
        n -= n % ch;
        self.buf.skip(n);
        self.played_frames = self.played_frames.saturating_add((n / ch) as u64);
    }

    /// A seek (and a stop): nothing queued, every slot zeroed, the clock starts over at the next sample.
    fn flush(&mut self) {
        self.buf.wipe();
        self.base_pts_ms = None;
        self.played_frames = 0;
        self.run_frames = 0;
        self.silent_after = 0;
    }

    /// Where playback has got to, in the file's time; `None` before the first sample played. The device hears a
    /// frame `latency_frames` after it took it, so the last frames taken are not heard yet; but once the ring ran
    /// empty, the silence the device took after them pushed them out: after `latency_frames` of it, the clock is
    /// at the end of the sound (P2-2 of the #462 review: the player's end comes on a real device too). Saturating
    /// throughout: a timestamp near the end of `u64` (a helper's answer) can neither wrap nor panic here.
    fn clock_ms(&self, latency_frames: u64) -> Option<u64> {
        let base = self.base_pts_ms?;
        if self.played_frames == 0 {
            return None;
        }
        let unheard = latency_frames.saturating_sub(self.silent_after).min(self.run_frames);
        let heard = self.played_frames.saturating_sub(unheard);
        Some(base.saturating_add(heard.saturating_mul(1000) / u64::from(self.rate)))
    }
}

/// What the stream's callback and the player's thread share.
struct Shared {
    ring: Mutex<Ring>,
    /// Device latency in frames, from the last callback's timestamps.
    latency_frames: AtomicU64,
    /// The error callback saw the device go.
    lost: AtomicBool,
    /// The volume (`f32` bits, 0.0 to 1.0), applied as each device buffer is filled.
    gain: AtomicU32,
}

impl Shared {
    fn new(rate: u32, channels: u16) -> Arc<Shared> {
        Arc::new(Shared {
            ring: Mutex::new(Ring::new(rate, channels)),
            latency_frames: AtomicU64::new(0),
            lost: AtomicBool::new(false),
            gain: AtomicU32::new(1.0f32.to_bits()),
        })
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
}

/// A device sample format the callback writes. On Windows and macOS every cpal format qualifies; on Linux (no cpal) the
/// tests give `f32` and `i16`.
trait DeviceSample: Copy {
    fn from_f32(s: f32) -> Self;
}

#[cfg(any(windows, target_os = "macos"))]
impl<T: cpal::Sample + cpal::FromSample<f32>> DeviceSample for T {
    fn from_f32(s: f32) -> T {
        <T as cpal::Sample>::from_sample(s)
    }
}

/// The per-buffer step of the output callback: the ring's sound in the device's format, silence when the ring is empty
/// or busy (never wait on the player's thread). `scratch` is reused between calls; zeroed before it grows (the old
/// buffer is freed) and, as a `Zeroizing`, when the stream drops it.
#[cfg(any(windows, target_os = "macos", test))]
fn fill_device<T: DeviceSample>(shared: &Shared, scratch: &mut Zeroizing<Vec<f32>>, data: &mut [T]) {
    if data.len() > scratch.capacity() {
        scratch.zeroize();
    }
    scratch.resize(data.len(), 0.0);
    match shared.ring.try_lock() {
        Ok(mut ring) => {
            ring.fill(scratch);
        }
        Err(_) => scratch.iter_mut().for_each(|s| *s = 0.0),
    }
    // The volume, here where the samples leave: a change is heard from the next buffer, not after the queue.
    let gain = shared.gain();
    if gain != 1.0 {
        scratch.iter_mut().for_each(|s| *s *= gain);
    }
    for (out, s) in data.iter_mut().zip(scratch.iter()) {
        *out = T::from_f32(*s);
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

    /// The gain the device buffers are filled at, clamped to 0.0 to 1.0 (NaN: silence).
    pub fn set_gain(&mut self, gain: f32) {
        let gain = if gain.is_nan() { 0.0 } else { gain.clamp(0.0, 1.0) };
        self.shared.set_gain(gain);
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
                    shared.set_gain(self.shared.gain());
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

    fn failed(&self) -> bool {
        AudioOut::failed(self)
    }

    fn set_gain(&mut self, gain: f32) {
        AudioOut::set_gain(self, gain)
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
    use super::{AudioError, Ordering, Shared, Zeroizing, fill_device};
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
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        let data_shared = Arc::clone(shared);
        let err_shared = Arc::clone(shared);
        // One buffer for the device's own format; sized on the first callbacks, then reused; zeroed with the stream.
        let mut scratch: Zeroizing<Vec<f32>> = Zeroizing::new(Vec::with_capacity(16_384));
        device.build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                let ts = info.timestamp();
                let latency = ts.playback.saturating_duration_since(ts.callback);
                data_shared
                    .latency_frames
                    .store((latency.as_secs_f64() * rate as f64) as u64, Ordering::Relaxed);
                fill_device(&data_shared, &mut scratch, data);
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
