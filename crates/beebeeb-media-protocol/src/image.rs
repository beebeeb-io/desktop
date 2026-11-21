//! One picture (HEIC/HEIF, a camera's RAW) decoded by the sandboxed helper: the image messages (task 048).
//!
//! The helper is started with [`IMAGE_MODE_ARG`] for a picture: then its first message from the app is
//! [`DecodeImage`], its bytes come in by the same path a video's do (the helper sends [`crate::FromHelper::ReadAt`],
//! the app answers [`crate::ToHelper::Data`] / `DataFailed`, or `Close` to give up), and its last message is one
//! [`ImageAnswer`]; then it exits. The app reads the helper's side with [`read_image_to_app`], which knows only
//! `ReadAt` and the two answer kinds.
//!
//! Windows' decoders (WIC) need COM, which the sandbox's default profile refuses: the app starts an image helper in
//! the system-decoder profile (`profile_for(Route::System, ..)`). Where COM does not start the answer is
//! [`ImageRefusal::NoDecoder`].
//!
//! **The helper is untrusted.** [`read_image_to_app`] checks the pixel size against [`MAX_IMAGE_PX`] and the bulk
//! against `width * height * 4` before it allocates; what it cannot know (that neither side is over the `max_px` the
//! app ASKED for) the app checks.

use std::fmt;
use std::io::Read;

use super::{
    Body, Message, NO_BULK, T_READ_AT, WireError, check_read_len, open, pixel_bytes, put_u32, put_u64, sealed,
};

/// The helper's command-line argument for a picture instead of a playback.
pub const IMAGE_MODE_ARG: &str = "--image";
/// The largest side an answer may have, and so the largest `max_px` the app may ask for.
pub const MAX_IMAGE_PX: u32 = 4096;
/// The largest picture file the helper reads (a camera's RAW is tens of megabytes); a larger one is `TooLarge`.
pub const MAX_IMAGE_FILE: u64 = 256 << 20;
/// The most pixels a picture may have before it is scaled (200 megapixels); more is `TooLarge`.
pub const MAX_IMAGE_SOURCE_PIXELS: u64 = 200_000_000;

const T_DECODE_IMAGE: u8 = 0x10;
const T_IMAGE_PIXELS: u8 = 0x90;
const T_IMAGE_REFUSED: u8 = 0xA0;

const IMAGE_BYTES_MAX: usize = MAX_IMAGE_PX as usize * MAX_IMAGE_PX as usize * 4;

/// From the app: decode the picture of `len` bytes to at most `max_px` on its longest side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeImage {
    pub max_px: u32,
    pub len: u64,
}

/// From the helper: the picture, upright (EXIF orientation applied), RGBA (straight alpha), or why not.
#[derive(Clone, PartialEq, Eq)]
pub enum ImageAnswer {
    Pixels { width: u32, height: u32, rgba: Vec<u8> },
    Refused(ImageRefusal),
}

/// Why a picture was not decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageRefusal {
    /// A kind this PC has no decoder for (HEIF without the HEIF and HEVC extensions, a RAW without the Raw Image
    /// Extension), or Windows' decoders do not run here.
    NoDecoder,
    /// The decoder took the picture and failed on it, or the app could not give its bytes.
    Damaged,
    /// Over [`MAX_IMAGE_FILE`] bytes or [`MAX_IMAGE_SOURCE_PIXELS`] pixels.
    TooLarge,
    /// Not a picture kind the helper decodes.
    Unsupported,
}

impl ImageRefusal {
    fn byte(self) -> u8 {
        match self {
            ImageRefusal::NoDecoder => 0,
            ImageRefusal::Damaged => 1,
            ImageRefusal::TooLarge => 2,
            ImageRefusal::Unsupported => 3,
        }
    }

    fn of(byte: u8) -> Result<ImageRefusal, WireError> {
        Ok(match byte {
            0 => ImageRefusal::NoDecoder,
            1 => ImageRefusal::Damaged,
            2 => ImageRefusal::TooLarge,
            3 => ImageRefusal::Unsupported,
            _ => return Err(WireError::Malformed),
        })
    }
}

/// Sizes, never a byte of the picture.
impl fmt::Debug for ImageAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageAnswer::Pixels { width, height, .. } => write!(f, "Pixels({width}x{height})"),
            ImageAnswer::Refused(why) => write!(f, "Refused({why:?})"),
        }
    }
}

/// What the app reads from an image helper.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImageFromHelper {
    /// Bytes `offset .. offset + len` of the picture, please (as [`crate::FromHelper::ReadAt`]).
    ReadAt {
        id: u32,
        offset: u64,
        len: u32,
    },
    Answer(ImageAnswer),
}

fn check_max_px(max_px: u32) -> Result<(), WireError> {
    if max_px == 0 || max_px > MAX_IMAGE_PX {
        return Err(WireError::TooLarge);
    }
    Ok(())
}

fn check_image(width: u32, height: u32) -> Result<(), WireError> {
    if width == 0 || height == 0 {
        return Err(WireError::Malformed);
    }
    if width > MAX_IMAGE_PX || height > MAX_IMAGE_PX {
        return Err(WireError::TooLarge);
    }
    Ok(())
}

impl sealed::Sealed for DecodeImage {}
impl sealed::Sealed for ImageAnswer {}

impl Message for DecodeImage {
    fn encode<'a>(&'a self, head: &mut Vec<u8>) -> Result<&'a [u8], WireError> {
        check_max_px(self.max_px)?;
        head.push(T_DECODE_IMAGE);
        put_u32(head, self.max_px);
        put_u64(head, self.len);
        Ok(NO_BULK)
    }
}

impl Message for ImageAnswer {
    fn encode<'a>(&'a self, head: &mut Vec<u8>) -> Result<&'a [u8], WireError> {
        match self {
            ImageAnswer::Pixels { width, height, rgba } => {
                check_image(*width, *height)?;
                if rgba.len() != pixel_bytes(*width, *height) {
                    return Err(WireError::Malformed);
                }
                head.push(T_IMAGE_PIXELS);
                put_u32(head, *width);
                put_u32(head, *height);
                Ok(rgba.as_slice())
            }
            ImageAnswer::Refused(why) => {
                head.push(T_IMAGE_REFUSED);
                head.push(why.byte());
                Ok(NO_BULK)
            }
        }
    }
}

fn to_image_helper_max(tag: u8) -> Option<usize> {
    match tag {
        T_DECODE_IMAGE => Some(4 + 8),
        _ => None,
    }
}

fn image_from_helper_max(tag: u8) -> Option<usize> {
    Some(match tag {
        T_READ_AT => 4 + 8 + 4,
        T_IMAGE_PIXELS => 4 + 4 + IMAGE_BYTES_MAX,
        T_IMAGE_REFUSED => 1,
        _ => return None,
    })
}

/// The image helper's first message. Anything but [`DecodeImage`] is refused.
pub fn read_decode_image<R: Read + ?Sized>(r: &mut R) -> Result<DecodeImage, WireError> {
    let (_, mut body) = open(r, to_image_helper_max)?;
    let max_px = body.u32()?;
    check_max_px(max_px)?;
    let len = body.u64()?;
    body.finish()?;
    Ok(DecodeImage { max_px, len })
}

/// One message from an image helper. Every length is checked before anything is allocated for it.
pub fn read_image_to_app<R: Read + ?Sized>(r: &mut R) -> Result<ImageFromHelper, WireError> {
    let (tag, mut body) = open(r, image_from_helper_max)?;
    let msg = match tag {
        T_READ_AT => {
            let id = body.u32()?;
            let offset = body.u64()?;
            let len = body.u32()?;
            check_read_len(len)?;
            ImageFromHelper::ReadAt { id, offset, len }
        }
        T_IMAGE_PIXELS => pixels(&mut body)?,
        T_IMAGE_REFUSED => ImageFromHelper::Answer(ImageAnswer::Refused(ImageRefusal::of(body.u8()?)?)),
        _ => return Err(WireError::UnknownTag(tag)),
    };
    body.finish()?;
    Ok(msg)
}

fn pixels<R: Read + ?Sized>(body: &mut Body<'_, R>) -> Result<ImageFromHelper, WireError> {
    let width = body.u32()?;
    let height = body.u32()?;
    check_image(width, height)?;
    let rgba = body.rest(pixel_bytes(width, height))?;
    Ok(ImageFromHelper::Answer(ImageAnswer::Pixels { width, height, rgba }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FromHelper, ToHelper, write_msg};

    fn wire(msg: &impl Message) -> Vec<u8> {
        let mut out = Vec::new();
        write_msg(&mut out, msg).unwrap();
        out
    }

    #[test]
    fn decode_image_round_trips_and_bounds_max_px() {
        let ask = DecodeImage {
            max_px: 512,
            len: 123_456,
        };
        assert_eq!(read_decode_image(&mut wire(&ask).as_slice()).unwrap(), ask);
        for max_px in [0, MAX_IMAGE_PX + 1] {
            assert!(write_msg(&mut Vec::new(), &DecodeImage { max_px, len: 1 }).is_err());
            let mut raw = 13u32.to_le_bytes().to_vec();
            raw.push(T_DECODE_IMAGE);
            raw.extend_from_slice(&max_px.to_le_bytes());
            raw.extend_from_slice(&1u64.to_le_bytes());
            assert_eq!(read_decode_image(&mut raw.as_slice()), Err(WireError::TooLarge));
        }
        // A media message is not a picture's first message.
        let close = wire(&ToHelper::Close);
        assert!(matches!(
            read_decode_image(&mut close.as_slice()),
            Err(WireError::UnknownTag(_))
        ));
    }

    #[test]
    fn answers_and_reads_round_trip() {
        let pixels = ImageAnswer::Pixels {
            width: 3,
            height: 2,
            rgba: (0..24).collect(),
        };
        let read = FromHelper::ReadAt {
            id: 7,
            offset: 9,
            len: 100,
        };
        let mut stream = wire(&read);
        stream.extend(wire(&pixels));
        for why in [
            ImageRefusal::NoDecoder,
            ImageRefusal::Damaged,
            ImageRefusal::TooLarge,
            ImageRefusal::Unsupported,
        ] {
            stream.extend(wire(&ImageAnswer::Refused(why)));
        }
        let r = &mut stream.as_slice();
        assert_eq!(
            read_image_to_app(r).unwrap(),
            ImageFromHelper::ReadAt {
                id: 7,
                offset: 9,
                len: 100
            }
        );
        assert_eq!(read_image_to_app(r).unwrap(), ImageFromHelper::Answer(pixels));
        for why in [
            ImageRefusal::NoDecoder,
            ImageRefusal::Damaged,
            ImageRefusal::TooLarge,
            ImageRefusal::Unsupported,
        ] {
            assert_eq!(
                read_image_to_app(r).unwrap(),
                ImageFromHelper::Answer(ImageAnswer::Refused(why))
            );
        }
        assert_eq!(read_image_to_app(r), Err(WireError::Closed));
    }

    #[test]
    fn a_lying_helper_is_refused_before_allocation() {
        // Wrong bulk length for the size.
        let mut raw = Vec::new();
        raw.extend_from_slice(&(1 + 8 + 5u32).to_le_bytes());
        raw.push(T_IMAGE_PIXELS);
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&[0; 5]);
        assert_eq!(read_image_to_app(&mut raw.as_slice()), Err(WireError::Malformed));
        // A side over the cap.
        let mut raw = Vec::new();
        raw.extend_from_slice(&(1 + 8u32).to_le_bytes());
        raw.push(T_IMAGE_PIXELS);
        raw.extend_from_slice(&(MAX_IMAGE_PX + 1).to_le_bytes());
        raw.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(read_image_to_app(&mut raw.as_slice()), Err(WireError::TooLarge));
        // A length over the kind's maximum.
        let mut raw = u32::MAX.to_le_bytes().to_vec();
        raw.push(T_IMAGE_PIXELS);
        assert_eq!(read_image_to_app(&mut raw.as_slice()), Err(WireError::TooLarge));
        // An unknown refusal.
        let mut raw = 2u32.to_le_bytes().to_vec();
        raw.extend_from_slice(&[T_IMAGE_REFUSED, 9]);
        assert_eq!(read_image_to_app(&mut raw.as_slice()), Err(WireError::Malformed));
        // A video frame is not an image answer.
        let ended = wire(&FromHelper::Ended);
        assert!(matches!(
            read_image_to_app(&mut ended.as_slice()),
            Err(WireError::UnknownTag(_))
        ));
        // The writer refuses what the reader would.
        let bad = ImageAnswer::Pixels {
            width: 2,
            height: 2,
            rgba: vec![0; 3],
        };
        assert!(write_msg(&mut Vec::new(), &bad).is_err());
    }
}
