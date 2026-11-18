//! A measurement, not a gate: the real player against the REAL confined helper and the PC's real sound device, with a
//! clip read from disk, printing where the time from the press to the first frame goes and how the picture keeps up.
//! `BB_PROBE_SOUND=timed` puts a device that only keeps time in the place of the PC's own (VM 401 has none): it takes
//! PCM at the rate a device would, so the file's audio track is decoded by the confined helper and its clock paces
//! the frames; the probe prints how much sound it took and its level. Sound proven by decode and clock, not by ear.
//! Run on a Windows PC (VM 401) with the helper built:
//! `BB_PROBE_HELPER=<beebeeb-media-helper.exe> BB_PROBE_CLIPS=<a.mp4;b.mp4> cargo test --release media::player::vm_probe -- --ignored --nocapture`

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_media_protocol::KindHint;
use beebeeb_office_sandbox::Profile;
use zeroize::Zeroizing;

use super::{
    AudioFactory, AudioSink, Link, LinkFactory, MediaBytes, MediaItem, MediaOpener, PlayerHandle, Press, Refusal,
};
use crate::media::audio_out::{AudioError, DefaultAudio};
use crate::media::transport::HelperLink;
use crate::media::video::VideoSurface;

struct FileOpener(Arc<Vec<u8>>);

impl MediaOpener for FileOpener {
    fn open(&self, _file_id: &str) -> Result<Box<dyn MediaBytes>, Refusal> {
        Ok(Box::new(FileBytes(Arc::clone(&self.0))))
    }
}

struct FileBytes(Arc<Vec<u8>>);

impl MediaBytes for FileBytes {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&mut self, offset: u64, len: u32, _cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()> {
        let start = usize::try_from(offset).map_err(|_| ())?;
        let end = start.checked_add(len as usize).ok_or(())?;
        self.0
            .get(start..end)
            .map(|bytes| Zeroizing::new(bytes.to_vec()))
            .ok_or(())
    }
}

struct Helpers(PathBuf);

impl LinkFactory for Helpers {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        self.start_in(Profile::default())
    }

    fn start_in(&self, profile: Profile) -> Result<Box<dyn Link>, Refusal> {
        Ok(Box::new(HelperLink::start_in(&self.0, profile)?))
    }
}

/// What the timed device took: samples, the sum of their squares, and its clock when last asked.
#[derive(Default)]
struct Taken {
    samples: u64,
    sum_sq: f64,
    clock_ms: u64,
}

/// A device that only keeps time: 48 kHz stereo, at most half a second queued, played at the wall clock's pace from
/// the first sample.
struct Timed {
    taken: Arc<Mutex<Taken>>,
    /// The first sample's time in the file, and when it was pushed.
    from: Option<(u64, Instant)>,
    pushed_frames: u64,
}

const TIMED_RATE: u32 = 48_000;
const TIMED_CHANNELS: u16 = 2;

impl Timed {
    /// Frames played by now: the wall clock's, never more than were pushed.
    fn played(&self) -> u64 {
        let Some((_, since)) = self.from else { return 0 };
        let due = since.elapsed().as_millis() as u64 * u64::from(TIMED_RATE) / 1000;
        due.min(self.pushed_frames)
    }
}

impl AudioSink for Timed {
    fn format(&self) -> (u32, u16) {
        (TIMED_RATE, TIMED_CHANNELS)
    }

    fn push(&mut self, pts_ms: u64, samples: &[f32]) -> usize {
        self.from.get_or_insert((pts_ms, Instant::now()));
        let room = (self.played() + u64::from(TIMED_RATE) / 2).saturating_sub(self.pushed_frames);
        let frames = (samples.len() / usize::from(TIMED_CHANNELS)).min(room as usize);
        let take = frames * usize::from(TIMED_CHANNELS);
        self.pushed_frames += frames as u64;
        let mut taken = self.taken.lock().unwrap();
        taken.samples += take as u64;
        taken.sum_sq += samples[..take]
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>();
        take
    }

    fn clock_ms(&self) -> Option<u64> {
        let (base, _) = self.from?;
        let clock = base + self.played() * 1000 / u64::from(TIMED_RATE);
        self.taken.lock().unwrap().clock_ms = clock;
        Some(clock)
    }

    fn pause(&mut self) {}

    fn resume(&mut self) {}

    fn flush(&mut self) {
        self.from = None;
        self.pushed_frames = 0;
    }
}

struct TimedDevices(Arc<Mutex<Taken>>);

impl AudioFactory for TimedDevices {
    fn start(&self, _rate_hint: u32) -> Result<Box<dyn AudioSink>, AudioError> {
        Ok(Box::new(Timed {
            taken: Arc::clone(&self.0),
            from: None,
            pushed_frames: 0,
        }))
    }
}

#[test]
#[ignore = "a measurement on a Windows PC: needs BB_PROBE_HELPER and BB_PROBE_CLIPS"]
fn where_the_time_to_the_first_frame_goes() {
    let helper = PathBuf::from(std::env::var("BB_PROBE_HELPER").expect("BB_PROBE_HELPER"));
    let clips = std::env::var("BB_PROBE_CLIPS").expect("BB_PROBE_CLIPS");
    let size: Vec<u32> = std::env::var("BB_PROBE_SIZE")
        .unwrap_or_else(|_| "1036x582".into())
        .split('x')
        .map(|n| n.parse().expect("a size like 1036x582"))
        .collect();
    let seconds: u64 = std::env::var("BB_PROBE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);
    for round in 0..3 {
        let started = Instant::now();
        let sound = DefaultAudio.start(48_000);
        println!(
            "PROBE sound-device open {round}: {} ms, {}",
            started.elapsed().as_millis(),
            match &sound {
                Ok(sink) => format!("a device {:?}", sink.format()),
                Err(e) => format!("{e:?}"),
            }
        );
    }
    for clip in clips.split(';').filter(|c| !c.is_empty()) {
        let bytes = Arc::new(std::fs::read(clip).expect("the clip reads"));
        let name = std::path::Path::new(clip)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let taken = Arc::new(Mutex::new(Taken::default()));
        let timed = std::env::var("BB_PROBE_SOUND").is_ok_and(|sound| sound == "timed");
        let audio: Arc<dyn AudioFactory> = if timed {
            Arc::new(TimedDevices(Arc::clone(&taken)))
        } else {
            Arc::new(DefaultAudio)
        };
        let player = PlayerHandle::new(
            Arc::new(FileOpener(bytes)),
            Arc::new(Helpers(helper.clone())),
            audio,
            None,
            VideoSurface::new(),
        );
        player.set_view_size(size[0], size[1]);
        let item = MediaItem {
            file_id: "probe".into(),
            kind: KindHint::Video,
            length_ms: 0,
            press: Press {
                session: SessionGeneration::for_test(1),
                account: None,
            },
        };
        let started = Instant::now();
        player.open(item, Vec::new()).expect("the open is taken");
        let mut steps_said = false;
        let mut rows = 0;
        while started.elapsed() < Duration::from_secs(seconds) {
            thread::sleep(Duration::from_millis(50));
            let (steps, pace) = player.with(|p| (p.record.steps.clone(), p.record.pace.clone()));
            if !steps_said && !steps.is_empty() {
                steps_said = true;
                println!("PROBE {name} steps (ms since the press): {steps:?}");
            }
            for row in &pace[rows..] {
                println!(
                    "PROBE {name} t={} received={} shown={} dropped={} clock={} shown_pts={} at_ms={}",
                    row[0], row[1], row[2], row[3], row[4], row[5], row[6]
                );
            }
            rows = pace.len();
        }
        let state = player.state();
        println!(
            "PROBE {name} end: phase={:?} frames shown={} state={state:?}",
            player.phase(),
            player.with(|p| p.record.shown.len())
        );
        if timed {
            let taken = taken.lock().unwrap();
            let rms = if taken.samples == 0 {
                0.0
            } else {
                (taken.sum_sq / taken.samples as f64).sqrt()
            };
            println!(
                "PROBE {name} sound (timed device): {} samples = {} ms taken, rms {rms:.4}, its clock {} ms",
                taken.samples,
                taken.samples * 1000 / u64::from(TIMED_RATE) / u64::from(TIMED_CHANNELS),
                taken.clock_ms
            );
        }
        player.stop(crate::media::StopReason::Closed);
        thread::sleep(Duration::from_millis(500));
    }
}
