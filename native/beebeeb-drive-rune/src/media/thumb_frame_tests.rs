//! The thumbnail frame over a scripted helper: the first frame comes back, and every refusal is no frame.

use std::io::Write;
use std::sync::{Arc, Mutex};

use beebeeb_media_protocol::{Codec, ErrorCode, Opened, VideoInfo};

use super::*;
use crate::media::player::Refusal;

#[derive(Default)]
struct Seen {
    sent: Vec<String>,
    data: Vec<(u32, Vec<u8>)>,
    failed: Vec<u32>,
    killed: usize,
}

struct Scripted {
    answers: std::collections::VecDeque<FromHelper>,
    seen: Arc<Mutex<Seen>>,
    confined: bool,
}

impl Link for Scripted {
    fn send(&mut self, msg: &ToHelper) -> std::io::Result<()> {
        let mut seen = self.seen.lock().unwrap();
        seen.sent.push(format!("{msg:?}"));
        match msg {
            ToHelper::Data { id, bytes } => seen.data.push((*id, bytes.clone())),
            ToHelper::DataFailed { id } => seen.failed.push(*id),
            _ => {}
        }
        Ok(())
    }
    fn recv(&mut self, _timeout: Duration) -> Result<FromHelper, LinkError> {
        self.answers.pop_front().ok_or(LinkError::Gone)
    }
    fn kill(&mut self) {
        self.seen.lock().unwrap().killed += 1;
    }
    fn confined(&self) -> bool {
        self.confined
    }
}

struct Factory {
    answers: Vec<FromHelper>,
    seen: Arc<Mutex<Seen>>,
    confined: bool,
    refuse: bool,
}

impl LinkFactory for Factory {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        if self.refuse {
            return Err(Refusal::NoSandbox);
        }
        Ok(Box::new(Scripted {
            answers: self.answers.clone().into(),
            seen: Arc::clone(&self.seen),
            confined: self.confined,
        }))
    }
}

fn opened(video: bool) -> FromHelper {
    FromHelper::Opened(Opened {
        duration_ms: 1000,
        video: video.then_some(VideoInfo {
            w: 4000,
            h: 2000,
            fps_milli: 25_000,
            codec: Codec::H264,
        }),
        audio: None,
        reduced_quality: false,
        no_sound: None,
    })
}

fn video_file() -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(&[7u8; 100]).unwrap();
    f
}

fn run(answers: Vec<FromHelper>, confined: bool, refuse: bool) -> (Result<Frame, Refused>, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let source = HelperFrames {
        factory: Arc::new(Factory {
            answers,
            seen: Arc::clone(&seen),
            confined,
            refuse,
        }),
        require_confined: true,
    };
    let file = video_file();
    (source.first_frame(file.path(), FrameRequest::THUMBNAIL), seen)
}

fn frame(w: u32, h: u32) -> FromHelper {
    FromHelper::Frame {
        pts_ms: 0,
        w,
        h,
        rgba: vec![1; (w * h * 4) as usize],
    }
}

#[test]
fn the_first_frame_comes_back_at_the_asked_size_and_the_helper_is_ended() {
    let (got, seen) = run(
        vec![
            FromHelper::ReadAt {
                id: 1,
                offset: 10,
                len: 20,
            },
            FromHelper::ReadAt {
                id: 2,
                offset: 90,
                len: 20,
            },
            opened(true),
            frame(1024, 512),
        ],
        true,
        false,
    );
    let got = got.expect("a frame");
    assert_eq!((got.width, got.height, got.rgba.len()), (1024, 512, 1024 * 512 * 4));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.data, vec![(1, vec![7u8; 20])], "an in-file range is served");
    assert_eq!(seen.failed, vec![2], "a range past the end of the file is refused");
    assert!(seen.sent.iter().any(|m| m == "WantVideo(1024x512)"), "{:?}", seen.sent);
    assert_eq!(seen.killed, 1);
}

#[test]
fn every_refusal_is_no_frame() {
    let cases: Vec<(&str, Vec<FromHelper>, bool, bool, Refused)> = vec![
        ("no helper to start", vec![], true, true, Refused::NoHelper),
        (
            "a helper that is not confined",
            vec![opened(true), frame(1024, 512)],
            false,
            false,
            Refused::NoHelper,
        ),
        (
            "an error",
            vec![FromHelper::Error(ErrorCode::NoDecoder)],
            true,
            false,
            Refused::NoFrame,
        ),
        ("no video track", vec![opened(false)], true, false, Refused::NoFrame),
        ("the helper is gone", vec![opened(true)], true, false, Refused::NoFrame),
        (
            "a frame of another size",
            vec![opened(true), frame(100, 50)],
            true,
            false,
            Refused::BadFrame,
        ),
        (
            "the stream ended",
            vec![opened(true), FromHelper::Ended],
            true,
            false,
            Refused::NoFrame,
        ),
        (
            "a frame nobody asked for",
            vec![frame(1024, 512)],
            true,
            false,
            Refused::NoFrame,
        ),
    ];
    for (what, answers, confined, refuse, want) in cases {
        let (got, seen) = run(answers, confined, refuse);
        assert_eq!(got.err(), Some(want), "{what}");
        if !refuse {
            assert_eq!(seen.lock().unwrap().killed, 1, "{what}: the helper is ended");
        }
    }
}

#[test]
fn the_asked_size_fits_the_request_and_never_collapses() {
    let ask = FrameRequest::THUMBNAIL;
    assert_eq!(asked_size(320, 240, ask), (320, 240));
    assert_eq!(asked_size(4000, 2000, ask), (1024, 512));
    assert_eq!(asked_size(2000, 4000, ask), (512, 1024));
    assert_eq!(asked_size(16_384, 1, ask), (1024, 1));
    assert_eq!(asked_size(0, 0, ask), (1, 1));
}
