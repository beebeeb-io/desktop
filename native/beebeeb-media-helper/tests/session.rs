//! The helper's loop in-process: a fake app on two in-memory pipes, no sandbox. The app is the test: it answers
//! `ReadAt` from a byte vector and records every ask.

use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use beebeeb_media_helper::session::run_with_system;
use beebeeb_media_helper::{ROUTES, Route};
use beebeeb_media_protocol::{Codec, ErrorCode, FromHelper, KindHint, ToHelper, WireError, read_to_app, write_msg};

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

struct App {
    to_helper: Sender<Vec<u8>>,
    from_helper: ChanRead,
    file: Vec<u8>,
    reads: Vec<(u64, u32)>,
    join: Option<JoinHandle<()>>,
}

impl App {
    fn start(file: Vec<u8>, table: &'static [(Codec, Route)]) -> App {
        // The switch held off: this helper's own decoders alone, the same on every OS (the shipped switch is on,
        // ux-02; Windows' decoders are `tests/system_decoder.rs`).
        App::start_with(file, table, false)
    }

    fn start_with(file: Vec<u8>, table: &'static [(Codec, Route)], system_on: bool) -> App {
        let (to_helper, helper_in) = channel();
        let (helper_out, from_helper) = channel();
        let input = ChanRead {
            rx: helper_in,
            buf: Vec::new(),
            pos: 0,
        };
        let join = std::thread::spawn(move || run_with_system(input, ChanWrite(helper_out), table, system_on));
        App {
            to_helper,
            from_helper: ChanRead {
                rx: from_helper,
                buf: Vec::new(),
                pos: 0,
            },
            file,
            reads: Vec::new(),
            join: Some(join),
        }
    }

    fn send(&self, msg: &ToHelper) {
        let mut bytes = Vec::new();
        write_msg(&mut bytes, msg).expect("encodes");
        self.to_helper.send(bytes).expect("helper is listening");
    }

    /// The helper's next answer, serving its `ReadAt`s from the file on the way.
    fn next(&mut self) -> FromHelper {
        loop {
            match read_to_app(&mut self.from_helper).expect("a message from the helper") {
                FromHelper::ReadAt { id, offset, len } => {
                    self.reads.push((offset, len));
                    let end = offset + u64::from(len);
                    if end <= self.file.len() as u64 {
                        let bytes = self.file[offset as usize..end as usize].to_vec();
                        self.send(&ToHelper::Data { id, bytes });
                    } else {
                        self.send(&ToHelper::DataFailed { id });
                    }
                }
                other => return other,
            }
        }
    }

    fn open(&mut self, hint: KindHint) -> FromHelper {
        self.send(&ToHelper::Open {
            len: self.file.len() as u64,
            hint,
        });
        self.next()
    }
}

/// 16-bit mono PCM WAV, one second of a 440 Hz sine at 8000 Hz.
fn wav_one_second() -> Vec<u8> {
    let rate = 8000u32;
    let samples: Vec<i16> = (0..rate)
        .map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 12_000.0) as i16)
        .collect();
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// 128x72 4:2:0 YUV4MPEG2, 5 frames at 25 fps, mid-grey.
fn y4m_five_frames() -> Vec<u8> {
    let (w, h) = (128usize, 72usize);
    let mut out = format!("YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C420jpeg\n").into_bytes();
    for n in 0..5u8 {
        out.extend_from_slice(b"FRAME\n");
        out.extend(std::iter::repeat_n(100 + n, w * h));
        out.extend(std::iter::repeat_n(128u8, w * h / 2));
    }
    out
}

#[test]
fn helper_answers_opened_for_a_wav() {
    let mut app = App::start(wav_one_second(), ROUTES);
    let FromHelper::Opened(opened) = app.open(KindHint::Audio) else {
        panic!("expected Opened");
    };
    let audio = opened.audio.expect("a WAV has audio");
    assert_eq!(audio.codec, Codec::Pcm);
    assert!(opened.video.is_none());
    assert_eq!(opened.duration_ms, 1000);
    app.send(&ToHelper::Close);
}

#[test]
fn helper_reads_only_through_read_at() {
    let file = wav_one_second();
    let len = file.len() as u64;
    let mut app = App::start(file, ROUTES);
    assert!(matches!(app.open(KindHint::Audio), FromHelper::Opened(_)));
    app.send(&ToHelper::WantAudio {
        rate: 8000,
        channels: 1,
        frames: 4000,
    });
    assert!(matches!(app.next(), FromHelper::Audio { .. }));
    assert!(
        !app.reads.is_empty(),
        "the helper read nothing: it has no other way to the bytes"
    );
    for (offset, n) in &app.reads {
        assert!(*n > 0 && *n <= beebeeb_media_protocol::MAX_READ, "{n} bytes");
        assert!(offset + u64::from(*n) <= len, "asked {n} bytes at {offset} of {len}");
    }
    app.send(&ToHelper::Close);
}

/// 320x240 4:2:0 YUV4MPEG2, `frames` frames at 25 fps, frame `n` all luma `40 + 40 * n`. A frame is 115 200 bytes,
/// more than the helper reads ahead (64 KiB), so a frame asked for after `Opened` is read from the app then.
fn y4m_large(frames: u8) -> Vec<u8> {
    let (w, h) = (320usize, 240usize);
    let mut out = format!("YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C420jpeg\n").into_bytes();
    for n in 0..frames {
        out.extend_from_slice(b"FRAME\n");
        out.extend(std::iter::repeat_n(40 + 40 * n, w * h));
        out.extend(std::iter::repeat_n(128u8, w * h / 2));
    }
    out
}

/// P1-1 of the #462 review, as the player sends it: `WantVideo` and `WantAudio` in the same tick, then a seek while
/// the helper waits for the frame's bytes. The read is answered, then each request in the order it came: the frame
/// from before the seek, `Ended` for the sound (this file has none), and the next frame from where the seek went.
#[test]
fn two_requests_and_a_seek_while_the_helper_reads_are_answered_in_order() {
    let mut app = App::start(y4m_large(4), ROUTES);
    assert!(matches!(app.open(KindHint::Video), FromHelper::Opened(_)));
    app.send(&ToHelper::WantVideo { w: 32, h: 24 });
    app.send(&ToHelper::WantAudio {
        rate: 48_000,
        channels: 2,
        frames: 4_800,
    });
    // The first frame is not in the helper's cache: it asks for its bytes before it answers anything.
    let FromHelper::ReadAt { id, offset, len } = read_to_app(&mut app.from_helper).expect("a message") else {
        panic!("the helper reads the frame's bytes first");
    };
    app.send(&ToHelper::Seek { at_ms: 80 });
    let end = (offset + u64::from(len)) as usize;
    let bytes = app.file[offset as usize..end].to_vec();
    app.send(&ToHelper::Data { id, bytes });
    let FromHelper::Frame { pts_ms, w, h, .. } = app.next() else {
        panic!("the frame asked before the seek");
    };
    assert_eq!((pts_ms, w, h), (0, 32, 24));
    assert_eq!(app.next(), FromHelper::Ended, "no sound in this file");
    let reads_before = app.reads.len();
    app.send(&ToHelper::WantVideo { w: 32, h: 24 });
    let FromHelper::Frame { pts_ms, .. } = app.next() else {
        panic!("the frame after the seek");
    };
    assert_eq!(pts_ms, 80, "the seek was taken after the frame it came behind");
    assert!(
        app.reads.len() > reads_before,
        "the third frame was read from the app too"
    );
    app.send(&ToHelper::Close);
}

#[test]
fn helper_frame_is_exactly_the_asked_size() {
    let mut app = App::start(y4m_five_frames(), ROUTES);
    let FromHelper::Opened(opened) = app.open(KindHint::Video) else {
        panic!("expected Opened");
    };
    assert_eq!(opened.video.expect("video").w, 128);
    app.send(&ToHelper::WantVideo { w: 320, h: 180 });
    let FromHelper::Frame { w, h, rgba, .. } = app.next() else {
        panic!("expected Frame");
    };
    assert_eq!((w, h), (320, 180));
    assert_eq!(rgba.len(), 320 * 180 * 4);
    app.send(&ToHelper::Close);
}

#[test]
fn helper_refuses_a_refused_route() {
    const NO_PCM: &[(Codec, Route)] = &[(Codec::Pcm, Route::Unavailable)];
    let mut app = App::start(wav_one_second(), NO_PCM);
    assert_eq!(
        app.open(KindHint::Audio),
        FromHelper::Error(ErrorCode::Unsupported(Codec::Pcm))
    );
    app.send(&ToHelper::Close);
}

#[test]
fn helper_never_answers_a_system_route_with_its_own_decoder() {
    const PCM_ON_SYSTEM: &[(Codec, Route)] = &[(Codec::Pcm, Route::System)];
    let mut app = App::start(wav_one_second(), PCM_ON_SYSTEM);
    assert_eq!(
        app.open(KindHint::Audio),
        FromHelper::Error(ErrorCode::NoSystemDecoder(Codec::Pcm))
    );
    app.send(&ToHelper::Close);
}

#[test]
fn helper_ends_on_eof() {
    let mut app = App::start(wav_one_second(), ROUTES);
    assert!(matches!(app.open(KindHint::Audio), FromHelper::Opened(_)));
    let mut frames = 0usize;
    let mut ended = false;
    for _ in 0..6 {
        app.send(&ToHelper::WantAudio {
            rate: 8000,
            channels: 1,
            frames: 4000,
        });
        match app.next() {
            FromHelper::Audio { samples, channels, .. } => frames += samples.len() / usize::from(channels),
            FromHelper::Ended => {
                ended = true;
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(ended, "no Ended after six asks of 4000 frames in a 8000 frame file");
    assert_eq!(frames, 8000);
    app.send(&ToHelper::Close);
}

#[test]
fn helper_closes_on_close() {
    let mut app = App::start(wav_one_second(), ROUTES);
    assert!(matches!(app.open(KindHint::Audio), FromHelper::Opened(_)));
    app.send(&ToHelper::Close);
    assert_eq!(read_to_app(&mut app.from_helper).unwrap_err(), WireError::Closed);
    app.join.take().expect("running").join().expect("the helper returned");
}

/// An AC-3 or E-AC-3 elementary stream: its bit stream id says which, and FilmCraft has no opener for either. The
/// answer comes from the route table before FilmCraft reads it: AC-3 compiled out is `Unsupported(Ac3)`; E-AC-3, on
/// the system's route with the switch off, is `NoSystemDecoder(Eac3)`.
fn bare_ac3(bsid: u8) -> Vec<u8> {
    let mut file = vec![0x0B, 0x77, 0x00, 0x00, 0x14, bsid << 3];
    file.resize(4096, 0);
    file
}

#[test]
fn helper_answers_a_bare_ac3_stream_from_the_route_table() {
    const POOL_OFF: &[(Codec, Route)] = &[(Codec::Ac3, Route::Unavailable), (Codec::Eac3, Route::System)];
    let mut app = App::start(bare_ac3(8), POOL_OFF);
    assert_eq!(
        app.open(KindHint::Audio),
        FromHelper::Error(ErrorCode::Unsupported(Codec::Ac3))
    );
    app.send(&ToHelper::Close);

    let mut app = App::start(bare_ac3(16), POOL_OFF);
    assert_eq!(
        app.open(KindHint::Audio),
        FromHelper::Error(ErrorCode::NoSystemDecoder(Codec::Eac3))
    );
    app.send(&ToHelper::Close);
}

/// A bare stream whose codec is FilmCraft's goes on to FilmCraft (with the made-up name): it is not answered from the
/// table. AC-3 with its decoder built in (a test table) has no FilmCraft opener, so FilmCraft's own refusal comes back.
#[test]
fn helper_hands_a_bare_stream_of_ours_to_filmcraft() {
    const AC3_OURS: &[(Codec, Route)] = &[(Codec::Ac3, Route::Rust)];
    let mut app = App::start(bare_ac3(8), AC3_OURS);
    assert_eq!(app.open(KindHint::Audio), FromHelper::Error(ErrorCode::NoDecoder));
    app.send(&ToHelper::Close);
}

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|_| panic!("fixture {name} is missing"))
}

/// `mp3.mp3` with ffmpeg's ID3v2 tag (an encoder line) replaced by one with a title and an artist.
fn tagged_mp3() -> Vec<u8> {
    let file = fixture("mp3.mp3");
    assert_eq!(&file[..3], b"ID3", "the fixture starts with ffmpeg's tag");
    let size = file[6..10].iter().fold(0usize, |size, b| (size << 7) | usize::from(*b));
    let mut body = Vec::new();
    for (id, text) in [(b"TIT2", "Harbour sessions"), (b"TPE1", "Ana Alvares")] {
        body.extend(id);
        body.extend(u32::try_from(text.len() + 1).expect("short").to_be_bytes());
        body.extend([0, 0, 0]);
        body.extend(text.as_bytes());
    }
    let mut out = vec![b'I', b'D', b'3', 3, 0, 0];
    out.extend([21, 14, 7, 0].map(|shift| ((body.len() >> shift) & 0x7F) as u8));
    out.extend(body);
    out.extend(&file[10 + size..]);
    out
}

/// D-MB2-1 (a): an audio file's title and artist come right after `Opened`, read through the app's ranges and never
/// past the end; a file without them, and a video, send none (the next answer is the asked PCM or frame).
#[test]
fn helper_sends_an_audio_files_title_and_artist_after_opened() {
    let mut app = App::start(tagged_mp3(), ROUTES);
    let FromHelper::Opened(opened) = app.open(KindHint::Audio) else {
        panic!("expected Opened");
    };
    assert_eq!(opened.audio.map(|a| a.codec), Some(Codec::Mp3));
    match app.next() {
        FromHelper::Tags(tags) => {
            assert_eq!(tags.title, "Harbour sessions");
            assert_eq!(tags.artist, "Ana Alvares");
            assert_eq!(tags.art, None);
        }
        other => panic!("expected Tags, got {other:?}"),
    }
    let len = app.file.len() as u64;
    assert!(app.reads.iter().all(|(offset, n)| offset + u64::from(*n) <= len));
    app.send(&ToHelper::Close);

    let mut app = App::start(wav_one_second(), ROUTES);
    assert!(matches!(app.open(KindHint::Audio), FromHelper::Opened(_)));
    app.send(&ToHelper::WantAudio {
        rate: 8000,
        channels: 1,
        frames: 800,
    });
    assert!(matches!(app.next(), FromHelper::Audio { .. }), "no tags in this WAV");
    app.send(&ToHelper::Close);

    let mut app = App::start(y4m_five_frames(), ROUTES);
    assert!(matches!(app.open(KindHint::Video), FromHelper::Opened(_)));
    app.send(&ToHelper::WantVideo { w: 64, h: 36 });
    assert!(matches!(app.next(), FromHelper::Frame { .. }), "a video sends no tags");
    app.send(&ToHelper::Close);
}

/// w12-video-start: the sound of an H.264 + MP3 file is proven by its decode and its clock, not by ear (the Windows test machine has no
/// sound device). The picture and the PCM are asked for in turn, as the player does: each PCM answer starts where the
/// one before ended, the tone is there (not silence), and after a second of sound the picture is a second on too.
#[test]
fn helper_decodes_an_mp3_track_beside_the_h264_picture() {
    let mut app = App::start(fixture("h264-mp3.avi"), ROUTES);
    let FromHelper::Opened(opened) = app.open(KindHint::Video) else {
        panic!("the file opens");
    };
    assert_eq!(opened.video.map(|video| video.codec), Some(Codec::H264));
    assert_eq!(opened.audio.map(|audio| audio.codec), Some(Codec::Mp3));
    assert_eq!(opened.no_sound, None, "its sound plays");
    let (rate, channels, part) = (48_000u32, 2u16, 4_800u32);
    let (mut next_pts, mut frames, mut sum_sq, mut count) = (0u64, 0u64, 0f64, 0u64);
    let mut last_picture = None;
    for _ in 0..10 {
        app.send(&ToHelper::WantAudio {
            rate,
            channels,
            frames: part,
        });
        let FromHelper::Audio {
            pts_ms,
            channels: got,
            samples,
        } = app.next()
        else {
            panic!("PCM for every ask in the first second");
        };
        assert_eq!(got, channels);
        assert_eq!(pts_ms, next_pts, "each answer starts where the one before ended");
        frames += (samples.len() / usize::from(channels)) as u64;
        next_pts = frames * 1000 / u64::from(rate);
        sum_sq += samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>();
        count += samples.len() as u64;
        for _ in 0..3 {
            app.send(&ToHelper::WantVideo { w: 64, h: 36 });
            let FromHelper::Frame { pts_ms, .. } = app.next() else {
                panic!("a frame for every ask in the first second");
            };
            assert!(last_picture.is_none_or(|last| pts_ms > last), "the picture moves on");
            last_picture = Some(pts_ms);
        }
    }
    assert_eq!(next_pts, 1_000, "a second of sound was decoded");
    let rms = (sum_sq / count as f64).sqrt();
    assert!(rms > 0.01, "the track's tone, not silence: rms {rms}");
    assert!(
        last_picture.is_some_and(|pts| pts >= 900),
        "the picture is a second on too"
    );
    app.send(&ToHelper::Close);
}

/// ux-57: H.264 + AAC with the switch on, in a process where the system's decoders do not start (every OS but
/// Windows; on Windows the sandbox's profiles). The first helper asks for the restart by naming AAC; the last helper
/// (`LastHelper` before `Open`) plays the picture and names the track that stays silent.
#[cfg(not(windows))]
#[test]
fn the_last_helper_plays_h264_without_the_aac_the_system_cannot_decode() {
    let mut first = App::start_with(fixture("h264-aac.mkv"), ROUTES, true);
    assert_eq!(
        first.open(KindHint::Video),
        FromHelper::Error(ErrorCode::NoSystemDecoder(Codec::Aac)),
        "the first helper asks for the system decoder's profile"
    );
    first.send(&ToHelper::Close);

    let mut last = App::start_with(fixture("h264-aac.mkv"), ROUTES, true);
    last.send(&ToHelper::LastHelper);
    let FromHelper::Opened(opened) = last.open(KindHint::Video) else {
        panic!("the last helper opens the video");
    };
    assert_eq!(opened.video.map(|video| video.codec), Some(Codec::H264));
    assert_eq!(opened.audio, None, "no audio stream is offered");
    assert_eq!(opened.no_sound, Some(Codec::Aac), "the silent track is named");
    last.send(&ToHelper::WantVideo { w: 64, h: 36 });
    assert!(matches!(last.next(), FromHelper::Frame { .. }), "the picture plays");
    last.send(&ToHelper::Close);
}
