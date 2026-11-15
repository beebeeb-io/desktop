//! The helper's loop in-process: a fake app on two in-memory pipes, no sandbox. The app is the test: it answers
//! `ReadAt` from a byte vector and records every ask.

use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use beebeeb_media_helper::session::run_with;
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
        let (to_helper, helper_in) = channel();
        let (helper_out, from_helper) = channel();
        let input = ChanRead {
            rx: helper_in,
            buf: Vec::new(),
            pos: 0,
        };
        let join = std::thread::spawn(move || run_with(input, ChanWrite(helper_out), table));
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
