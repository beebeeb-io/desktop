//! The queue, split out of `own_data_proof.rs`.
//!
//! Proven changes upload; the rest wait for the person. These tests cover the operation
//! kinds that can be tied to this account's proven data, the ones held for review in
//! `found_while_paused` instead of blocking the adoption, and the unchanged rows that are
//! left alone.

use super::*;

// ---- the queue: proven changes upload, the rest wait for the person ----

/// The lead's case: all of this account's own data, plus a new folder at the top level (with a file in it) made
/// while sync was refused. The data is adopted; the new folder's two changes are held for review in the same
/// transaction as the owner record, and NOTHING of them is due; the edit of an own file uploads.
#[test]
fn an_all_own_database_with_a_new_top_level_folder_adopts_and_holds_the_folder() {
    let pc = pc_with(&IDS);
    add_folder(&pc, "new-folder", None, Shape::Local);
    add_row(&pc, "new-file", Some("new-folder"), Shape::Local);
    pc.db
        .0
        .enqueue_operation(&create("op-folder", "new-folder", None, true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-file", "new-file", Some("new-folder"), false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();

    let outcome = prove(&pc, &session(), MINE, &own_server());

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.own == 3 && t.proven_changes == 1 && t.held_for_review == 2),
        "{outcome:?}"
    );
    assert_eq!(pc.db.0.owner().unwrap(), Some(session()));
    assert_eq!(due(&pc), vec!["held-1".to_string()], "nothing of the new folder is due");
    assert_eq!(
        held_for_review(&pc),
        vec!["op-file".to_string(), "op-folder".to_string()]
    );
    assert_eq!(pc.db.0.list_files().unwrap().len(), 5, "nothing deleted");
}

/// A new file in one of this account's own folders is tied to its data: it uploads.
#[test]
fn a_new_file_in_an_own_folder_uploads() {
    let pc = pc_with(&IDS);
    add_folder(&pc, "own-folder", None, Shape::Snapshot);
    add_folder(&pc, "new-sub", Some("own-folder"), Shape::Local);
    add_row(&pc, "new-file", Some("new-sub"), Shape::Local);
    pc.db
        .0
        .enqueue_operation(&create("op-sub", "new-sub", Some("own-folder"), true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-file", "new-file", Some("new-sub"), false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(mine("own-folder"));

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.proven_changes == 3 && t.held_for_review == 0),
        "{outcome:?}"
    );
    let mut due = due(&pc);
    due.sort();
    #[cfg(target_os = "macos")]
    assert_eq!(due, ["held-1", "op-file", "op-sub"]);
    // Tasks 1888, 2226 (spec D7): released, the new file still waits for its folder's create to land.
    #[cfg(not(target_os = "macos"))]
    assert_eq!(due, ["held-1", "op-sub"]);
}

/// 128 of the 2,979 nested rows made on the maintainer's PC have no parent link, only their path. A new file whose
/// row has none is tied to its folder by its path: in one of this account's own folders it uploads, at the top
/// level it is held.
#[test]
fn a_new_item_whose_row_has_no_parent_link_is_judged_by_its_path() {
    let pc = pc_with(&IDS);
    add_row_at(
        &pc,
        "own-folder",
        "Projects",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "new-file",
        "Projects/new.txt",
        None,
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::File,
    );
    add_row_at(
        &pc,
        "new-top",
        "new-top.txt",
        None,
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::File,
    );
    pc.db
        .0
        .enqueue_operation(&create("op-file", "new-file", Some("own-folder"), false))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-top", "new-top", None, false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(mine("own-folder"));

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.proven_changes == 2 && t.held_for_review == 1),
        "{outcome:?}"
    );
    assert_eq!(held_for_review(&pc), vec!["op-top".to_string()]);
}

/// The maintainer's PC (task 1888): 2,832 folders made here whose create never reached the server share their
/// path with the server's own folder (their twin). A folder create there is tied to this account's data through
/// its own twin: it is not held. Without this rule all 2,984 queued changes on that PC were held for review. The
/// new file's own row here is linked to the server's folder (a real parent link), so its upload is proven by more
/// than a path (lead review of round 2, P1: content needs one such link). On that PC the uploads name the folder
/// made here instead, and wait for the click: `on_the_maintainers_pc_an_upload_into_a_folder_made_here_waits`.
#[test]
fn a_change_in_a_folder_made_here_whose_server_twin_is_own_is_not_held() {
    let pc = pc_with(&IDS);
    add_row_at(
        &pc,
        "twin",
        "Backups",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "twin-sub",
        "Backups/PC",
        Some("twin"),
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "ghost",
        "Backups",
        None,
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "ghost-sub",
        "Backups/PC",
        Some("ghost"),
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "new-file",
        "Backups/a.txt",
        Some("twin"),
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::File,
    );
    pc.db
        .0
        .enqueue_operation(&create("op-ghost", "ghost", None, true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-ghost-sub", "ghost-sub", Some("ghost"), true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-new-file", "new-file", Some("ghost"), false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(mine("twin"));
    tree.push(mine("twin-sub"));

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.own == 5 && t.proven_changes == 4 && t.held_for_review == 0),
        "{outcome:?}"
    );
    assert!(held_for_review(&pc).is_empty());
}

/// The maintainer's PC as it is (measured read-only: 145 of its 2,984 queued changes): an upload whose row names
/// a folder made here as its parent is tied to the account by that folder's twin, a path only. It is new content,
/// so it waits for the person's click (lead review of round 2, P1), while the folder's own create is proven.
#[test]
fn on_the_maintainers_pc_an_upload_into_a_folder_made_here_waits() {
    let pc = pc_with(&IDS);
    add_row_at(
        &pc,
        "twin",
        "Backups",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "ghost",
        "Backups",
        None,
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "new-file",
        "Backups/a.txt",
        Some("ghost"),
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::File,
    );
    pc.db
        .0
        .enqueue_operation(&create("op-ghost", "ghost", None, true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-new-file", "new-file", Some("ghost"), false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(mine("twin"));

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.proven_changes == 2 && t.held_for_review == 1),
        "{outcome:?}"
    );
    assert_eq!(held_for_review(&pc), vec!["op-new-file".to_string()]);
}

/// Lead review of round 2, P1: account A's rows made on this PC sit at the paths of account B's own rows (a
/// legacy database where A's synced rows are gone and A's unsent changes are not). A's new "Documents" folder is
/// tied to B's own "Documents" by its path, and a folder create sends no content, so it is proven (task 1888 folds
/// it into B's folder). A's files are tied to B's data by a path only: through the folder (its twin), as a file at
/// the path of one of B's files, or by the folder at their path when their row has no parent link. None of them
/// is sent without the person's click.
#[test]
fn a_path_twin_proves_a_folder_made_here_but_never_the_content_in_it() {
    let pc = pc_with(&IDS);
    add_row_at(
        &pc,
        "b-documents",
        "Documents",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "b-notes",
        "Documents/notes.txt",
        Some("b-documents"),
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::File,
    );
    add_row_at(
        &pc,
        "a-documents",
        "Documents",
        None,
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "a-report",
        "Documents/report.pdf",
        Some("a-documents"),
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::File,
    );
    add_row_at(
        &pc,
        "a-notes",
        "Documents/notes.txt",
        Some("a-documents"),
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::File,
    );
    add_row_at(
        &pc,
        "a-loose",
        "Documents/loose.txt",
        None,
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::File,
    );
    pc.db
        .0
        .enqueue_operation(&create("op-a-documents", "a-documents", None, true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-a-report", "a-report", Some("a-documents"), false))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-a-notes", "a-notes", Some("a-documents"), false))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-a-loose", "a-loose", None, false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(mine("b-documents"));
    tree.push(mine("b-notes"));

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.own == 5 && t.proven_changes == 2 && t.held_for_review == 3),
        "{outcome:?}"
    );
    let mut now_due = due(&pc);
    now_due.sort();
    assert_eq!(
        now_due,
        vec!["held-1".to_string(), "op-a-documents".to_string()],
        "only the folder create"
    );
    assert_eq!(
        held_for_review(&pc),
        vec![
            "op-a-loose".to_string(),
            "op-a-notes".to_string(),
            "op-a-report".to_string()
        ],
        "no content is sent on a path's word"
    );
}

/// Self-review of round 3 (F1): a server row gone from this account's tree, with no parent link, is explained by
/// the own folder at its path, so it is tied to that folder by a path only. New content for it is held. The same
/// row with a real parent link to that folder is gone from it, tied by the link, and its edit uploads.
#[test]
fn content_for_a_gone_row_explained_only_by_the_folder_at_its_path_is_held() {
    let pc = pc_with(&IDS);
    add_row_at(
        &pc,
        "own-folder",
        "Projects",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "gone",
        "Projects/old.txt",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::File,
    );
    add_row_at(
        &pc,
        "gone-linked",
        "Projects/linked.txt",
        Some("own-folder"),
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::File,
    );
    pc.db.0.enqueue_operation(&edit("op-gone", "gone")).unwrap();
    pc.db
        .0
        .enqueue_operation(&edit("op-gone-linked", "gone-linked"))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(mine("own-folder"));

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.gone == 2 && t.held_for_review == 1),
        "{outcome:?}"
    );
    assert_eq!(held_for_review(&pc), vec!["op-gone".to_string()]);
}

/// Self-review of round 3 (F2): a path proves a folder create only for the folder at that path, its own twin. A
/// new folder inside a folder made here has no twin: its create would give the account a folder name it never
/// had, on a path's word, so it is held. The folders made here that do have an own twin are proven.
#[test]
fn a_new_folder_inside_a_folder_made_here_is_held_when_it_has_no_twin_of_its_own() {
    let pc = pc_with(&IDS);
    add_row_at(
        &pc,
        "twin",
        "Backups",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "twin-sub",
        "Backups/PC",
        Some("twin"),
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "ghost",
        "Backups",
        None,
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "ghost-sub",
        "Backups/PC",
        Some("ghost"),
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "ghost-new",
        "Backups/New",
        Some("ghost"),
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    pc.db
        .0
        .enqueue_operation(&create("op-ghost", "ghost", None, true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-ghost-sub", "ghost-sub", Some("ghost"), true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-ghost-new", "ghost-new", Some("ghost"), true))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(mine("twin"));
    tree.push(mine("twin-sub"));

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.proven_changes == 3 && t.held_for_review == 1),
        "{outcome:?}"
    );
    assert_eq!(held_for_review(&pc), vec!["op-ghost-new".to_string()]);
}

/// Lead review of round 2, P2-1: a row counts as made on this PC only when a queued create names it. Every other
/// row is judged against the tree, even with both version columns at 0: one that is not in it is missing, and
/// blocks. Before, such a row was taken for this PC's own new item and skipped.
#[test]
fn a_row_is_made_here_only_when_a_queued_create_names_it() {
    let pc = pc_with(&IDS);
    add_row(&pc, "never-queued", None, Shape::Local);
    add_row(&pc, "only-edited", None, Shape::Local);
    add_row(&pc, "new-here", None, Shape::Local);
    pc.db.0.enqueue_operation(&edit("op-edit", "only-edited")).unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-new", "new-here", None, false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();

    let outcome = prove(&pc, &session(), MINE, &own_server());

    assert!(
        matches!(outcome, Outcome::NotProven(t) if t.checked == 5 && t.own == 3 && t.missing == 2),
        "{outcome:?}"
    );
    assert_eq!(pc.db.0.owner().unwrap(), None);
    let made_here: Vec<String> = pc
        .db
        .0
        .own_data_proof_rows()
        .unwrap()
        .into_iter()
        .filter(|row| !row.server_named)
        .map(|row| row.file_id)
        .collect();
    assert_eq!(made_here, vec!["new-here".to_string()]);
}

/// Lead review of round 2, P3-2: an unproven change paused for any reason (quota, permission, a locked file) is held
/// for review too: queued again, it would lose that pause and be sent unjudged. A proven one keeps its reason.
#[test]
fn an_unproven_change_paused_for_any_reason_is_held() {
    use crate::state_db::OperationPauseReason::{Locked, Permission, Quota};
    let pc = pc_with(&IDS);
    for (n, reason) in [Quota, Permission, Locked].into_iter().enumerate() {
        let id = format!("new-{n}");
        add_folder(&pc, &id, None, Shape::Local);
        pc.db
            .0
            .enqueue_operation(&create(&format!("op-{n}"), &id, None, true))
            .unwrap();
        pc.db
            .0
            .record_operation_pause(&format!("op-{n}"), reason, None, 5)
            .unwrap();
    }
    pc.db.0.enqueue_operation(&edit("op-own-edit", IDS[1])).unwrap();
    pc.db.0.record_operation_pause("op-own-edit", Quota, None, 5).unwrap();
    adopt_unbound(&pc.db, None).unwrap();

    assert!(matches!(prove(&pc, &session(), MINE, &own_server()), Outcome::Adopted(t) if t.held_for_review == 3));
    assert_eq!(
        held_for_review(&pc),
        vec!["op-0".to_string(), "op-1".to_string(), "op-2".to_string()]
    );
    let quota = pc
        .db
        .0
        .queue_hold_summary()
        .unwrap()
        .held
        .into_iter()
        .find(|group| group.reason == "quota");
    assert_eq!(
        quota.map(|group| group.count),
        Some(1),
        "the proven edit keeps its own pause"
    );
}

/// The twin rule proves nothing by itself: a folder made here whose server twin's name proves nothing (or is not
/// in this account's tree) keeps its changes held.
#[test]
fn a_change_in_a_folder_made_here_whose_server_twin_proves_nothing_is_held() {
    let pc = pc_with(&IDS);
    add_row_at(
        &pc,
        "twin",
        "Backups",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "ghost",
        "Backups",
        None,
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "new-file",
        "Backups/a.txt",
        Some("ghost"),
        Shape::Local,
        Namespace::MyFiles,
        ItemKind::File,
    );
    pc.db
        .0
        .enqueue_operation(&create("op-ghost", "ghost", None, true))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-new-file", "new-file", Some("ghost"), false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(with(mine("twin"), "name_encrypted", serde_json::json!("Backups")));

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.neutral == 1 && t.held_for_review == 2),
        "{outcome:?}"
    );
    assert_eq!(
        held_for_review(&pc),
        vec!["op-ghost".to_string(), "op-new-file".to_string()]
    );
}

/// A server row with no parent link that is gone from this account's tree is explained by the folder at its path.
#[test]
fn a_deleted_row_without_a_parent_link_is_explained_by_the_folder_at_its_path() {
    let pc = pc_with(&IDS);
    add_row_at(
        &pc,
        "own-folder",
        "Projects",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::Folder,
    );
    add_row_at(
        &pc,
        "deleted",
        "Projects/old.txt",
        None,
        Shape::Snapshot,
        Namespace::MyFiles,
        ItemKind::File,
    );
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(mine("own-folder"));
    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.gone == 1 && t.missing == 0),
        "{outcome:?}"
    );
}

/// Changes in a folder shared with this account, and a new file in a folder whose name proves nothing, are not
/// tied to this account's own data: held for review, never sent on their own. They do not block the adoption.
#[test]
fn changes_in_a_shared_folder_or_an_unproven_folder_are_held() {
    let pc = pc_with(&IDS);
    add_row_in(
        &pc,
        "shared-folder",
        None,
        Shape::Delta,
        Namespace::SharedWithMe,
        ItemKind::Folder,
    );
    add_row_in(
        &pc,
        "shared-file",
        Some("shared-folder"),
        Shape::Delta,
        Namespace::SharedWithMe,
        ItemKind::File,
    );
    add_row(&pc, "new-in-shared", Some("shared-folder"), Shape::Local);
    add_folder(&pc, "plain-folder", None, Shape::Snapshot);
    add_row(&pc, "new-in-plain", Some("plain-folder"), Shape::Local);
    pc.db
        .0
        .enqueue_operation(&edit("op-shared-edit", "shared-file"))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-shared-new", "new-in-shared", Some("shared-folder"), false))
        .unwrap();
    pc.db
        .0
        .enqueue_operation(&create("op-plain-new", "new-in-plain", Some("plain-folder"), false))
        .unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
    tree.push(with(
        mine("plain-folder"),
        "name_encrypted",
        serde_json::json!("Photos"),
    ));
    tree.push(mine("own-extra")); // the tree may hold more than this PC

    let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.held_for_review == 3 && t.proven_changes == 1),
        "{outcome:?}"
    );
    assert_eq!(due(&pc), vec!["held-1".to_string()]);
    assert_eq!(
        held_for_review(&pc),
        vec![
            "op-plain-new".to_string(),
            "op-shared-edit".to_string(),
            "op-shared-new".to_string()
        ]
    );
}

/// A change queued after the proof read the queue was never judged: it is held for review, not sent.
#[test]
fn a_change_queued_after_the_proof_read_the_queue_is_held() {
    let pc = pc_with(&IDS);
    adopt_unbound(&pc.db, None).unwrap();
    let server = own_server();
    let checks = Cell::new(0);
    let outcome = block_on(prove_and_adopt(
        &pc.db.0,
        &session(),
        &MINE,
        || server.list(),
        || {
            checks.set(checks.get() + 1);
            if server.listed.get() == 1 && checks.get() == 2 {
                // Right before the record: the watcher queues an edit of an own file.
                pc.db.0.enqueue_operation(&edit("late", IDS[1])).unwrap();
            }
            true
        },
    ))
    .unwrap();
    assert!(
        matches!(outcome, Outcome::Adopted(t) if t.held_for_review == 1),
        "{outcome:?}"
    );
    assert_eq!(due(&pc), vec!["held-1".to_string()]);
    assert_eq!(held_for_review(&pc), vec!["late".to_string()]);
}

/// A change paused because the server refused the old session (`auth`) that the proof cannot tie to this
/// account's data is held for review too, so the account's resume does not send it; a proven one stays `auth`.
#[test]
fn an_unproven_change_paused_for_auth_is_held_and_a_proven_one_is_not() {
    let pc = pc_with(&IDS);
    add_folder(&pc, "new-folder", None, Shape::Local);
    pc.db
        .0
        .enqueue_operation(&create("op-folder", "new-folder", None, true))
        .unwrap();
    pc.db.0.enqueue_operation(&edit("op-own-edit", IDS[1])).unwrap();
    for op_id in ["op-folder", "op-own-edit"] {
        pc.db
            .0
            .record_operation_pause(op_id, crate::state_db::OperationPauseReason::Auth, None, 5)
            .unwrap();
    }
    adopt_unbound(&pc.db, None).unwrap();

    assert!(matches!(prove(&pc, &session(), MINE, &own_server()), Outcome::Adopted(t) if t.held_for_review == 1));
    assert_eq!(
        pc.db.0.resume_operations_paused_for_auth(0).unwrap(),
        1,
        "only the proven one resumes"
    );
    assert_eq!(held_for_review(&pc), vec!["op-folder".to_string()]);
}

/// "Upload to my Beebeeb": every change held for review is due again; nothing else changes.
#[test]
fn upload_releases_only_the_changes_held_for_review() {
    let pc = pc_with(&IDS);
    add_folder(&pc, "new-folder", None, Shape::Local);
    pc.db
        .0
        .enqueue_operation(&create("op-folder", "new-folder", None, true))
        .unwrap();
    pc.db.0.enqueue_operation(&edit("op-auth", IDS[1])).unwrap();
    adopt_unbound(&pc.db, None).unwrap();
    assert!(matches!(
        prove(&pc, &session(), MINE, &own_server()),
        Outcome::Adopted(_)
    ));
    pc.db
        .0
        .record_operation_pause("op-auth", crate::state_db::OperationPauseReason::Auth, None, 5)
        .unwrap();

    assert_eq!(
        pc.db
            .0
            .release_found_while_paused(&["op-folder".to_string(), "op-auth".to_string()], 5)
            .unwrap(),
        1
    );

    let mut now_due = due(&pc);
    now_due.sort();
    assert_eq!(
        now_due,
        vec!["held-1".to_string(), "op-folder".to_string()],
        "op-auth stays paused for auth"
    );
}
