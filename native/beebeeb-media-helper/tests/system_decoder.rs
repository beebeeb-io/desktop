//! The `System` route on Windows, in-process and unconfined: the helper's loop with a fake app on two in-memory
//! pipes, Windows' own decoders (Media Foundation) behind it, the media fixtures as the files
//! (`tests/fixtures/`, written by `scripts/media-fixtures.sh` at the checkpoint). A missing fixture is a failure.
//!
//! What the sandbox's system-decoder profile allows is the sandbox crate's tests; that the decode works inside it is
//! the VM rung's. The switch is given as on (`run_with_system(…, true)`, contract C6), so these tests keep their
//! meaning whatever the shipped switch `MEDIA_SYSTEM_DECODER_ON` is (on since 2026-11-20).
#![cfg(windows)]

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use beebeeb_media_helper::session::run_with_system;
use beebeeb_media_helper::sniff::{Bare, sniff};
use beebeeb_media_helper::{ROUTES, Route, route};
use beebeeb_media_protocol::{Codec, FromHelper, KindHint, ToHelper, read_to_app, write_msg};

struct ChanRead {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChanRead {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(next) => {
                    self.buf = next;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct ChanWrite(Sender<Vec<u8>>);

impl Write for ChanWrite {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.send(bytes.to_vec()).map_err(|_| io::ErrorKind::BrokenPipe)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The app: answers `ReadAt` from the file, counts the reads past the end (there must be none).
struct App {
    to_helper: Sender<Vec<u8>>,
    from_helper: ChanRead,
    file: Vec<u8>,
    reads_past_end: usize,
    join: Option<JoinHandle<()>>,
}

impl App {
    fn start(file: Vec<u8>) -> App {
        let (to_helper, helper_in) = channel();
        let (helper_out, from_helper) = channel();
        let input = ChanRead {
            rx: helper_in,
            buf: Vec::new(),
            pos: 0,
        };
        let join = std::thread::spawn(move || run_with_system(input, ChanWrite(helper_out), ROUTES, true));
        App {
            to_helper,
            from_helper: ChanRead {
                rx: from_helper,
                buf: Vec::new(),
                pos: 0,
            },
            file,
            reads_past_end: 0,
            join: Some(join),
        }
    }

    fn send(&self, msg: &ToHelper) {
        let mut bytes = Vec::new();
        write_msg(&mut bytes, msg).expect("encodes");
        self.to_helper.send(bytes).expect("the helper is listening");
    }

    fn next(&mut self) -> FromHelper {
        loop {
            match read_to_app(&mut self.from_helper).expect("a message from the helper") {
                FromHelper::ReadAt { id, offset, len } => {
                    let end = offset + u64::from(len);
                    if end <= self.file.len() as u64 {
                        let bytes = self.file[offset as usize..end as usize].to_vec();
                        self.send(&ToHelper::Data { id, bytes });
                    } else {
                        self.reads_past_end += 1;
                        self.send(&ToHelper::DataFailed { id });
                    }
                }
                other => return other,
            }
        }
    }

    fn ask(&mut self, msg: ToHelper) -> FromHelper {
        self.send(&msg);
        self.next()
    }

    fn close(mut self) {
        self.send(&ToHelper::Close);
        if let Some(join) = self.join.take() {
            join.join().expect("the helper's loop ends on Close");
        }
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|_| panic!("fixture {name} is missing: run scripts/media-fixtures.sh"))
}

/// The lossy-audio reference of a fixture: the RMS of its first second as ffmpeg decodes it (`manifest.tsv`).
fn reference_rms(name: &str) -> f64 {
    let manifest = String::from_utf8(fixture("manifest.tsv")).expect("the manifest is text");
    let row = manifest
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|cells| cells.first() == Some(&name))
        .unwrap_or_else(|| panic!("the manifest has a row for {name}"));
    row.last()
        .and_then(|cell| cell.trim().parse().ok())
        .expect("a numeric reference")
}

fn rms(samples: &[f32]) -> f64 {
    (samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / samples.len().max(1) as f64).sqrt()
}

/// AAC is compiled out of the helper until counsel answers, so Windows' decoder plays it: the first second's level
/// matches ffmpeg's decode, a seek lands where it was asked, and the stream ends.
#[test]
fn system_decoder_plays_aac_through_media_foundation() {
    assert_eq!(
        route(Codec::Aac),
        Route::System,
        "this test is about the system route: AAC must not be built in"
    );
    let mut app = App::start(fixture("aac-lc.m4a"));
    let opened = match app.ask(ToHelper::Open {
        len: app.file.len() as u64,
        hint: KindHint::Audio,
    }) {
        FromHelper::Opened(opened) => opened,
        other => panic!("expected Opened, got {other:?}"),
    };
    let audio = opened.audio.expect("an audio stream");
    assert_eq!(audio.codec, Codec::Aac);
    assert!(!opened.reduced_quality);

    // One second at 48 kHz stereo, in tenths (the app's chunk size is its own).
    let mut first_second = Vec::new();
    let mut at = Vec::new();
    for _ in 0..10 {
        match app.ask(ToHelper::WantAudio {
            rate: 48_000,
            channels: 2,
            frames: 4_800,
        }) {
            FromHelper::Audio {
                pts_ms,
                channels,
                samples,
            } => {
                assert_eq!(channels, 2);
                assert!(samples.iter().all(|s| s.is_finite()));
                at.push(pts_ms);
                first_second.extend(samples);
            }
            other => panic!("expected Audio, got {other:?}"),
        }
    }
    assert_eq!(
        at,
        (0..10).map(|i| i * 100).collect::<Vec<u64>>(),
        "the clock advances by what was given"
    );
    assert_eq!(first_second.len(), 48_000 * 2);
    let left: Vec<f32> = first_second.iter().step_by(2).copied().collect();
    let (level, reference) = (rms(&left), reference_rms("aac-lc.m4a"));
    assert!(
        (level - reference).abs() <= reference * 0.10,
        "RMS {level:.6} against ffmpeg's {reference:.6}"
    );
    app.close();
}

/// A seek has no answer of its own: the PCM after it starts where it was asked, and the stream ends.
#[test]
fn system_decoder_seeks_and_ends() {
    let mut app = App::start(fixture("aac-lc.m4a"));
    assert!(matches!(
        app.ask(ToHelper::Open {
            len: app.file.len() as u64,
            hint: KindHint::Audio
        }),
        FromHelper::Opened(_)
    ));
    app.send(&ToHelper::Seek { at_ms: 1_000 });
    match app.ask(ToHelper::WantAudio {
        rate: 48_000,
        channels: 2,
        frames: 4_800,
    }) {
        FromHelper::Audio { pts_ms, .. } => assert_eq!(pts_ms, 1_000),
        other => panic!("expected Audio, got {other:?}"),
    }
    let mut chunks = 0;
    loop {
        match app.ask(ToHelper::WantAudio {
            rate: 48_000,
            channels: 2,
            frames: 48_000,
        }) {
            FromHelper::Audio { .. } => chunks += 1,
            FromHelper::Ended => break,
            other => panic!("expected Audio or Ended, got {other:?}"),
        }
        assert!(chunks < 10, "a 2-second file ends");
    }
    assert_eq!(app.reads_past_end, 0, "the helper never asks beyond the file");
    app.close();
}

/// HEVC plays only through Windows' decoder (Microsoft's extension must be on the PC: it is on the build VM). The
/// frame is exactly the asked size, letterboxed, and frames come in presentation order.
#[test]
fn system_decoder_plays_hevc_frames_at_the_asked_size() {
    assert_eq!(route(Codec::Hevc), Route::System);
    let mut app = App::start(fixture("hevc-main.mp4"));
    let opened = match app.ask(ToHelper::Open {
        len: app.file.len() as u64,
        hint: KindHint::Video,
    }) {
        FromHelper::Opened(opened) => opened,
        FromHelper::Error(code) => panic!("this PC does not play HEVC through Windows ({code:?})"),
        other => panic!("expected Opened, got {other:?}"),
    };
    let video = opened.video.expect("a video stream");
    assert_eq!((video.codec, video.w, video.h), (Codec::Hevc, 320, 180));

    let (w, h) = (640u32, 480u32);
    let mut times = Vec::new();
    for _ in 0..3 {
        match app.ask(ToHelper::WantVideo { w, h }) {
            FromHelper::Frame {
                pts_ms,
                w: fw,
                h: fh,
                rgba,
            } => {
                assert_eq!((fw, fh, rgba.len()), (w, h, (w * h * 4) as usize));
                // 16:9 in 4:3: black bars above and below, the picture in the middle.
                assert_eq!(&rgba[..4], &[0, 0, 0, 255], "the top-left corner is the letterbox");
                let middle = ((h / 2 * w + w / 2) * 4) as usize;
                assert_ne!(&rgba[middle..middle + 3], &[0, 0, 0], "the middle is the picture");
                times.push(pts_ms);
            }
            other => panic!("expected Frame, got {other:?}"),
        }
    }
    assert_eq!(times, vec![0, 40, 80], "25 frames a second, in order");

    app.send(&ToHelper::Seek { at_ms: 1_000 });
    match app.ask(ToHelper::WantVideo { w, h }) {
        FromHelper::Frame { pts_ms, .. } => assert!((1_000..1_040).contains(&pts_ms), "{pts_ms}"),
        other => panic!("expected Frame, got {other:?}"),
    }
    assert_eq!(app.reads_past_end, 0);
    app.close();
}

/// The ADTS AAC elementary stream inside a 188-byte transport stream (`h264-aac.ts`, as ffmpeg muxes it): the
/// payloads of the PID whose packets start a PES of stream id `0xC0`, PES headers dropped.
fn adts_of_ts(ts: &[u8]) -> Vec<u8> {
    let mut audio_pid = None;
    let mut out = Vec::new();
    for packet in ts.chunks_exact(188) {
        assert_eq!(packet[0], 0x47, "a 188-byte transport stream");
        let start = packet[1] & 0x40 != 0;
        let pid = (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2]);
        let control = (packet[3] >> 4) & 0x03;
        let mut at = 4;
        if control & 0x02 != 0 {
            at += 1 + usize::from(packet[4]);
        }
        if control & 0x01 == 0 || at >= packet.len() {
            continue;
        }
        let payload = &packet[at..];
        if start && payload.starts_with(&[0, 0, 1, 0xC0]) {
            audio_pid.get_or_insert(pid);
        }
        if audio_pid != Some(pid) {
            continue;
        }
        if start {
            out.extend(&payload[9 + usize::from(payload[8])..]);
        } else {
            out.extend(payload);
        }
    }
    assert!(!out.is_empty(), "the transport stream has an audio stream");
    out
}

/// Row 14 (MCP2): a bare ADTS AAC stream, with the switch on and Media Foundation running, plays through Windows'
/// decoder (before, it was refused with `NoSystemDecoder`): `Opened` from Windows' own description, sound that is not
/// silence, the end, and never a read past the end.
#[test]
fn system_decoder_plays_a_bare_adts_stream() {
    let adts = adts_of_ts(&fixture("h264-aac.ts"));
    assert_eq!(sniff(&adts[..adts.len().min(64 << 10)]), Some(Bare::Aac));
    let mut app = App::start(adts);
    let opened = match app.ask(ToHelper::Open {
        len: app.file.len() as u64,
        hint: KindHint::Audio,
    }) {
        FromHelper::Opened(opened) => opened,
        FromHelper::Error(code) => panic!("a bare ADTS stream with Media Foundation running: {code:?}"),
        other => panic!("expected Opened, got {other:?}"),
    };
    let audio = opened.audio.expect("an audio stream");
    assert_eq!(audio.codec, Codec::Aac);
    assert!(audio.rate > 0 && audio.channels > 0);
    assert!(opened.video.is_none());
    assert!(!opened.reduced_quality, "Windows decodes HE-AAC in full");

    let mut samples = Vec::new();
    let mut chunks = 0;
    loop {
        match app.ask(ToHelper::WantAudio {
            rate: 48_000,
            channels: 2,
            frames: 48_000,
        }) {
            FromHelper::Audio { samples: more, .. } => {
                assert!(more.iter().all(|s| s.is_finite()));
                samples.extend(more);
                chunks += 1;
            }
            FromHelper::Ended => break,
            other => panic!("expected Audio or Ended, got {other:?}"),
        }
        assert!(chunks < 30, "a short stream ends");
    }
    assert!(rms(&samples) > 0.001, "sound, not silence (RMS {:.6})", rms(&samples));
    assert_eq!(app.reads_past_end, 0, "the helper never asks beyond the stream");
    app.close();
}
