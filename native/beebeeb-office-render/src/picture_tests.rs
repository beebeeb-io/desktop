//! The helper's picture decode (drive-macos-92): a HEIC and a DNG made in memory open as one upright page, garbage
//! is `Damaged`, and only page 0 at scale 1 is drawn. (No file is written: the helper's sandbox has no write.)

use std::ptr::{self, addr_of};

use beebeeb_office_protocol::office::{DocKind, ErrorCode, PictureKind};

use super::MAX_PICTURE_PX;
use super::mac::{
    CFIndex, CFNumberCreate, CFStringCreateWithBytes, CFTypeRef, CGBitmapContextCreate, CGColorSpaceCreateWithName, Cf,
    NUMBER_SINT64, RGBA_PREMULTIPLIED, STRING_UTF8, dictionary, kCGColorSpaceSRGB, unpremultiply,
};
use crate::Office;
use crate::engine::{Engine, Opening};

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

/// A decoded picture as the app gets it: the page's size and its pixels at scale 1.
struct Pic {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// What the helper answers to `Open { Picture(kind) }` and then `Render { page 0, scale 1 }`.
fn open(bytes: &[u8], kind: PictureKind) -> Result<Pic, ErrorCode> {
    let doc = Office.open(DocKind::Picture(kind), bytes)?;
    let Opening::Pages(opened) = doc.opened() else {
        panic!("a picture opens as pages")
    };
    assert_eq!(opened.pages, 1);
    let (width, height) = (opened.sizes[0].w_pt as u32, opened.sizes[0].h_pt as u32);
    assert_eq!(
        (opened.sizes[0].w_pt, opened.sizes[0].h_pt),
        (width as f32, height as f32)
    );
    assert_eq!(doc.render_size(0, 1.0, None)?, (width, height));
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    assert!(!doc.render(0, 1.0, None, &mut rgba)?, "nothing is missing");
    Ok(Pic { width, height, rgba })
}

const RED: [u8; 3] = [200, 40, 40];
const BLUE: [u8; 3] = [40, 40, 200];

/// A `width` x `height` HEIC made by ImageIO in memory, its left half red and right half blue, tagged with the
/// EXIF `orientation`.
pub(crate) fn heic(width: usize, height: usize, orientation: i64) -> Vec<u8> {
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
        let orientation = Cf::new(CFNumberCreate(ptr::null(), NUMBER_SINT64, addr_of!(orientation).cast())).unwrap();
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
pub(crate) fn dng(width: u32, height: u32, orientation: u16) -> Vec<u8> {
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

fn pixel(pic: &Pic, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * pic.width + x) * 4) as usize;
    pic.rgba[at..at + 4].try_into().unwrap()
}

/// Clearly more red than blue.
fn redder(px: [u8; 4]) -> bool {
    i32::from(px[0]) > i32::from(px[2]) + 60
}

/// Red on top and blue at the bottom: a left-red picture turned upright from EXIF orientation 6.
fn assert_red_over_blue(pic: &Pic) {
    let (top, bottom) = (pixel(pic, pic.width / 2, 1), pixel(pic, pic.width / 2, pic.height - 2));
    assert!(redder(top) && top[3] == 255, "top is not red: {top:?}");
    assert!(
        redder([bottom[2], 0, bottom[0], 0]) && bottom[3] == 255,
        "bottom is not blue: {bottom:?}"
    );
}

#[test]
fn heic_and_dng_open_upright_from_memory() {
    let turned = open(&heic(64, 32, 6), PictureKind::Heic).expect("a HEIC decodes");
    assert_eq!(
        (turned.width, turned.height),
        (32, 64),
        "EXIF orientation 6 turns it upright"
    );
    assert_red_over_blue(&turned);

    let upright = open(&heic(64, 32, 1), PictureKind::Heif).expect("a HEIF decodes");
    assert_eq!((upright.width, upright.height), (64, 32));
    let (left, right) = (pixel(&upright, 2, 16), pixel(&upright, 61, 16));
    assert!(
        redder(left) && redder([right[2], 0, right[0], 0]),
        "left {left:?} right {right:?}"
    );

    let large = open(&heic(4000, 1000, 6), PictureKind::Heic).expect("a large HEIC decodes");
    assert_eq!(
        (large.width, large.height),
        (512, 2048),
        "the long edge is at most MAX_PICTURE_PX"
    );
    assert_eq!(MAX_PICTURE_PX, 2048, "the app's SOURCE_PX");

    let bytes = dng(384, 192, 6);
    assert!(bytes.len() < 200 * 1024);
    let raw = open(&bytes, PictureKind::Dng).expect("a DNG decodes");
    assert_eq!(
        (raw.width, raw.height),
        (192, 384),
        "the raw data turned upright, not the 96 x 48 preview"
    );
    assert_red_over_blue(&raw);
}

#[test]
fn garbage_or_cut_bytes_are_damaged_without_a_panic() {
    let garbage: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    for kind in PictureKind::ALL {
        assert_eq!(open(&garbage, kind).err(), Some(ErrorCode::Damaged), "{kind:?}");
        assert_eq!(open(&[], kind).err(), Some(ErrorCode::Damaged));
        assert_eq!(open(b"ftypheic", kind).err(), Some(ErrorCode::Damaged));
    }
    // A file cut short: whatever ImageIO makes of it, no panic and nothing larger than MAX_PICTURE_PX.
    let whole = heic(64, 32, 1);
    if let Ok(cut) = open(&whole[..whole.len() / 2], PictureKind::Heic) {
        assert!(cut.width.max(cut.height) <= MAX_PICTURE_PX);
    }
    let whole = dng(384, 192, 1);
    if let Ok(cut) = open(&whole[..whole.len() / 3], PictureKind::Dng) {
        assert!(cut.width.max(cut.height) <= MAX_PICTURE_PX);
    }
}

#[test]
fn only_page_0_whole_at_scale_1_is_drawn() {
    let doc = Office
        .open(DocKind::Picture(PictureKind::Heic), &heic(16, 8, 1))
        .expect("a HEIC decodes");
    let mut out = vec![0u8; 16 * 8 * 4];
    let region = beebeeb_office_protocol::office::Rect { x: 0, y: 0, w: 4, h: 4 };
    assert_eq!(doc.render_size(1, 1.0, None), Err(ErrorCode::Unsupported));
    assert_eq!(doc.render_size(0, 2.0, None), Err(ErrorCode::Unsupported));
    assert_eq!(doc.render_size(0, 1.0, Some(region)), Err(ErrorCode::Unsupported));
    assert_eq!(
        doc.render(0, 1.0, None, &mut out[..8]),
        Err(ErrorCode::Internal),
        "a buffer of another size"
    );
    assert!(
        doc.cells(
            0,
            beebeeb_office_protocol::office::CellRange {
                row: 0,
                col: 0,
                rows: 1,
                cols: 1
            }
        )
        .is_err()
    );
    assert_eq!(doc.render(0, 1.0, None, &mut out), Ok(false));
}

#[test]
fn premultiplied_pixels_are_made_straight() {
    let mut px = [64u8, 32, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0];
    unpremultiply(&mut px);
    assert_eq!(px, [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
}
