//! An original on this PC to a JPEG preview, in memory only.
//!
//! Decoded with `image` (a source wider or higher than [`MAX_SIDE`] is refused while decoding, and one whose decoding
//! would take more memory than [`MAX_ALLOC`] is refused before a byte of it is allocated: [`memory_needed`]), turned
//! upright by its EXIF orientation, scaled so the long edge is `min(px, its own long edge)` (never upscaled, Lanczos3), then encoded
//! as JPEG at quality 85, or 70 when that does not fit the server's cap. The output carries no EXIF or other APP1
//! metadata: the encoder writes none. Every buffer this module owns holding pixels or encoded bytes is wiped when it
//! is dropped, and an encoded output that grows wipes the smaller buffer it leaves ([`Wiping`]); the decoder's, the
//! scaler's and the encoders' own scratch memory is freed, not overwritten.
//!
//! Decodable here: jpg, jpeg, png, webp, gif (its first frame), bmp. Not a source: heic, heif, raw, tif and tiff,
//! and videos (this PC has no decoder for them).

use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageEncoder as _, ImageReader, RgbImage, imageops::FilterType};
use zeroize::{Zeroize, Zeroizing};

use super::seal::MAX_PLAINTEXT;

/// The widest and highest original that is decoded at all.
pub const MAX_SIDE: u32 = 16_384;
/// The most memory making one preview may take, counted before decoding ([`memory_needed`]).
pub const MAX_ALLOC: u64 = 768 * 1024 * 1024;
/// The largest long edge a preview is scaled to (4K): the scaler's intermediate is counted at this height.
const LARGEST_EDGE: u64 = 3840;
/// The JPEG quality of a preview, and the one tried once when that is over the server's cap.
pub const QUALITY: u8 = 85;
pub const QUALITY_SMALLER: u8 = 70;
/// The long edge of the pictures the compare view draws (its photo at twice its size).
pub const SHOWN_LONG_EDGE: u32 = 1120;

/// The extensions this PC makes previews from (lowercase, no dot).
pub const DECODABLE: [&str; 6] = ["jpg", "jpeg", "png", "webp", "gif", "bmp"];

/// Whether a file named `name` is a photo this PC can decode.
pub fn decodable(name: &str) -> bool {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .is_some_and(|ext| DECODABLE.contains(&ext.as_str()))
}

/// Why no preview was made. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeRefused {
    /// Not a picture this PC decodes, a broken one, or over the size or memory caps.
    Undecodable,
    /// Even at the lower quality the preview is over the server's cap.
    TooLarge,
    /// Decoding, turning and scaling it would take more than [`MAX_ALLOC`] at once: refused before decoding.
    OverMemory,
    Failed,
}

/// A decoded original, upright, as RGB. Wiped when dropped.
pub struct Upright {
    rgb: RgbImage,
}

impl Drop for Upright {
    fn drop(&mut self) {
        let raw: &mut [u8] = &mut self.rgb;
        raw.zeroize();
    }
}

impl std::fmt::Debug for Upright {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upright").field("size", &self.size()).finish()
    }
}

impl Upright {
    /// Width and height after the EXIF orientation.
    pub fn size(&self) -> (u32, u32) {
        self.rgb.dimensions()
    }
}

/// Decode `original` (an encoded picture) and turn it upright by its EXIF orientation.
pub fn decode(original: &[u8]) -> Result<Upright, EncodeRefused> {
    let mut reader = ImageReader::new(Cursor::new(original))
        .with_guessed_format()
        .map_err(|_| EncodeRefused::Undecodable)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| EncodeRefused::Undecodable)?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(EncodeRefused::Undecodable);
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let rgb8 = decoder.color_type() == image::ColorType::Rgb8;
    if memory_needed(w, h, decoder.total_bytes(), rgb8, original.len() as u64) > MAX_ALLOC {
        return Err(EncodeRefused::OverMemory);
    }
    let image = DynamicImage::from_decoder(decoder).map_err(|_| EncodeRefused::Undecodable)?;
    let image = upright(image, orientation);
    let rgb = match image {
        DynamicImage::ImageRgb8(rgb) => rgb,
        other => {
            let rgb = other.to_rgb8();
            wipe(other);
            rgb
        }
    };
    Ok(Upright { rgb })
}

/// The most memory decoding a `w`×`h` picture of `decoded` bytes and making its largest preview holds at once: the
/// largest of its phases, each counting the buffers alive together. Decoding: the picture and about as much of the
/// decoder's own. Turning it upright: the picture and its turned copy. Making it RGB (when it is not already 8-bit
/// RGB): the picture and its RGB copy. Scaling it: the RGB picture and the scaler's intermediate (16 bytes a pixel,
/// the picture's long edge wide and at most [`LARGEST_EDGE`] high). The `original`'s bytes are held through all of
/// them.
pub fn memory_needed(w: u32, h: u32, decoded: u64, rgb8: bool, original: u64) -> u64 {
    let (w, h) = (u64::from(w), u64::from(h));
    let rgb = w.saturating_mul(h).saturating_mul(3);
    let long = w.max(h);
    let intermediate = long.saturating_mul(LARGEST_EDGE.min(long)).saturating_mul(16);
    let decoding = decoded.saturating_mul(2);
    let converting = if rgb8 { 0 } else { decoded.saturating_add(rgb) };
    let scaling = rgb.saturating_add(intermediate);
    decoding.max(converting).max(scaling).saturating_add(original)
}

/// `image` turned upright by `orientation`. A quarter turn makes a new buffer: the one it leaves is wiped before it is
/// freed. The other orientations turn it in place.
fn upright(mut image: DynamicImage, orientation: image::metadata::Orientation) -> DynamicImage {
    use image::metadata::Orientation::{Rotate90, Rotate90FlipH, Rotate270, Rotate270FlipH};
    let (clockwise, flip) = match orientation {
        Rotate90 => (true, false),
        Rotate270 => (false, false),
        Rotate90FlipH => (true, true),
        Rotate270FlipH => (false, true),
        other => {
            image.apply_orientation(other);
            return image;
        }
    };
    let mut turned = if clockwise { image.rotate90() } else { image.rotate270() };
    wipe(image);
    if flip {
        // In place: no third buffer.
        turned.apply_orientation(image::metadata::Orientation::FlipHorizontal);
    }
    turned
}

/// Zero `image`'s samples where they are, whatever their type, then free them. (`into_bytes` would first copy a
/// picture that is not 8-bit into new bytes, and free the original unwiped.)
fn wipe(mut image: DynamicImage) {
    match &mut image {
        DynamicImage::ImageLuma8(samples) => (**samples).zeroize(),
        DynamicImage::ImageLumaA8(samples) => (**samples).zeroize(),
        DynamicImage::ImageRgb8(samples) => (**samples).zeroize(),
        DynamicImage::ImageRgba8(samples) => (**samples).zeroize(),
        DynamicImage::ImageLuma16(samples) => (**samples).zeroize(),
        DynamicImage::ImageLumaA16(samples) => (**samples).zeroize(),
        DynamicImage::ImageRgb16(samples) => (**samples).zeroize(),
        DynamicImage::ImageRgba16(samples) => (**samples).zeroize(),
        DynamicImage::ImageRgb32F(samples) => (**samples).zeroize(),
        DynamicImage::ImageRgba32F(samples) => (**samples).zeroize(),
        // A kind a later `image` adds: its bytes, wiped (a copy, when they are not bytes already).
        _ => image.into_bytes().zeroize(),
    }
}

/// The size of `(w, h)` with its long edge at most `long`: never larger than it is.
pub fn fitted(w: u32, h: u32, long: u32) -> (u32, u32) {
    let own = w.max(h);
    if own <= long || own == 0 {
        return (w, h);
    }
    let scale = |side: u32| ((u64::from(side) * u64::from(long) + u64::from(own) / 2) / u64::from(own)).max(1) as u32;
    (scale(w), scale(h))
}

/// The picture scaled down to a long edge of at most `long` (Lanczos3), never up. Wiped when dropped.
pub fn scaled(upright: &Upright, long: u32) -> Upright {
    let (w, h) = upright.size();
    let (to_w, to_h) = fitted(w, h, long);
    let rgb = if (to_w, to_h) == (w, h) {
        upright.rgb.clone()
    } else {
        image::imageops::resize(&upright.rgb, to_w, to_h, FilterType::Lanczos3)
    };
    Upright { rgb }
}

/// An encoder's output that grows without leaving a copy behind: a `Vec` that grows frees its old block as it is,
/// so this one moves to a larger buffer itself and wipes the one it leaves.
struct Wiping(Zeroizing<Vec<u8>>);

impl Wiping {
    fn new() -> Self {
        Self(Zeroizing::new(Vec::with_capacity(64 * 1024)))
    }
}

impl std::io::Write for Wiping {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let needed = self.0.len().saturating_add(data.len());
        if needed > self.0.capacity() {
            let mut larger = Zeroizing::new(Vec::with_capacity(needed.max(self.0.capacity().saturating_mul(2))));
            larger.extend_from_slice(&self.0);
            // The smaller buffer is dropped here: wiped, its spare room too.
            self.0 = larger;
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// JPEG at `quality`, no metadata.
fn jpeg(picture: &Upright, quality: u8) -> Result<Zeroizing<Vec<u8>>, EncodeRefused> {
    let mut out = Wiping::new();
    let (w, h) = picture.size();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode(picture.rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|_| EncodeRefused::Failed)?;
    Ok(out.0)
}

/// JPEG bytes (wiped on drop) and the pixel size they encode.
pub type Encoded = (Zeroizing<Vec<u8>>, (u32, u32));

/// A JPEG preview of `upright` with its long edge at `min(px, own)`: quality 85, or 70 once when 85 is over the
/// server's cap; over that, refused.
pub fn preview_jpeg(upright: &Upright, px: u32) -> Result<Encoded, EncodeRefused> {
    jpeg_fitting(&scaled(upright, px))
}

/// `picture` as it is, as JPEG: quality 85, or 70 once when 85 is over the server's cap; over that, refused.
pub fn jpeg_fitting(picture: &Upright) -> Result<Encoded, EncodeRefused> {
    let size = picture.size();
    let first = jpeg(picture, QUALITY)?;
    if first.len() <= MAX_PLAINTEXT {
        return Ok((first, size));
    }
    drop(first);
    let second = jpeg(picture, QUALITY_SMALLER)?;
    if second.len() <= MAX_PLAINTEXT {
        return Ok((second, size));
    }
    Err(EncodeRefused::TooLarge)
}

/// `original` (encoded) to a JPEG preview at `px`: [`decode`] then [`preview_jpeg`].
pub fn encode_preview(original: &[u8], px: u32) -> Result<Encoded, EncodeRefused> {
    let upright = decode(original)?;
    preview_jpeg(&upright, px)
}

/// A picture as PNG bytes for a window to draw, its long edge at most [`SHOWN_LONG_EDGE`]. Wiped when dropped.
pub fn shown_png(picture: &Upright) -> Result<Zeroizing<Vec<u8>>, EncodeRefused> {
    let shown = scaled(picture, SHOWN_LONG_EDGE);
    let (w, h) = shown.size();
    let mut out = Wiping::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(shown.rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|_| EncodeRefused::Failed)?;
    Ok(out.0)
}
