//! Tests of `photo_meta.rs`, which reads untrusted file headers. Ported from
//! `src-tauri/src/drive_v2/status_tests.rs`.

use super::photo_meta::{self, PhotoMeta};

#[test]
fn exif_date_maps_and_rejects_empty_dates() {
    assert_eq!(
        photo_meta::exif_date("2026:10:12 14:03:22").as_deref(),
        Some("2026-10-12T14:03:22")
    );
    assert_eq!(photo_meta::exif_date("0000:00:00 00:00:00"), None);
    assert_eq!(photo_meta::exif_date("garbage"), None);
}

/// A little-endian TIFF block: IFD0 with Make, Model and the Exif IFD pointer; the Exif IFD with DateTimeOriginal and
/// the pixel size.
fn tiff() -> Vec<u8> {
    let mut b = b"II*\0".to_vec();
    b.extend(8u32.to_le_bytes());
    let ifd0 = 8;
    let exif = ifd0 + 2 + 3 * 12 + 4;
    let data = exif + 2 + 3 * 12 + 4;
    let make = b"Apple\0";
    let model = b"iPhone 15\0";
    let date = b"2026:10:12 14:03:22\0";
    let (make_at, model_at, date_at) = (data, data + make.len(), data + make.len() + model.len());
    let tag = |b: &mut Vec<u8>, t: u16, ty: u16, n: u32, v: u32| {
        b.extend(t.to_le_bytes());
        b.extend(ty.to_le_bytes());
        b.extend(n.to_le_bytes());
        b.extend(v.to_le_bytes());
    };
    b.extend(3u16.to_le_bytes());
    tag(&mut b, 0x010F, 2, make.len() as u32, make_at as u32);
    tag(&mut b, 0x0110, 2, model.len() as u32, model_at as u32);
    tag(&mut b, 0x8769, 4, 1, exif as u32);
    b.extend(0u32.to_le_bytes());
    b.extend(3u16.to_le_bytes());
    tag(&mut b, 0x9003, 2, date.len() as u32, date_at as u32);
    tag(&mut b, 0xA002, 3, 1, 4032);
    tag(&mut b, 0xA003, 4, 1, 3024);
    b.extend(0u32.to_le_bytes());
    b.extend(make);
    b.extend(model);
    b.extend(date);
    b
}

#[test]
fn exif_gives_camera_date_taken_and_size() {
    let mut meta = PhotoMeta::default();
    photo_meta::parse_exif(&tiff(), &mut meta);
    assert_eq!(meta.camera.as_deref(), Some("Apple iPhone 15"));
    assert_eq!(meta.taken_at.as_deref(), Some("2026-10-12T14:03:22"));
    assert_eq!((meta.width, meta.height), (Some(4032), Some(3024)));
    // Inside a JPEG's APP1 segment, through the head reader.
    let t = tiff();
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
    jpeg.extend(((t.len() + 8) as u16).to_be_bytes());
    jpeg.extend(b"Exif\0\0");
    jpeg.extend(&t);
    jpeg.extend([0xFF, 0xDA, 0, 2]);
    assert_eq!(
        photo_meta::from_head(&jpeg, false).camera.as_deref(),
        Some("Apple iPhone 15")
    );
}

#[test]
fn mp4_duration_reads_mvhd() {
    let mut mvhd = vec![0u8; 4]; // version 0, flags
    mvhd.extend(0u32.to_be_bytes()); // created
    mvhd.extend(0u32.to_be_bytes()); // modified
    mvhd.extend(600u32.to_be_bytes()); // time scale
    mvhd.extend(75_000u32.to_be_bytes()); // duration: 125 s
    let boxed = |kind: &[u8], body: &[u8]| {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend(kind);
        b.extend(body);
        b
    };
    let mut file = boxed(b"ftyp", b"isom0000");
    file.extend(boxed(b"moov", &boxed(b"mvhd", &mvhd)));
    assert_eq!(photo_meta::mp4_duration(&file), Some(125));
    assert_eq!(photo_meta::from_head(&file, true).duration_seconds, Some(125));
    assert_eq!(photo_meta::mp4_duration(b"not a movie"), None);
}

#[test]
fn image_header_gives_dimensions() {
    let mut png = Vec::new();
    image::RgbImage::new(3, 2)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let meta = photo_meta::from_head(&png, false);
    assert_eq!((meta.width, meta.height, meta.taken_at), (Some(3), Some(2), None));
}

#[test]
fn photo_meta_device_is_the_backups_device_folder() {
    let roots = vec!["Backups".to_string(), "/Old backups/".to_string()];
    assert_eq!(
        photo_meta::backup_device(&roots, "Backups/Anna’s iPhone/Camera uploads/IMG_1.heic").as_deref(),
        Some("Anna’s iPhone")
    );
    assert_eq!(
        photo_meta::backup_device(&roots, "/Backups/iPad/IMG_2.heic").as_deref(),
        Some("iPad"),
        "a leading slash in the stored path"
    );
    assert_eq!(
        photo_meta::backup_device(&roots, "Old backups/Old iPhone/IMG_3.heic").as_deref(),
        Some("Old iPhone"),
        "a second Backups root"
    );
    assert_eq!(
        photo_meta::backup_device(&roots, "Backups/IMG_4.heic"),
        None,
        "directly in the root"
    );
    assert_eq!(
        photo_meta::backup_device(&roots, "Lisbon trip/IMG_5.heic"),
        None,
        "outside Backups"
    );
    assert_eq!(
        photo_meta::backup_device(&roots, "Backups old/x/IMG_6.heic"),
        None,
        "a sibling with the root's prefix"
    );
    assert_eq!(
        photo_meta::backup_device(&[], "Backups/iPad/IMG_7.heic"),
        None,
        "no Backups root yet"
    );
}

/// Not in the Tauri file: the parsers read untrusted bytes, so truncated and empty input answers
/// "nothing" and never panics.
#[test]
fn truncated_and_empty_headers_give_nothing_and_do_not_panic() {
    let whole = tiff();
    for cut in 0..whole.len() {
        let mut meta = PhotoMeta::default();
        photo_meta::parse_exif(&whole[..cut], &mut meta);
    }
    assert_eq!(photo_meta::mp4_duration(&[]), None);
    assert_eq!(photo_meta::mp4_duration(&[0, 0, 0, 1, b'm', b'o', b'o', b'v']), None);
    let empty = photo_meta::from_head(&[], false);
    assert_eq!(
        (empty.width, empty.height, empty.camera, empty.taken_at),
        (None, None, None, None)
    );
}
