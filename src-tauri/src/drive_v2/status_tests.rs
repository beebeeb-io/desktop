//! Tests for batch B02b's mappings (`status.rs`, `photo_meta.rs`): one per new field, over plain inputs.

use std::collections::HashMap;
use std::path::PathBuf;

use super::model;
use super::photo_meta::{self, PhotoMeta};
use super::status::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind, UnsentChange};
use crate::transfer_progress::{ActiveTransferDto, Direction};

fn moving(id: &str, direction: Direction, done: u64, total: u64, bps: u64) -> ActiveTransferDto {
    ActiveTransferDto {
        file_id: id.into(),
        name: format!("{id}.jpg"),
        folder: "Trip".into(),
        direction,
        done,
        total,
        buffered: 0,
        speed_bps: bps,
        attention: None,
        action: None,
        failed: false,
        failed_at_secs: None,
    }
}

fn change(id: &str, file_id: Option<&str>, kind: &'static str, group: &str) -> UnsentChange {
    UnsentChange {
        id: id.into(),
        file_id: file_id.map(str::to_string),
        kind,
        path: format!("Trip/{id}.heic"),
        backup: false,
        group: group.into(),
        queued_at: 1_700_000_000,
        retry_at: None,
        parked: false,
        retryable: group == "gave_up",
        blocked: None,
    }
}

#[test]
fn backup_view_maps_state_and_last_backup() {
    let marks: HashMap<String, Option<i64>> = [
        ("documents".to_string(), Some(1_700_000_000_000)),
        ("pictures".to_string(), Some(1_700_000_500_000)),
        ("music".to_string(), None),
    ]
    .into();
    assert_eq!(backup_view(&[], &marks, 0).state, BackupState::Off);
    let ready = backup_view(&["music"], &marks, 0);
    assert_eq!(
        (ready.state, ready.last_backup_at, ready.folders_getting_ready),
        (BackupState::GettingReady, None, 1)
    );
    let on = backup_view(&["documents", "pictures", "music"], &marks, 0);
    assert_eq!(on.state, BackupState::On);
    assert_eq!(
        on.last_backup_at.as_deref(),
        Some(model::iso(1_700_000_500).as_str()),
        "the newest pass wins"
    );
    assert_eq!((on.folders_on, on.folders_getting_ready), (3, 1));
    assert_eq!(backup_view(&["documents"], &marks, 1).state, BackupState::Stopped);
}

#[test]
fn upload_rate_counts_uploads_only_and_rounds_time_up() {
    assert_eq!(upload_rate(&[]), (None, None));
    assert_eq!(
        upload_rate(&[moving("a", Direction::Up, 0, 100, 0)]),
        (None, None),
        "no rate measured yet"
    );
    let board = [
        moving("a", Direction::Up, 100, 1_100, 300),
        moving("b", Direction::Down, 0, 9_000, 9_000),
        moving("c", Direction::Up, 0, 1_000, 200),
    ];
    // 1,000 + 1,000 bytes left at 500 B/s.
    assert_eq!(upload_rate(&board), (Some(500), Some(4)));
}

#[test]
fn free_space_takes_the_longest_mount_prefix() {
    let disks = [
        (PathBuf::from("/"), 10),
        (PathBuf::from("/home"), 20),
        (PathBuf::from("/home/guus/other"), 30),
    ];
    assert_eq!(free_space_for(&PathBuf::from("/home/guus/Beebeeb"), &disks), Some(20));
    assert_eq!(free_space_for(&PathBuf::from("/srv"), &disks), Some(10));
    assert_eq!(free_space_for(&PathBuf::from("/srv"), &[]), None);
}

#[test]
fn reasons_are_plain_words_and_only_problems_have_one() {
    assert_eq!(
        reason_for(&change("1", None, "edit", "gave_up")),
        Some("Stopped after 5 tries.")
    );
    let mut blocked = change("2", None, "edit", "waiting");
    blocked.blocked = Some("superseded");
    assert_eq!(reason_for(&blocked), Some("A newer version is already in Beebeeb."));
    assert!(is_problem(&blocked));
    let waiting = change("3", None, "new_file", "waiting");
    assert_eq!(reason_for(&waiting), None);
    assert!(!is_problem(&waiting));
    let rows = sync_problems(&[change("1", Some("f1"), "edit", "gave_up"), waiting]);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].name.as_str(), rows[0].folder.as_str(), rows[0].retryable),
        ("1.heic", "Trip", true)
    );
    assert_eq!(rows[0].at.as_deref(), Some(model::iso(1_700_000_000).as_str()));
}

#[test]
fn transfer_rows_merge_board_and_queue_once() {
    let mut failed = moving("f2", Direction::Up, 5, 10, 0);
    failed.failed = true;
    let board = [
        moving("f1", Direction::Up, 2_100_000, 3_600_000, 1),
        moving("s", Direction::Stream, 1, 2, 1),
        failed,
    ];
    let queue = [
        change("op1", Some("f1"), "new_file", "waiting"),
        change("op2", Some("f9"), "new_file", "waiting"),
        change("op3", None, "edit", "gave_up"),
        change("op4", None, "rename", "waiting"),
    ];
    let rows = transfer_rows(&board, &queue);
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        ["f1", "f2", "op2", "op3"],
        "a stream, a duplicate and a waiting rename are left out"
    );
    assert_eq!(
        (rows[0].state, rows[0].bytes, rows[0].total),
        (TransferState::Moving, 2_100_000, Some(3_600_000))
    );
    assert_eq!(rows[1].state, TransferState::Stopped);
    assert_eq!(
        (rows[2].state, rows[2].total, rows[2].folder.as_str()),
        (TransferState::Waiting, None, "Trip")
    );
    assert_eq!(
        (rows[3].state, rows[3].reason, rows[3].retryable),
        (TransferState::Stopped, Some("Stopped after 5 tries."), true)
    );
}

fn entry(id: &str, path: &str, parent: Option<&str>, folder: bool, size: i64) -> FileEntry {
    FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: size,
        modified_at: 100,
        content_hash: None,
        remote_updated_at: 100,
        parent_id: parent.map(str::to_string),
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    }
}

#[test]
fn offline_list_has_top_most_kept_items_with_folder_totals() {
    let rows: Vec<(FileEntry, bool, super::dto::DriveItem)> = [
        (entry("d", "Documents", None, true, 0), true),
        (entry("d1", "Documents/a.pdf", Some("d"), false, 1_000), true),
        (entry("d2", "Documents/Sub", Some("d"), true, 0), true),
        (entry("d3", "Documents/Sub/b.pdf", Some("d2"), false, 500), true),
        (entry("x", "Budget.xlsx", None, false, 84), true),
        (entry("y", "Other.txt", None, false, 9), false),
        (entry("dd", "Documents2/c.pdf", None, false, 7), false),
    ]
    .into_iter()
    .map(|(e, k)| {
        let item = model::item_from(&e, None, None);
        (e, k, item)
    })
    .collect();
    let list = offline_entries(&rows);
    let got: Vec<(&str, Option<u64>, i64)> = list.iter().map(|o| (o.item.id.as_str(), o.items, o.bytes)).collect();
    assert_eq!(got, [("d", Some(2), 1_500), ("x", None, 84)]);
}

#[test]
fn drive_status_serialises_unknowns_as_absent_and_no_secret_field() {
    let value = serde_json::to_value(DriveStatus::default()).unwrap();
    for absent in [
        "metered",
        "uploadBps",
        "etaSeconds",
        "fileCount",
        "freeSpaceBytes",
        "listing",
    ] {
        assert!(value.get(absent).is_none(), "{absent} must be absent when unknown");
    }
    assert_eq!(value["backup"]["state"], "off");
    super::tests::assert_no_secret_fields(&value, "DriveStatus");
    let rows = transfer_rows(
        &[moving("f1", Direction::Up, 1, 2, 1)],
        &[change("op", None, "edit", "gave_up")],
    );
    super::tests::assert_no_secret_fields(&serde_json::to_value(&rows).unwrap(), "TransferRow");
    super::tests::assert_no_secret_fields(
        &serde_json::to_value(sync_problems(&[change("op", None, "edit", "gave_up")])).unwrap(),
        "SyncProblem",
    );
    super::tests::assert_no_secret_fields(
        &serde_json::to_value(PhotoMeta {
            uploading: true,
            ..PhotoMeta::default()
        })
        .unwrap(),
        "PhotoMeta",
    );
}

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
