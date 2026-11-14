//! What a picture's header declares, and the limits it is held to, on bytes made here: a real PNG from the `png`
//! encoder, and hand-built headers of each format (only the fields a decoder sizes its buffers from).

use super::*;

fn png_header(width: u32, height: u32, frames: Option<u32>) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    if let Some(frames) = frames {
        // acTL: the number of frames and of plays, then a CRC (not read here).
        out.extend_from_slice(&[0, 0, 0, 8]);
        out.extend_from_slice(b"acTL");
        out.extend_from_slice(&frames.to_be_bytes());
        out.extend_from_slice(&[0; 8]);
    }
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(b"IDAT");
    // After the first IDAT nothing counts: an acTL here is not one.
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 8]);
    out.extend_from_slice(b"acTL");
    out.extend_from_slice(&9_999u32.to_be_bytes());
    out
}

fn jpeg_header(width: u16, height: u16, frame_marker: u8) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    // An application segment of 14 bytes, a fill byte, then a table segment that is NOT a frame header (DHT).
    out.extend_from_slice(&[0xFF, 0xE0, 0, 16]);
    out.extend_from_slice(&[0; 14]);
    out.extend_from_slice(&[0xFF, 0xFF, 0xC4, 0, 4, 9, 9]);
    out.extend_from_slice(&[0xFF, frame_marker, 0, 11, 8]);
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[1, 1, 0x11, 0]);
    out
}

/// A GIF with `frames` images of `size`, the first of them placed at (`left`, 0), on a `screen` canvas.
fn gif(screen: (u16, u16), size: (u16, u16), frames: usize, left: u16) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&screen.0.to_le_bytes());
    out.extend_from_slice(&screen.1.to_le_bytes());
    // A global colour table of 2 entries.
    out.extend_from_slice(&[0x80, 0, 0]);
    out.extend_from_slice(&[0; 6]);
    for frame in 0..frames {
        // A graphic control extension before each image.
        out.extend_from_slice(&[0x21, 0xF9, 4, 0, 0, 0, 0, 0]);
        out.push(0x2C);
        out.extend_from_slice(&(if frame == 0 { left } else { 0 }).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&size.0.to_le_bytes());
        out.extend_from_slice(&size.1.to_le_bytes());
        // No local table; code size 2; one sub-block of 2 bytes; the end of the image data.
        out.extend_from_slice(&[0, 2, 2, 0x4C, 0x01, 0]);
    }
    out.push(0x3B);
    out
}

/// One chunk: its name, its length, its data, and a byte of padding after an odd length.
fn chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = name.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A WebP file around `body`, the bytes after its 12 bytes of file header.
fn file(body: &[u8]) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(body);
    out
}

pub(crate) fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let body: Vec<u8> = chunks.iter().flat_map(|(name, data)| chunk(name, data)).collect();
    file(&body)
}

fn webp_extended(width: u32, height: u32, frames: usize) -> Vec<u8> {
    let mut header = vec![0x02, 0, 0, 0];
    header.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    header.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    // A chunk of an odd length before the frames: the next chunk starts after one byte of padding.
    let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"VP8X", header), (b"ANIM", vec![0; 6]), (b"EXIF", vec![0; 3])];
    for _ in 0..frames {
        chunks.push((b"ANMF", frame(1, 1, &[(b"VP8 ", lossy(1, 1))])));
    }
    riff(&chunks)
}

fn still(width: u32, height: u32) -> Declared {
    Declared {
        width,
        height,
        frames: 1,
    }
}

#[test]
fn each_format_declares_its_size_from_the_fields_a_decoder_reads() {
    // A real PNG, from the encoder.
    let mut real = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut real, 37, 21);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&vec![9u8; 37 * 21 * 4]).unwrap();
    }
    assert_eq!(declared(&real), Some(still(37, 21)));
    assert_eq!(declared(&png_header(640, 480, None)), Some(still(640, 480)));
    // Baseline, progressive and lossless frame headers; the table segment before them is not one.
    for marker in [0xC0, 0xC2, 0xC3, 0xCF] {
        assert_eq!(
            declared(&jpeg_header(1_600, 900, marker)),
            Some(still(1_600, 900)),
            "{marker:#x}"
        );
    }
    assert_eq!(declared(&gif((320, 200), (320, 200), 1, 0)), Some(still(320, 200)));
    assert_eq!(declared(&webp_extended(4_000, 3_000, 0)), Some(still(4_000, 3_000)));
    // A lossy WebP: 14 bits of width and of height after the start code (the two top bits are a scale).
    let mut lossy = vec![0, 0, 0, 0x9D, 0x01, 0x2A];
    lossy.extend_from_slice(&(0xC000u16 | 800).to_le_bytes());
    lossy.extend_from_slice(&(0x4000u16 | 600).to_le_bytes());
    assert_eq!(declared(&riff(&[(b"VP8 ", lossy)])), Some(still(800, 600)));
    // A lossless WebP: 14 bits of width - 1, then 14 of height - 1.
    let packed: u32 = (1_023) | (767 << 14);
    let mut lossless = vec![0x2F];
    lossless.extend_from_slice(&packed.to_le_bytes());
    assert_eq!(declared(&riff(&[(b"VP8L", lossless)])), Some(still(1_024, 768)));
}

#[test]
fn an_animation_declares_its_frames() {
    assert_eq!(
        declared(&png_header(100, 80, Some(24))),
        Some(Declared {
            width: 100,
            height: 80,
            frames: 24
        }),
        "the frame count before the first IDAT, never one after it"
    );
    assert_eq!(declared(&gif((64, 48), (64, 48), 7, 0)).map(|d| d.frames), Some(7));
    assert_eq!(declared(&webp_extended(64, 48, 5)).map(|d| d.frames), Some(5));
}

#[test]
fn a_gif_image_placed_past_its_screen_counts_as_the_canvas_it_needs() {
    // A 10 x 10 screen with an image of 5,000 x 4,000 at 3,000 from the left.
    let declared = declared(&gif((10, 10), (5_000, 4_000), 1, 3_000)).unwrap();
    assert_eq!((declared.width, declared.height), (8_000, 4_000));
}

#[test]
fn what_is_not_one_of_the_four_formats_by_its_content_is_not_a_picture() {
    let others: [&[u8]; 7] = [
        b"",
        b"plain words",
        br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#,
        b"icns\0\0\0\x08",
        b"\0\0\0\x1cftypavif\0\0\0\0",
        b"BM\0\0\0\0",
        b"%PDF-1.7",
    ];
    for bytes in others {
        assert_eq!(check(bytes), Err(Refused::NotAPicture), "{bytes:?}");
    }
}

#[test]
fn a_header_cut_short_or_without_a_size_is_not_a_picture_and_nothing_is_read_past_the_end() {
    let whole = [
        png_header(640, 480, None),
        jpeg_header(1_600, 900, 0xC0),
        gif((320, 200), (320, 200), 2, 0),
        webp_extended(4_000, 3_000, 2),
    ];
    for bytes in &whole {
        assert!(check(bytes).is_ok());
        // Every prefix: never a panic; a size, when one is answered, is the file's own.
        for end in 0..bytes.len() {
            if let Some(found) = declared(&bytes[..end]) {
                let full = declared(bytes).unwrap();
                assert_eq!((found.width, found.height), (full.width, full.height), "{end}");
            }
        }
    }
    // A JPEG whose scan starts before any frame header, and one that ends: no size.
    assert_eq!(declared(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]), None);
    assert_eq!(declared(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
    // A size of zero is not a picture.
    assert_eq!(check(&png_header(0, 10, None)), Err(Refused::NotAPicture));
    assert_eq!(check(&jpeg_header(10, 0, 0xC0)), Err(Refused::NotAPicture));
}

#[test]
fn a_side_a_pixel_count_and_an_animations_total_each_have_their_own_limit() {
    let too_big = |bytes: Vec<u8>| matches!(check(&bytes), Err(Refused::TooBig(_)));
    // A side, with few pixels.
    assert!(check(&png_header(SIDE_MAX, 16, None)).is_ok());
    assert!(too_big(png_header(SIDE_MAX + 1, 16, None)));
    assert!(too_big(png_header(16, SIDE_MAX + 1, None)));
    // Pixels, with both sides within the side limit.
    assert!(u64::from(SIDE_MAX) * u64::from(SIDE_MAX) > PIXELS_MAX);
    assert!(
        check(&jpeg_header(8_000, 6_000, 0xC0)).is_ok(),
        "a 48-megapixel photo is shown"
    );
    assert!(too_big(jpeg_header(8_000, 6_300, 0xC0)));
    assert!(too_big(webp_extended(SIDE_MAX, SIDE_MAX, 0)));
    // An animation: each frame is small, all of them together are over what is decoded.
    let frame_bytes = 2_000u64 * 2_000 * 4;
    let fit = u32::try_from(DECODED_BYTES_MAX / frame_bytes).unwrap();
    assert!(check(&png_header(2_000, 2_000, Some(fit))).is_ok());
    assert!(too_big(png_header(2_000, 2_000, Some(fit + 1))));
    assert!(too_big(gif((2_000, 2_000), (2_000, 2_000), fit as usize + 1, 0)));
    assert!(check(&gif((2_000, 2_000), (2_000, 2_000), fit as usize, 0)).is_ok());
    assert!(too_big(webp_extended(2_000, 2_000, fit as usize + 1)));
    // The figures are handed over for the view to say.
    assert_eq!(
        check(&png_header(9_000, 7_000, None)),
        Err(Refused::TooBig(still(9_000, 7_000)))
    );
    // A still of the same canvas as one animation frame is not held to the animation's budget.
    assert!(check(&png_header(8_000, 6_000, None)).is_ok());
}

/// A lossy bitstream's first bytes, stating `width` x `height`.
fn lossy(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 0x9D, 0x01, 0x2A];
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out
}

/// A lossless bitstream's first bytes, stating `width` x `height`.
fn lossless(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x2F];
    out.extend_from_slice(&((width - 1) | ((height - 1) << 14)).to_le_bytes());
    out
}

/// The extended header of a `width` x `height` canvas.
fn canvas(width: u32, height: u32) -> Vec<u8> {
    flagged(0, width, height)
}

/// The extended header of a `width` x `height` canvas with `flags` (0x02: an animation; 0x10: it has alpha).
pub(crate) fn flagged(flags: u8, width: u32, height: u32) -> Vec<u8> {
    let mut header = vec![flags, 0, 0, 0];
    header.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    header.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    header
}

/// One animation frame of `width` x `height`, shown for a tenth of a second, holding `inner` chunks.
pub(crate) fn frame(width: u32, height: u32, inner: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0; 6];
    out.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    out.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    out.extend_from_slice(&[100, 0, 0, 0]);
    // The chunks of `riff`, without its 12 bytes of file header.
    out.extend_from_slice(&riff(inner)[12..]);
    out
}

#[test]
fn an_extended_webp_is_as_large_as_the_largest_size_stated_anywhere_inside_it() {
    // A decoder decodes each bitstream at the size that bitstream states, and looks at the canvas afterwards: a
    // 1 x 1 canvas in front of a 16383 x 16383 frame is a 16383 x 16383 decode.
    let huge = Declared {
        width: 16_383,
        height: 16_383,
        frames: 1,
    };
    let lossy_inside = riff(&[(b"VP8X", canvas(1, 1)), (b"VP8 ", lossy(16_383, 16_383))]);
    assert_eq!(declared(&lossy_inside), Some(huge));
    assert_eq!(check(&lossy_inside), Err(Refused::TooBig(huge)));
    let lossless_inside = riff(&[
        (b"VP8X", canvas(1, 1)),
        (b"ALPH", vec![0; 3]),
        (b"VP8L", lossless(16_383, 16_383)),
    ]);
    assert_eq!(check(&lossless_inside), Err(Refused::TooBig(huge)));
    // An animation: a frame's own size, and the bitstream inside a frame (after its alpha), each count; so does
    // the one in the third frame, not only the first.
    let small = || (b"ANMF", frame(1, 1, &[(b"VP8 ", lossy(1, 1))]));
    for big in [
        frame(16_383, 16_383, &[(b"VP8 ", lossy(1, 1))]),
        frame(1, 1, &[(b"VP8 ", lossy(16_383, 16_383))]),
        frame(1, 1, &[(b"VP8L", lossless(16_383, 16_383))]),
        frame(1, 1, &[(b"ALPH", vec![0; 5]), (b"VP8 ", lossy(16_383, 16_383))]),
    ] {
        let file = riff(&[
            (b"VP8X", canvas(1, 1)),
            (b"ANIM", vec![0; 6]),
            small(),
            small(),
            (b"ANMF", big),
        ]);
        let declared = declared(&file).expect("a size");
        assert_eq!((declared.width, declared.height, declared.frames), (16_383, 16_383, 3));
        assert!(matches!(check(&file), Err(Refused::TooBig(_))));
    }
    // A bitstream whose header cannot be read: no size is known, so it is not a picture to decode.
    for unreadable in [
        riff(&[(b"VP8X", canvas(64, 48)), (b"VP8 ", vec![0; 10])]),
        riff(&[(b"VP8X", canvas(64, 48)), (b"VP8L", vec![0x2F, 0])]),
        riff(&[
            (b"VP8X", canvas(64, 48)),
            (b"ANMF", frame(64, 48, &[(b"VP8 ", vec![0; 4])])),
        ]),
        // After an alpha the decoder reads a lossy bitstream whatever the chunk is named: a lossless one there
        // has no size the check can vouch for.
        riff(&[
            (b"VP8X", canvas(64, 48)),
            (
                b"ANMF",
                frame(1, 1, &[(b"ALPH", vec![0; 5]), (b"VP8L", lossless(16_383, 16_383))]),
            ),
        ]),
    ] {
        assert_eq!(check(&unreadable), Err(Refused::NotAPicture));
    }
    // What agrees with its canvas is that canvas, and passes.
    let agrees = riff(&[(b"VP8X", canvas(640, 480)), (b"VP8 ", lossy(640, 480))]);
    assert_eq!(check(&agrees), Ok(still(640, 480)));
    let animated = riff(&[
        (b"VP8X", canvas(640, 480)),
        (b"ANMF", frame(640, 480, &[(b"VP8L", lossless(640, 480))])),
        (b"ANMF", frame(320, 240, &[(b"VP8 ", lossy(320, 240))])),
    ]);
    assert_eq!(
        check(&animated).map(|d| (d.width, d.height, d.frames)),
        Ok((640, 480, 2))
    );
}

// ── WebP animation frames: accounted for byte for byte, the way the decoder walks them ───────────────

/// A real lossy bitstream of 16 x 16, from an encoder.
pub(crate) const LOSSY: [u8; 24] = [
    0x50, 0x01, 0x00, 0x9D, 0x01, 0x2A, 0x10, 0x00, 0x10, 0x00, 0x02, 0x00, 0x34, 0x25, 0xA4, 0x00, 0x04, 0x74, 0x00,
    0x00, 0xE4, 0x40, 0x00, 0x00,
];

/// A real lossless bitstream of 8 x 6 with alpha, from an encoder.
pub(crate) const LOSSLESS: [u8; 180] = [
    0x2F, 0x07, 0x40, 0x01, 0x10, 0xCD, 0x55, 0x20, 0x22, 0x02, 0x1E, 0x88, 0x04, 0x00, 0x00, 0x00, 0x00, 0x80, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x78, 0x20, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xE7, 0x1F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE0,
    0x81, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9C, 0x7F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x80, 0x07, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x70, 0xFE, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF9, 0x83, 0x06,
];

/// An alpha chunk of `width` x `height`, not compressed: every pixel opaque.
pub(crate) fn alpha(width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0];
    out.resize(1 + width * height, 0xFF);
    out
}

/// [`LOSSY`] with the size it states changed to 16383 x 16383 and nothing else: a lossy decoder sizes its
/// buffers from these bytes before it reads any further.
fn lossy_stating_huge() -> Vec<u8> {
    let mut out = LOSSY.to_vec();
    out[6..10].copy_from_slice(&[0xFF, 0x3F, 0xFF, 0x3F]);
    out
}

/// The WebP files an encoder writes, each with the canvas it shows: a lossy and a lossless still, a lossy still
/// with alpha, and an animation of two frames of each of the three.
pub(crate) fn webps_an_encoder_writes() -> Vec<(&'static str, Vec<u8>, (u32, u32))> {
    let animation = |flags: u8, size: (u32, u32), inner: &[(&[u8; 4], Vec<u8>)]| {
        riff(&[
            (b"VP8X", flagged(flags, size.0, size.1)),
            (b"ANIM", vec![0; 6]),
            (b"ANMF", frame(size.0, size.1, inner)),
            (b"ANMF", frame(size.0, size.1, inner)),
        ])
    };
    let with_alpha: [(&[u8; 4], Vec<u8>); 2] = [(b"ALPH", alpha(16, 16)), (b"VP8 ", LOSSY.to_vec())];
    vec![
        ("a lossy still", riff(&[(b"VP8 ", LOSSY.to_vec())]), (16, 16)),
        (
            "a lossless still with alpha",
            riff(&[(b"VP8L", LOSSLESS.to_vec())]),
            (8, 6),
        ),
        (
            "a lossy still with alpha",
            riff(&[
                (b"VP8X", flagged(0x10, 16, 16)),
                with_alpha[0].clone(),
                with_alpha[1].clone(),
            ]),
            (16, 16),
        ),
        (
            "a lossy animation",
            animation(0x02, (16, 16), &[(b"VP8 ", LOSSY.to_vec())]),
            (16, 16),
        ),
        (
            "a lossless animation with alpha",
            animation(0x12, (8, 6), &[(b"VP8L", LOSSLESS.to_vec())]),
            (8, 6),
        ),
        (
            "a lossy animation with alpha",
            animation(0x12, (16, 16), &with_alpha),
            (16, 16),
        ),
    ]
}

/// Two animations of a 512 x 512 canvas whose frames the decoder reads differently from a walk by chunk name
/// and padded length: in each, the decoder reaches a lossy bitstream that states 16383 x 16383.
pub(crate) fn webps_with_a_frame_not_accounted_for() -> [(&'static str, Vec<u8>); 2] {
    let huge = lossy_stating_huge();
    let after_alpha = riff(&[
        (b"VP8X", flagged(0x12, 512, 512)),
        (b"ANIM", vec![0; 6]),
        (
            b"ANMF",
            frame(16, 16, &[(b"ALPH", alpha(16, 16)), (b"JUNK", huge.clone())]),
        ),
    ]);
    let mut first = frame(16, 16, &[(b"VP8 ", LOSSY.to_vec())]);
    first.push(0);
    let mut body = chunk(b"VP8X", &flagged(0x02, 512, 512));
    body.extend(chunk(b"ANIM", &[0; 6]));
    body.extend_from_slice(b"ANMF");
    body.extend_from_slice(&(first.len() as u32).to_le_bytes());
    body.extend_from_slice(&first);
    let mut second = frame(16, 16, &[(b"VP8 ", huge)]);
    let covered = second.len() + (second.len() & 1);
    second.resize(1 + covered, 0);
    second.extend(chunk(b"ANMF", &frame(16, 16, &[(b"XXXX", Vec::new())])));
    body.extend_from_slice(b"ANMF");
    body.extend_from_slice(&((covered as u32) << 8).to_le_bytes());
    body.extend_from_slice(&second);
    [
        ("after a frame's alpha", after_alpha),
        ("after a frame of an odd length", file(&body)),
    ]
}

#[test]
fn a_webp_frame_the_decoder_reads_differently_from_the_check_is_not_a_picture() {
    // The two measured files: before this rule each was answered as its 512 x 512 canvas, and allowed.
    for (which, bytes) in webps_with_a_frame_not_accounted_for() {
        assert_eq!(check(&bytes), Err(Refused::NotAPicture), "{which}");
    }
}

#[test]
fn a_webp_frame_is_one_bitstream_after_at_most_an_alpha_and_nothing_else() {
    let animation = |frames: Vec<Vec<u8>>| {
        let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"VP8X", flagged(0x12, 64, 48)), (b"ANIM", vec![0; 6])];
        chunks.extend(frames.into_iter().map(|data| (b"ANMF", data)));
        riff(&chunks)
    };
    let good = || frame(16, 16, &[(b"VP8 ", LOSSY.to_vec())]);
    assert_eq!(
        check(&animation(vec![good(), good()])).map(|d| (d.width, d.height, d.frames)),
        Ok((64, 48, 2))
    );
    // After a frame's alpha the decoder decodes the next chunk as lossy WHATEVER its name: only a chunk named
    // lossy is one, and it is sized as lossy.
    let huge = Declared {
        width: 16_383,
        height: 16_383,
        frames: 2,
    };
    let after_alpha = |name: &[u8; 4], data: Vec<u8>| {
        animation(vec![good(), frame(16, 16, &[(b"ALPH", alpha(16, 16)), (name, data)])])
    };
    assert_eq!(
        check(&after_alpha(b"VP8 ", lossy_stating_huge())),
        Err(Refused::TooBig(huge))
    );
    assert!(check(&after_alpha(b"VP8 ", LOSSY.to_vec())).is_ok());
    for name in [b"VP8L", b"JUNK", b"ALPH", b"ANMF"] {
        for data in [LOSSY.to_vec(), lossy_stating_huge(), LOSSLESS.to_vec()] {
            assert_eq!(
                check(&after_alpha(name, data)),
                Err(Refused::NotAPicture),
                "{}",
                String::from_utf8_lossy(name)
            );
        }
    }
    // A frame the walk cannot account for byte for byte, each in the second place and in the first.
    let mut odd = good();
    odd.push(0);
    let mut cut_short = good();
    cut_short.truncate(cut_short.len() - 2);
    let mut longer = good();
    longer.extend_from_slice(&[0; 2]);
    let unaccounted = [
        ("an odd length", odd),
        ("no chunk at all", vec![0; 16]),
        ("a chunk of another name", frame(16, 16, &[(b"JUNK", LOSSY.to_vec())])),
        (
            "an alpha and nothing after it",
            frame(16, 16, &[(b"ALPH", alpha(16, 16))]),
        ),
        (
            "a chunk after the bitstream",
            frame(16, 16, &[(b"VP8 ", LOSSY.to_vec()), (b"JUNK", lossy_stating_huge())]),
        ),
        (
            "a second bitstream",
            frame(16, 16, &[(b"VP8 ", LOSSY.to_vec()), (b"VP8 ", LOSSY.to_vec())]),
        ),
        ("bytes after the bitstream", longer),
        ("a bitstream that runs past the frame", cut_short),
        (
            "a bitstream shorter than its header",
            frame(16, 16, &[(b"VP8 ", LOSSY[..8].to_vec())]),
        ),
        (
            "a lossless bitstream shorter than its header",
            frame(16, 16, &[(b"VP8L", LOSSLESS[..4].to_vec())]),
        ),
    ];
    for (which, bad) in unaccounted {
        assert_eq!(
            check(&animation(vec![good(), bad.clone()])),
            Err(Refused::NotAPicture),
            "{which}, second"
        );
        assert_eq!(
            check(&animation(vec![bad, good()])),
            Err(Refused::NotAPicture),
            "{which}, first"
        );
    }
    // A last frame that states more bytes than the file has.
    let whole = animation(vec![good(), good()]);
    assert_eq!(check(&whole[..whole.len() - 2]), Err(Refused::NotAPicture));
    // The same frames in a file whose header does not say it is an animation: held to the same rule.
    let unflagged = |data: Vec<u8>| riff(&[(b"VP8X", flagged(0, 64, 48)), (b"ANMF", data)]);
    assert!(check(&unflagged(good())).is_ok());
    assert_eq!(
        check(&unflagged(frame(16, 16, &[(b"JUNK", lossy_stating_huge())]))),
        Err(Refused::NotAPicture)
    );
}

#[test]
fn every_webp_an_encoder_writes_passes_with_its_own_canvas() {
    for (which, bytes, (width, height)) in webps_an_encoder_writes() {
        assert_eq!(
            check(&bytes).map(|d| (d.width, d.height)),
            Ok((width, height)),
            "{which}"
        );
    }
}
