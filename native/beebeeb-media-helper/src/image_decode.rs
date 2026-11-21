//! One picture, decoded inside the sandbox with Windows' own decoders (WIC): HEIC/HEIF (needs Microsoft's HEIF and
//! HEVC extensions) and a camera's RAW (DNG, CR2, CR3, NEF, ARW, …; needs the Raw Image Extension), and the common
//! kinds WIC always has (JPEG, PNG, GIF, BMP, TIFF, WebP). Task 048.
//!
//! The helper runs this when started with `beebeeb_media_protocol::image::IMAGE_MODE_ARG`: the app's first message
//! is `DecodeImage { max_px, len }`; the bytes come in by the same path a video's do ([`PipeReader`]: `ReadAt` →
//! `Data`), whole (at most `MAX_IMAGE_FILE`), into this process's memory only; out goes one `ImageAnswer`, and the
//! process ends.
//!
//! Before any decoder sees a byte, the picture's first bytes must be a kind named above ([`sniff_picture`]): other
//! bytes are `Unsupported` and never reach whatever other codecs are installed on the PC. A picture over
//! `MAX_IMAGE_SOURCE_PIXELS` is `TooLarge` before its pixels are decoded. The picture is scaled while it is decoded
//! (WIC's scaler pulls from the decoder) to at most `max_px` on its longest side, made RGBA, then turned upright by its
//! EXIF orientation ([`orient`]). No decoder for the kind on this PC, or no COM in this process (the sandbox's default
//! profile refuses it: the app starts an image helper in the system-decoder profile), is `NoDecoder`.
//!
//! Never logs, never writes anything but protocol messages, holds no key, no name and no path.

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

use beebeeb_media_protocol::image::{
    DecodeImage, ImageAnswer, ImageRefusal, MAX_IMAGE_FILE, MAX_IMAGE_SOURCE_PIXELS, read_decode_image,
};
use beebeeb_media_protocol::write_msg;
use filmcraft_media::ByteReader;

use crate::read::{PipeReader, ReadFlags, SharedWire, Wire};

/// The bytes are fetched in pieces of this size (each one at most two `MAX_READ` asks in flight, over the cache's
/// size so nothing is kept twice).
const FETCH: usize = 16 << 20;

/// A decoded picture: `rgba` is `width * height * 4` bytes, rows top to bottom, straight alpha.
#[derive(Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The kinds of picture handed to a decoder, known by their first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureKind {
    Jpeg,
    Png,
    Gif,
    Bmp,
    WebP,
    /// TIFF, and the RAW kinds built on it (DNG, CR2, NEF, ARW, PEF, …): one header, the decoder tells them apart.
    Tiff,
    /// HEIC / HEIF / AVIF (an ISO-BMFF `ftyp` with an image brand).
    Heif,
    /// The RAW kinds with their own header: CR3, RAF, ORF, RW2.
    Raw,
}

/// The picture kind of `head` (the file's first bytes), or `None`: not a picture this helper hands to a decoder.
pub fn sniff_picture(head: &[u8]) -> Option<PictureKind> {
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some(PictureKind::Jpeg);
    }
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Some(PictureKind::Png);
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(PictureKind::Gif);
    }
    if starts(b"BM") && head.len() >= 26 {
        return Some(PictureKind::Bmp);
    }
    if starts(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Some(PictureKind::WebP);
    }
    // Olympus (IIRO, IIRS) and Panasonic (IIU\0) RAW change TIFF's magic number.
    if starts(b"IIRO") || starts(b"IIRS") || starts(b"IIU\0") || starts(b"FUJIFILMCCD-RAW") {
        return Some(PictureKind::Raw);
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return Some(PictureKind::Tiff);
    }
    if head.get(4..8) == Some(b"ftyp") {
        let size = u32::from_be_bytes(head[0..4].try_into().ok()?) as usize;
        if size < 16 || size % 4 != 0 {
            return None;
        }
        let ftyp = head.get(8..size.min(head.len()))?;
        // The major brand, then (after the minor version) the compatible brands.
        let brands = std::iter::once(&ftyp[..4]).chain(ftyp.get(8..)?.chunks_exact(4));
        for brand in brands {
            match brand {
                b"crx " => return Some(PictureKind::Raw),
                b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis" => {
                    return Some(PictureKind::Heif);
                }
                _ => {}
            }
        }
    }
    None
}

/// The size a `width` × `height` picture is scaled to so its longest side is at most `max_px`: its own size when it
/// fits, never a side of 0.
pub fn fit(width: u32, height: u32, max_px: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_px || longest == 0 {
        return (width, height);
    }
    let scale =
        |side: u32| ((u64::from(side) * u64::from(max_px) + u64::from(longest) / 2) / u64::from(longest)).max(1);
    (scale(width) as u32, scale(height) as u32)
}

/// `picture` turned upright by its EXIF orientation (1 to 8: the TIFF tag 274; anything else is 1, as stored).
pub fn orient(picture: Picture, orientation: u16) -> Picture {
    if !(2..=8).contains(&orientation) {
        return picture;
    }
    let (w, h) = (picture.width as usize, picture.height as usize);
    let swaps = orientation >= 5;
    let (ow, oh) = if swaps { (h, w) } else { (w, h) };
    let mut out = vec![0u8; picture.rgba.len()];
    for y in 0..oh {
        for x in 0..ow {
            // Where the displayed pixel (x, y) is stored.
            let (sx, sy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x), // 8
            };
            let from = (sy * w + sx) * 4;
            let to = (y * ow + x) * 4;
            out[to..to + 4].copy_from_slice(&picture.rgba[from..from + 4]);
        }
    }
    Picture {
        width: ow as u32,
        height: oh as u32,
        rgba: out,
    }
}

/// What a failed WIC call (its `HRESULT`) means for the picture.
pub fn refusal_for(hresult: i32) -> ImageRefusal {
    const COMPONENT_NOT_FOUND: u32 = 0x8898_2F50; // WINCODEC_ERR_COMPONENTNOTFOUND
    const UNKNOWN_IMAGE_FORMAT: u32 = 0x8898_2F07; // WINCODEC_ERR_UNKNOWNIMAGEFORMAT
    const CLASS_NOT_REGISTERED: u32 = 0x8004_0154; // REGDB_E_CLASSNOTREG
    const CODEC_NOT_FOUND: u32 = 0xC00D_5212; // MF_E_TOPO_CODEC_NOT_FOUND (HEIF without the HEVC extension)
    const OUT_OF_MEMORY: u32 = 0x8007_000E; // E_OUTOFMEMORY
    const OVERFLOW: u32 = 0x8007_0216; // INTSAFE_E_ARITHMETIC_OVERFLOW
    match hresult as u32 {
        COMPONENT_NOT_FOUND | UNKNOWN_IMAGE_FORMAT | CLASS_NOT_REGISTERED | CODEC_NOT_FOUND => ImageRefusal::NoDecoder,
        OUT_OF_MEMORY | OVERFLOW => ImageRefusal::TooLarge,
        _ => ImageRefusal::Damaged,
    }
}

/// Whether a `width` × `height` picture may be decoded at all.
pub fn within_caps(width: u32, height: u32) -> Result<(), ImageRefusal> {
    if width == 0 || height == 0 {
        return Err(ImageRefusal::Damaged);
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_SOURCE_PIXELS {
        return Err(ImageRefusal::TooLarge);
    }
    Ok(())
}

/// The picture in `bytes`, at most `max_px` on its longest side and upright.
pub fn decode(bytes: &[u8], max_px: u32) -> Result<Picture, ImageRefusal> {
    if max_px == 0 {
        return Err(ImageRefusal::Unsupported);
    }
    if sniff_picture(bytes).is_none() {
        return Err(ImageRefusal::Unsupported);
    }
    #[cfg(windows)]
    {
        wic::decode(bytes, max_px)
    }
    #[cfg(not(windows))]
    {
        Err(ImageRefusal::NoDecoder)
    }
}

/// The helper's output, shared by the reader (its `ReadAt` asks) and the answer at the end.
struct Out(Arc<Mutex<Box<dyn Write + Send>>>);

impl Write for Out {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

/// Serve one picture over standard input and output: `DecodeImage`, the reads, one answer.
pub fn run(mut input: impl Read + Send + 'static, output: impl Write + Send + 'static) {
    let shared: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(Box::new(output)));
    let Ok(ask) = read_decode_image(&mut input) else {
        return;
    };
    let answer = answer(ask, input, Out(shared.clone()));
    let _ = write_msg(&mut Out(shared), &answer);
}

fn answer(ask: DecodeImage, input: impl Read + Send + 'static, out: Out) -> ImageAnswer {
    let refused = ImageAnswer::Refused;
    if ask.len > MAX_IMAGE_FILE {
        return refused(ImageRefusal::TooLarge);
    }
    if ask.len == 0 {
        return refused(ImageRefusal::Damaged);
    }
    let wire: SharedWire = Arc::new(Mutex::new(Wire::new(input, out)));
    let reader = PipeReader::new(wire, ask.len, Arc::new(ReadFlags::default()));
    // `ask.len` is at most MAX_IMAGE_FILE: it fits.
    let mut bytes = vec![0u8; ask.len as usize];
    for (i, piece) in bytes.chunks_mut(FETCH).enumerate() {
        if reader.read_at((i * FETCH) as u64, piece).is_err() {
            return refused(ImageRefusal::Damaged);
        }
    }
    match decode(&bytes, ask.max_px) {
        Ok(picture) => ImageAnswer::Pixels {
            width: picture.width,
            height: picture.height,
            rgba: picture.rgba,
        },
        Err(why) => refused(why),
    }
}

/// Windows Imaging Component: started through COM, whose functions are looked up at run time (ole32 is a delay-loaded
/// import, `build.rs`; nothing of WIC is an import at all: `windowscodecs.dll` is loaded by COM when the factory is made).
#[cfg(windows)]
mod wic {
    #![allow(unsafe_code)]

    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::sync::OnceLock;

    use beebeeb_media_protocol::image::ImageRefusal;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICBitmapFrameDecode, IWICBitmapSource,
        IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
        WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
    use windows::Win32::System::Variant::VT_UI2;
    use windows::core::{GUID, HRESULT, Interface, PCSTR, PCWSTR, w};

    use super::{Picture, fit, orient, refusal_for, within_caps};

    /// `COINIT_MULTITHREADED`.
    const MULTITHREADED: u32 = 0;
    /// `CLSCTX_INPROC_SERVER`.
    const IN_PROCESS: u32 = 1;

    type CoInitializeEx = unsafe extern "system" fn(*const c_void, u32) -> HRESULT;
    type CoCreateInstance =
        unsafe extern "system" fn(*const GUID, *mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT;

    static COM: OnceLock<Option<CoCreateInstance>> = OnceLock::new();

    fn library(name: &str) -> Option<HMODULE> {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        // SAFETY: a terminated UTF-16 name that lives past the call; the system folder only.
        unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()
    }

    fn start() -> Option<CoCreateInstance> {
        let ole32 = library("ole32.dll")?;
        // The delay-loaded COM libraries, from the system folder before anything calls through them.
        library("combase.dll")?;
        library("oleaut32.dll")?;
        // SAFETY: terminated ASCII names in a library this process loaded.
        let init = unsafe { GetProcAddress(ole32, PCSTR(c"CoInitializeEx".as_ptr().cast())) }?;
        // SAFETY: as above.
        let create = unsafe { GetProcAddress(ole32, PCSTR(c"CoCreateInstance".as_ptr().cast())) }?;
        // SAFETY: Windows documents both exports with exactly these signatures.
        let (init, create) = unsafe {
            (
                std::mem::transmute::<unsafe extern "system" fn() -> isize, CoInitializeEx>(init),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, CoCreateInstance>(create),
            )
        };
        // SAFETY: no reserved pointer; this thread joins the process's multithreaded apartment. The sandbox's default
        // profile refuses COM: no decoder here then.
        if unsafe { init(null(), MULTITHREADED) }.is_err() {
            return None;
        }
        Some(create)
    }

    fn factory() -> Result<IWICImagingFactory, ImageRefusal> {
        let create = (*COM.get_or_init(start)).ok_or(ImageRefusal::NoDecoder)?;
        let mut raw = null_mut();
        // SAFETY: the factory's class and interface ids; `raw` receives one reference to it.
        unsafe {
            create(
                &CLSID_WICImagingFactory,
                null_mut(),
                IN_PROCESS,
                &IWICImagingFactory::IID,
                &mut raw,
            )
        }
        .ok()
        .map_err(|_| ImageRefusal::NoDecoder)?;
        if raw.is_null() {
            return Err(ImageRefusal::NoDecoder);
        }
        // SAFETY: a non-null `IWICImagingFactory` whose one reference is now this value's.
        Ok(unsafe { IWICImagingFactory::from_raw(raw) })
    }

    fn why(e: windows::core::Error) -> ImageRefusal {
        refusal_for(e.code().0)
    }

    /// The frame's EXIF orientation (1 when it has none).
    fn orientation(frame: &IWICBitmapFrameDecode) -> u16 {
        // SAFETY: a live frame.
        let Ok(reader) = (unsafe { frame.GetMetadataQueryReader() }) else {
            return 1;
        };
        for name in [
            w!("System.Photo.Orientation"),
            w!("/app1/ifd/{ushort=274}"),
            w!("/ifd/{ushort=274}"),
        ] {
            let mut value = PROPVARIANT::default();
            // SAFETY: a terminated name; `value` receives a value this function clears.
            let found = unsafe { reader.GetMetadataByName(name, &mut value) }.is_ok();
            // SAFETY: the union's tag says which member is set.
            let read = unsafe {
                let inner = &value.Anonymous.Anonymous;
                (inner.vt == VT_UI2).then(|| inner.Anonymous.uiVal)
            };
            // SAFETY: a value WIC filled in (or the zeroed default).
            let _ = unsafe { PropVariantClear(&mut value) };
            if let (true, Some(orientation)) = (found, read) {
                return orientation;
            }
        }
        1
    }

    pub(super) fn decode(bytes: &[u8], max_px: u32) -> Result<Picture, ImageRefusal> {
        let factory = factory()?;
        // SAFETY: every call below is on live objects made here; `bytes` outlives the stream, the decoder and every
        // source pulled from them (all dropped before this function returns).
        unsafe {
            let stream = factory.CreateStream().map_err(why)?;
            stream.InitializeFromMemory(bytes).map_err(why)?;
            let decoder = factory
                .CreateDecoderFromStream(&stream, null(), WICDecodeMetadataCacheOnDemand)
                .map_err(why)?;
            let frame = decoder.GetFrame(0).map_err(why)?;
            let (mut width, mut height) = (0u32, 0u32);
            frame.GetSize(&mut width, &mut height).map_err(why)?;
            within_caps(width, height)?;
            let turn = orientation(&frame);
            let (to_w, to_h) = fit(width, height, max_px);
            let source: IWICBitmapSource = if (to_w, to_h) == (width, height) {
                frame.cast().map_err(why)?
            } else {
                let scaler = factory.CreateBitmapScaler().map_err(why)?;
                scaler
                    .Initialize(&frame, to_w, to_h, WICBitmapInterpolationModeFant)
                    .map_err(why)?;
                scaler.cast().map_err(why)?
            };
            let converter = factory.CreateFormatConverter().map_err(why)?;
            converter
                .Initialize(
                    &source,
                    &GUID_WICPixelFormat32bppRGBA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(why)?;
            let mut rgba = vec![0u8; to_w as usize * to_h as usize * 4];
            converter.CopyPixels(null(), to_w * 4, &mut rgba).map_err(why)?;
            Ok(orient(
                Picture {
                    width: to_w,
                    height: to_h,
                    rgba,
                },
                turn,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hresults_name_the_refusal() {
        assert_eq!(refusal_for(0x8898_2F50_u32 as i32), ImageRefusal::NoDecoder);
        assert_eq!(refusal_for(0xC00D_5212_u32 as i32), ImageRefusal::NoDecoder);
        assert_eq!(refusal_for(0x8007_000E_u32 as i32), ImageRefusal::TooLarge);
        assert_eq!(refusal_for(0x8898_2F61_u32 as i32), ImageRefusal::Damaged); // WINCODEC_ERR_BADHEADER
    }

    #[test]
    fn caps() {
        assert_eq!(within_caps(0, 5), Err(ImageRefusal::Damaged));
        assert_eq!(within_caps(20_000, 10_001), Err(ImageRefusal::TooLarge));
        assert_eq!(within_caps(8_000, 6_000), Ok(()));
    }
}
