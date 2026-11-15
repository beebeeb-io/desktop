//! The preview seal against the iPhone app's recipe (both directions and a frozen vector made outside core), the
//! encoder (never upscaled, upright, no metadata), the plan's rules and the averages. Every key, id and picture is
//! invented.

use std::collections::HashMap;

use beebeeb_core::kdf::{MasterKey, derive_file_key};
use beebeeb_types::{CipherSuite, EncryptedBlob};

use super::encode::{self, decode, encode_preview, fitted};
use super::plan::{LocalPhoto, plan};
use super::seal::{SealRefused, open_preview, seal_preview};
use super::*;
use crate::state_db::FileStatus;

const MASTER: [u8; 32] = [7; 32];
const OWNED: &str = "c0000000-0000-4000-8000-000000000001";
/// A foreign-class id (version 8): its key is never derived.
const FOREIGN: &str = "c0000000-0000-8000-8000-000000000002";

/// The iPhone app's seal, composed as its native module does: `MasterKeyHandle::derive_file_key(id utf8)` then
/// `FileKeyHandle::encrypt_chunk`, sent as `nonce || ciphertext`.
fn phone_seal(master: [u8; 32], id: &str, plain: &[u8]) -> Vec<u8> {
    let key = derive_file_key(&MasterKey::from_bytes(master), id.as_bytes());
    let blob = beebeeb_core::encrypt::encrypt_chunk(&key, plain).unwrap();
    [blob.nonce, blob.ciphertext].concat()
}

/// The iPhone app's open: `FileKeyHandle::decrypt_chunk(nonce, ciphertext)`.
fn phone_open(master: [u8; 32], id: &str, wire: &[u8]) -> Option<Vec<u8>> {
    let key = derive_file_key(&MasterKey::from_bytes(master), id.as_bytes());
    let blob = EncryptedBlob {
        cipher_suite: CipherSuite::V1Aes256Gcm,
        nonce: wire[..12].to_vec(),
        ciphertext: wire[12..].to_vec(),
    };
    beebeeb_core::encrypt::decrypt_chunk(&key, &blob).ok()
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

#[test]
fn seal_opens_with_the_phone_recipe() {
    let plain = b"\xff\xd8\xff\xe0 an invented preview";
    let wire = seal_preview(&MASTER, OWNED, plain).unwrap();
    assert_eq!(wire.len(), plain.len() + seal::OVERHEAD);
    assert!(!wire.starts_with(&[0xFF, 0xD8]), "sealed, never the JPEG's bytes");
    assert_eq!(phone_open(MASTER, OWNED, &wire).as_deref(), Some(&plain[..]));
}

#[test]
fn phone_recipe_blob_opens_here() {
    let plain = b"\xff\xd8\xff\xe0 made on the phone";
    let wire = phone_seal(MASTER, OWNED, plain);
    assert_eq!(open_preview(&MASTER, OWNED, &wire).unwrap().as_slice(), plain);
    // Another file's key does not open it.
    assert_eq!(
        open_preview(&MASTER, "c0000000-0000-4000-8000-000000000009", &wire).unwrap_err(),
        SealRefused::Failed
    );
    // A blob of the thumbnail sealer (its own subkey and bound data) is no preview: the opener is the chunk
    // recipe's, not the thumbnail opener (which also takes the old unbound frame, so it would open both).
    let key = derive_file_key(&MasterKey::from_bytes(MASTER), OWNED.as_bytes());
    let thumbnail = beebeeb_core::encrypt::encrypt_thumbnail(&key, plain).unwrap();
    assert_eq!(
        beebeeb_core::encrypt::decrypt_thumbnail(&key, &thumbnail).unwrap(),
        plain
    );
    assert_eq!(
        open_preview(&MASTER, OWNED, &thumbnail).unwrap_err(),
        SealRefused::Failed
    );
}

/// Made once OUTSIDE core (Node's HKDF-SHA256 and AES-256-GCM, `info = "beebeeb-file-key-v1" || id`, no salt, a
/// fixed nonce): master key bytes `(i * 7 + 3) mod 256`, the id below. It pins the wire recipe, not core's functions.
#[test]
fn frozen_preview_vector_opens() {
    let master: [u8; 32] = std::array::from_fn(|i| ((i * 7 + 3) & 0xff) as u8);
    let id = "3f2a6c1e-8b4d-4e7a-9c15-2d8e6f0a1b3c";
    let wire = hex(
        "0102030405060708090a0b0cfe5ba58d139dd5b51338d8bc5a41c485305ab686709a0db16370151e4e9d23f731a2fe1dbe8a88e52fe7f7e5f08b1af8ed7f",
    );
    let plain = hex("ffd8ffe020612066726f7a656e2070686f746f207072657669657720766563746f72");
    assert_eq!(open_preview(&master, id, &wire).unwrap().as_slice(), &plain[..]);
}

#[test]
fn seal_refuses_a_foreign_id() {
    assert_eq!(seal_preview(&MASTER, FOREIGN, b"x").unwrap_err(), SealRefused::NotOwned);
    assert_eq!(
        seal_preview(&MASTER, "not-an-id", b"x").unwrap_err(),
        SealRefused::NotOwned
    );
    let wire = phone_seal(MASTER, OWNED, b"x");
    assert_eq!(
        open_preview(&MASTER, FOREIGN, &wire).unwrap_err(),
        SealRefused::NotOwned
    );
    // An owned id seals (the check can pass).
    assert!(seal_preview(&MASTER, OWNED, b"x").is_ok());
}

#[test]
fn seal_refuses_over_the_servers_cap() {
    let big = vec![0u8; seal::MAX_PLAINTEXT + 1];
    assert_eq!(seal_preview(&MASTER, OWNED, &big).unwrap_err(), SealRefused::TooLarge);
    let fits = vec![0u8; seal::MAX_PLAINTEXT];
    assert_eq!(seal_preview(&MASTER, OWNED, &fits).unwrap().len(), seal::MAX_WIRE);
}

/// A JPEG `w` by `h`: a smooth gradient, its top-left corner red.
pub(crate) fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let picture = image::RgbImage::from_fn(w, h, |x, y| {
        if x < w / 8 && y < h / 8 {
            image::Rgb([250, 0, 0])
        } else {
            image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, 120])
        }
    });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode(picture.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

/// `jpeg` with an EXIF APP1 segment right after its start: orientation `orientation`, and an invented camera make.
pub(crate) fn with_exif(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    // TIFF, big-endian: two entries (Make, Orientation), no next IFD.
    let mut tiff = b"MM\x00\x2a\x00\x00\x00\x08".to_vec();
    tiff.extend_from_slice(&2u16.to_be_bytes());
    // Make (0x010F), ASCII, 4 bytes inline: "Inv\0".
    tiff.extend_from_slice(&[0x01, 0x0F, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, b'I', b'n', b'v', 0]);
    // Orientation (0x0112), SHORT, 1.
    tiff.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01]);
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&[0, 0, 0, 0]);
    let mut app1 = b"Exif\x00\x00".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn size_of(jpeg: &[u8]) -> (u32, u32) {
    let picture = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).unwrap();
    (picture.width(), picture.height())
}

#[test]
fn encode_never_upscales_and_fits_long_edge() {
    assert_eq!(fitted(4000, 3000, 1920), (1920, 1440));
    assert_eq!(fitted(3000, 4000, 1920), (1440, 1920));
    assert_eq!(fitted(800, 600, 1920), (800, 600));
    let (big, at) = encode_preview(&jpeg(1600, 1200), 1280).unwrap();
    assert_eq!((at, size_of(&big)), ((1280, 960), (1280, 960)));
    let (small, at) = encode_preview(&jpeg(800, 600), 1920).unwrap();
    assert_eq!((at, size_of(&small)), ((800, 600), (800, 600)));
}

#[test]
fn encode_applies_exif_orientation() {
    // Stored 1600 wide and 1200 high; orientation 6 says "turn it a quarter clockwise": upright it is portrait.
    let source = with_exif(&jpeg(1600, 1200), 6);
    let (preview, at) = encode_preview(&source, 1280).unwrap();
    assert_eq!(at, (960, 1280));
    assert_eq!(size_of(&preview), (960, 1280));
    // The red corner was top-left as stored: upright it is top-right.
    let upright = decode(&preview).unwrap();
    assert_eq!(upright.size(), (960, 1280));
    let picture = image::load_from_memory(&preview).unwrap().to_rgb8();
    let corner = picture.get_pixel(940, 20);
    assert!(corner[0] > 200 && corner[1] < 60, "{corner:?}");
}

#[test]
fn encode_carries_no_exif() {
    let source = with_exif(&jpeg(1600, 1200), 1);
    assert!(
        source.windows(6).any(|w| w == b"Exif\x00\x00"),
        "the source carries EXIF"
    );
    let (preview, _) = encode_preview(&source, 1280).unwrap();
    assert!(preview.starts_with(&[0xFF, 0xD8]));
    assert!(!preview.windows(2).any(|w| w == [0xFF, 0xE1]), "an APP1 segment");
    assert!(!preview.windows(6).any(|w| w == b"Exif\x00\x00"));
    assert!(!preview.windows(4).any(|w| w == b"Inv\x00"), "the camera's make");
}

#[test]
fn encode_refuses_what_it_cannot_decode() {
    assert_eq!(
        encode_preview(b"these bytes are no picture", 1280).unwrap_err(),
        encode::EncodeRefused::Undecodable
    );
    for name in [
        "a.heic", "a.HEIF", "a.dng", "a.cr2", "a.tif", "a.tiff", "a.mov", "a.mp4", "a",
    ] {
        assert!(!encode::decodable(name), "{name}");
    }
    for name in ["a.jpg", "a.JPEG", "a.png", "a.webp", "a.gif", "a.bmp"] {
        assert!(encode::decodable(name), "{name}");
    }
}

fn photo(id: &str, name: &str, status: FileStatus) -> LocalPhoto {
    LocalPhoto {
        id: id.into(),
        rel_path: format!("Photos/{name}"),
        name: name.into(),
        bytes: 10,
        status,
        held_write: false,
        remote_updated_at: 0,
        behind: false,
    }
}

fn listed(ids: &[&str]) -> HashMap<String, PreviewState> {
    ids.iter()
        .map(|id| {
            (
                id.to_string(),
                PreviewState {
                    id: id.to_string(),
                    has_preview: false,
                    format: None,
                    px: None,
                    bytes: None,
                    updated_at: Some(0),
                    size: Some(10),
                },
            )
        })
        .collect()
}

const JPEG_1920: PreviewSetting = PreviewSetting {
    enabled: true,
    format: PreviewFormat::Jpeg,
    px: 1920,
};

fn planned(photos: &[LocalPhoto], owned: &HashMap<String, PreviewState>, force: bool) -> Vec<String> {
    plan(photos, owned, JPEG_1920, force)
        .into_iter()
        .map(|photo| photo.id.clone())
        .collect()
}

const A: &str = "c0000000-0000-4000-8000-0000000000a1";
const B: &str = "c0000000-0000-4000-8000-0000000000a2";
const C: &str = "c0000000-0000-4000-8000-0000000000a3";
const D: &str = "c0000000-0000-4000-8000-0000000000a4";
const E: &str = "c0000000-0000-4000-8000-0000000000a5";

#[test]
fn plan_skips_heic_raw_video_and_shared() {
    // The one that may be a source, so each "no" below is for its own reason.
    let good = photo(A, "Invented good.jpg", FileStatus::Local);
    let photos = vec![
        good.clone(),
        photo(B, "Invented phone.heic", FileStatus::Local),
        photo(C, "Invented raw.dng", FileStatus::Local),
        photo(D, "Invented clip.mov", FileStatus::Local),
        // In a folder shared with this account: the server does not list it among the account's own media.
        photo(E, "Invented shared.jpg", FileStatus::Local),
        // A foreign-class id (adopted): its key is not derived.
        photo(FOREIGN, "Invented adopted.jpg", FileStatus::Local),
    ];
    let owned = listed(&[A, B, C, D, FOREIGN]);
    assert_eq!(planned(&photos, &owned, false), [A]);
    assert_eq!(planned(&photos, &owned, true), [A]);
}

#[test]
fn plan_skips_uploading_held_and_pending() {
    let mut held = photo(C, "Invented held.jpg", FileStatus::Local);
    held.held_write = true;
    let photos = vec![
        photo(A, "Invented good.jpg", FileStatus::Local),
        photo(B, "Invented going.jpg", FileStatus::Uploading),
        held,
        photo(D, "Invented cloud.jpg", FileStatus::CloudOnly),
        photo(E, "Invented conflict.jpg", FileStatus::Conflict),
    ];
    assert_eq!(planned(&photos, &listed(&[A, B, C, D, E]), false), [A]);
}

#[test]
fn plan_skips_matching_unless_rebuild() {
    let photos = vec![
        photo(A, "Invented one.jpg", FileStatus::Local),
        photo(B, "Invented two.jpg", FileStatus::Local),
        photo(C, "Invented three.jpg", FileStatus::Local),
    ];
    let mut owned = listed(&[A, B, C]);
    let stored = |format, px| PreviewState {
        id: String::new(),
        has_preview: true,
        format: Some(format),
        px: Some(px),
        bytes: Some(1),
        updated_at: Some(0),
        size: Some(10),
    };
    // A matches; B is the same format at another size; C another format at the same size.
    owned.insert(A.into(), stored(PreviewFormat::Jpeg, 1920));
    owned.insert(B.into(), stored(PreviewFormat::Jpeg, 2560));
    owned.insert(C.into(), stored(PreviewFormat::Heic, 1920));
    assert_eq!(planned(&photos, &owned, false), [B, C]);
    assert_eq!(planned(&photos, &owned, true), [A, B, C]);
}

#[test]
fn plan_skips_a_dehydrated_local_row() {
    use crate::cloud_attrs::bytes_not_here;
    // The three attributes of a placeholder whose bytes are in the cloud, each alone and with ordinary ones.
    for attrs in [0x0040_0000, 0x0004_0000, 0x0000_1000, 0x0040_0020, 0x0004_0001 | 0x80] {
        assert!(bytes_not_here(attrs), "{attrs:#x}");
    }
    // A file whose bytes are here (archive, normal, pinned, read-only).
    for attrs in [0x20, 0x80, 0x0008_0000 | 0x20, 0x01] {
        assert!(!bytes_not_here(attrs), "{attrs:#x}");
    }
}

/// On Windows: a real file marked OFFLINE is skipped by the plan's last check and never opened.
#[cfg(target_os = "windows")]
#[test]
fn plan_skips_an_offline_file_on_windows() {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_OFFLINE, FILE_FLAGS_AND_ATTRIBUTES, SetFileAttributesW};
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("Photos")).unwrap();
    let mut here = photo(A, "Invented here.jpg", FileStatus::Local);
    let mut freed = photo(B, "Invented freed.jpg", FileStatus::Local);
    for p in [&mut here, &mut freed] {
        std::fs::write(root.path().join(&p.rel_path), b"0123456789").unwrap();
        p.bytes = 10;
    }
    let path: Vec<u16> = root
        .path()
        .join(&freed.rel_path)
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    unsafe {
        SetFileAttributesW(
            windows::core::PCWSTR(path.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(FILE_ATTRIBUTE_OFFLINE.0),
        )
        .unwrap();
    }
    assert!(super::plan::ready_to_read(root.path(), &here).is_some());
    assert!(super::plan::ready_to_read(root.path(), &freed).is_none());
}

#[test]
fn a_file_whose_size_is_not_the_rows_is_not_read() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("Photos")).unwrap();
    let mut changed = photo(A, "Invented changed.jpg", FileStatus::Local);
    std::fs::write(root.path().join(&changed.rel_path), b"0123456789").unwrap();
    changed.bytes = 10;
    assert!(super::plan::ready_to_read(root.path(), &changed).is_some());
    changed.bytes = 11;
    assert!(super::plan::ready_to_read(root.path(), &changed).is_none());
    // A path that leaves the sync folder is never read.
    changed.rel_path = "../outside.jpg".into();
    assert!(super::plan::ready_to_read(root.path(), &changed).is_none());
}

/// The server's summary has no bytes per setting: each setting's average comes from the per-file `preview_bytes`.
#[test]
fn averages_are_per_setting_from_each_files_bytes() {
    let state = |format, px, bytes| PreviewState {
        id: String::new(),
        has_preview: true,
        format: Some(format),
        px: Some(px),
        bytes: Some(bytes),
        updated_at: Some(0),
        size: Some(10),
    };
    let states = vec![
        state(PreviewFormat::Jpeg, 1920, 300),
        state(PreviewFormat::Jpeg, 1920, 500),
        state(PreviewFormat::Heic, 2560, 900),
        // Not counted: no preview, or its bytes unknown.
        PreviewState {
            has_preview: false,
            ..state(PreviewFormat::Jpeg, 1920, 10_000)
        },
        PreviewState {
            bytes: None,
            ..state(PreviewFormat::Jpeg, 1920, 0)
        },
    ];
    let mut got = averages(&states);
    got.sort_by_key(|(at, _, _)| at.1);
    assert_eq!(
        got,
        [
            ((PreviewFormat::Jpeg, 1920), 400, 2),
            ((PreviewFormat::Heic, 2560), 900, 1)
        ]
    );
}

#[test]
fn the_setting_reads_leniently_and_round_trips() {
    let value = serde_json::json!({ "enabled": true, "format": "jpeg", "px": 1920 });
    let setting = PreviewSetting::from_value(&value);
    assert_eq!(setting, JPEG_1920);
    assert_eq!(setting.to_value(), value);
    assert_eq!(
        PreviewSetting::from_value(&serde_json::Value::Null),
        PreviewSetting::default()
    );
    let odd = serde_json::json!({ "enabled": "yes", "format": "png", "px": 1000 });
    assert_eq!(PreviewSetting::from_value(&odd), PreviewSetting::default());
}

/// A start that lands as the run's thread found no more work, while it still says it runs, leaves its work to that
/// thread: the thread looks at the queue again after it said it no longer runs, and runs it.
#[test]
fn a_start_as_the_thread_goes_idle_is_run() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use super::runner::{PreviewRun, RunEnd};
    use crate::account::SessionGeneration;

    let run = PreviewRun::new();
    let (at_idle_tx, at_idle) = mpsc::channel::<()>();
    let (go_on_tx, go_on) = mpsc::channel::<()>();
    let go_on = Mutex::new(go_on);
    let first = AtomicBool::new(true);
    run.before_idle(Arc::new(move || {
        if first.swap(false, Ordering::SeqCst) {
            at_idle_tx.send(()).unwrap();
            go_on
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .expect("the test let the thread go");
        }
    }));
    let ran = Arc::new(AtomicU32::new(0));
    let work = |ran: &Arc<AtomicU32>| {
        let ran = ran.clone();
        Box::new(move |_: &AtomicBool, _: &dyn Fn(u32, u32)| {
            ran.fetch_add(1, Ordering::SeqCst);
            RunEnd::default()
        })
    };
    assert!(run.start(SessionGeneration::for_test(1), work(&ran)));
    at_idle
        .recv_timeout(Duration::from_secs(10))
        .expect("the thread found no more work");
    assert!(run.running(), "the thread still says it runs");
    assert!(run.start(SessionGeneration::for_test(1), work(&ran)));
    go_on_tx.send(()).unwrap();
    let began = Instant::now();
    while ran.load(Ordering::SeqCst) < 2 || run.running() {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "the start that came as the thread went idle never ran ({} ran)",
            ran.load(Ordering::SeqCst)
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(ran.load(Ordering::SeqCst), 2);
}

/// A start stops the run on its way, never the work it queued: that work runs to its end.
#[test]
fn a_start_stops_the_run_before_it_and_not_itself() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use super::runner::{PreviewRun, RunEnd};
    use crate::account::SessionGeneration;

    let run = PreviewRun::new();
    let (began_tx, began) = mpsc::channel();
    let first_saw_stop = Arc::new(AtomicBool::new(false));
    let saw = first_saw_stop.clone();
    assert!(run.start(
        SessionGeneration::for_test(1),
        Box::new(move |stop: &AtomicBool, _: &dyn Fn(u32, u32)| {
            began_tx.send(()).unwrap();
            for _ in 0..1_000 {
                if stop.load(Ordering::SeqCst) {
                    saw.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            RunEnd::default()
        })
    ));
    began.recv_timeout(Duration::from_secs(5)).expect("the first run began");
    let (done_tx, done) = mpsc::channel();
    let polls = Arc::new(AtomicU32::new(0));
    let counted = polls.clone();
    assert!(run.start(
        SessionGeneration::for_test(1),
        Box::new(move |stop: &AtomicBool, _: &dyn Fn(u32, u32)| {
            for _ in 0..20 {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                counted.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(1));
            }
            done_tx.send(()).unwrap();
            RunEnd::default()
        })
    ));
    done.recv_timeout(Duration::from_secs(10)).expect("the second run ran");
    assert!(first_saw_stop.load(Ordering::SeqCst), "the first run was asked to stop");
    assert_eq!(
        polls.load(Ordering::SeqCst),
        20,
        "the second run was never told to stop"
    );
}

/// A row is a source only while the server's version is the one it holds: not changed after the row's version, the
/// same size, and both known.
#[test]
fn plan_skips_a_row_that_is_not_the_servers_version() {
    let photos = [photo(A, "a.jpg", FileStatus::Local)];
    let mut owned = listed(&[A]);
    assert_eq!(
        planned(&photos, &owned, false),
        [A],
        "the same version (the check can pass)"
    );
    owned.get_mut(A).unwrap().updated_at = Some(1);
    assert!(
        planned(&photos, &owned, false).is_empty(),
        "changed on the server after this PC's version"
    );
    owned.get_mut(A).unwrap().updated_at = Some(0);
    owned.get_mut(A).unwrap().size = Some(11);
    assert!(planned(&photos, &owned, false).is_empty(), "another size");
    owned.get_mut(A).unwrap().size = None;
    assert!(planned(&photos, &owned, false).is_empty(), "an unknown size");
    owned.get_mut(A).unwrap().size = Some(10);
    owned.get_mut(A).unwrap().updated_at = None;
    assert!(planned(&photos, &owned, false).is_empty(), "an unknown version");
}

/// The CRC-32 of a PNG chunk (its type and data).
fn png_crc(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut typed = kind.to_vec();
    typed.extend_from_slice(data);
    out.extend_from_slice(&typed);
    out.extend_from_slice(&png_crc(&typed).to_be_bytes());
}

/// A few hundred bytes that declare a 16384x16384 16-bit RGBA picture with an EXIF orientation (a quarter turn):
/// decoding it would take 2 GiB, and turning it 2 GiB more. Its pixel data is not even there.
fn declared_huge_png() -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::new();
    header.extend_from_slice(&16_384u32.to_be_bytes());
    header.extend_from_slice(&16_384u32.to_be_bytes());
    header.extend_from_slice(&[16, 6, 0, 0, 0]);
    png_chunk(&mut png, b"IHDR", &header);
    // EXIF (big-endian TIFF): one IFD entry, Orientation (0x0112) = 6.
    let exif = [
        b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0,
    ];
    png_chunk(&mut png, b"eXIf", &exif);
    png_chunk(&mut png, b"IDAT", &[0x78, 0x9C, 0x03, 0x00]);
    png_chunk(&mut png, b"IEND", &[]);
    png
}

/// A picture whose decoding would take more than the cap is refused before anything of it is allocated (the refusal
/// is the memory cap's own, not a decode failure), while a 50 MP phone photo still fits.
#[test]
fn encode_refuses_a_picture_over_the_memory_cap() {
    let png = declared_huge_png();
    assert!(png.len() < 200, "{} bytes", png.len());
    assert_eq!(decode(&png).unwrap_err(), encode::EncodeRefused::OverMemory);
    // A 50 MP phone photo (8160 x 6120, 8-bit RGB) with a 30 MB original.
    assert!(encode::memory_needed(8_160, 6_120, 8_160 * 6_120 * 3, true, 30_000_000) <= encode::MAX_ALLOC);
}

/// The original's bytes count: a picture whose phases fit with 4 MiB to spare is refused beside an original of more.
#[test]
fn the_originals_bytes_count_toward_the_memory_cap() {
    // 16-bit RGBA, 8192 x 6112: decoding it holds twice 400.6 MB, 4 MiB under the cap.
    let decoded = 8_192 * 6_112 * 8;
    let spare = 4 * 1024 * 1024;
    assert!(encode::memory_needed(8_192, 6_112, decoded, false, 0) <= encode::MAX_ALLOC);
    assert!(encode::memory_needed(8_192, 6_112, decoded, false, spare) <= encode::MAX_ALLOC);
    assert!(encode::memory_needed(8_192, 6_112, decoded, false, spare + 1) > encode::MAX_ALLOC);
}
