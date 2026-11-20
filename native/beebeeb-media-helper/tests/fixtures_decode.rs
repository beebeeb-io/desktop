//! M9: every fixture row decodes right, or is refused honestly, through the helper's session over in-memory pipes.
//!
//! The fixtures are written by `scripts/media-fixtures.sh` (ffmpeg's own test sources, nothing third-party) with a
//! `manifest.tsv`: `file kind codec expect sha256 ref`. A fixtures dir with fewer than 26 rows, a row without its file,
//! or a file whose hash is not the manifest's is a failure, never a skip.
//!
//! The session entry point used here is `beebeeb_media_helper::session::run_with_system(input, output, table, false)`:
//! the switch HELD OFF on every OS (what this helper's own decoders do alone, the same on Linux and on Windows), with
//! the shipped route table (features: `h264` only). The shipped switch is on (ux-02, 2026-11-20): what Windows'
//! decoders then do with the `system` and `silent` rows is `tests/system_decoder.rs` and the VM rung.
//!
//! The `expect` column is exact, by the shipped routes: `play`, `silent` (the video plays, `no_sound` names the audio
//! codec), `system` (`NoSystemDecoder`), `unsupported` (`Unsupported`), `refuse` (the code `refused_with` pins per file).
//!
//! Video claim, honestly stated: the helper answers RGBA, not planes, so a "bit-exact Y plane" cannot be compared. The
//! test converts the RGBA picture back to luma (BT.601 or BT.709, limited range: the better of the two) and requires
//! PSNR >= 45 dB against ffmpeg's decoded Y planes (`fixtures/ref/<file>.y`) for every row that plays.
//! The manifest's `ref` (the SHA-256 of those planes) is checked against the kept planes, so the reference cannot drift.

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use beebeeb_media_helper::{ROUTES, Route};
use beebeeb_media_protocol::{Codec, ErrorCode, FromHelper, KindHint, Opened, ToHelper, read_to_app, write_msg};

const ROWS_EXPECTED: usize = 26;
const W: u32 = 320;
const H: u32 = 180;
const PSNR_MIN_DB: f64 = 45.0;
const RMS_TOLERANCE: f64 = 0.01;
const STEP: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------- manifest

#[derive(Debug, Clone)]
struct Row {
    file: String,
    kind: String,
    codec: String,
    expect: String,
    sha256: String,
    reference: String,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn rows() -> Vec<Row> {
    let path = fixtures_dir().join("manifest.tsv");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("manifest.tsv missing ({e}): run scripts/media-fixtures.sh"));
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 6, "manifest row has {} columns: {l}", f.len());
            Row {
                file: f[0].into(),
                kind: f[1].into(),
                codec: f[2].into(),
                expect: f[3].into(),
                sha256: f[4].into(),
                reference: f[5].into(),
            }
        })
        .collect()
}

fn row(file: &str) -> Row {
    rows()
        .into_iter()
        .find(|r| r.file == file)
        .unwrap_or_else(|| panic!("no manifest row for {file}"))
}

// ---------------------------------------------------------------- SHA-256 (for the reference check only)

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[test]
fn sha256_matches_known_vectors() {
    // The instrument proves it can fail: the NIST vectors for "" and "abc".
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

// ---------------------------------------------------------------- the fake app on two in-memory pipes

struct ChanReader {
    rx: Receiver<Vec<u8>>,
    buf: VecDeque<u8>,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.buf.is_empty() {
            match self.rx.recv_timeout(STEP) {
                Ok(chunk) => self.buf.extend(chunk),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "pipe silent for 60 s"));
                }
            }
        }
        let n = out.len().min(self.buf.len());
        for slot in out.iter_mut().take(n) {
            *slot = self.buf.pop_front().expect("length checked");
        }
        Ok(n)
    }
}

struct ChanWriter(Sender<Vec<u8>>);

impl Write for ChanWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0
            .send(data.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "peer gone"))?;
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The app's side: the whole file is the byte source; every `ReadAt` is answered from it (and counted).
struct App {
    to_helper: ChanWriter,
    from_helper: ChanReader,
    file: Vec<u8>,
    reads: usize,
    helper: Option<std::thread::JoinHandle<()>>,
}

impl App {
    /// Starts the helper's session in a thread with `table` as its route table.
    fn start(file: Vec<u8>, table: &'static [(Codec, Route)]) -> App {
        let (app_tx, helper_rx) = mpsc::channel();
        let (helper_tx, app_rx) = mpsc::channel();
        let helper = std::thread::spawn(move || {
            let input = ChanReader {
                rx: helper_rx,
                buf: VecDeque::new(),
            };
            let output = ChanWriter(helper_tx);
            // The result is the session's own end (Close, EOF or a refused message); the test reads the pipe.
            let _ = beebeeb_media_helper::session::run_with_system(input, output, table, false);
        });
        App {
            to_helper: ChanWriter(app_tx),
            from_helper: ChanReader {
                rx: app_rx,
                buf: VecDeque::new(),
            },
            file,
            reads: 0,
            helper: Some(helper),
        }
    }

    fn send(&mut self, msg: &ToHelper) {
        write_msg(&mut self.to_helper, msg).expect("the helper's pipe is open");
    }

    /// The next message that is not a read request; read requests are answered from the file on the way.
    fn next(&mut self) -> FromHelper {
        loop {
            let msg = read_to_app(&mut self.from_helper).expect("a protocol message from the helper");
            match msg {
                FromHelper::ReadAt { id, offset, len } => {
                    self.reads += 1;
                    let end = offset.checked_add(u64::from(len)).expect("offset + len overflowed");
                    assert!(
                        end <= self.file.len() as u64,
                        "the helper asked beyond the file: {offset}+{len}"
                    );
                    let bytes = self.file[offset as usize..end as usize].to_vec();
                    self.send(&ToHelper::Data { id, bytes });
                }
                other => return other,
            }
        }
    }

    /// `Open`, then the answer: `Opened` (tags that follow are skipped by the next call) or an `Error`.
    fn open(&mut self, hint: KindHint) -> Result<Opened, ErrorCode> {
        let len = self.file.len() as u64;
        self.send(&ToHelper::Open { len, hint });
        loop {
            match self.next() {
                FromHelper::Opened(o) => return Ok(o),
                FromHelper::Error(code) => return Err(code),
                FromHelper::Tags(_) => {}
                other => panic!("unexpected answer to Open: {other:?}"),
            }
        }
    }

    /// The next picture, `W`x`H`, as RGBA.
    fn frame(&mut self) -> Result<Vec<u8>, ErrorCode> {
        self.send(&ToHelper::WantVideo { w: W, h: H });
        loop {
            match self.next() {
                FromHelper::Frame { w, h, rgba, .. } => {
                    assert_eq!((w, h), (W, H), "a frame must be exactly the size asked");
                    assert_eq!(rgba.len(), (W * H * 4) as usize);
                    return Ok(rgba);
                }
                FromHelper::Error(code) => return Err(code),
                FromHelper::Tags(_) => {}
                other => panic!("unexpected answer to WantVideo: {other:?}"),
            }
        }
    }

    /// The first second as interleaved f32, 48 kHz stereo: asks until 48 000 frames came or the end.
    fn first_second(&mut self) -> Result<Vec<f32>, ErrorCode> {
        let mut out: Vec<f32> = Vec::new();
        for _ in 0..64 {
            let have = (out.len() / 2) as u32;
            if have >= 48_000 {
                break;
            }
            self.send(&ToHelper::WantAudio {
                rate: 48_000,
                channels: 2,
                frames: 48_000 - have,
            });
            loop {
                match self.next() {
                    FromHelper::Audio { channels, samples, .. } => {
                        assert_eq!(channels, 2, "PCM must be the channel count asked");
                        out.extend(samples);
                        break;
                    }
                    FromHelper::Ended => return Ok(out),
                    FromHelper::Error(code) => return Err(code),
                    FromHelper::Tags(_) => {}
                    other => panic!("unexpected answer to WantAudio: {other:?}"),
                }
            }
        }
        out.truncate(96_000);
        Ok(out)
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = write_msg(&mut self.to_helper, &ToHelper::Close);
        if let Some(h) = self.helper.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------- measures

fn read_file(file: &str) -> Vec<u8> {
    fs::read(fixtures_dir().join(file)).unwrap_or_else(|e| panic!("{file}: {e} (a row with no file fails)"))
}

fn rgba_luma(rgba: &[u8], bt709: bool) -> Vec<u8> {
    let (kr, kg, kb) = if bt709 {
        (46.559, 156.629, 15.812)
    } else {
        (65.738, 129.057, 25.064)
    };
    rgba.chunks_exact(4)
        .map(|p| {
            let y = 16.0 + (kr * f64::from(p[0]) + kg * f64::from(p[1]) + kb * f64::from(p[2])) / 256.0;
            y.round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mse = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            let d = f64::from(*x) - f64::from(*y);
            d * d
        })
        .sum::<f64>()
        / a.len() as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    }
}

fn rms(samples: &[f32]) -> f64 {
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    (sum / samples.len() as f64).sqrt()
}

fn s16_bytes(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| {
            let v = (f64::from(*s) * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            v.to_le_bytes()
        })
        .collect()
}

// ---------------------------------------------------------------- one case per row

/// The manifest's `codec` column as the protocol's codec (for the codes a row pins).
fn codec_of(column: &str) -> Codec {
    match column {
        "h264" => Codec::H264,
        "hevc" => Codec::Hevc,
        "vp8" => Codec::Vp8,
        "vp9" => Codec::Vp9,
        "av1" => Codec::Av1,
        "mpeg2video" => Codec::Mpeg2,
        "prores" => Codec::ProRes,
        "mpeg4" => Codec::Mpeg4Part2,
        "wmv2" => Codec::Vc1,
        "mp3" => Codec::Mp3,
        "aac" => Codec::Aac,
        "flac" => Codec::Flac,
        "alac" => Codec::Alac,
        "opus" => Codec::Opus,
        "vorbis" => Codec::Vorbis,
        "pcm_s16le" => Codec::Pcm,
        "wmav2" => Codec::Wma,
        "ac3" => Codec::Ac3,
        "eac3" => Codec::Eac3,
        other => panic!("manifest codec {other} has no protocol codec"),
    }
}

/// The audio codec a `silent` row's container carries (the `codec` column names its video).
fn silent_audio_codec(file: &str) -> Codec {
    if file.contains("-aac") {
        Codec::Aac
    } else {
        panic!("{file}: no audio codec known for a silent row")
    }
}

/// The code a `refuse` row is refused with. MPEG-4 part 2 is routed to the system's decoder (none is wired), the
/// other two have no decoder at all.
fn refused_with(file: &str) -> ErrorCode {
    match file {
        "mpeg4p2.avi" => ErrorCode::NoSystemDecoder(Codec::Mpeg4Part2),
        "wmv2.wmv" | "wmav2.wma" => ErrorCode::NoDecoder,
        other => panic!("{other}: a refuse row with no pinned refusal"),
    }
}

/// The refusal of a row, at `Open` or at the first want: both are honest, a picture or PCM is not.
fn refusal(app: &mut App, file: &str, kind: &str, hint: KindHint) -> ErrorCode {
    let answer = match app.open(hint) {
        Err(code) => Err(code),
        Ok(_) if kind == "audio" => app.first_second().map(|_| ()),
        Ok(_) => app.frame().map(|_| ()),
    };
    match answer {
        Err(code) => code,
        Ok(()) => panic!("{file}: a refused row played"),
    }
}

fn run_row(file: &str) {
    let r = row(file);
    let bytes = read_file(file);
    assert_eq!(
        sha256_hex(&bytes),
        r.sha256,
        "{file}: the file is not the one the manifest hashed"
    );
    let hint = if r.kind == "audio" {
        KindHint::Audio
    } else {
        KindHint::Video
    };
    let mut app = App::start(bytes, ROUTES);

    match r.expect.as_str() {
        "refuse" => {
            // The SPECIFIC refusal, from the first run's printed codes (MCP2 row 23): a wrong refusal is red.
            let code = refusal(&mut app, file, &r.kind, hint);
            assert_eq!(
                code,
                refused_with(file),
                "{file}: refused, but not with the code this row pins"
            );
        }
        "system" => {
            let code = refusal(&mut app, file, &r.kind, hint);
            assert_eq!(
                code,
                ErrorCode::NoSystemDecoder(codec_of(&r.codec)),
                "{file}: this codec's route is the system's decoder"
            );
        }
        "unsupported" => {
            let code = refusal(&mut app, file, &r.kind, hint);
            assert_eq!(
                code,
                ErrorCode::Unsupported(codec_of(&r.codec)),
                "{file}: this codec's route is off"
            );
        }
        "silent" => {
            let opened = app.open(hint).unwrap_or_else(|c| panic!("{file}: refused with {c:?}"));
            assert!(opened.video.is_some(), "{file}: no video stream reported");
            assert!(opened.audio.is_none(), "{file}: a silent row reports an audio stream");
            assert_eq!(
                opened.no_sound,
                Some(silent_audio_codec(file)),
                "{file}: a silent row names the audio codec it cannot play"
            );
            check_video(file, &r, &mut app);
        }
        "play" => {
            let opened = app.open(hint).unwrap_or_else(|c| panic!("{file}: refused with {c:?}"));
            assert!(!opened.reduced_quality, "{file}: not a reduced-quality row");
            assert_eq!(opened.no_sound, None, "{file}: a play row has every track");
            if r.kind == "video" {
                assert!(opened.video.is_some(), "{file}: no video stream reported");
                check_video(file, &r, &mut app);
                if file.contains("-mp3") {
                    assert!(
                        opened.audio.is_some(),
                        "{file}: the container's audio stream is missing"
                    );
                }
            } else {
                assert!(opened.audio.is_some(), "{file}: no audio stream reported");
                check_audio(file, &r, &mut app);
            }
        }
        other => panic!("{file}: manifest says expect={other}"),
    }
}

fn check_video(file: &str, r: &Row, app: &mut App) {
    let planes = read_file(&format!("ref/{file}.y"));
    assert_eq!(
        sha256_hex(&planes),
        r.reference,
        "{file}: the kept Y planes are not the manifest's reference"
    );
    let ysize = (W * H) as usize;
    assert_eq!(planes.len(), ysize * 3);
    for i in 0..3 {
        let rgba = app.frame().unwrap_or_else(|c| panic!("{file}: frame {i}: {c:?}"));
        let want = &planes[i * ysize..(i + 1) * ysize];
        let best = [false, true]
            .iter()
            .map(|m| psnr(&rgba_luma(&rgba, *m), want))
            .fold(f64::MIN, f64::max);
        assert!(
            best >= PSNR_MIN_DB,
            "{file}: frame {i} PSNR {best:.1} dB, needs {PSNR_MIN_DB} dB"
        );
    }
}

fn check_audio(file: &str, r: &Row, app: &mut App) {
    let samples = app.first_second().unwrap_or_else(|c| panic!("{file}: {c:?}"));
    assert_eq!(
        samples.len(),
        96_000,
        "{file}: the first second is 48 000 stereo frames"
    );
    assert!(samples.iter().all(|s| s.is_finite()), "{file}: PCM must be finite");
    if r.expect == "play" && r.reference.len() == 64 {
        // lossless: the s16 PCM hash of ffmpeg's decode
        assert_eq!(
            sha256_hex(&s16_bytes(&samples)),
            r.reference,
            "{file}: lossless PCM differs from ffmpeg's"
        );
    } else {
        let want: f64 = r
            .reference
            .parse()
            .unwrap_or_else(|_| panic!("{file}: ref {} is not an RMS", r.reference));
        let got = rms(&samples);
        assert!(
            want > 0.0 && (got - want).abs() <= want * RMS_TOLERANCE,
            "{file}: RMS {got:.6}, ffmpeg {want:.6}"
        );
    }
}

// ---------------------------------------------------------------- the tests

#[test]
fn fixtures_manifest_has_every_row() {
    let rows = rows();
    assert_eq!(
        rows.len(),
        ROWS_EXPECTED,
        "the fixtures dir has {} rows, needs {ROWS_EXPECTED}",
        rows.len()
    );
    for r in &rows {
        let path = fixtures_dir().join(&r.file);
        let meta = fs::metadata(&path).unwrap_or_else(|e| panic!("{}: {e}", r.file));
        assert!(
            meta.len() > 0 && meta.len() <= 300 * 1024,
            "{}: {} bytes",
            r.file,
            meta.len()
        );
        assert!(
            matches!(
                r.expect.as_str(),
                "play" | "silent" | "system" | "unsupported" | "refuse"
            ),
            "{}: expect {}",
            r.file,
            r.expect
        );
        assert!(
            matches!(r.kind.as_str(), "video" | "audio"),
            "{}: kind {}",
            r.file,
            r.kind
        );
        assert_eq!(r.sha256.len(), 64, "{}: sha256", r.file);
        assert!(
            r.reference != "-" || !matches!(r.expect.as_str(), "play" | "silent"),
            "{}: a play or silent row needs its reference",
            r.file
        );
        assert!(
            r.reference == "-" || r.expect != "refuse",
            "{}: a refuse row has no reference",
            r.file
        );
    }
    let mut names: Vec<&str> = rows.iter().map(|r| r.file.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ROWS_EXPECTED, "a file is listed twice");
    assert!(fixtures_dir().join("LICENSE.txt").is_file());

    // One kept reference per picture row, named after its whole file name (MCP2 row 22): two rows never share one.
    let mut want: Vec<String> = rows
        .iter()
        .filter(|r| r.kind == "video" && r.reference != "-")
        .map(|r| format!("{}.y", r.file))
        .collect();
    want.sort_unstable();
    let mut have: Vec<String> = fs::read_dir(fixtures_dir().join("ref"))
        .expect("fixtures/ref")
        .map(|e| e.expect("ref entry").file_name().to_string_lossy().into_owned())
        .collect();
    have.sort_unstable();
    assert_eq!(have, want, "ref/ holds exactly one <file>.y per picture row");
    // Three different pictures per reference: the test source changes every frame, so a repeated one is a reference
    // that ffmpeg padded (AVI has no pts: its default output repeats picture 0).
    for name in &have {
        let planes = read_file(&format!("ref/{name}"));
        let mut frames: Vec<&[u8]> = planes.chunks_exact(planes.len() / 3).collect();
        assert_eq!(frames.len(), 3, "{name}: three frames of Y planes");
        frames.sort_unstable();
        frames.dedup();
        assert_eq!(
            frames.len(),
            3,
            "{name}: its 3 reference frames are not 3 distinct pictures"
        );
    }
}

/// One test per row: the shipped route table, the shipped switch (OFF).
macro_rules! fixture {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            run_row($file);
        }
    };
}

fixture!(fixture_h264, "h264.mp4");
fixture!(fixture_h264_interlaced, "h264-interlaced.ts");
fixture!(fixture_h264_10bit, "h264-10bit.mp4");
fixture!(fixture_hevc_main, "hevc-main.mp4");
fixture!(fixture_hevc_main10, "hevc-main10.mov");
fixture!(fixture_vp8, "vp8.webm");
fixture!(fixture_vp9, "vp9.webm");
fixture!(fixture_av1, "av1.mkv");
fixture!(fixture_mpeg2, "mpeg2.mpg");
fixture!(fixture_mpeg4p2, "mpeg4p2.avi");
fixture!(fixture_prores, "prores.mov");
fixture!(fixture_wmv2, "wmv2.wmv");
fixture!(fixture_mp3, "mp3.mp3");
fixture!(fixture_aac_lc, "aac-lc.m4a");
fixture!(fixture_he_aac, "he-aac.m4a");
fixture!(fixture_flac, "flac.flac");
fixture!(fixture_alac, "alac.m4a");
fixture!(fixture_opus, "opus.ogg");
fixture!(fixture_vorbis, "vorbis.ogg");
fixture!(fixture_pcm, "pcm.wav");
fixture!(fixture_wmav2, "wmav2.wma");
fixture!(fixture_ac3, "ac3.ac3");
fixture!(fixture_eac3, "eac3.mka");
fixture!(fixture_h264_aac, "h264-aac.mkv");
fixture!(fixture_h264_mp3, "h264-mp3.avi");
fixture!(fixture_h264_aac_ts, "h264-aac.ts");
