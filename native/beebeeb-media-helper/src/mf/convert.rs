//! The pure half of the system-decoder route: what Media Foundation's values mean, and a decoded 4:2:0 picture as
//! RGBA. No Windows call here, so all of it is tested on every system.

use beebeeb_media_protocol::{Codec, ErrorCode};

/// `MF_E_TOPO_CODEC_NOT_FOUND`: this PC has no decoder for the stream (HEVC without Microsoft's extension; measured:
/// every output type is refused with it).
pub const CODEC_NOT_FOUND: u32 = 0xC00D_5212;
/// `MF_E_INVALIDMEDIATYPE`: a decoder is listed but would not start for the stream (measured in a sandbox that can see
/// the HEVC decoder but not start it). Not "missing", and never shown as such.
pub const INVALID_MEDIA_TYPE: u32 = 0xC00D_36B4;
/// `MF_E_UNSUPPORTED_BYTESTREAM_TYPE`: Windows has no reader for this container.
pub const UNSUPPORTED_CONTAINER: u32 = 0xC00D_36C4;
/// `MF_E_INVALID_FILE_FORMAT`: the container is damaged.
pub const INVALID_FILE_FORMAT: u32 = 0xC00D_36BE;
/// `MF_E_INVALID_STREAM_DATA`: the stream's data is damaged.
pub const INVALID_STREAM_DATA: u32 = 0xC00D_36CB;
/// `E_FAIL` at the first read: the decoder took the type and then refused the stream (10-bit and 4:2:2 H.264,
/// measured).
pub const FAILED: u32 = 0x8000_4005;

/// Whether an `E_FAIL` at the first read of this codec means the decoder refused the stream's bit depth (10-bit
/// H.264 and HEVC Main 10, measured). Every codec is named, no wildcard (ux-26).
fn system_rejects_bit_depth(codec: Codec) -> bool {
    match codec {
        Codec::H264 | Codec::Hevc => true,
        Codec::Vp8
        | Codec::Vp9
        | Codec::Av1
        | Codec::Mpeg2
        | Codec::ProRes
        | Codec::Dnx
        | Codec::Mp3
        | Codec::Aac
        | Codec::Flac
        | Codec::Alac
        | Codec::Opus
        | Codec::Vorbis
        | Codec::Pcm
        | Codec::Ac3
        | Codec::Eac3
        | Codec::Mp2
        | Codec::Mpeg4Part2
        | Codec::Vc1
        | Codec::Wma
        | Codec::Other => false,
    }
}

/// What a failed Media Foundation call on a `System` stream of `codec` tells the app. A failure of the app's own reads
/// is decided before this (the session's read flags win).
pub fn failure(hresult: u32, codec: Codec) -> ErrorCode {
    match hresult {
        CODEC_NOT_FOUND => ErrorCode::NoSystemDecoder(codec),
        FAILED if system_rejects_bit_depth(codec) => ErrorCode::TenBit,
        UNSUPPORTED_CONTAINER | FAILED => ErrorCode::NoDecoder,
        INVALID_FILE_FORMAT | INVALID_STREAM_DATA => ErrorCode::Corrupt,
        // INVALID_MEDIA_TYPE among them: a decoder that is there but did not start is not a missing one.
        _ => ErrorCode::Internal,
    }
}

/// The YUV → RGB matrix of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
}

impl Matrix {
    /// `(Kr, Kb)`.
    fn weights(self) -> (f32, f32) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020 => (0.2627, 0.0593),
        }
    }
}

/// The matrix a media type names (`MF_MT_YUV_MATRIX`: 1 BT.709, 2 BT.601, 3 SMPTE 240M, 4 and 5 BT.2020), or the usual
/// one for its height when it names none: BT.709 from 720 lines up, else BT.601.
pub fn matrix_of(named: Option<u32>, height: u32) -> Matrix {
    match named {
        Some(1) | Some(3) => Matrix::Bt709,
        Some(2) => Matrix::Bt601,
        Some(4) | Some(5) => Matrix::Bt2020,
        _ if height >= 720 => Matrix::Bt709,
        _ => Matrix::Bt601,
    }
}

/// Whether a media type's nominal range (`MF_MT_VIDEO_NOMINAL_RANGE`) is full range: 1 is 0–255; anything else, or
/// none, is the video range 16–235.
pub fn full_range_of(named: Option<u32>) -> bool {
    named == Some(1)
}

/// The two halves of a packed 64-bit attribute (`MF_MT_FRAME_SIZE`, `MF_MT_PIXEL_ASPECT_RATIO`): high, then low.
pub fn halves(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

/// The visible part of a decoded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The visible area from an `MF_MT_MINIMUM_DISPLAY_APERTURE` blob (`MFVideoArea`: offset x and y as 16.16 fixed
/// point, then the size as two 32-bit values), held inside the `width × height` picture. A missing, empty or
/// out-of-range aperture is the whole picture.
pub fn area_of(blob: Option<[u8; 16]>, width: u32, height: u32) -> Area {
    let whole = Area {
        x: 0,
        y: 0,
        w: width,
        h: height,
    };
    let Some(b) = blob else {
        return whole;
    };
    let x = i16::from_le_bytes([b[2], b[3]]);
    let y = i16::from_le_bytes([b[6], b[7]]);
    let w = i32::from_le_bytes([b[8], b[9], b[10], b[11]]);
    let h = i32::from_le_bytes([b[12], b[13], b[14], b[15]]);
    if x < 0 || y < 0 || w <= 0 || h <= 0 {
        return whole;
    }
    let (x, y, w, h) = (x as u32, y as u32, w as u32, h as u32);
    if u64::from(x) + u64::from(w) > u64::from(width) || u64::from(y) + u64::from(h) > u64::from(height) {
        return whole;
    }
    Area { x, y, w, h }
}

/// A decoded 4:2:0 picture in one buffer, as Media Foundation hands it: the luma plane (`stride` bytes per row,
/// `height` rows), then the interleaved chroma plane (`stride` bytes per row, `height / 2` rows, rounded up). 8-bit
/// NV12, or P010 (`ten_bit`: 16-bit little-endian samples with the value in the top 10 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Planes {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub ten_bit: bool,
}

impl Planes {
    /// The bytes the buffer must hold.
    pub fn byte_len(&self) -> Option<usize> {
        let stride = self.stride as usize;
        let rows = (self.height as usize).checked_add((self.height as usize).div_ceil(2))?;
        stride.checked_mul(rows)
    }
}

/// `area` of the picture in `buf` as RGBA (`area.w × area.h × 4` bytes, opaque), with the stream's matrix and range.
/// `None` when the buffer is shorter than the planes, the stride is narrower than a row, a size is zero or the area is
/// outside the picture.
pub fn yuv420_to_rgba(buf: &[u8], planes: Planes, area: Area, matrix: Matrix, full_range: bool) -> Option<Vec<u8>> {
    let sample_bytes = if planes.ten_bit { 2usize } else { 1 };
    let (width, height, stride) = (planes.width as usize, planes.height as usize, planes.stride as usize);
    if width == 0 || height == 0 || area.w == 0 || area.h == 0 {
        return None;
    }
    if stride < width.checked_mul(sample_bytes)? || buf.len() < planes.byte_len()? {
        return None;
    }
    if area.x as usize + area.w as usize > width || area.y as usize + area.h as usize > height {
        return None;
    }
    let chroma_rows = height.div_ceil(2);
    let chroma_at = stride * height;
    let bits = if planes.ten_bit { 10 } else { 8 };
    let max = f32::from((1u16 << bits) - 1);
    let half = f32::from(1u16 << (bits - 1));
    let scale = f32::from(1u16 << (bits - 8));
    let (luma_low, luma_span, chroma_span) = if full_range {
        (0.0, max, max)
    } else {
        (16.0 * scale, 219.0 * scale, 224.0 * scale)
    };
    let (kr, kb) = matrix.weights();
    let kg = 1.0 - kr - kb;
    let sample = |at: usize| -> f32 {
        if planes.ten_bit {
            f32::from(u16::from_le_bytes([buf[at], buf[at + 1]]) >> 6)
        } else {
            f32::from(buf[at])
        }
    };
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;

    let mut out = Vec::with_capacity(area.w as usize * area.h as usize * 4);
    for row in area.y as usize..(area.y + area.h) as usize {
        let chroma_row = (row / 2).min(chroma_rows - 1);
        for column in area.x as usize..(area.x + area.w) as usize {
            let y = (sample(row * stride + column * sample_bytes) - luma_low) / luma_span;
            // One Cb, Cr pair per two columns.
            let pair = chroma_at + chroma_row * stride + (column / 2) * 2 * sample_bytes;
            let cb = (sample(pair) - half) / chroma_span;
            let cr = (sample(pair + sample_bytes) - half) / chroma_span;
            let r = y + 2.0 * (1.0 - kr) * cr;
            let b = y + 2.0 * (1.0 - kb) * cb;
            let g = (y - kr * r - kb * b) / kg;
            out.extend_from_slice(&[byte(r), byte(g), byte(b), 255]);
        }
    }
    Some(out)
}

/// Clockwise quarter turns to show a frame upright, from `MF_MT_VIDEO_ROTATION` (degrees: 0, 90, 180, 270). Media
/// Foundation names the turn the stored frame has against upright, counter-clockwise; undoing it is the same number
/// of degrees clockwise. An iPhone's portrait video (stored landscape) is 90: one clockwise turn, as FilmCraft turns it.
pub fn quarter_turns(degrees: Option<u32>) -> u8 {
    match degrees {
        Some(90) => 1,
        Some(180) => 2,
        Some(270) => 3,
        _ => 0,
    }
}

/// `rgba` (`w × h`) turned clockwise by `turns` quarter turns, with its new size.
pub fn rotate_rgba(rgba: &[u8], w: u32, h: u32, turns: u8) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h) = (w as usize, h as usize);
    if rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let turns = turns % 4;
    if turns == 0 {
        return Some((rgba.to_vec(), w as u32, h as u32));
    }
    let (nw, nh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    let mut out = vec![0u8; rgba.len()];
    for y in 0..nh {
        for x in 0..nw {
            // Where the new pixel (x, y) comes from in the stored frame.
            let (sx, sy) = match turns {
                1 => (y, h - 1 - x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (w - 1 - y, x),
            };
            let from = (sy * w + sx) * 4;
            let to = (y * nw + x) * 4;
            out[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    Some((out, nw as u32, nh as u32))
}

/// A name for the source resolver to start from, by FilmCraft's container name: Windows reads the bytes either way
/// (the resolver may look past a wrong hint), this only saves it guessing. The helper has no file name.
pub fn container_hint(container: &str) -> &'static str {
    let c = container.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| c.contains(n));
    if has(&["quicktime", "mov"]) {
        "media.mov"
    } else if has(&["webm"]) {
        "media.webm"
    } else if has(&["matroska", "mkv"]) {
        "media.mkv"
    } else if has(&["avi"]) {
        "media.avi"
    } else if has(&["asf", "wmv", "wma", "windows media"]) {
        "media.asf"
    } else if has(&["transport", "mpeg-ts", "m2ts", "mpegts"]) {
        "media.ts"
    } else if has(&["adts"]) {
        "media.aac"
    } else if has(&["e-ac-3", "e-ac3", "eac3"]) {
        "media.ec3"
    } else if has(&["ac-3", "ac3"]) {
        "media.ac3"
    } else if has(&["3gp"]) {
        "media.3gp"
    } else {
        "media.mp4"
    }
}

/// Media Foundation's time (100-nanosecond units) in milliseconds; a time before 0 is 0.
pub fn pts_ms(hundred_ns: i64) -> u64 {
    (hundred_ns.max(0) / 10_000) as u64
}

/// Milliseconds in Media Foundation's time.
pub fn hundred_ns(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX).saturating_mul(10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w × h` NV12 picture with every luma sample `y` and every chroma pair `(u, v)`, rows `stride` bytes long.
    fn nv12(w: u32, h: u32, stride: u32, y: u8, u: u8, v: u8) -> Vec<u8> {
        let mut buf = vec![0u8; stride as usize * (h as usize + (h as usize).div_ceil(2))];
        for row in 0..h as usize {
            for col in 0..w as usize {
                buf[row * stride as usize + col] = y;
            }
        }
        let chroma = stride as usize * h as usize;
        for row in 0..(h as usize).div_ceil(2) {
            for pair in 0..(w as usize).div_ceil(2) {
                buf[chroma + row * stride as usize + pair * 2] = u;
                buf[chroma + row * stride as usize + pair * 2 + 1] = v;
            }
        }
        buf
    }

    fn whole(w: u32, h: u32) -> Area {
        Area { x: 0, y: 0, w, h }
    }

    fn planes(w: u32, h: u32, stride: u32) -> Planes {
        Planes {
            width: w,
            height: h,
            stride,
            ten_bit: false,
        }
    }

    #[test]
    fn video_range_white_grey_and_black_convert_exactly() {
        for (y, want) in [(235u8, 255u8), (16, 0), (126, 128)] {
            let buf = nv12(4, 2, 4, y, 128, 128);
            let rgba = yuv420_to_rgba(&buf, planes(4, 2, 4), whole(4, 2), Matrix::Bt709, false).unwrap();
            assert_eq!(rgba.len(), 4 * 2 * 4);
            assert!(
                rgba.chunks_exact(4).all(|px| px == [want, want, want, 255]),
                "luma {y}: {rgba:?}"
            );
        }
    }

    #[test]
    fn full_range_uses_the_whole_scale() {
        let buf = nv12(2, 2, 2, 255, 128, 128);
        let rgba = yuv420_to_rgba(&buf, planes(2, 2, 2), whole(2, 2), Matrix::Bt601, true).unwrap();
        assert_eq!(&rgba[..4], &[255, 255, 255, 255]);
        let buf = nv12(2, 2, 2, 235, 128, 128);
        let rgba = yuv420_to_rgba(&buf, planes(2, 2, 2), whole(2, 2), Matrix::Bt601, true).unwrap();
        assert_eq!(rgba[0], 235, "full range: 235 is not white");
    }

    /// BT.709 red in video range is Y 63, Cb 102, Cr 240; BT.601 red is Y 81, Cb 90, Cr 240. Each matrix gives red
    /// only with its own numbers.
    #[test]
    fn each_matrix_turns_its_own_red_into_red() {
        let close = |px: &[u8], want: [u8; 3]| px.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2);
        let red709 = nv12(2, 2, 2, 63, 102, 240);
        let px = yuv420_to_rgba(&red709, planes(2, 2, 2), whole(2, 2), Matrix::Bt709, false).unwrap();
        assert!(close(&px[..3], [255, 0, 0]), "{px:?}");
        let red601 = nv12(2, 2, 2, 81, 90, 240);
        let px = yuv420_to_rgba(&red601, planes(2, 2, 2), whole(2, 2), Matrix::Bt601, false).unwrap();
        assert!(close(&px[..3], [255, 0, 0]), "{px:?}");
        // The instrument can fail: BT.709's numbers read as BT.601 are not red.
        let px = yuv420_to_rgba(&red709, planes(2, 2, 2), whole(2, 2), Matrix::Bt601, false).unwrap();
        assert!(!close(&px[..3], [255, 0, 0]), "{px:?}");
    }

    #[test]
    fn ten_bit_samples_sit_in_the_top_bits() {
        // P010 white: luma 940 and chroma 512, shifted up by 6.
        let (w, h, stride) = (2u32, 2u32, 4u32);
        let mut buf = vec![0u8; stride as usize * 3];
        for at in (0..stride as usize * 2).step_by(2) {
            buf[at..at + 2].copy_from_slice(&(940u16 << 6).to_le_bytes());
        }
        for at in (stride as usize * 2..stride as usize * 3).step_by(2) {
            buf[at..at + 2].copy_from_slice(&(512u16 << 6).to_le_bytes());
        }
        let ten = Planes {
            width: w,
            height: h,
            stride,
            ten_bit: true,
        };
        let rgba = yuv420_to_rgba(&buf, ten, whole(w, h), Matrix::Bt2020, false).unwrap();
        assert!(rgba.chunks_exact(4).all(|px| px == [255, 255, 255, 255]), "{rgba:?}");
    }

    #[test]
    fn the_stride_padding_and_the_hidden_rows_are_not_shown() {
        // 4 × 4 decoded, rows padded to 8 bytes; only the top-left 2 × 2 is visible. The padding and the other pixels
        // are black, the visible ones white.
        let mut buf = nv12(4, 4, 8, 16, 128, 128);
        for row in 0..2 {
            for col in 0..2 {
                buf[row * 8 + col] = 235;
            }
        }
        let area = Area { x: 0, y: 0, w: 2, h: 2 };
        let rgba = yuv420_to_rgba(&buf, planes(4, 4, 8), area, Matrix::Bt709, false).unwrap();
        assert_eq!(rgba.len(), 2 * 2 * 4);
        assert!(rgba.chunks_exact(4).all(|px| px == [255, 255, 255, 255]), "{rgba:?}");
    }

    #[test]
    fn a_short_buffer_a_narrow_stride_or_an_area_outside_is_refused() {
        let buf = nv12(4, 2, 4, 128, 128, 128);
        assert!(
            yuv420_to_rgba(
                &buf[..buf.len() - 1],
                planes(4, 2, 4),
                whole(4, 2),
                Matrix::Bt709,
                false
            )
            .is_none()
        );
        assert!(yuv420_to_rgba(&buf, planes(4, 2, 3), whole(4, 2), Matrix::Bt709, false).is_none());
        let outside = Area { x: 1, y: 0, w: 4, h: 2 };
        assert!(yuv420_to_rgba(&buf, planes(4, 2, 4), outside, Matrix::Bt709, false).is_none());
        assert!(yuv420_to_rgba(&buf, planes(0, 2, 4), whole(0, 2), Matrix::Bt709, false).is_none());
        assert!(yuv420_to_rgba(&buf, planes(4, 2, 4), whole(4, 2), Matrix::Bt709, false).is_some());
    }

    #[test]
    fn an_aperture_inside_the_picture_is_kept_and_one_outside_is_the_whole_picture() {
        let blob = |x: i16, y: i16, w: i32, h: i32| {
            let mut b = [0u8; 16];
            b[2..4].copy_from_slice(&x.to_le_bytes());
            b[6..8].copy_from_slice(&y.to_le_bytes());
            b[8..12].copy_from_slice(&w.to_le_bytes());
            b[12..16].copy_from_slice(&h.to_le_bytes());
            b
        };
        // 1080p coded as 1088 lines.
        assert_eq!(
            area_of(Some(blob(0, 0, 1920, 1080)), 1920, 1088),
            Area {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080
            }
        );
        assert_eq!(area_of(Some(blob(0, 0, 1920, 1089)), 1920, 1088), whole(1920, 1088));
        assert_eq!(area_of(Some(blob(-1, 0, 8, 8)), 1920, 1088), whole(1920, 1088));
        assert_eq!(area_of(None, 640, 360), whole(640, 360));
    }

    #[test]
    fn media_type_values_read_as_documented() {
        assert_eq!(halves((1920u64 << 32) | 1080), (1920, 1080));
        assert_eq!(matrix_of(Some(1), 480), Matrix::Bt709);
        assert_eq!(matrix_of(Some(2), 2160), Matrix::Bt601);
        assert_eq!(matrix_of(Some(4), 2160), Matrix::Bt2020);
        assert_eq!(matrix_of(None, 1080), Matrix::Bt709);
        assert_eq!(matrix_of(Some(0), 576), Matrix::Bt601);
        assert!(full_range_of(Some(1)) && !full_range_of(Some(2)) && !full_range_of(None));
        assert_eq!(pts_ms(400_000), 40);
        assert_eq!(pts_ms(-5), 0);
        assert_eq!(hundred_ns(40), 400_000);
        assert_eq!(hundred_ns(u64::MAX), i64::MAX);
    }

    /// The honest words depend on this: only "codec not found" means the PC lacks the decoder.
    #[test]
    fn only_codec_not_found_says_the_system_decoder_is_missing() {
        assert_eq!(
            failure(CODEC_NOT_FOUND, Codec::Hevc),
            ErrorCode::NoSystemDecoder(Codec::Hevc)
        );
        assert_eq!(failure(INVALID_MEDIA_TYPE, Codec::Hevc), ErrorCode::Internal);
        assert_eq!(failure(UNSUPPORTED_CONTAINER, Codec::Aac), ErrorCode::NoDecoder);
        assert_eq!(failure(FAILED, Codec::H264), ErrorCode::TenBit);
        assert_eq!(failure(FAILED, Codec::Hevc), ErrorCode::TenBit);
        assert_eq!(failure(FAILED, Codec::Aac), ErrorCode::NoDecoder);
        assert_eq!(failure(INVALID_STREAM_DATA, Codec::Aac), ErrorCode::Corrupt);
        assert_eq!(failure(0x8007_0005, Codec::Aac), ErrorCode::Internal);
    }

    /// A 2 × 1 frame, left pixel A and right pixel B. One clockwise turn puts A on top (an iPhone's portrait video:
    /// the left of the stored frame is the top of the picture); three put it at the bottom.
    #[test]
    fn a_quarter_turn_clockwise_puts_the_left_on_top() {
        let (a, b) = ([1u8, 1, 1, 255], [2u8, 2, 2, 255]);
        let frame = [a, b].concat();
        assert_eq!(rotate_rgba(&frame, 2, 1, 1), Some(([a, b].concat(), 1, 2)));
        assert_eq!(rotate_rgba(&frame, 2, 1, 3), Some(([b, a].concat(), 1, 2)));
        assert_eq!(rotate_rgba(&frame, 2, 1, 2), Some(([b, a].concat(), 2, 1)));
        assert_eq!(rotate_rgba(&frame, 2, 1, 0), Some((frame.clone(), 2, 1)));
        // 2 × 2: A B / C D turned once clockwise is C A / D B.
        let (c, d) = ([3u8, 3, 3, 255], [4u8, 4, 4, 255]);
        let square = [a, b, c, d].concat();
        assert_eq!(rotate_rgba(&square, 2, 2, 1), Some(([c, a, d, b].concat(), 2, 2)));
        assert_eq!(rotate_rgba(&square[..12], 2, 2, 1), None);
        assert_eq!(quarter_turns(Some(90)), 1);
        assert_eq!(quarter_turns(Some(270)), 3);
        assert_eq!(quarter_turns(Some(45)), 0);
        assert_eq!(quarter_turns(None), 0);
    }

    #[test]
    fn container_names_give_a_hint_the_resolver_knows() {
        assert_eq!(container_hint("QuickTime"), "media.mov");
        assert_eq!(container_hint("MP4"), "media.mp4");
        assert_eq!(container_hint("WebM"), "media.webm");
        assert_eq!(container_hint("Matroska"), "media.mkv");
        assert_eq!(container_hint("AVI"), "media.avi");
        assert_eq!(container_hint("ADTS"), "media.aac");
        // Row 14 (MCP2): a bare E-AC-3 / AC-3 stream's hint.
        assert_eq!(container_hint("E-AC-3"), "media.ec3");
        assert_eq!(container_hint("AC-3"), "media.ac3");
        assert_eq!(container_hint(""), "media.mp4");
    }
}
