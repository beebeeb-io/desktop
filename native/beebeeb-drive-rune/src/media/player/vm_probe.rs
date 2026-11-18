//! A measurement, not a gate: the real player against the REAL confined helper and the PC's real sound device, with a
//! clip read from disk, printing where the time from the press to the first frame goes and how the picture keeps up.
//! Run on a Windows PC (VM 401) with the helper built:
//! `BB_PROBE_HELPER=<beebeeb-media-helper.exe> BB_PROBE_CLIPS=<a.mp4;b.mp4> cargo test --release media::player::vm_probe -- --ignored --nocapture`

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_media_protocol::KindHint;
use beebeeb_office_sandbox::Profile;
use zeroize::Zeroizing;

use super::{AudioFactory, Link, LinkFactory, MediaBytes, MediaItem, MediaOpener, PlayerHandle, Press, Refusal};
use crate::media::audio_out::DefaultAudio;
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
        self.0.get(start..end).map(|bytes| Zeroizing::new(bytes.to_vec())).ok_or(())
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
    let seconds: u64 = std::env::var("BB_PROBE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(12);
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
        let name = std::path::Path::new(clip).file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        let player = PlayerHandle::new(
            Arc::new(FileOpener(bytes)),
            Arc::new(Helpers(helper.clone())),
            Arc::new(DefaultAudio),
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
        player.stop(crate::media::StopReason::Closed);
        thread::sleep(Duration::from_millis(500));
    }
}
