//! Hostile video and pictures end only the helper (dc-104): malformed MP4, MKV and HEVC, and HEIC and DNG, all made
//! here (no third-party files), go through the helper's own process over its pipes. For each, the helper refuses on
//! the wire or exits; it never hangs past the watchdog, never answers with more than the protocol allows (the app's
//! reader refuses that), and the app's side (this test) keeps working: the next, valid request on a new helper is
//! still answered. On Windows the system decoder is on (`MEDIA_SYSTEM_DECODER_ON`), so HEVC and HEIC go to Media
//! Foundation and WIC; where those are missing the refusal is `NoSystemDecoder` / `NoDecoder`, equally fine.

use std::io::{BufReader, BufWriter, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use beebeeb_media_protocol::image::{
    DecodeImage, IMAGE_MODE_ARG, ImageAnswer, ImageFromHelper, ImageRefusal, read_image_to_app,
};
use beebeeb_media_protocol::{ErrorCode, FromHelper, KindHint, ToHelper, read_to_app, write_msg};

const WATCHDOG: Duration = Duration::from_secs(90);

/// What the app saw of one hostile file.
#[derive(Debug)]
enum Outcome {
    /// The helper answered with a refusal (an error code or a refused picture).
    Refused(String),
    /// The helper's pipe closed (it exited, crashed or was ended by its limits) with no further answer.
    Exited,
    /// The helper opened or decoded the file; harmless for the app, but only if it was within the limits.
    Decoded(String),
}

fn spawn(args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_beebeeb-media-helper"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the helper starts")
}

/// Runs `drive` against a fresh helper under the watchdog; the helper is killed at the end.
fn with_watchdog<T: Send + 'static>(
    args: &'static [&'static str],
    drive: impl FnOnce(&mut dyn std::io::Write, &mut dyn std::io::Read) -> T + Send + 'static,
) -> T {
    let mut child = spawn(args);
    let mut to = BufWriter::new(child.stdin.take().unwrap());
    let mut from = BufReader::new(child.stdout.take().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(drive(&mut to, &mut from));
    });
    let got = rx.recv_timeout(WATCHDOG);
    let _ = child.kill();
    let _ = child.wait();
    got.expect("the helper hung: the watchdog ended it")
}

fn serve(bytes: &[u8], id: u32, offset: u64, len: u32) -> ToHelper {
    let end = offset.saturating_add(u64::from(len));
    if end <= bytes.len() as u64 {
        ToHelper::Data {
            id,
            bytes: bytes[offset as usize..end as usize].to_vec(),
        }
    } else {
        ToHelper::DataFailed { id }
    }
}

/// The app side of a media file: Open, then ask for a frame and a little audio, answering reads from `bytes`.
/// `claimed_len` lets a file lie about its size.
fn play(bytes: Vec<u8>, claimed_len: u64, hint: KindHint) -> Outcome {
    with_watchdog(&[], move |to, from| {
        let mut send = |msg: ToHelper| write_msg(&mut *to, &msg).and_then(|()| to.flush()).is_ok();
        if !send(ToHelper::Open { len: claimed_len, hint }) {
            return Outcome::Exited;
        }
        let mut opened = false;
        let mut wanted = 0;
        loop {
            match read_to_app(&mut *from) {
                Err(_) => return Outcome::Exited,
                Ok(FromHelper::ReadAt { id, offset, len }) => {
                    if !send(serve(&bytes, id, offset, len)) {
                        return Outcome::Exited;
                    }
                }
                Ok(FromHelper::Error(code)) => return Outcome::Refused(format!("{code:?}")),
                Ok(FromHelper::Opened(o)) => {
                    opened = true;
                    wanted += 1;
                    let ok = match hint {
                        KindHint::Video => send(ToHelper::WantVideo { w: 320, h: 240 }),
                        KindHint::Audio => send(ToHelper::WantAudio {
                            rate: 48_000,
                            channels: 2,
                            frames: 4_800,
                        }),
                    };
                    if !ok {
                        return Outcome::Exited;
                    }
                    let _ = o;
                }
                Ok(FromHelper::Frame { w, h, rgba, .. }) => {
                    assert_eq!(rgba.len() as u64, u64::from(w) * u64::from(h) * 4, "the frame is what it says");
                    if wanted >= 3 {
                        return Outcome::Decoded(format!("frame {w}x{h}"));
                    }
                    wanted += 1;
                    if !send(ToHelper::WantVideo { w: 320, h: 240 }) {
                        return Outcome::Exited;
                    }
                }
                Ok(FromHelper::Audio { samples, .. }) => return Outcome::Decoded(format!("{} samples", samples.len())),
                Ok(FromHelper::Tags(_)) => {}
                Ok(FromHelper::Ended) => {
                    return if opened {
                        Outcome::Decoded("ended".into())
                    } else {
                        Outcome::Refused("Ended".into())
                    };
                }
            }
        }
    })
}

/// The app side of a picture.
fn picture(bytes: Vec<u8>, claimed_len: u64) -> Outcome {
    with_watchdog(&[IMAGE_MODE_ARG], move |to, from| {
        if write_msg(&mut *to, &DecodeImage { max_px: 256, len: claimed_len })
            .and_then(|()| to.flush())
            .is_err()
        {
            return Outcome::Exited;
        }
        loop {
            match read_image_to_app(&mut *from) {
                Err(_) => return Outcome::Exited,
                Ok(ImageFromHelper::ReadAt { id, offset, len }) => {
                    let msg = serve(&bytes, id, offset, len);
                    if write_msg(&mut *to, &msg).and_then(|()| to.flush()).is_err() {
                        return Outcome::Exited;
                    }
                }
                Ok(ImageFromHelper::Answer(ImageAnswer::Refused(why))) => return Outcome::Refused(format!("{why:?}")),
                Ok(ImageFromHelper::Answer(ImageAnswer::Pixels { width, height, rgba })) => {
                    assert!(width <= 256 && height <= 256, "never over the size asked");
                    assert_eq!(rgba.len() as u64, u64::from(width) * u64::from(height) * 4);
                    return Outcome::Decoded(format!("{width}x{height}"));
                }
            }
        }
    })
}

// ---- generators ------------------------------------------------------------------------------------------------

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
    let mut body = brand.to_vec();
    body.extend_from_slice(&[0, 0, 0, 0]);
    body.extend_from_slice(brand);
    bx(b"ftyp", &body)
}

/// An `stsd` with one video entry of `fourcc` claiming `w` x `h`.
fn video_entry(fourcc: &[u8; 4], w: u16, h: u16) -> Vec<u8> {
    let mut e = vec![0u8; 6];
    e.extend_from_slice(&1u16.to_be_bytes());
    e.extend_from_slice(&[0; 16]);
    e.extend_from_slice(&w.to_be_bytes());
    e.extend_from_slice(&h.to_be_bytes());
    e.extend_from_slice(&[0, 0x48, 0, 0, 0, 0x48, 0, 0, 0, 0, 0, 0, 0, 1]);
    e.extend_from_slice(&[0; 32]);
    e.extend_from_slice(&[0, 0x18, 0xff, 0xff]);
    let mut full = vec![0u8; 4];
    full.extend_from_slice(&1u32.to_be_bytes());
    full.extend_from_slice(&bx(fourcc, &e));
    bx(b"stsd", &full)
}

fn full(entries: &[u32]) -> Vec<u8> {
    let mut v = vec![0u8; 4];
    for e in entries {
        v.extend_from_slice(&e.to_be_bytes());
    }
    v
}

/// A one-track MP4 with the given sample-table boxes (`stts`, `stsc`, `stsz`, `stco`) and sample entry.
fn mp4(fourcc: &[u8; 4], w: u16, h: u16, tables: [Vec<u8>; 4], mdat: &[u8]) -> Vec<u8> {
    let [stts, stsc, stsz, stco] = tables;
    let mut stbl = video_entry(fourcc, w, h);
    stbl.extend(bx(b"stts", &stts));
    stbl.extend(bx(b"stsc", &stsc));
    stbl.extend(bx(b"stsz", &stsz));
    stbl.extend(bx(b"stco", &stco));
    let mut mdhd = vec![0u8; 12];
    mdhd.extend_from_slice(&1000u32.to_be_bytes());
    mdhd.extend_from_slice(&1000u32.to_be_bytes());
    mdhd.extend_from_slice(&[0, 0, 0, 0]);
    let mut hdlr = vec![0u8; 8];
    hdlr.extend_from_slice(b"vide");
    hdlr.extend_from_slice(&[0; 13]);
    let minf = bx(b"minf", &bx(b"stbl", &stbl));
    let mut mdia = bx(b"mdhd", &mdhd);
    mdia.extend(bx(b"hdlr", &hdlr));
    mdia.extend(minf);
    let trak = bx(b"trak", &bx(b"mdia", &mdia));
    let mut out = ftyp(b"isom");
    out.extend(bx(b"moov", &trak));
    out.extend(bx(b"mdat", mdat));
    out
}

fn sane_tables() -> [Vec<u8>; 4] {
    [
        full(&[1, 1, 40]),
        full(&[1, 1, 1, 1]),
        full(&[0, 16, 1, 16]),
        full(&[1, 0]),
    ]
}

fn ebml_id_size(id: &[u8], size: u64, body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    let _ = size;
    out.push(0x01);
    out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

fn mkv(video_w: u32, video_h: u32, codec: &[u8], cluster_len_lie: bool) -> Vec<u8> {
    let uint = |id: &[u8], v: u32| ebml_id_size(id, 4, &v.to_be_bytes());
    let header = ebml_id_size(&[0x1A, 0x45, 0xDF, 0xA3], 0, &ebml_id_size(&[0x42, 0x82], 0, b"matroska"));
    let mut video = uint(&[0xB0], video_w);
    video.extend(uint(&[0xBA], video_h));
    let mut track = uint(&[0xD7], 1);
    track.extend(uint(&[0x83], 1));
    track.extend(ebml_id_size(&[0x86], 0, codec));
    track.extend(ebml_id_size(&[0xE0], 0, &video));
    let tracks = ebml_id_size(&[0x16, 0x54, 0xAE, 0x6B], 0, &ebml_id_size(&[0xAE], 0, &track));
    let mut cluster_body = uint(&[0xE7], 0);
    cluster_body.extend(ebml_id_size(&[0xA3], 0, &[0x81, 0, 0, 0x80, 1, 2, 3, 4]));
    let mut cluster = ebml_id_size(&[0x1F, 0x43, 0xB6, 0x75], 0, &cluster_body);
    if cluster_len_lie {
        // The cluster's size field claims 2^56 - 2 bytes.
        cluster[4..12].copy_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE]);
    }
    let mut segment_body = tracks;
    segment_body.extend(cluster);
    let mut out = header;
    out.extend(ebml_id_size(&[0x18, 0x53, 0x80, 0x67], 0, &segment_body));
    out
}

// ---- helpers over outcomes -------------------------------------------------------------------------------------

/// Never a panic or hang in the app: any refusal or exit is the wanted end. A helper that decodes a hostile file
/// is allowed only when it stays inside what the wire allows (checked while reading).
fn ends_safely(name: &str, outcome: Outcome) {
    eprintln!("hostile {name}: {outcome:?}");
    if let Outcome::Refused(why) = &outcome {
        assert!(!why.is_empty());
    }
}

fn truncations(whole: &[u8]) -> Vec<Vec<u8>> {
    [4, 8, 12, 20, whole.len() / 2, whole.len().saturating_sub(3)]
        .into_iter()
        .filter(|n| *n < whole.len())
        .map(|n| whole[..n].to_vec())
        .collect()
}

// ---- the tests -------------------------------------------------------------------------------------------------

#[test]
fn hostile_mp4_truncated_boxes() {
    let whole = mp4(b"avc1", 64, 48, sane_tables(), &[0u8; 64]);
    for (i, cut) in truncations(&whole).into_iter().enumerate() {
        let len = cut.len() as u64;
        ends_safely(&format!("mp4 cut {i}"), play(cut, len, KindHint::Video));
    }
}

#[test]
fn hostile_mp4_box_larger_than_file() {
    let mut whole = mp4(b"avc1", 64, 48, sane_tables(), &[0u8; 64]);
    // The moov box claims 4 GiB - 1 bytes; and a 64-bit size of 2^63.
    let moov = 20;
    whole[moov..moov + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    let len = whole.len() as u64;
    ends_safely("mp4 moov 4GiB", play(whole.clone(), len, KindHint::Video));
    whole[moov..moov + 4].copy_from_slice(&1u32.to_be_bytes());
    let mut huge = whole[..moov + 8].to_vec();
    huge.extend_from_slice(&(1u64 << 63).to_be_bytes());
    huge.extend_from_slice(&whole[moov + 8..]);
    let len = huge.len() as u64;
    ends_safely("mp4 64-bit size 2^63", play(huge, len, KindHint::Video));
    // A box of size 0..7 (smaller than its own header) must not loop forever.
    let mut tiny = whole.clone();
    tiny[moov..moov + 4].copy_from_slice(&3u32.to_be_bytes());
    let len = tiny.len() as u64;
    ends_safely("mp4 box size 3", play(tiny, len, KindHint::Video));
}

#[test]
fn hostile_mp4_huge_dimensions() {
    let whole = mp4(b"avc1", u16::MAX, u16::MAX, sane_tables(), &[0u8; 64]);
    let len = whole.len() as u64;
    ends_safely("mp4 65535x65535 avc1", play(whole, len, KindHint::Video));
    let whole = mp4(b"hvc1", u16::MAX, u16::MAX, sane_tables(), &[0u8; 64]);
    let len = whole.len() as u64;
    ends_safely("mp4 65535x65535 hvc1", play(whole, len, KindHint::Video));
}

#[test]
fn hostile_mp4_bad_sample_tables() {
    let big = u32::MAX;
    let cases: Vec<(&str, [Vec<u8>; 4])> = vec![
        ("stsz count 2^32-1", [full(&[1, 1, 40]), full(&[1, 1, 1, 1]), full(&[0, 16, big]), full(&[1, 0])]),
        ("stts count 2^32-1", [full(&[1, big, 40]), full(&[1, 1, 1, 1]), full(&[0, 16, 1, 16]), full(&[1, 0])]),
        ("stsc runs 2^32-1", [full(&[1, 1, 40]), full(&[1, 1, big, 1]), full(&[0, 16, 1, 16]), full(&[1, 0])]),
        ("stco offset past file", [full(&[1, 1, 40]), full(&[1, 1, 1, 1]), full(&[0, 16, 1, 16]), full(&[1, big])]),
        ("stsz sample 4 GiB", [full(&[1, 1, 40]), full(&[1, 1, 1, 1]), full(&[0, 0, 1, big]), full(&[1, 0])]),
        ("stsc chunk 0", [full(&[1, 1, 40]), full(&[1, 0, 0, 0]), full(&[0, 16, 1, 16]), full(&[1, 0])]),
        ("empty tables", [full(&[0]), full(&[0]), full(&[0, 0, 0]), full(&[0])]),
    ];
    for (name, tables) in cases {
        let whole = mp4(b"avc1", 64, 48, tables, &[0u8; 64]);
        let len = whole.len() as u64;
        ends_safely(name, play(whole, len, KindHint::Video));
    }
}

#[test]
fn hostile_mp4_hevc_garbage_and_lying_length() {
    let whole = mp4(b"hvc1", 64, 48, sane_tables(), &[0xFF; 256]);
    let real = whole.len() as u64;
    ends_safely("hevc garbage", play(whole.clone(), real, KindHint::Video));
    // The app is told a length the file does not have: every read past the end fails.
    ends_safely("len lie 1 TiB", play(whole.clone(), 1 << 40, KindHint::Video));
    ends_safely("len 0", play(whole, 0, KindHint::Video));
}

#[test]
fn hostile_mkv_malformed() {
    for (name, whole) in [
        ("mkv sane header, junk frame", mkv(64, 48, b"V_MPEGH/ISO/HEVC", false)),
        ("mkv huge dimensions", mkv(u32::MAX, u32::MAX, b"V_MPEG4/ISO/AVC", false)),
        ("mkv cluster size 2^56", mkv(64, 48, b"V_VP9", true)),
        ("mkv unknown codec", mkv(64, 48, b"V_NOPE", false)),
    ] {
        let len = whole.len() as u64;
        ends_safely(name, play(whole.clone(), len, KindHint::Video));
        for (i, cut) in truncations(&whole).into_iter().enumerate() {
            let len = cut.len() as u64;
            ends_safely(&format!("{name} cut {i}"), play(cut, len, KindHint::Video));
        }
    }
}

#[test]
fn hostile_raw_bytes_and_no_data() {
    ends_safely("empty", play(Vec::new(), 0, KindHint::Video));
    ends_safely("zeros 1 MiB", play(vec![0; 1 << 20], 1 << 20, KindHint::Video));
    ends_safely("0xFF 1 MiB audio", play(vec![0xFF; 1 << 20], 1 << 20, KindHint::Audio));
}

/// A TIFF-shaped DNG header with `tags` as (tag, type, count, value) entries in one IFD.
fn tiff(tags: &[(u16, u16, u32, u32)], tail: &[u8]) -> Vec<u8> {
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&(tags.len() as u16).to_le_bytes());
    for (t, ty, c, v) in tags {
        out.extend_from_slice(&t.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&c.to_le_bytes());
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(tail);
    out
}

fn heic(ispe_w: u32, ispe_h: u32) -> Vec<u8> {
    let mut ispe = vec![0u8; 4];
    ispe.extend_from_slice(&ispe_w.to_be_bytes());
    ispe.extend_from_slice(&ispe_h.to_be_bytes());
    let ipco = bx(b"ipco", &bx(b"ispe", &ispe));
    let iprp = bx(b"iprp", &ipco);
    let mut meta = vec![0u8; 4];
    meta.extend(bx(b"hdlr", &[0; 24]));
    meta.extend(iprp);
    let mut out = ftyp(b"heic");
    out.extend(bx(b"meta", &meta));
    out.extend(bx(b"mdat", &[0xAB; 128]));
    out
}

#[test]
fn hostile_heic() {
    let sane = heic(64, 48);
    let mut cases: Vec<(String, Vec<u8>)> = vec![
        ("heic sane shell, no image data".into(), sane.clone()),
        ("heic 4G x 4G ispe".into(), heic(u32::MAX, u32::MAX)),
        ("heic 65536 x 65536 ispe".into(), heic(65_536, 65_536)),
        ("heic 0 x 0 ispe".into(), heic(0, 0)),
    ];
    for (i, cut) in truncations(&sane).into_iter().enumerate() {
        cases.push((format!("heic cut {i}"), cut));
    }
    for (name, bytes) in cases {
        let len = bytes.len() as u64;
        ends_safely(&name, picture(bytes, len));
    }
}

#[test]
fn hostile_dng() {
    const SHORT: u16 = 3;
    const LONG: u16 = 4;
    let base = |w: u32, h: u32, strip_off: u32, strip_len: u32| {
        vec![
            (254, LONG, 1, 0),
            (256, LONG, 1, w),
            (257, LONG, 1, h),
            (258, SHORT, 1, 8),
            (259, SHORT, 1, 1),
            (262, SHORT, 1, 2),
            (273, LONG, 1, strip_off),
            (277, SHORT, 1, 3),
            (278, LONG, 1, h),
            (279, LONG, 1, strip_len),
            (50706, 1, 4, 0x0000_0401),
        ]
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("dng huge dimensions", tiff(&base(u32::MAX, u32::MAX, 200, 12), &[0; 64])),
        ("dng 20000 x 20000", tiff(&base(20_000, 20_000, 200, 12), &[0; 64])),
        ("dng strip offset past file", tiff(&base(8, 8, u32::MAX, 192), &[0; 64])),
        ("dng strip length 4 GiB", tiff(&base(8, 8, 200, u32::MAX), &[0; 64])),
        ("dng IFD offset loops", {
            let mut t = tiff(&base(8, 8, 200, 192), &[0; 64]);
            let next = 8 + 2 + 11 * 12;
            t[next..next + 4].copy_from_slice(&8u32.to_le_bytes());
            t
        }),
        ("dng tag count 2^32-1", tiff(&[(256, LONG, u32::MAX, 100), (257, LONG, 1, 8)], &[0; 64])),
        ("dng IFD past file", {
            let mut t = tiff(&base(8, 8, 200, 192), &[0; 64]);
            t[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
            t
        }),
        ("tiff header only", b"II*\0\x08\0\0\0".to_vec()),
    ];
    for (name, bytes) in cases {
        let len = bytes.len() as u64;
        ends_safely(name, picture(bytes.clone(), len));
        for (i, cut) in truncations(&bytes).into_iter().enumerate() {
            let len = cut.len() as u64;
            ends_safely(&format!("{name} cut {i}"), picture(cut, len));
        }
    }
}

#[test]
fn hostile_picture_sizes_over_limits_are_refused() {
    // A file claimed larger than the helper reads at all: refused TooLarge before any read, never allocated.
    let sane = heic(64, 48);
    match picture(sane.clone(), 1 << 40) {
        Outcome::Refused(why) => assert_eq!(why, format!("{:?}", ImageRefusal::TooLarge)),
        other => panic!("over MAX_IMAGE_FILE must be refused TooLarge, got {other:?}"),
    }
    // Not a picture at all.
    match picture(vec![0x41; 4096], 4096) {
        Outcome::Refused(_) | Outcome::Exited => {}
        other => panic!("noise must not decode: {other:?}"),
    }
}

#[test]
fn hostile_the_app_survives_and_the_next_file_works() {
    // After hostile input on earlier helpers, a valid picture on a new helper is still answered: nothing is shared.
    let _ = picture(heic(u32::MAX, u32::MAX), 0);
    let bmp = {
        let (w, h) = (4u32, 4u32);
        let mut out = b"BM".to_vec();
        out.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        out.extend(std::iter::repeat_n(0x80u8, (w * h * 4) as usize));
        out
    };
    let len = bmp.len() as u64;
    let _ = picture(bmp, len); // a valid or refused answer, both are protocol-clean; the point is no hang or panic
    let _ = ErrorCode::Corrupt;
}
