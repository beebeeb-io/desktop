//! The wire's three rules: every message comes back as it was written; a length over the limits is refused before
//! anything is allocated for it (and before its bytes are read); a message cut short is refused.

use std::io::{self, Cursor, Read};

use super::*;

/// A reader that counts what was taken from it.
struct Counting<R> {
    inner: R,
    taken: usize,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.taken += n;
        Ok(n)
    }
}

fn counting(bytes: Vec<u8>) -> Counting<Cursor<Vec<u8>>> {
    Counting {
        inner: Cursor::new(bytes),
        taken: 0,
    }
}

fn wire<M: Message>(msg: &M) -> Vec<u8> {
    let mut out = Vec::new();
    write_msg(&mut out, msg).expect("a message within the limits is written");
    out
}

fn to_helper_messages() -> Vec<ToHelper> {
    vec![
        ToHelper::Open {
            len: 7_340_032_123,
            hint: KindHint::Video,
        },
        ToHelper::Open {
            len: 0,
            hint: KindHint::Audio,
        },
        ToHelper::Data {
            id: 9,
            bytes: (0..=255u8).cycle().take(70_001).collect(),
        },
        ToHelper::Data {
            id: 10,
            bytes: vec![0xAB; MAX_READ as usize],
        },
        ToHelper::DataFailed { id: u32::MAX },
        ToHelper::WantVideo {
            w: MAX_FRAME_W,
            h: MAX_FRAME_H,
        },
        ToHelper::WantVideo { w: 1, h: 1 },
        ToHelper::WantAudio {
            rate: 48_000,
            channels: 2,
            frames: MAX_AUDIO_FRAMES,
        },
        ToHelper::Seek { at_ms: 72_000 },
        ToHelper::Close,
    ]
}

fn from_helper_messages() -> Vec<FromHelper> {
    let mut all = vec![
        FromHelper::ReadAt {
            id: 1,
            offset: u64::MAX - 10,
            len: MAX_READ,
        },
        FromHelper::Opened(Opened {
            duration_ms: 242_000,
            video: None,
            audio: Some(AudioInfo {
                rate: 44_100,
                channels: 2,
                codec: Codec::Flac,
            }),
            reduced_quality: true,
        }),
        FromHelper::Frame {
            pts_ms: 33,
            w: 640,
            h: 360,
            rgba: (0..640 * 360 * 4).map(|i| (i % 251) as u8).collect(),
        },
        FromHelper::Frame {
            pts_ms: 0,
            w: MAX_FRAME_W,
            h: MAX_FRAME_H,
            rgba: vec![7; pixel_bytes(MAX_FRAME_W, MAX_FRAME_H)],
        },
        FromHelper::Audio {
            pts_ms: 1_000,
            channels: 2,
            samples: (0..2 * 480).map(|i| (i as f32 / 960.0) - 0.5).collect(),
        },
        FromHelper::Audio {
            pts_ms: 2_000,
            channels: 1,
            samples: Vec::new(),
        },
        FromHelper::Tags(Tags {
            title: "Harbour sessions, take 3".into(),
            artist: "Ana \u{00C1}lvares".into(),
            art: Some((3, 2, (0..24).collect())),
        }),
        FromHelper::Tags(Tags {
            title: "\u{1F3B5}".repeat(MAX_TEXT),
            artist: String::new(),
            art: None,
        }),
        FromHelper::Ended,
        FromHelper::Error(ErrorCode::NoDecoder),
        FromHelper::Error(ErrorCode::Corrupt),
        FromHelper::Error(ErrorCode::TooLarge),
        FromHelper::Error(ErrorCode::ReadFailed),
        FromHelper::Error(ErrorCode::Timeout),
        FromHelper::Error(ErrorCode::Internal),
    ];
    // Every codec, in the file's video, its audio, and both codes that name one.
    for codec in Codec::ALL {
        all.push(FromHelper::Opened(Opened {
            duration_ms: 24_000,
            video: Some(VideoInfo {
                w: 3840,
                h: 2160,
                fps_milli: 29_970,
                codec,
            }),
            audio: Some(AudioInfo {
                rate: 48_000,
                channels: 6,
                codec,
            }),
            reduced_quality: false,
        }));
        all.push(FromHelper::Error(ErrorCode::Unsupported(codec)));
        all.push(FromHelper::Error(ErrorCode::NoSystemDecoder(codec)));
        all.push(FromHelper::Error(ErrorCode::NoSystemMedia(codec)));
    }
    all
}

#[test]
fn protocol_round_trips_every_variant() {
    let to_helper = to_helper_messages();
    let from_helper = from_helper_messages();

    // Each of the 7 + 7 kinds is in the lists.
    let to_kinds = to_helper
        .iter()
        .map(|m| wire(m)[4])
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(to_kinds.len(), 7, "every ToHelper kind is written once at least");
    let from_kinds = from_helper
        .iter()
        .map(|m| wire(m)[4])
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(from_kinds.len(), 7, "every FromHelper kind is written once at least");

    // One message at a time, and all of them back to back on one pipe.
    let mut pipe = Vec::new();
    for msg in &to_helper {
        let bytes = wire(msg);
        assert_eq!(read_to_helper(&mut Cursor::new(&bytes)).as_ref(), Ok(msg), "{msg:?}");
        pipe.extend_from_slice(&bytes);
    }
    let mut r = Cursor::new(pipe);
    for msg in &to_helper {
        assert_eq!(read_to_helper(&mut r).as_ref(), Ok(msg));
    }
    assert_eq!(
        read_to_helper(&mut r),
        Err(WireError::Closed),
        "the pipe ended between two messages"
    );

    let mut pipe = Vec::new();
    for msg in &from_helper {
        let bytes = wire(msg);
        assert_eq!(read_to_app(&mut Cursor::new(&bytes)).as_ref(), Ok(msg), "{msg:?}");
        pipe.extend_from_slice(&bytes);
    }
    let mut r = Cursor::new(pipe);
    for msg in &from_helper {
        assert_eq!(read_to_app(&mut r).as_ref(), Ok(msg));
    }
    assert_eq!(read_to_app(&mut r), Err(WireError::Closed));

    // A message sent the wrong way is refused by its tag.
    let open = wire(&ToHelper::Close);
    assert_eq!(
        read_to_app(&mut Cursor::new(&open)),
        Err(WireError::UnknownTag(open[4]))
    );
    let ended = wire(&FromHelper::Ended);
    assert_eq!(
        read_to_helper(&mut Cursor::new(&ended)),
        Err(WireError::UnknownTag(ended[4]))
    );

    // The writer refuses what the reader would: nothing is written.
    let mut out = Vec::new();
    let too_long = ToHelper::Data {
        id: 1,
        bytes: vec![0; MAX_READ as usize + 1],
    };
    assert!(write_msg(&mut out, &too_long).is_err());
    let wrong_size = FromHelper::Frame {
        pts_ms: 0,
        w: 2,
        h: 2,
        rgba: vec![0; 15],
    };
    assert!(write_msg(&mut out, &wrong_size).is_err());
    let long_title = FromHelper::Tags(Tags {
        title: "a".repeat(MAX_TEXT + 1),
        artist: String::new(),
        art: None,
    });
    assert!(write_msg(&mut out, &long_title).is_err());
    assert!(out.is_empty());
    assert_eq!(cut_text(&"\u{00E9}".repeat(MAX_TEXT + 9)).chars().count(), MAX_TEXT);
}

#[test]
fn protocol_refuses_oversize_frame_before_allocating() {
    // A length over every kind's maximum: refused once the length and the tag were read, nothing more taken. (With
    // the check gone the reader would allocate the 4 GiB it claims and then find the pipe short: `Truncated`.)
    let mut bytes = u32::MAX.to_le_bytes().to_vec();
    bytes.push(T_FRAME);
    bytes.extend_from_slice(&[0; 64]);
    let mut r = counting(bytes);
    assert_eq!(read_to_app(&mut r).map(|_| ()), Err(WireError::TooLarge));
    assert_eq!(r.taken, 5, "only the length and the tag were read");

    // The largest frame plus one byte: over the frame's maximum, refused the same way.
    let max_frame = 1 + 8 + 4 + 4 + pixel_bytes(MAX_FRAME_W, MAX_FRAME_H);
    let mut bytes = ((max_frame + 1) as u32).to_le_bytes().to_vec();
    bytes.push(T_FRAME);
    let mut r = counting(bytes);
    assert_eq!(read_to_app(&mut r).map(|_| ()), Err(WireError::TooLarge));
    assert_eq!(r.taken, 5);

    // A frame wider than MAX_FRAME_W whose length says exactly its pixels: refused on its header, before the pixels.
    let (w, h) = (MAX_FRAME_W + 1, 1u32);
    let mut bytes = ((1 + 16 + w as usize * 4) as u32).to_le_bytes().to_vec();
    bytes.push(T_FRAME);
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&w.to_le_bytes());
    bytes.extend_from_slice(&h.to_le_bytes());
    bytes.extend_from_slice(&vec![0; w as usize * 4]);
    let mut r = counting(bytes);
    assert_eq!(read_to_app(&mut r).map(|_| ()), Err(WireError::TooLarge));
    assert_eq!(r.taken, 4 + 1 + 16, "the header, not one pixel");

    // A 2 x 2 frame whose length leaves 15 bytes for its 16: refused before the pixels are read.
    let mut bytes = (1u32 + 16 + 15).to_le_bytes().to_vec();
    bytes.push(T_FRAME);
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 15]);
    let mut r = counting(bytes);
    assert_eq!(read_to_app(&mut r).map(|_| ()), Err(WireError::Malformed));
    assert_eq!(r.taken, 4 + 1 + 16);

    // The same for the other bulk: a range over MAX_READ, PCM over a second, a title over MAX_TEXT_BYTES.
    let mut bytes = ((1 + 8 + MAX_READ as usize) as u32).to_le_bytes().to_vec();
    bytes.push(T_DATA);
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&(MAX_READ + 1).to_le_bytes());
    let mut r = counting(bytes);
    assert_eq!(read_to_helper(&mut r), Err(WireError::TooLarge));
    assert_eq!(r.taken, 4 + 1 + 8);

    let samples = (MAX_AUDIO_FRAMES + 1) * 2;
    let mut bytes = ((1 + 14 + 64) as u32).to_le_bytes().to_vec();
    bytes.push(T_AUDIO);
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&samples.to_le_bytes());
    let mut r = counting(bytes);
    assert_eq!(read_to_app(&mut r).map(|_| ()), Err(WireError::TooLarge));
    assert_eq!(r.taken, 4 + 1 + 14);

    let mut bytes = ((1 + 2 + 64) as u32).to_le_bytes().to_vec();
    bytes.push(T_TAGS);
    bytes.extend_from_slice(&((MAX_TEXT_BYTES + 1) as u16).to_le_bytes());
    let mut r = counting(bytes);
    assert_eq!(read_to_app(&mut r).map(|_| ()), Err(WireError::TooLarge));
    assert_eq!(r.taken, 4 + 1 + 2);
}

#[test]
fn protocol_refuses_truncated_message() {
    // Every message of both directions, cut at every length short of whole: never a message, never a panic.
    // Cut inside its length prefix or after it, it is `Truncated`; cut before its first byte, the pipe `Closed`.
    let mut wires: Vec<(Vec<u8>, bool)> = to_helper_messages().iter().map(|m| (wire(m), true)).collect();
    wires.extend(from_helper_messages().iter().map(|m| (wire(m), false)));
    for (bytes, to_helper) in wires {
        // The large ones are cut near their ends and at a few points inside; the small ones everywhere.
        let cuts: Vec<usize> = if bytes.len() <= 4096 {
            (0..bytes.len()).collect()
        } else {
            (0..64)
                .chain([bytes.len() / 2, bytes.len() - 2, bytes.len() - 1])
                .collect()
        };
        for cut in cuts {
            let mut r = Cursor::new(&bytes[..cut]);
            let got = if to_helper {
                read_to_helper(&mut r).map(|_| ())
            } else {
                read_to_app(&mut r).map(|_| ())
            };
            let want = if cut == 0 {
                WireError::Closed
            } else {
                WireError::Truncated
            };
            assert_eq!(got, Err(want), "cut at {cut} of {}", bytes.len());
        }
    }

    // A length that claims less than the fields need: refused (the fields never read past their message).
    let mut bytes = wire(&FromHelper::ReadAt {
        id: 1,
        offset: 2,
        len: 3,
    });
    let short = (bytes.len() - 4 - 1) as u32;
    bytes[..4].copy_from_slice(&short.to_le_bytes());
    assert_eq!(
        read_to_app(&mut Cursor::new(&bytes)).map(|_| ()),
        Err(WireError::Malformed)
    );

    // A length of zero (no tag): refused. A byte after the fields of a message shorter than its kind's maximum
    // (an `Opened` with no video and no audio): refused. (Past the maximum it is `TooLarge`, above.)
    assert_eq!(
        read_to_app(&mut Cursor::new(&[0u8, 0, 0, 0])).map(|_| ()),
        Err(WireError::Malformed)
    );
    let mut bytes = wire(&FromHelper::Opened(Opened {
        duration_ms: 1,
        video: None,
        audio: None,
        reduced_quality: false,
    }));
    bytes[0] += 1;
    bytes.push(0);
    assert_eq!(
        read_to_app(&mut Cursor::new(&bytes)).map(|_| ()),
        Err(WireError::Malformed)
    );
}
