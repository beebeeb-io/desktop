//! Dm-68: a kept item's row moves through "Not on this Mac yet" -> n% -> "On this Mac" from what the engine reads
//! (the transfer board's downloads and the files' states), never from a made-up figure.

use super::*;

fn kept_item(id: &str, kind: Kind, status: KeptStatus) -> Kept {
    Kept {
        id: id.into(),
        name: "x".into(),
        kind,
        bytes: 100,
        contains: Contains::In("Files".into()),
        status,
    }
}

fn snapshot(kind: Kind, files: &[(&str, FileStatus)], downloads: &[(&str, u64, u64)]) -> Snapshot {
    let mut s = Snapshot::default();
    s.kept.push(kept_item("top", kind, KeptStatus::NotOnPcYet));
    s.kept_files
        .insert("top".into(), files.iter().map(|(id, st)| (id.to_string(), st.clone())).collect());
    s.downloads = downloads.iter().map(|(id, d, t)| (id.to_string(), (*d, *t))).collect();
    s
}

fn running(state: Option<CopyState>) -> KeptProgress {
    match state {
        Some(CopyState::Running(p)) => p,
        other => panic!("expected a running copy, got {other:?}"),
    }
}

#[test]
fn a_kept_file_goes_from_waiting_through_percent_to_done() {
    // Waiting: kept, no row on the board yet: nothing to say beyond "Not on this Mac yet".
    let waiting = snapshot(Kind::Document, &[("top", FileStatus::Downloading)], &[]);
    assert_eq!(copy_state(&waiting, "top"), None);
    // Coming down: 25 %, then 75 %.
    let p = running(copy_state(
        &snapshot(Kind::Document, &[("top", FileStatus::Downloading)], &[("top", 25, 100)]),
        "top",
    ));
    assert_eq!((p.percent, p.done, p.total, p.files), (25, 25, 100, None));
    let p = running(copy_state(
        &snapshot(Kind::Document, &[("top", FileStatus::Downloading)], &[("top", 75, 100)]),
        "top",
    ));
    assert_eq!(p.percent, 75);
    // Landed: the file is Local, the board row is gone: the row reads the listing's own words again.
    let done = snapshot(Kind::Document, &[("top", FileStatus::Local)], &[]);
    assert_eq!(copy_state(&done, "top"), None);
}

#[test]
fn a_kept_folder_counts_its_files_and_a_running_one_by_its_bytes() {
    let s = snapshot(
        Kind::Folder,
        &[
            ("a", FileStatus::Local),
            ("b", FileStatus::Downloading),
            ("c", FileStatus::CloudOnly),
            ("d", FileStatus::CloudOnly),
        ],
        &[("b", 50, 100)],
    );
    let p = running(copy_state(&s, "top"));
    assert_eq!(p.files, Some((1, 4)));
    // One of four is here, one is half down: (1 + 0.5) / 4.
    assert_eq!(p.percent, 37);
}

#[test]
fn a_file_that_failed_says_so() {
    let s = snapshot(Kind::Folder, &[("a", FileStatus::Local), ("b", FileStatus::Error)], &[]);
    assert_eq!(copy_state(&s, "top"), Some(CopyState::Failed));
}

#[test]
fn an_item_the_read_does_not_hold_says_nothing() {
    assert_eq!(copy_state(&Snapshot::default(), "top"), None);
}
