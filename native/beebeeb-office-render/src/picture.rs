//! Drive-macos-92: a photo the app's own decoders do not read (a HEIC or HEIF, a camera's RAW), decoded HERE, in the
//! confined helper, never in the app that holds the keys. The app asks for it as [`DocKind::Picture`]: the decode is
//! the `Open`, its answer one page whose size in points is the picture's size in pixels, and page 0 at scale 1 the
//! picture itself, upright (EXIF orientation applied), at most [`MAX_PICTURE_PX`] a side, straight sRGB RGBA.
//!
//! On macOS ImageIO decodes it from memory: the bytes go to ImageIO as a `CFData` (no file URL, no temp file, no
//! QuickLook), and the copy ImageIO reads is wiped when CoreFoundation frees it. Elsewhere there is no decoder here:
//! `Unsupported`.
//!
//! The process holds one document (the serve loop takes one `Open`); its memory watch and the app's timers bound the
//! decode as they bound a page. Plain C calls to CoreFoundation, ImageIO and CoreGraphics: no crate is added.
//!
//! [`DocKind::Picture`]: beebeeb_office_protocol::office::DocKind::Picture

use beebeeb_office_protocol::office::{
    CellData, CellRange, ErrorCode, Opened, OutlineEntry, PageSize, PictureKind, Rect,
};

use crate::engine::{OpenDoc, Opening};

/// The longest side of the decoded picture: the app's thumbnails' source size (`thumbs.rs`, `SOURCE_PX`).
pub const MAX_PICTURE_PX: u32 = 2048;

/// A decoded picture: straight RGBA, `width` x `height`. Wiped when dropped.
pub(crate) struct PictureDoc {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Drop for PictureDoc {
    fn drop(&mut self) {
        wipe(&mut self.rgba);
    }
}

/// Zeroes `bytes` with writes the compiler keeps.
#[allow(unsafe_code)]
fn wipe(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: a valid, exclusive reference into the slice.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// Called once before the helper confines itself ([`crate::start`]): ImageIO reads the process's main bundle
/// (`CFBundleGetMainBundle`, its info dictionary) before it decodes, and a helper that confined itself outside a
/// bundle cannot load it any more (measured: ImageIO then crashes on the null bundle). Loaded here, it is cached for
/// the process. Reads no document; in the XPC service it finds the service's own bundle.
pub fn prepare() {
    #[cfg(target_os = "macos")]
    mac::prepare();
}

/// Decodes `bytes` as a `kind` photo. `Damaged`: the system's decoder could not read it; `Unsupported`: no decoder
/// on this system.
pub(crate) fn open(kind: PictureKind, bytes: &[u8]) -> Result<PictureDoc, ErrorCode> {
    if bytes.is_empty() {
        return Err(ErrorCode::Damaged);
    }
    #[cfg(target_os = "macos")]
    {
        let (width, height, rgba) = mac::decode(bytes, mac::uti(kind)).ok_or(ErrorCode::Damaged)?;
        Ok(PictureDoc { width, height, rgba })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Err(ErrorCode::Unsupported)
    }
}

impl PictureDoc {
    /// Only page 0, whole, at scale 1: the picture as it was decoded.
    fn check(&self, page: u32, scale: f32, region: Option<Rect>) -> Result<(), ErrorCode> {
        if page != 0 || scale != 1.0 || region.is_some() {
            return Err(ErrorCode::Unsupported);
        }
        Ok(())
    }
}

impl OpenDoc for PictureDoc {
    fn opened(&self) -> Opening {
        Opening::Pages(Opened {
            pages: 1,
            sizes: vec![PageSize {
                w_pt: self.width as f32,
                h_pt: self.height as f32,
            }],
        })
    }

    fn render_size(&self, page: u32, scale: f32, region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        self.check(page, scale, region)?;
        Ok((self.width, self.height))
    }

    fn render(&self, page: u32, scale: f32, region: Option<Rect>, out: &mut [u8]) -> Result<bool, ErrorCode> {
        self.check(page, scale, region)?;
        if out.len() != self.rgba.len() {
            return Err(ErrorCode::Internal);
        }
        out.copy_from_slice(&self.rgba);
        Ok(false)
    }

    fn outline(&self) -> Vec<OutlineEntry> {
        Vec::new()
    }

    fn words(&self) -> u32 {
        0
    }

    fn cells(&self, _sheet: u32, _range: CellRange) -> Result<CellData, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn anchor_size(&self, _sheet: u32, _anchor: u32, _scale: f32) -> Result<(u32, u32), ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn render_anchor(&self, _sheet: u32, _anchor: u32, _scale: f32, _out: &mut [u8]) -> Result<bool, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac {
    //! ImageIO, from memory. Moved here from the app's `thumbs_mac.rs` (drive-macos-81), unchanged in what it does.

    use std::ffi::c_void;
    use std::ptr::{self, addr_of};
    use std::sync::OnceLock;

    use beebeeb_office_protocol::office::PictureKind;

    use super::{MAX_PICTURE_PX, wipe};

    pub(super) type CFTypeRef = *const c_void;
    pub(super) type CFIndex = isize;

    #[repr(C)]
    pub(super) struct CallBacks {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub(super) struct CGRect {
        pub(super) x: f64,
        pub(super) y: f64,
        pub(super) width: f64,
        pub(super) height: f64,
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
        pub(super) fn CFNumberCreate(alloc: CFTypeRef, the_type: CFIndex, value: *const c_void) -> CFTypeRef;
        pub(super) fn CFStringCreateWithBytes(
            alloc: CFTypeRef,
            bytes: *const u8,
            len: CFIndex,
            encoding: u32,
            external: u8,
        ) -> CFTypeRef;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFBundleGetMainBundle() -> CFTypeRef;
        fn CFBundleGetInfoDictionary(bundle: CFTypeRef) -> CFTypeRef;
    }

    /// See [`super::prepare`]. Neither object is owned here ("Get" calls): nothing is released.
    pub(super) fn prepare() {
        // SAFETY: plain CF getters; the dictionary is asked only of a bundle that exists.
        unsafe {
            let bundle = CFBundleGetMainBundle();
            if !bundle.is_null() {
                CFBundleGetInfoDictionary(bundle);
            }
        }
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
        pub(super) static kCGColorSpaceSRGB: CFTypeRef;
        pub(super) fn CGColorSpaceCreateWithName(name: CFTypeRef) -> CFTypeRef;
        fn CGImageGetWidth(image: CFTypeRef) -> usize;
        fn CGImageGetHeight(image: CFTypeRef) -> usize;
        pub(super) fn CGBitmapContextCreate(
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

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    unsafe extern "C" {
        fn malloc(size: usize) -> *mut c_void;
        fn free(ptr: *mut c_void);
        fn malloc_size(ptr: *const c_void) -> usize;
    }

    pub(super) const NUMBER_SINT64: CFIndex = 4;
    pub(super) const STRING_UTF8: u32 = 0x0800_0100;
    /// `kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big`: R, G, B, A bytes in that order.
    pub(super) const RGBA_PREMULTIPLIED: u32 = 1 | (4 << 12);
    const BLEND_COPY: i32 = 17;

    /// A CoreFoundation object this code owns: released when dropped.
    pub(super) struct Cf(pub(super) CFTypeRef);

    impl Cf {
        pub(super) fn new(made: CFTypeRef) -> Option<Cf> {
            (!made.is_null()).then_some(Cf(made))
        }
    }

    impl Drop for Cf {
        fn drop(&mut self) {
            // SAFETY: a Cf holds one reference it was given by a CF "Create" call, released once.
            unsafe { CFRelease(self.0) }
        }
    }

    /// The UTI ImageIO is told the bytes are.
    pub(super) fn uti(kind: PictureKind) -> &'static str {
        match kind {
            PictureKind::Heic => "public.heic",
            PictureKind::Heif => "public.heif",
            PictureKind::Dng => "com.adobe.raw-image",
            PictureKind::Cr2 => "com.canon.cr2-raw-image",
            PictureKind::Cr3 => "com.canon.cr3-raw-image",
            PictureKind::Nef => "com.nikon.raw-image",
            PictureKind::Arw => "com.sony.arw-raw-image",
            PictureKind::Raf => "com.fuji.raw-image",
            PictureKind::Orf => "com.olympus.raw-image",
            PictureKind::Rw2 => "com.panasonic.rw2-raw-image",
        }
    }

    /// Balances an autorelease pool push when dropped: what ImageIO autoreleases on this thread goes with it.
    struct Pool(*mut c_void);

    impl Drop for Pool {
        fn drop(&mut self) {
            // SAFETY: the token of the push on this thread, popped once, in order.
            unsafe { objc_autoreleasePoolPop(self.0) }
        }
    }

    /// `bytes` as an upright picture of at most [`MAX_PICTURE_PX`] a side: width, height, straight RGBA. `None`:
    /// ImageIO could not read it.
    pub(super) fn decode(bytes: &[u8], hint: &str) -> Option<(u32, u32, Vec<u8>)> {
        // SAFETY: a push with its pop when `_pool` drops, on this thread.
        let _pool = Pool(unsafe { objc_autoreleasePoolPush() });
        let data = wiped_copy(bytes)?;
        // SAFETY: every pointer handed to CF below is a live CF object or a live local; every object made is owned by
        // a Cf and released once.
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
            let max_px = i64::from(MAX_PICTURE_PX);
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

    /// The CGImage `image` drawn as straight (not premultiplied) sRGB RGBA.
    unsafe fn rgba_of(image: &Cf) -> Option<(u32, u32, Vec<u8>)> {
        // SAFETY (whole body): `image` is a live CGImage; the context draws into `rgba`, which outlives it.
        unsafe {
            let (width, height) = (CGImageGetWidth(image.0), CGImageGetHeight(image.0));
            if width == 0 || height == 0 || width.max(height) > MAX_PICTURE_PX as usize {
                return None;
            }
            let mut rgba = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
            let drawn = (|| {
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
                Some(())
            })();
            // Nothing drawn: ImageIO's decoder could not run here (measured under the dev profile, where the HEVC
            // decoder is denied: the picture comes back fully transparent). No picture beats a blank one.
            if drawn.is_none() || rgba.chunks_exact(4).all(|px| px[3] == 0) {
                wipe(&mut rgba);
                return None;
            }
            unpremultiply(&mut rgba);
            Some((width as u32, height as u32, rgba))
        }
    }

    pub(super) fn unpremultiply(rgba: &mut [u8]) {
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
    pub(super) unsafe fn dictionary(keys: &[CFTypeRef], values: &[CFTypeRef]) -> Option<Cf> {
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

    // ---- the wiped copy of the file's bytes -----------------------------------------------------------------------

    /// A copy of `bytes` as a CFData that owns it: freed (and wiped first) by CoreFoundation whenever the last of
    /// ImageIO's references goes, so no reference ImageIO keeps can outlive the bytes it reads.
    fn wiped_copy(bytes: &[u8]) -> Option<Cf> {
        let allocator = WIPING.get_or_init(make_wiping).0;
        if allocator.is_null() {
            return None;
        }
        // SAFETY: a fresh block of `bytes.len()` bytes; on success the CFData owns it and frees it through
        // `allocator`.
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
            wipe(std::slice::from_raw_parts_mut(block.cast::<u8>(), len));
            free(block);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(unsafe_code)]
#[path = "picture_tests.rs"]
mod tests;
