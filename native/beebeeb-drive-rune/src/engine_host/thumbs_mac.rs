//! macOS: the thumbnails' source of a HEIC/HEIF photo or a camera's RAW (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2),
//! decoded by macOS's own ImageIO from the bytes in memory. The media helper's Rust decoders read neither.
//!
//! Plaintext never touches disk: the bytes go to ImageIO as a `CFData` (no file URL, no temp file, no QuickLook),
//! ImageIO makes an upright thumbnail of at most [`SOURCE_PX`] a side (EXIF orientation applied), and it is drawn
//! into a buffer that is wiped when dropped. The copy of the file's bytes ImageIO reads is wiped when CoreFoundation
//! frees it (`WIPING`). ImageIO's own decode buffers are its own.
//!
//! Plain C calls to CoreFoundation, ImageIO and CoreGraphics: no crate is added.

use std::ffi::c_void;
use std::ptr::{self, addr_of};
use std::sync::OnceLock;

use super::{SOURCE_PX, Thumbs};

type CFTypeRef = *const c_void;
type CFIndex = isize;

#[repr(C)]
struct CallBacks {
    _opaque: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[repr(C)]
struct CFAllocatorContext {
    version: CFIndex,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    allocate: Option<unsafe extern "C" fn(CFIndex, usize, *mut c_void) -> *mut c_void>,
    reallocate: *const c_void,
    deallocate: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
    preferred_size: *const c_void,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: CallBacks;
    static kCFTypeDictionaryValueCallBacks: CallBacks;
    fn CFAllocatorCreate(allocator: CFTypeRef, context: *const CFAllocatorContext) -> CFTypeRef;
    fn CFDataCreateWithBytesNoCopy(
        alloc: CFTypeRef,
        bytes: *const u8,
        len: CFIndex,
        deallocator: CFTypeRef,
    ) -> CFTypeRef;
    fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: CFIndex,
        key_callbacks: *const CallBacks,
        value_callbacks: *const CallBacks,
    ) -> CFTypeRef;
    fn CFNumberCreate(alloc: CFTypeRef, the_type: CFIndex, value: *const c_void) -> CFTypeRef;
    fn CFStringCreateWithBytes(
        alloc: CFTypeRef,
        bytes: *const u8,
        len: CFIndex,
        encoding: u32,
        external: u8,
    ) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    static kCGImageSourceCreateThumbnailFromImageAlways: CFTypeRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFTypeRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFTypeRef;
    static kCGImageSourceShouldCache: CFTypeRef;
    static kCGImageSourceTypeIdentifierHint: CFTypeRef;
    fn CGImageSourceCreateWithData(data: CFTypeRef, options: CFTypeRef) -> CFTypeRef;
    fn CGImageSourceGetCount(source: CFTypeRef) -> usize;
    fn CGImageSourceGetPrimaryImageIndex(source: CFTypeRef) -> usize;
    fn CGImageSourceCreateThumbnailAtIndex(source: CFTypeRef, index: usize, options: CFTypeRef) -> CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: CFTypeRef;
    fn CGColorSpaceCreateWithName(name: CFTypeRef) -> CFTypeRef;
    fn CGImageGetWidth(image: CFTypeRef) -> usize;
    fn CGImageGetHeight(image: CFTypeRef) -> usize;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CFTypeRef,
        bitmap_info: u32,
    ) -> CFTypeRef;
    fn CGContextSetBlendMode(context: CFTypeRef, mode: i32);
    fn CGContextDrawImage(context: CFTypeRef, rect: CGRect, image: CFTypeRef);
}

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn malloc_size(ptr: *const c_void) -> usize;
}

const NUMBER_SINT64: CFIndex = 4;
const STRING_UTF8: u32 = 0x0800_0100;
/// `kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big`: R, G, B, A bytes in that order.
const RGBA_PREMULTIPLIED: u32 = 1 | (4 << 12);
const BLEND_COPY: i32 = 17;

/// A CoreFoundation object this code owns: released when dropped.
struct Cf(CFTypeRef);

impl Cf {
    fn new(made: CFTypeRef) -> Option<Cf> {
        (!made.is_null()).then_some(Cf(made))
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        // SAFETY: a Cf holds one reference it was given by a CF "Create" call, released once.
        unsafe { CFRelease(self.0) }
    }
}

/// The UTI ImageIO is told the bytes are, for the kinds decoded here; `None`: not decoded here.
pub(super) fn kind_of(mime: &str) -> Option<&'static str> {
    Some(match mime.to_ascii_lowercase().as_str() {
        "image/heic" | "image/heic-sequence" => "public.heic",
        "image/heif" | "image/heif-sequence" => "public.heif",
        "image/x-adobe-dng" | "image/dng" => "com.adobe.raw-image",
        "image/x-canon-cr2" => "com.canon.cr2-raw-image",
        "image/x-canon-cr3" => "com.canon.cr3-raw-image",
        "image/x-nikon-nef" => "com.nikon.raw-image",
        "image/x-sony-arw" => "com.sony.arw-raw-image",
        "image/x-fuji-raf" | "image/x-fujifilm-raf" => "com.fuji.raw-image",
        "image/x-olympus-orf" => "com.olympus.raw-image",
        "image/x-panasonic-rw2" | "image/x-panasonic-raw" => "com.panasonic.rw2-raw-image",
        _ => return None,
    })
}

/// `bytes` (a HEIC, a RAW, by `mime`) as an upright picture of at most [`SOURCE_PX`] a side, decoded by ImageIO in
/// memory. `None`: not a kind decoded here, or ImageIO could not read it. Never writes a file. BLOCKS: the decode.
pub(super) fn generate(bytes: &[u8], mime: &str) -> Option<Thumbs> {
    let hint = kind_of(mime)?;
    if bytes.is_empty() {
        return None;
    }
    objc2::rc::autoreleasepool(|_| decode(bytes, hint))
}

fn decode(bytes: &[u8], hint: &str) -> Option<Thumbs> {
    let data = wiped_copy(bytes)?;
    // SAFETY: every pointer handed to CF below is a live CF object or a live local; every object made is owned by a
    // Cf and released once.
    unsafe {
        let hint = Cf::new(CFStringCreateWithBytes(
            ptr::null(),
            hint.as_ptr(),
            hint.len() as CFIndex,
            STRING_UTF8,
            0,
        ))?;
        let source_options = dictionary(
            &[kCGImageSourceTypeIdentifierHint, kCGImageSourceShouldCache],
            &[hint.0, kCFBooleanFalse],
        )?;
        let source = Cf::new(CGImageSourceCreateWithData(data.0, source_options.0))?;
        if CGImageSourceGetCount(source.0) == 0 {
            return None;
        }
        let max_px = i64::from(SOURCE_PX);
        let max_px = Cf::new(CFNumberCreate(ptr::null(), NUMBER_SINT64, addr_of!(max_px).cast()))?;
        let thumb_options = dictionary(
            &[
                kCGImageSourceCreateThumbnailFromImageAlways,
                kCGImageSourceThumbnailMaxPixelSize,
                kCGImageSourceCreateThumbnailWithTransform,
                kCGImageSourceShouldCache,
            ],
            &[kCFBooleanTrue, max_px.0, kCFBooleanTrue, kCFBooleanFalse],
        )?;
        let index = CGImageSourceGetPrimaryImageIndex(source.0);
        let image = Cf::new(CGImageSourceCreateThumbnailAtIndex(source.0, index, thumb_options.0))?;
        drop(source);
        drop(data);
        rgba_of(&image)
    }
}

/// The CGImage `image` drawn as straight (not premultiplied) sRGB RGBA, in a buffer wiped when dropped.
unsafe fn rgba_of(image: &Cf) -> Option<Thumbs> {
    // SAFETY (whole body): `image` is a live CGImage; the context draws into `rgba`, which outlives it.
    unsafe {
        let (width, height) = (CGImageGetWidth(image.0), CGImageGetHeight(image.0));
        if width == 0 || height == 0 || width.max(height) > SOURCE_PX as usize {
            return None;
        }
        let mut rgba = zeroize::Zeroizing::new(vec![0u8; width.checked_mul(height)?.checked_mul(4)?]);
        let space = Cf::new(CGColorSpaceCreateWithName(kCGColorSpaceSRGB))?;
        let context = Cf::new(CGBitmapContextCreate(
            rgba.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            space.0,
            RGBA_PREMULTIPLIED,
        ))?;
        CGContextSetBlendMode(context.0, BLEND_COPY);
        let whole = CGRect {
            x: 0.0,
            y: 0.0,
            width: width as f64,
            height: height as f64,
        };
        CGContextDrawImage(context.0, whole, image.0);
        drop(context);
        unpremultiply(&mut rgba);
        Some(Thumbs {
            width: width as u32,
            height: height as u32,
            rgba,
        })
    }
}

fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        if a == 0 || a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// An immutable CFDictionary of CF keys and values (retained by the dictionary).
unsafe fn dictionary(keys: &[CFTypeRef], values: &[CFTypeRef]) -> Option<Cf> {
    debug_assert_eq!(keys.len(), values.len());
    // SAFETY: the keys and values are live CF objects; the callbacks are CoreFoundation's own.
    unsafe {
        Cf::new(CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            keys.len() as CFIndex,
            addr_of!(kCFTypeDictionaryKeyCallBacks),
            addr_of!(kCFTypeDictionaryValueCallBacks),
        ))
    }
}

// ---- the wiped copy of the file's bytes ---------------------------------------------------------------------------

/// A copy of `bytes` as a CFData that owns it: freed (and wiped first) by CoreFoundation whenever the last of
/// ImageIO's references goes, so no reference ImageIO keeps can outlive the bytes it reads.
fn wiped_copy(bytes: &[u8]) -> Option<Cf> {
    let allocator = WIPING.get_or_init(make_wiping).0;
    if allocator.is_null() {
        return None;
    }
    // SAFETY: a fresh block of `bytes.len()` bytes; on success the CFData owns it and frees it through `allocator`.
    unsafe {
        let copy = malloc(bytes.len()).cast::<u8>();
        if copy.is_null() {
            return None;
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), copy, bytes.len());
        let data = CFDataCreateWithBytesNoCopy(ptr::null(), copy, bytes.len() as CFIndex, allocator);
        if data.is_null() {
            wipe_and_free(copy.cast(), ptr::null_mut());
            return None;
        }
        Some(Cf(data))
    }
}

/// The CFAllocator the copies are freed through, made once, never released.
struct Allocator(CFTypeRef);
// SAFETY: a CFAllocator is immutable after creation and CoreFoundation's objects are thread-safe to share.
unsafe impl Send for Allocator {}
unsafe impl Sync for Allocator {}

static WIPING: OnceLock<Allocator> = OnceLock::new();

fn make_wiping() -> Allocator {
    let context = CFAllocatorContext {
        version: 0,
        info: ptr::null_mut(),
        retain: ptr::null(),
        release: ptr::null(),
        copy_description: ptr::null(),
        allocate: Some(allocate),
        reallocate: ptr::null(),
        deallocate: Some(wipe_and_free),
        preferred_size: ptr::null(),
    };
    // SAFETY: CFAllocatorCreate copies the context.
    Allocator(unsafe { CFAllocatorCreate(ptr::null(), &context) })
}

unsafe extern "C" fn allocate(size: CFIndex, _hint: usize, _info: *mut c_void) -> *mut c_void {
    if size <= 0 {
        return ptr::null_mut();
    }
    // SAFETY: plain malloc.
    unsafe { malloc(size as usize) }
}

unsafe extern "C" fn wipe_and_free(block: *mut c_void, _info: *mut c_void) {
    if block.is_null() {
        return;
    }
    // SAFETY: `block` came from malloc (`allocate`, `wiped_copy`); malloc_size is its usable size.
    unsafe {
        let len = malloc_size(block);
        zeroize::Zeroize::zeroize(std::slice::from_raw_parts_mut(block.cast::<u8>(), len));
        free(block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataCreateMutable(alloc: CFTypeRef, capacity: CFIndex) -> CFTypeRef;
        fn CFDataGetLength(data: CFTypeRef) -> CFIndex;
        fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
    }

    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        static kCGImagePropertyOrientation: CFTypeRef;
        fn CGImageDestinationCreateWithData(
            data: CFTypeRef,
            kind: CFTypeRef,
            count: usize,
            options: CFTypeRef,
        ) -> CFTypeRef;
        fn CGImageDestinationAddImage(destination: CFTypeRef, image: CFTypeRef, properties: CFTypeRef);
        fn CGImageDestinationFinalize(destination: CFTypeRef) -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGBitmapContextCreateImage(context: CFTypeRef) -> CFTypeRef;
    }

    const RED: [u8; 3] = [200, 40, 40];
    const BLUE: [u8; 3] = [40, 40, 200];

    /// A `width` x `height` HEIC made by ImageIO in memory, its left half red and right half blue, tagged with the
    /// EXIF `orientation`.
    fn heic(width: usize, height: usize, orientation: i64) -> Vec<u8> {
        let mut pixels = vec![0u8; width * height * 4];
        for (i, px) in pixels.chunks_exact_mut(4).enumerate() {
            let colour = if i % width < width / 2 { RED } else { BLUE };
            px[..3].copy_from_slice(&colour);
            px[3] = 255;
        }
        unsafe {
            let space = Cf::new(CGColorSpaceCreateWithName(kCGColorSpaceSRGB)).unwrap();
            let context = Cf::new(CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * 4,
                space.0,
                RGBA_PREMULTIPLIED,
            ))
            .unwrap();
            let image = Cf::new(CGBitmapContextCreateImage(context.0)).unwrap();
            let out = Cf::new(CFDataCreateMutable(ptr::null(), 0)).unwrap();
            let kind = "public.heic";
            let kind = Cf::new(CFStringCreateWithBytes(
                ptr::null(),
                kind.as_ptr(),
                kind.len() as CFIndex,
                STRING_UTF8,
                0,
            ))
            .unwrap();
            let destination = Cf::new(CGImageDestinationCreateWithData(out.0, kind.0, 1, ptr::null())).unwrap();
            let orientation =
                Cf::new(CFNumberCreate(ptr::null(), NUMBER_SINT64, addr_of!(orientation).cast())).unwrap();
            let properties = dictionary(&[kCGImagePropertyOrientation], &[orientation.0]).unwrap();
            CGImageDestinationAddImage(destination.0, image.0, properties.0);
            assert!(CGImageDestinationFinalize(destination.0), "ImageIO wrote no HEIC");
            drop(destination);
            std::slice::from_raw_parts(CFDataGetBytePtr(out.0), CFDataGetLength(out.0) as usize).to_vec()
        }
    }

    /// A DNG the way cameras lay it out (IFD0 a small RGB preview, its SubIFD the raw Bayer data, RGGB, 16 bits), raw
    /// `width` x `height`, left half red and right half blue, tagged with the EXIF `orientation`. ImageIO writes no
    /// DNG, so it is built here. ImageIO reads a file this small as a plain TIFF (its preview), so the raw is large
    /// enough (about 160 KB at 384 x 192) to be read as `com.adobe.raw-image`; the preview is a quarter of the raw's
    /// size, so the thumbnail's size tells which one was decoded.
    fn dng(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        const SHORT: u16 = 3;
        const LONG: u16 = 4;
        const ASCII: u16 = 2;
        const BYTE: u16 = 1;
        const RATIONAL: u16 = 5;
        const SRATIONAL: u16 = 10;
        enum Value {
            Bytes(Vec<u8>),
            Blob(usize),
            Ifd(usize),
        }
        let le16 = |v: &[u16]| v.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let le32 = |v: &[u32]| v.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let (pw, ph) = (width / 4, height / 4);
        let mut raw = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let (r, g, b) = if x < width / 2 {
                    (3000u16, 600, 600)
                } else {
                    (600, 600, 3000)
                };
                let sample = [r, g, g, b][((y % 2) * 2 + (x % 2)) as usize];
                raw.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let mut preview = Vec::new();
        for _ in 0..ph {
            for x in 0..pw {
                preview.extend_from_slice(if x < pw / 2 { &RED } else { &BLUE });
            }
        }
        let blobs = [raw, preview];
        use Value::{Blob, Bytes, Ifd};
        let raw_ifd: Vec<(u16, u16, u32, Value)> = vec![
            (254, LONG, 1, Bytes(le32(&[0]))),
            (256, LONG, 1, Bytes(le32(&[width]))),
            (257, LONG, 1, Bytes(le32(&[height]))),
            (258, SHORT, 1, Bytes(le16(&[16]))),
            (259, SHORT, 1, Bytes(le16(&[1]))),
            (262, SHORT, 1, Bytes(le16(&[32803]))),
            (273, LONG, 1, Blob(0)),
            (277, SHORT, 1, Bytes(le16(&[1]))),
            (278, LONG, 1, Bytes(le32(&[height]))),
            (279, LONG, 1, Bytes(le32(&[blobs[0].len() as u32]))),
            (284, SHORT, 1, Bytes(le16(&[1]))),
            (33421, SHORT, 2, Bytes(le16(&[2, 2]))),
            (33422, BYTE, 4, Bytes(vec![0, 1, 1, 2])),
            (50714, LONG, 1, Bytes(le32(&[0]))),
            (50717, LONG, 1, Bytes(le32(&[4095]))),
            (50719, LONG, 2, Bytes(le32(&[0, 0]))),
            (50720, LONG, 2, Bytes(le32(&[width, height]))),
        ];
        let identity: Vec<u8> = [1i32, 0, 0, 0, 1, 0, 0, 0, 1]
            .iter()
            .flat_map(|v| [v.to_le_bytes(), 1i32.to_le_bytes()].concat())
            .collect();
        let main: Vec<(u16, u16, u32, Value)> = vec![
            (254, LONG, 1, Bytes(le32(&[1]))),
            (256, LONG, 1, Bytes(le32(&[pw]))),
            (257, LONG, 1, Bytes(le32(&[ph]))),
            (258, SHORT, 3, Bytes(le16(&[8, 8, 8]))),
            (259, SHORT, 1, Bytes(le16(&[1]))),
            (262, SHORT, 1, Bytes(le16(&[2]))),
            (271, ASCII, 8, Bytes(b"Beebeeb\0".to_vec())),
            (272, ASCII, 5, Bytes(b"Test\0".to_vec())),
            (273, LONG, 1, Blob(1)),
            (274, SHORT, 1, Bytes(le16(&[orientation]))),
            (277, SHORT, 1, Bytes(le16(&[3]))),
            (278, LONG, 1, Bytes(le32(&[ph]))),
            (279, LONG, 1, Bytes(le32(&[blobs[1].len() as u32]))),
            (284, SHORT, 1, Bytes(le16(&[1]))),
            (330, LONG, 1, Ifd(1)),
            (50706, BYTE, 4, Bytes(vec![1, 4, 0, 0])),
            (50707, BYTE, 4, Bytes(vec![1, 1, 0, 0])),
            (50708, ASCII, 13, Bytes(b"Beebeeb Test\0".to_vec())),
            (50721, SRATIONAL, 9, Bytes(identity)),
            (50728, RATIONAL, 3, Bytes(le32(&[1, 1, 1, 1, 1, 1]))),
            (50778, SHORT, 1, Bytes(le16(&[21]))),
        ];
        let ifds = [main, raw_ifd];
        let extra_of = |ifd: &[(u16, u16, u32, Value)]| -> usize {
            ifd.iter()
                .map(|e| match &e.3 {
                    Bytes(b) if b.len() > 4 => b.len().next_multiple_of(2),
                    _ => 0,
                })
                .sum()
        };
        let mut at = 8usize;
        let mut ifd_at = [0usize; 2];
        for (i, ifd) in ifds.iter().enumerate() {
            ifd_at[i] = at;
            at += 2 + 12 * ifd.len() + 4 + extra_of(ifd.as_slice());
        }
        let blob_at = [at, at + blobs[0].len()];
        let mut out = b"II*\0".to_vec();
        out.extend_from_slice(&(ifd_at[0] as u32).to_le_bytes());
        for (i, ifd) in ifds.iter().enumerate() {
            let extra_at = ifd_at[i] + 2 + 12 * ifd.len() + 4;
            let mut extra = Vec::new();
            out.extend_from_slice(&(ifd.len() as u16).to_le_bytes());
            for (tag, kind, count, value) in ifd {
                out.extend_from_slice(&tag.to_le_bytes());
                out.extend_from_slice(&kind.to_le_bytes());
                out.extend_from_slice(&count.to_le_bytes());
                let field = match value {
                    Blob(b) => (blob_at[*b] as u32).to_le_bytes().to_vec(),
                    Ifd(f) => (ifd_at[*f] as u32).to_le_bytes().to_vec(),
                    Bytes(b) if b.len() <= 4 => [b.as_slice(), &[0u8; 4][b.len()..]].concat(),
                    Bytes(b) => {
                        let field = ((extra_at + extra.len()) as u32).to_le_bytes().to_vec();
                        extra.extend_from_slice(b);
                        extra.resize(extra.len().next_multiple_of(2), 0);
                        field
                    }
                };
                out.extend_from_slice(&field);
            }
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&extra);
        }
        assert_eq!(out.len(), blob_at[0]);
        for blob in &blobs {
            out.extend_from_slice(blob);
        }
        out
    }

    fn pixel(thumbs: &Thumbs, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * thumbs.width + x) * 4) as usize;
        thumbs.rgba[at..at + 4].try_into().unwrap()
    }

    /// Clearly more red than blue.
    fn redder(px: [u8; 4]) -> bool {
        i32::from(px[0]) > i32::from(px[2]) + 60
    }

    /// Red on top and blue at the bottom: a left-red picture turned upright from EXIF orientation 6.
    fn assert_red_over_blue(thumbs: &Thumbs) {
        let (top, bottom) = (
            pixel(thumbs, thumbs.width / 2, 1),
            pixel(thumbs, thumbs.width / 2, thumbs.height - 2),
        );
        assert!(redder(top) && top[3] == 255, "top is not red: {top:?}");
        assert!(
            redder([bottom[2], 0, bottom[0], 0]) && bottom[3] == 255,
            "bottom is not blue: {bottom:?}"
        );
    }

    /// The entries of the temporary directories (`$TMPDIR` and the per-user one), less the `.tmp…` ones the
    /// `tempfile` crate makes for other tests running at the same time.
    fn temp_entries() -> BTreeSet<std::path::PathBuf> {
        let mut dirs = vec![std::env::temp_dir()];
        let mut buf = vec![0u8; 1024];
        unsafe extern "C" {
            fn confstr(name: i32, buf: *mut u8, len: usize) -> usize;
        }
        // _CS_DARWIN_USER_TEMP_DIR
        let len = unsafe { confstr(65537, buf.as_mut_ptr(), buf.len()) };
        if len > 1 && len <= buf.len() {
            dirs.push(String::from_utf8_lossy(&buf[..len - 1]).into_owned().into());
        }
        dirs.iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with(".tmp"))
            .map(|entry| entry.path())
            .collect()
    }

    #[test]
    fn heic_and_dng_thumbs_upright_from_memory_and_no_file_written() {
        let before = temp_entries();

        let turned = super::super::generate_for(&heic(64, 32, 6), "image/heic").expect("a HEIC decodes");
        assert_eq!(
            (turned.width, turned.height),
            (32, 64),
            "EXIF orientation 6 turns it upright"
        );
        assert_red_over_blue(&turned);

        let upright = super::super::generate_for(&heic(64, 32, 1), "image/heif").expect("a HEIF decodes");
        assert_eq!((upright.width, upright.height), (64, 32));
        let (left, right) = (pixel(&upright, 2, 16), pixel(&upright, 61, 16));
        assert!(
            redder(left) && redder([right[2], 0, right[0], 0]),
            "left {left:?} right {right:?}"
        );

        let large = super::super::generate_for(&heic(4000, 1000, 6), "image/heic").expect("a large HEIC decodes");
        assert_eq!(
            (large.width, large.height),
            (512, 2048),
            "the long edge is at most SOURCE_PX"
        );

        let bytes = dng(384, 192, 6);
        assert!(bytes.len() < 200 * 1024);
        let raw = super::super::generate_for(&bytes, "image/x-adobe-dng").expect("a DNG decodes");
        assert_eq!(
            (raw.width, raw.height),
            (192, 384),
            "the raw data turned upright, not the 96 x 48 preview"
        );
        assert_red_over_blue(&raw);

        let after = temp_entries();
        let new: Vec<_> = after.difference(&before).collect();
        assert!(new.is_empty(), "the decode wrote to the temp directory: {new:?}");
    }

    #[test]
    fn garbage_or_cut_bytes_are_none_without_a_panic() {
        let garbage: Vec<u8> = (0..4096u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        for mime in [
            "image/heic",
            "image/heif",
            "image/x-adobe-dng",
            "image/x-canon-cr3",
            "image/x-nikon-nef",
        ] {
            assert!(generate(&garbage, mime).is_none(), "{mime}");
            assert!(super::super::generate_for(&garbage, mime).is_none(), "{mime}");
            assert!(generate(&[], mime).is_none());
            assert!(generate(b"ftypheic", mime).is_none());
        }
        // A file cut short: whatever ImageIO makes of it, no panic and nothing larger than SOURCE_PX.
        let whole = heic(64, 32, 1);
        if let Some(cut) = generate(&whole[..whole.len() / 2], "image/heic") {
            assert!(cut.width.max(cut.height) <= SOURCE_PX);
        }
        let whole = dng(384, 192, 1);
        if let Some(cut) = generate(&whole[..whole.len() / 3], "image/x-adobe-dng") {
            assert!(cut.width.max(cut.height) <= SOURCE_PX);
        }
    }

    #[test]
    fn only_heic_and_raw_kinds_are_decoded_here() {
        assert!(
            generate(&heic(16, 8, 1), "image/jpeg").is_none(),
            "a JPEG is the helper's"
        );
        assert_eq!(kind_of("IMAGE/HEIC"), Some("public.heic"));
        for mime in [
            "image/x-canon-cr2",
            "image/x-canon-cr3",
            "image/x-nikon-nef",
            "image/x-sony-arw",
            "image/x-adobe-dng",
            "image/x-fuji-raf",
            "image/x-olympus-orf",
            "image/x-panasonic-rw2",
        ] {
            assert!(kind_of(mime).is_some(), "{mime}");
        }
        assert!(kind_of("image/png").is_none() && kind_of("video/quicktime").is_none());
    }

    #[test]
    fn premultiplied_pixels_are_made_straight() {
        let mut px = [64u8, 32, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0];
        unpremultiply(&mut px);
        assert_eq!(px, [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
    }
}
