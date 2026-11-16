//! Tests of `case_place`: two server items at one place on disk (names that differ only by case, or the very same
//! name). Linux tells case apart, so a test plays a volume that does not with [`case_place::with_rule_for_test`]: the
//! folder holds the ONE entry such a volume would hold (`Report.pdf`), and the rows say `Report.pdf` and `report.pdf`.
//! What the real volume then does with a press by the other spelling (it reaches `Report.pdf`) is not played here: each
//! press is refused before it reaches the disk, and the row whose spelling the entry has keeps its presses.

use std::collections::HashSet;
use std::path::Path;

use super::DriveError;
use super::case_place::{self, CaseRule, with_rule_for_test};
use super::model::{self, Sort};
use crate::file_ops;
use crate::state_db::{self, FileEntry, FileStatus, ItemKind, Namespace, PrunedRow, StateDb};

struct Fixture {
    _dir: tempfile::TempDir,
    db: StateDb,
    root: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    Fixture {
        _dir: dir,
        db,
        root: tempfile::tempdir().unwrap(),
    }
}

/// A row of the person's own: `server` rows the server confirmed (a remote time), the others this PC made.
fn row(db: &StateDb, id: &str, path: &str, parent: Option<&str>, folder: bool, server: bool, status: FileStatus) {
    let kind = if folder { ItemKind::Folder } else { ItemKind::File };
    let entry = FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status,
        size_bytes: if folder { 0 } else { 7 },
        modified_at: 1_700_000_000,
        content_hash: None,
        remote_updated_at: if server { 1_700_000_000 } else { 0 },
        parent_id: parent.map(str::to_string),
        item_kind: kind.clone(),
    };
    db.upsert_file_with_contract(&entry, |c| {
        c.parent_id = parent.map(str::to_string);
        c.item_kind = kind;
        c.namespace = Namespace::MyFiles;
        c.permission_bits = state_db::PERMISSION_OWNER;
        c.current_version = if server { 3 } else { 0 };
    })
    .unwrap();
}

fn get(db: &StateDb, id: &str) -> FileEntry {
    file_ops::resolve(db, id).unwrap()
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// The names in `dir` below the root, sorted.
fn names(root: &Path, dir: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(dir))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Two server items in the top folder whose names differ only by case, and the one entry a volume that compares
/// names without case holds for them: `Report.pdf`, `upper`'s (its placeholder was made first). `Archive` is a folder.
fn case_twins() -> Fixture {
    let f = fixture();
    row(&f.db, "upper", "Report.pdf", None, false, true, FileStatus::Local);
    row(&f.db, "lower", "report.pdf", None, false, true, FileStatus::CloudOnly);
    row(&f.db, "archive", "Archive", None, true, true, FileStatus::Local);
    write(f.root.path(), "Report.pdf", b"upper's");
    std::fs::create_dir(f.root.path().join("Archive")).unwrap();
    f
}

/// Windows and macOS compare names without case; Linux does not.
#[test]
fn windows_and_macos_compare_names_without_case() {
    assert_eq!(CaseRule::of_os("windows"), CaseRule::Insensitive);
    assert_eq!(CaseRule::of_os("macos"), CaseRule::Insensitive);
    assert_eq!(CaseRule::of_os("linux"), CaseRule::Sensitive);
    assert!(CaseRule::Insensitive.same_name("Report.pdf", "rEPORT.PDF"));
    assert!(!CaseRule::Sensitive.same_name("Report.pdf", "report.pdf"));
    assert_eq!(CaseRule::Insensitive.fold("/Docs/Report.pdf/"), "docs/report.pdf");
}

/// The entries on disk along a path carry its names, or are another spelling of them (another row's place).
#[test]
fn a_place_spelled_otherwise_on_disk_is_another_rows() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "Report.pdf", b"x");
    write(root.path(), "Docs/a.txt", b"x");
    let spelled = |rel: &str, rule| case_place::spelled_as_on_disk(root.path(), rel, rule).unwrap();
    let insensitive = CaseRule::Insensitive;
    assert!(spelled("Report.pdf", insensitive));
    assert!(!spelled("report.pdf", insensitive));
    assert!(!spelled("REPORT.PDF", insensitive));
    assert!(
        spelled("not there.pdf", insensitive),
        "nothing on disk yet is no other row's"
    );
    assert!(spelled("Docs/a.txt", insensitive));
    assert!(
        !spelled("docs/a.txt", insensitive),
        "a folder on the way spelled otherwise"
    );
    assert!(spelled("Docs/new.txt", insensitive));
    assert!(
        spelled("report.pdf", CaseRule::Sensitive),
        "a volume that tells case apart: its own name"
    );
}

/// D-T-15: both server items stay listed, each by its own id (the window lists rows, not entries on disk).
#[test]
fn two_server_items_whose_names_differ_only_by_case_are_both_listed() {
    let f = case_twins();
    for rule in [CaseRule::Insensitive, CaseRule::Sensitive] {
        with_rule_for_test(rule, || {
            let (items, total) = model::children_of(&f.db, None, Sort::default(), 0, usize::MAX).unwrap();
            let mut ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
            ids.sort();
            assert_eq!((ids, total), (vec!["archive", "lower", "upper"], 3));
            let mut live: Vec<String> = model::live_rows(&f.db)
                .unwrap()
                .into_iter()
                .map(|e| e.file_id)
                .collect();
            live.sort();
            assert_eq!(live, ["archive", "lower", "upper"]);
        });
    }
}

/// Open, Show in File Explorer and Open in File Explorer hand the system the row's place only when it is that row's:
/// the entry spelled as `upper` is never handed over as `lower`.
#[test]
fn open_and_show_hand_over_only_the_rows_own_entry() {
    let f = case_twins();
    with_rule_for_test(CaseRule::Insensitive, || {
        assert_eq!(
            file_ops::item_path_for(&f.db, f.root.path(), &get(&f.db, "upper")),
            Ok(f.root.path().join("Report.pdf"))
        );
        assert_eq!(
            file_ops::item_path_for(&f.db, f.root.path(), &get(&f.db, "lower")),
            Err(DriveError::Conflict)
        );
    });
}

/// Rename of the twin is refused and renames nothing; the row whose spelling the entry has renames its own entry.
#[test]
fn rename_of_a_case_twin_never_renames_the_other_rows_entry() {
    let f = case_twins();
    with_rule_for_test(CaseRule::Insensitive, || {
        assert_eq!(
            file_ops::rename(&f.db, f.root.path(), &get(&f.db, "lower"), "Renamed.pdf"),
            Err(DriveError::Conflict)
        );
        assert_eq!(names(f.root.path(), ""), ["Archive", "Report.pdf"]);
        assert_eq!(
            file_ops::rename(&f.db, f.root.path(), &get(&f.db, "upper"), "Kept.pdf"),
            Ok("Kept.pdf".to_string())
        );
        assert_eq!(names(f.root.path(), ""), ["Archive", "Kept.pdf"]);
    });
}

/// Move of the twin is refused and moves nothing; the row whose spelling the entry has moves its own entry.
#[test]
fn move_of_a_case_twin_never_moves_the_other_rows_entry() {
    let f = case_twins();
    with_rule_for_test(CaseRule::Insensitive, || {
        let archive = get(&f.db, "archive");
        assert_eq!(
            file_ops::move_items(&f.db, f.root.path(), &[get(&f.db, "lower")], Some(&archive)),
            Err(DriveError::Conflict)
        );
        assert_eq!(names(f.root.path(), ""), ["Archive", "Report.pdf"]);
        assert!(names(f.root.path(), "Archive").is_empty());
        file_ops::move_items(&f.db, f.root.path(), &[get(&f.db, "upper")], Some(&archive)).unwrap();
        assert_eq!(names(f.root.path(), "Archive"), ["Report.pdf"]);
    });
}

/// Duplicate of the twin copies no bytes (they would be the other row's); the row whose spelling the entry has does.
#[test]
fn duplicate_of_a_case_twin_copies_none_of_the_other_rows_bytes() {
    let f = case_twins();
    with_rule_for_test(CaseRule::Insensitive, || {
        assert_eq!(
            file_ops::duplicate(&f.db, f.root.path(), &get(&f.db, "lower")),
            Err(DriveError::Conflict)
        );
        assert_eq!(names(f.root.path(), ""), ["Archive", "Report.pdf"]);
        assert_eq!(
            file_ops::duplicate(&f.db, f.root.path(), &get(&f.db, "upper")),
            Ok("Report - Copy.pdf".to_string())
        );
    });
}

/// Twin folders (`Docs` placed, `docs` not), each with an `a.txt`: the child of the twin folder reaches `Docs/a.txt`,
/// and anything made, uploaded or moved into the twin folder would land in `Docs`. All refused; `Docs` keeps its
/// presses.
#[test]
fn presses_into_or_below_a_case_twin_folder_are_refused() {
    let f = fixture();
    row(&f.db, "docs-upper", "Docs", None, true, true, FileStatus::Local);
    row(&f.db, "docs-lower", "docs", None, true, true, FileStatus::CloudOnly);
    row(
        &f.db,
        "a-upper",
        "Docs/a.txt",
        Some("docs-upper"),
        false,
        true,
        FileStatus::Local,
    );
    row(
        &f.db,
        "a-lower",
        "docs/a.txt",
        Some("docs-lower"),
        false,
        true,
        FileStatus::CloudOnly,
    );
    row(&f.db, "loose", "loose.txt", None, false, true, FileStatus::Local);
    write(f.root.path(), "Docs/a.txt", b"a-upper's");
    write(f.root.path(), "loose.txt", b"loose");
    let elsewhere = tempfile::tempdir().unwrap();
    let source = elsewhere.path().join("upload.txt");
    std::fs::write(&source, b"upload").unwrap();
    with_rule_for_test(CaseRule::Insensitive, || {
        let (upper, lower) = (get(&f.db, "docs-upper"), get(&f.db, "docs-lower"));
        assert_eq!(
            file_ops::item_path_for(&f.db, f.root.path(), &get(&f.db, "a-lower")),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            file_ops::item_path_for(&f.db, f.root.path(), &lower),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            file_ops::item_path_for(&f.db, f.root.path(), &get(&f.db, "a-upper")),
            Ok(f.root.path().join("Docs").join("a.txt"))
        );
        assert_eq!(
            file_ops::create_folder(&f.db, f.root.path(), Some(&lower), "New"),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            file_ops::create_file(&f.db, f.root.path(), Some(&lower), "New.txt", b""),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            file_ops::copy_in(&f.db, f.root.path(), Some(&lower), std::slice::from_ref(&source)),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            file_ops::move_items(&f.db, f.root.path(), &[get(&f.db, "loose")], Some(&lower)),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            names(f.root.path(), "Docs"),
            ["a.txt"],
            "nothing landed in the other row's folder"
        );
        assert_eq!(names(f.root.path(), ""), ["Docs", "loose.txt"]);
        assert_eq!(
            file_ops::create_folder(&f.db, f.root.path(), Some(&upper), "New"),
            Ok("Docs/New".to_string())
        );
    });
}

/// D-T-15's two server items with the very same name are one entry on every volume, and nothing says whose: Open,
/// rename, move and duplicate of either are refused and touch nothing.
#[test]
fn two_server_items_with_the_same_name_change_nothing_on_disk() {
    let f = fixture();
    row(&f.db, "srv-a", "Photo.jpg", None, false, true, FileStatus::Local);
    row(&f.db, "srv-b", "Photo.jpg", None, false, true, FileStatus::Local);
    row(&f.db, "archive", "Archive", None, true, true, FileStatus::Local);
    write(f.root.path(), "Photo.jpg", b"one entry");
    std::fs::create_dir(f.root.path().join("Archive")).unwrap();
    let archive = get(&f.db, "archive");
    for id in ["srv-a", "srv-b"] {
        let entry = get(&f.db, id);
        assert_eq!(
            file_ops::item_path_for(&f.db, f.root.path(), &entry),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            file_ops::rename(&f.db, f.root.path(), &entry, "Renamed.jpg"),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            file_ops::move_items(&f.db, f.root.path(), std::slice::from_ref(&entry), Some(&archive)),
            Err(DriveError::Conflict)
        );
        assert_eq!(
            file_ops::duplicate(&f.db, f.root.path(), &entry),
            Err(DriveError::Conflict)
        );
    }
    assert_eq!(names(f.root.path(), ""), ["Archive", "Photo.jpg"]);
    assert!(names(f.root.path(), "Archive").is_empty());
}

/// No false refusal: a row this PC made and never sent, at the server row's path (the ghost the listing folds), and a
/// volume that tells case apart (each spelling its own entry) leave every press as it was.
#[test]
fn a_ghost_at_the_path_and_a_case_sensitive_volume_refuse_nothing() {
    let f = fixture();
    row(&f.db, "ghost", "Archive", None, true, false, FileStatus::Local);
    row(&f.db, "srv-archive", "Archive", None, true, true, FileStatus::Local);
    std::fs::create_dir(f.root.path().join("Archive")).unwrap();
    with_rule_for_test(CaseRule::Insensitive, || {
        assert_eq!(
            file_ops::rename(&f.db, f.root.path(), &get(&f.db, "srv-archive"), "Old"),
            Ok("Old".to_string())
        );
    });

    let f = case_twins();
    write(f.root.path(), "report.pdf", b"lower's");
    with_rule_for_test(CaseRule::Sensitive, || {
        assert_eq!(
            file_ops::item_path_for(&f.db, f.root.path(), &get(&f.db, "lower")),
            Ok(f.root.path().join("report.pdf"))
        );
        assert_eq!(
            file_ops::rename(&f.db, f.root.path(), &get(&f.db, "lower"), "Renamed.pdf"),
            Ok("Renamed.pdf".to_string())
        );
        assert_eq!(names(f.root.path(), ""), ["Archive", "Renamed.pdf", "Report.pdf"]);
    });
}

/// A row alone at its place whose entry on disk keeps an older spelling (renamed by case alone on the web; a remote
/// rename leaves the entry where it is): the entry is its own, and every press keeps working.
#[test]
fn a_row_alone_at_its_place_keeps_its_presses_whatever_the_spelling_on_disk() {
    let f = fixture();
    row(&f.db, "only", "report.pdf", None, false, true, FileStatus::Local);
    row(&f.db, "archive", "Archive", None, true, true, FileStatus::Local);
    write(f.root.path(), "Report.pdf", b"only's");
    std::fs::create_dir(f.root.path().join("Archive")).unwrap();
    with_rule_for_test(CaseRule::Insensitive, || {
        assert_eq!(
            file_ops::item_path_for(&f.db, f.root.path(), &get(&f.db, "only")),
            Ok(f.root.path().join("report.pdf"))
        );
        assert!(!case_place::place_is_another_rows(&f.db, f.root.path(), &get(&f.db, "only")).unwrap());
        let archive = get(&f.db, "archive");
        assert!(!case_place::place_is_another_rows(&f.db, f.root.path(), &archive).unwrap());
    });
}

fn pruned(id: &str, path: &str, is_dir: bool) -> PrunedRow {
    PrunedRow {
        file_id: id.to_string(),
        path: path.to_string(),
        is_dir,
        local_hash: None,
        fetched_hash: None,
    }
}

fn ids(rows: &[PrunedRow]) -> Vec<&str> {
    rows.iter().map(|r| r.file_id.as_str()).collect()
}

/// Move to Trash of the twin goes to the server by its id; the snapshot then drops its row, and the entry at its path
/// is removed. On a volume that compares names without case that path reaches the other row's `Report.pdf`: it stays.
#[test]
fn a_case_twin_gone_from_the_server_leaves_the_other_rows_entry() {
    let f = case_twins();
    let seen: HashSet<String> = ["upper", "archive"].into_iter().map(str::to_string).collect();
    let gone = model::drop_absent_rows(&f.db, &seen, 1_800_000_000).unwrap();
    assert_eq!(gone, vec![pruned("lower", "report.pdf", false)]);
    assert_eq!(
        case_place::removable_places(&f.db, f.root.path(), &gone, CaseRule::Insensitive),
        vec![],
        "`Report.pdf` is upper's: never removed for lower"
    );
    assert_eq!(
        case_place::removable_places(&f.db, f.root.path(), &gone, CaseRule::Sensitive),
        gone,
        "a volume that tells case apart: `report.pdf` is lower's own"
    );
}

/// The twin that went is the one whose spelling the entry has: its entry is removed (the other row gets its own
/// placeholder next). So is one whose name only changed case on another device, with no row left at its place.
#[test]
fn the_entry_of_the_row_that_went_is_still_removed() {
    let f = fixture();
    row(&f.db, "upper", "Report.pdf", None, false, true, FileStatus::CloudOnly);
    write(f.root.path(), "report.pdf", b"lower's placeholder");
    write(f.root.path(), "Notes.txt", b"renamed elsewhere");
    let gone = vec![
        pruned("lower", "report.pdf", false),
        pruned("notes", "notes.txt", false),
    ];
    assert_eq!(
        ids(&case_place::removable_places(
            &f.db,
            f.root.path(),
            &gone,
            CaseRule::Insensitive
        )),
        ["lower", "notes"]
    );
}

/// A twin folder that went, with its children (children first): the child that has a twin below `Docs` stays on
/// disk, the child of its own (placed in `Docs`, its own entry) is removed, the folder `Docs` stays.
#[test]
fn a_case_twin_folder_that_went_removes_only_its_own_children() {
    let f = fixture();
    row(&f.db, "docs-upper", "Docs", None, true, true, FileStatus::Local);
    row(
        &f.db,
        "a-upper",
        "Docs/a.txt",
        Some("docs-upper"),
        false,
        true,
        FileStatus::Local,
    );
    write(f.root.path(), "Docs/a.txt", b"a-upper's");
    write(f.root.path(), "Docs/b.txt", b"b-lower's own");
    let gone = vec![
        pruned("a-lower", "docs/a.txt", false),
        pruned("b-lower", "docs/b.txt", false),
        pruned("docs-lower", "docs", true),
    ];
    assert_eq!(
        ids(&case_place::removable_places(
            &f.db,
            f.root.path(),
            &gone,
            CaseRule::Insensitive
        )),
        ["b-lower"]
    );
}

/// The two pairs every press below is asked about, and a row alone: case twins (`Report.pdf` on disk, `upper`'s),
/// two server items named `Photo.jpg`, and `alone.jpg`.
fn twins_and_one_alone() -> Fixture {
    let f = case_twins();
    row(&f.db, "same-a", "Photo.jpg", None, false, true, FileStatus::Local);
    row(&f.db, "same-b", "Photo.jpg", None, false, true, FileStatus::Local);
    row(&f.db, "alone", "alone.jpg", None, false, true, FileStatus::Local);
    write(f.root.path(), "Photo.jpg", b"one entry");
    write(f.root.path(), "alone.jpg", b"alone's");
    f
}

/// One pass says why a place is another row's: the very same name, or the same name but for case.
#[test]
fn a_pass_over_many_places_says_why_each_is_another_rows() {
    let f = twins_and_one_alone();
    with_rule_for_test(CaseRule::Insensitive, || {
        let mut places = case_place::PlaceCheck::new(&f.db, f.root.path(), CaseRule::host());
        let clash = |places: &mut case_place::PlaceCheck<'_>, id: &str| places.clash(&get(&f.db, id)).unwrap();
        assert_eq!(clash(&mut places, "lower"), Some(case_place::Clash::Case));
        assert_eq!(clash(&mut places, "same-a"), Some(case_place::Clash::SameName));
        assert_eq!(clash(&mut places, "same-b"), Some(case_place::Clash::SameName));
        assert_eq!(clash(&mut places, "upper"), None);
        assert_eq!(clash(&mut places, "alone"), None);
        assert!(!places.is_own_place("lower") && !places.is_own_place("same-a"));
        assert!(places.is_own_place("upper") && places.is_own_place("alone"));
        assert!(
            !places.is_own_place("no such row"),
            "a row that is not there has no place of its own"
        );
    });
}

/// Free up space (`free_up_space::dehydrate_these`, Windows): the place it would make online only is never the entry of
/// another row; the owner row and a row alone keep theirs.
#[test]
fn free_up_space_never_reclaims_another_rows_entry() {
    let f = twins_and_one_alone();
    with_rule_for_test(CaseRule::Insensitive, || {
        let mut places = case_place::PlaceCheck::new(&f.db, f.root.path(), CaseRule::host());
        let mut place = |id: &str, rel: &str| crate::name_alias::reclaim_place_of(&mut places, id, rel);
        assert_eq!(place("lower", "report.pdf"), None, "the case twin");
        assert_eq!(place("same-a", "Photo.jpg"), None, "a same-name twin");
        assert_eq!(place("same-b", "Photo.jpg"), None, "the other same-name twin");
        assert_eq!(place("upper", "Report.pdf"), Some(f.root.path().join("Report.pdf")));
        assert_eq!(place("alone", "alone.jpg"), Some(f.root.path().join("alone.jpg")));
    });
}

/// Photo previews: a photo's original is read for its preview only from its own entry; never the other row's bytes as
/// its preview.
#[test]
fn a_photo_preview_is_never_made_from_another_rows_entry() {
    use crate::photo_previews::plan;
    let f = twins_and_one_alone();
    row(&f.db, "pic-upper", "Pic.jpg", None, false, true, FileStatus::Local);
    row(&f.db, "pic-lower", "pic.jpg", None, false, true, FileStatus::Local);
    write(f.root.path(), "Pic.jpg", b"pic-upper's");
    let photos = plan::local_photos(&f.db).unwrap();
    let own = |id: &str| {
        let photo = photos.iter().find(|photo| photo.id == id).unwrap();
        plan::own_place(&f.db, f.root.path(), photo)
    };
    with_rule_for_test(CaseRule::Insensitive, || {
        assert!(!own("pic-lower"), "the case twin");
        assert!(!own("same-a") && !own("same-b"), "the same-name twins");
        assert!(own("pic-upper"), "the row whose spelling the entry has");
        assert!(own("alone"), "a photo alone at its place");
    });
    with_rule_for_test(CaseRule::Sensitive, || {
        assert!(own("pic-lower"), "a volume that tells case apart: its own entry");
        assert!(!own("same-a"), "one name is one entry on every volume");
    });
}

/// Two server items named `Photo.jpg`, one entry on disk; `srv-a` went to Trash on the web, so the snapshot drops its
/// row. The entry goes only when its placeholder names `srv-a`: one that names `srv-b`, or nothing (unreadable), stays.
/// A pruned row with no twin is removed whatever its entry says (as before).
#[test]
fn the_prune_of_one_of_two_same_name_items_removes_only_its_own_entry() {
    let f = fixture();
    row(&f.db, "srv-a", "Photo.jpg", None, false, true, FileStatus::Local);
    row(&f.db, "srv-b", "Photo.jpg", None, false, true, FileStatus::Local);
    row(&f.db, "loose", "loose.txt", None, false, true, FileStatus::Local);
    write(f.root.path(), "Photo.jpg", b"one entry");
    write(f.root.path(), "loose.txt", b"loose");
    let seen: HashSet<String> = ["srv-b"].into_iter().map(str::to_string).collect();
    let mut gone = model::drop_absent_rows(&f.db, &seen, 1_800_000_000).unwrap();
    gone.sort_by(|a, b| a.file_id.cmp(&b.file_id));
    assert_eq!(ids(&gone), ["loose", "srv-a"]);
    let photo = f.root.path().join("Photo.jpg");
    let removable = |names: Option<&'static str>| {
        let asked = std::cell::RefCell::new(Vec::new());
        let read = |path: &Path| {
            asked.borrow_mut().push(path.to_path_buf());
            names.map(str::to_string)
        };
        let kept = case_place::removable_by_identity(&f.db, f.root.path(), gone.clone(), &read);
        (
            ids(&kept).iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            asked.into_inner(),
        )
    };
    assert_eq!(
        removable(Some("srv-b")),
        (vec!["loose".to_string()], vec![photo.clone()]),
        "the entry is the other item's: it stays"
    );
    assert_eq!(
        removable(None),
        (vec!["loose".to_string()], vec![photo.clone()]),
        "nobody can tell whose: it stays"
    );
    assert_eq!(
        removable(Some("srv-a")),
        (vec!["loose".to_string(), "srv-a".to_string()], vec![photo]),
        "its own placeholder goes"
    );
}
