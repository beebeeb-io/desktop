//! The held-queue reconcile pass tests: the ghost/twin folds, the holds and the
//! replays, the scaffolding drops and the re-armed 404s, planned from plain rows.
//! They reach the parent's private helpers through `use super::*`.
use super::*;

const NOT_FOUND: &str = "HTTP status client error (404 Not Found) for url (https://api.example/api/v1/uploads/init)";

fn opts() -> Options {
    Options {
        device_segment: Some("PC-1".to_string()),
        case_insensitive: true,
    }
}

fn folder(id: &str, parent: Option<&str>, path: &str, server_known: bool) -> Row {
    Row {
        file_id: id.to_string(),
        parent_id: parent.map(str::to_string),
        path: path.to_string(),
        is_folder: true,
        server_known,
        current_version: if server_known { 3 } else { 0 },
    }
}

fn file(id: &str, parent: Option<&str>, path: &str, server_known: bool, current_version: i64) -> Row {
    Row {
        file_id: id.to_string(),
        parent_id: parent.map(str::to_string),
        path: path.to_string(),
        is_folder: false,
        server_known,
        current_version,
    }
}

fn create_folder(op_id: &str, file_id: &str, parent: Option<&str>) -> Op {
    Op {
        op_id: op_id.to_string(),
        kind: "create_folder".to_string(),
        file_id: Some(file_id.to_string()),
        parent_id: parent.map(str::to_string),
        ..Op::default()
    }
}

fn upload(op_id: &str, file_id: &str, parent: Option<&str>) -> Op {
    Op {
        op_id: op_id.to_string(),
        kind: "upload_version".to_string(),
        file_id: Some(file_id.to_string()),
        parent_id: parent.map(str::to_string),
        create_file: true,
        payload_path: Some(format!("staged/{op_id}")),
        ..Op::default()
    }
}

fn gave_up(op: Op, error: &str) -> Op {
    Op {
        given_up: true,
        last_error: Some(error.to_string()),
        ..op
    }
}

fn paused(op: Op, reason: &str) -> Op {
    Op {
        paused_reason: Some(reason.to_string()),
        ..op
    }
}

fn always(_: &str) -> bool {
    true
}

fn run(rows: &[Row], ops: &[Op]) -> (Vec<Unit>, Counts) {
    plan(rows, ops, &opts(), &always, &always)
}

fn remap(ghost: &str, twin: &str, ghost_op: &str, rearm: &[&str]) -> Unit {
    remap_holding(ghost, twin, ghost_op, rearm, &[])
}

fn remap_holding(ghost: &str, twin: &str, ghost_op: &str, rearm: &[&str], hold: &[&str]) -> Unit {
    let ids = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect();
    Unit::Remap {
        ghost: ghost.to_string(),
        twin: twin.to_string(),
        ghost_op: ghost_op.to_string(),
        rearm: ids(rearm),
        hold: ids(hold),
    }
}

fn hold(op_id: &str) -> Unit {
    Unit::Hold {
        op_id: op_id.to_string(),
    }
}

#[test]
fn a_ghost_with_one_twin_moves_its_children_and_disappears() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        file("c", Some("g"), "Photos/a.jpg", false, 0),
    ];
    let ops = [create_folder("op-g", "g", None), upload("op-c", "c", Some("g"))];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![remap("g", "t", "op-g", &[])]);
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn nested_ghosts_are_remapped_top_down_in_one_pass() {
    let rows = [
        folder("g2", Some("g1"), "A/B", false), // listed first: the order comes from depth, not from the input
        folder("t1", None, "A", true),
        folder("t2", Some("t1"), "A/B", true),
        folder("g1", None, "A", false),
    ];
    let ops = [
        create_folder("op-g2", "g2", Some("g1")),
        create_folder("op-g1", "g1", None),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![remap("g1", "t1", "op-g1", &[]), remap("g2", "t2", "op-g2", &[])]
    );
    assert_eq!(counts.remapped, 2);
}

#[test]
fn a_ghost_with_no_twin_keeps_its_create_and_its_children_wait() {
    let rows = [
        folder("g", None, "New", false),
        file("c", Some("g"), "New/a.txt", false, 0),
    ];
    let ops = [
        paused(create_folder("op-g", "g", None), "auth"),
        upload("op-c", "c", Some("g")),
    ];
    let (units, counts) = run(&rows, &ops);
    assert!(units.is_empty(), "{units:?}");
    assert_eq!(
        counts,
        Counts {
            replay: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_ghost_under_a_new_folder_replays_after_it() {
    let rows = [
        folder("g1", None, "New", false),
        folder("g2", Some("g1"), "New/Sub", false),
    ];
    let ops = [
        create_folder("op-g1", "g1", None),
        create_folder("op-g2", "g2", Some("g1")),
    ];
    let (units, counts) = run(&rows, &ops);
    assert!(units.is_empty(), "{units:?}");
    assert_eq!(
        counts,
        Counts {
            replay: 2,
            ..Counts::default()
        }
    );
}

#[test]
fn two_twins_hold_the_ghost_and_move_nothing() {
    let rows = [
        folder("t1", None, "Photos", true),
        folder("t2", None, "photos", true),
        folder("g", None, "Photos", false),
    ];
    let ops = [paused(create_folder("op-g", "g", None), "auth")];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![hold("op-g")]);
    assert_eq!(
        counts,
        Counts {
            held_ambiguous: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn an_op_acting_on_the_ghost_itself_holds_it() {
    let rows = [folder("t", None, "Photos", true), folder("g", None, "Photos", false)];
    let rename = Op {
        op_id: "op-rename".to_string(),
        kind: "rename_file".to_string(),
        file_id: Some("g".to_string()),
        ..Op::default()
    };
    let ops = [create_folder("op-g", "g", None), rename];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![hold("op-g")], "the rename op is untouched");
    assert_eq!(
        counts,
        Counts {
            held_acts_on_ghost: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_twin_with_its_own_pending_op_holds_the_ghost() {
    let rows = [folder("t", None, "Photos", true), folder("g", None, "Photos", false)];
    let trash = Op {
        op_id: "op-trash".to_string(),
        kind: "trash_file".to_string(),
        file_id: Some("t".to_string()),
        ..Op::default()
    };
    let ops = [create_folder("op-g", "g", None), trash];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![hold("op-g")]);
    assert_eq!(
        counts,
        Counts {
            held_twin_busy: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn parents_that_disagree_hold_the_ghost() {
    let rows = [
        folder("s1", None, "X", true),
        folder("s2", None, "Y", true),
        folder("t", Some("s1"), "X/Photos", true),
        folder("g", Some("s1"), "X/Photos", false),
    ];
    let ops = [create_folder("op-g", "g", Some("s2"))];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![hold("op-g")]);
    assert_eq!(
        counts,
        Counts {
            held_parents_disagree: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_landed_create_drops_only_its_op() {
    let rows = [folder("l", None, "Landed", true)];
    let ops = [create_folder("op-l", "l", None)];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![Unit::DropLanded {
            op_id: "op-l".to_string()
        }]
    );
    assert_eq!(
        counts,
        Counts {
            landed: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_twin_seen_only_through_a_delta_is_found() {
    // A delta row: stamped `remote_updated_at`, `current_version` still 0. The reader marks it server-known.
    let twin = Row {
        current_version: 0,
        ..folder("t", None, "Photos", true)
    };
    let rows = [twin, folder("g", None, "Photos", false)];
    let ops = [create_folder("op-g", "g", None)];
    let (units, _) = run(&rows, &ops);
    assert_eq!(units, vec![remap("g", "t", "op-g", &[])]);
}

#[test]
fn letter_case_matches_on_windows_only() {
    assert!(same_leaf("Photos", "photos", true));
    assert!(!same_leaf("Photos", "photos", false));
    assert!(same_leaf("Photos", "Photos", false));
    assert!(!same_leaf("Photos", "Photo", true));
}

#[test]
fn a_parent_404_under_a_ghost_is_rearmed_and_nothing_else_is() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        file("f1", Some("g"), "Photos/1.jpg", false, 0),
        file("f2", Some("g"), "Photos/2.jpg", false, 0),
        file("f3", Some("g"), "Photos/3.jpg", false, 0),
        file("f4", Some("g"), "Photos/4.jpg", false, 0),
    ];
    let park = Op {
        parked: true,
        last_error: Some(NOT_FOUND.to_string()),
        ..upload("op-park", "f2", Some("g"))
    };
    let ops = [
        paused(create_folder("op-g", "g", None), "auth"),
        gave_up(upload("op-404", "f1", Some("g")), NOT_FOUND),
        park,
        gave_up(
            upload("op-500", "f3", Some("g")),
            "HTTP status server error (500 Internal Server Error) for url (https://api.example/api/v1/files/404a0000-0000-4000-8000-000000000001)",
        ),
        gave_up(
            upload("op-401", "f4", Some("g")),
            "HTTP status client error (401 Unauthorized) for url (https://api.example/x)",
        ),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![remap("g", "t", "op-g", &["op-404"])]);
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            rearmed: 1,
            ..Counts::default()
        }
    );
}

/// D8 amendment (lead ruling 2026-10-29): a change held for the person that gave up on the 404 its ghost caused gets
/// its retry budget back in the remap, keeps its pause, and is never re-labelled; a park or another error is left.
#[test]
fn a_person_paused_upload_on_a_parent_404_under_a_remapped_ghost_gets_its_budget_back_and_keeps_its_pause() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        file("f1", Some("g"), "Photos/1.jpg", false, 0),
        file("f2", Some("g"), "Photos/2.jpg", false, 0),
        file("f3", Some("g"), "Photos/3.jpg", false, 0),
        file("f4", Some("g"), "Photos/4.jpg", false, 0),
    ];
    let not_found = |op: Op| Op {
        last_error: Some(NOT_FOUND.to_string()),
        ..op
    };
    let ops = [
        paused(create_folder("op-g", "g", None), "auth"),
        paused(not_found(upload("op-held", "f1", Some("g"))), "found_while_paused"),
        paused(not_found(upload("op-quota", "f2", Some("g"))), "quota"),
        paused(
            Op {
                parked: true,
                ..not_found(upload("op-park", "f3", Some("g")))
            },
            "found_while_paused",
        ),
        paused(
            Op {
                last_error: Some(
                    "HTTP status server error (500 Internal Server Error) for url (https://api.example/x)".to_string(),
                ),
                ..upload("op-500", "f4", Some("g"))
            },
            "found_while_paused",
        ),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![remap("g", "t", "op-g", &["op-held", "op-quota"])]);
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            rearmed_paused: 2,
            ..Counts::default()
        }
    );
}

/// D8 amendment: the same under a ghost kept for replay (no twin).
#[test]
fn a_person_paused_upload_on_a_parent_404_under_a_replayed_ghost_gets_its_budget_back() {
    let rows = [
        folder("g", None, "New", false),
        file("f1", Some("g"), "New/1.jpg", false, 0),
    ];
    let ops = [
        create_folder("op-g", "g", None),
        paused(
            Op {
                last_error: Some(NOT_FOUND.to_string()),
                ..upload("op-held", "f1", Some("g"))
            },
            "found_while_paused",
        ),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![Unit::Rearm {
            op_id: "op-held".to_string()
        }]
    );
    assert_eq!(
        counts,
        Counts {
            replay: 1,
            rearmed_paused: 1,
            ..Counts::default()
        }
    );
}

/// D8 amendment: a held change the server already superseded is left as it is: no budget, no `needs_review`.
#[test]
fn a_superseded_person_paused_upload_is_left_as_it_is() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        file("f1", Some("g"), "Photos/1.jpg", true, 4),
    ];
    let ops = [
        paused(create_folder("op-g", "g", None), "auth"),
        paused(
            Op {
                last_error: Some(NOT_FOUND.to_string()),
                ..upload("op-held", "f1", Some("g"))
            },
            "found_while_paused",
        ),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![remap("g", "t", "op-g", &[])]);
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_rearm_needs_its_staged_payload() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        file("f1", Some("g"), "Photos/1.jpg", false, 0),
    ];
    let ops = [
        create_folder("op-g", "g", None),
        gave_up(upload("op-404", "f1", Some("g")), NOT_FOUND),
    ];
    let (units, counts) = plan(&rows, &ops, &opts(), &|_| false, &always);
    assert_eq!(units, vec![remap("g", "t", "op-g", &[])]);
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            rearm_no_payload: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn an_upload_superseded_by_a_landed_version_is_held_not_rearmed() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        file("landed", Some("g"), "Photos/new.jpg", true, 1),
        file("newer", Some("g"), "Photos/old.jpg", true, 3),
    ];
    let behind = Op {
        create_file: false,
        base_version: Some(2),
        ..upload("op-ver", "newer", Some("g"))
    };
    let ops = [
        create_folder("op-g", "g", None),
        gave_up(upload("op-create", "landed", Some("g")), NOT_FOUND),
        gave_up(behind, NOT_FOUND),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![remap_holding("g", "t", "op-g", &[], &["op-create", "op-ver"])]
    );
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            rearm_superseded: 2,
            ..Counts::default()
        }
    );
}

#[test]
fn a_given_up_folder_under_a_new_folder_is_rearmed_and_replays() {
    // The 28 Oct shape, inside this device's own backup folder: never dropped.
    let rows = [
        folder("b", None, "Backups", true),
        folder("pc", Some("b"), "Backups/PC-1", true),
        folder("g1", Some("pc"), "Backups/PC-1/Docs", false),
        folder("g2", Some("g1"), "Backups/PC-1/Docs/2026", false),
    ];
    let ops = [
        paused(create_folder("op-g1", "g1", Some("pc")), "auth"),
        gave_up(create_folder("op-g2", "g2", Some("g1")), NOT_FOUND),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![Unit::Rearm {
            op_id: "op-g2".to_string()
        }]
    );
    assert_eq!(
        counts,
        Counts {
            replay: 2,
            rearmed: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_held_ghost_is_remapped_once_its_ambiguity_is_gone() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        folder("t1", None, "Music", true),
        folder("t2", None, "music", true),
        folder("h", None, "Music", false),
    ];
    let ops = [
        paused(create_folder("op-g", "g", None), "needs_review"),
        paused(create_folder("op-h", "h", None), "needs_review"),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![remap("g", "t", "op-g", &[])],
        "the still-ambiguous one is neither held again nor released"
    );
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            held_ambiguous: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_held_ghost_whose_twins_are_gone_is_released_to_replay() {
    let rows = [folder("g", None, "Photos", false)];
    let ops = [paused(create_folder("op-g", "g", None), "needs_review")];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![Unit::Release {
            op_id: "op-g".to_string()
        }]
    );
    assert_eq!(
        counts,
        Counts {
            replay: 1,
            released: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_ghost_under_a_held_ghost_waits_with_it() {
    let rows = [
        folder("t1", None, "Photos", true),
        folder("t2", None, "photos", true),
        folder("g", None, "Photos", false),
        folder("g-sub", Some("g"), "Photos/2026", false),
    ];
    let ops = [
        create_folder("op-g", "g", None),
        create_folder("op-sub", "g-sub", Some("g")),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![hold("op-g")]);
    assert_eq!(
        counts,
        Counts {
            held_ambiguous: 1,
            waiting_under_held: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_held_ghost_paused_for_another_reason_keeps_its_reason() {
    // 2267: a change found while sync was paused waits for the person; the pass never re-labels it.
    let rows = [
        folder("t1", None, "Photos", true),
        folder("t2", None, "photos", true),
        folder("g", None, "Photos", false),
    ];
    let ops = [paused(create_folder("op-g", "g", None), "found_while_paused")];
    let (units, counts) = run(&rows, &ops);
    assert!(units.is_empty(), "{units:?}");
    assert_eq!(
        counts,
        Counts {
            left_paused: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_person_paused_ghost_with_one_twin_is_left_alone_and_its_sub_ghost_waits() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        folder("g-sub", Some("g"), "Photos/2026", false),
    ];
    let ops = [
        paused(create_folder("op-g", "g", None), "found_while_paused"),
        create_folder("op-sub", "g-sub", Some("g")),
    ];
    let (units, counts) = run(&rows, &ops);
    assert!(units.is_empty(), "{units:?}");
    assert_eq!(
        counts,
        Counts {
            left_paused: 1,
            waiting_under_held: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_nested_ghost_without_a_parent_id_is_never_matched_at_the_top_level() {
    // D6 has no path to go by (its create names no parent and no target path): it is held for review, never matched
    // at the top level and never resumed into a create at the vault root.
    let rows = [
        folder("a", None, "A", true),
        folder("t", Some("a"), "A/Photos", true),
        folder("decoy", None, "Photos", true),
        folder("g", None, "A/photos", false),
    ];
    let ops = [create_folder("op-g", "g", None)];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![Unit::Hold {
            op_id: "op-g".to_string()
        }]
    );
    assert_eq!(
        counts,
        Counts {
            parent_unresolved: 1,
            ..Counts::default()
        }
    );
}

/// D4, cross-lane review P2-1: the row names no parent while its create names `a`. Left as it is, the create is
/// resumed and sent under `a`, next to `t`: a duplicate folder. It is held, and the folder under it waits with it.
#[test]
fn a_nested_ghost_whose_row_names_no_parent_is_held_and_its_subtree_waits() {
    let rows = [
        folder("a", None, "A", true),
        folder("t", Some("a"), "A/Photos", true),
        folder("g", None, "A/Photos", false),
        folder("g2", Some("g"), "A/Photos/2024", false),
    ];
    let ops = [
        create_folder("op-g", "g", Some("a")),
        create_folder("op-g2", "g2", Some("g")),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![Unit::Hold {
            op_id: "op-g".to_string()
        }]
    );
    assert_eq!(
        counts,
        Counts {
            parent_unresolved: 1,
            waiting_under_held: 1,
            ..Counts::default()
        }
    );

    // Already held: a second pass adds nothing.
    let ops = [
        paused(create_folder("op-g", "g", Some("a")), NEEDS_REVIEW),
        create_folder("op-g2", "g2", Some("g")),
    ];
    let (units, _) = run(&rows, &ops);
    assert!(units.is_empty(), "{units:?}");
}

#[test]
fn a_twin_whose_own_create_landed_is_not_busy() {
    let rows = [folder("t", None, "Photos", true), folder("g", None, "Photos", false)];
    let ops = [create_folder("op-t", "t", None), create_folder("op-g", "g", None)];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![
            Unit::DropLanded {
                op_id: "op-t".to_string()
            },
            remap("g", "t", "op-g", &[])
        ]
    );
    assert_eq!(
        counts,
        Counts {
            landed: 1,
            remapped: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_replayed_ghost_whose_create_gave_up_on_something_else_is_reported_not_rearmed() {
    let rows = [folder("g", None, "New", false)];
    let ops = [gave_up(
        create_folder("op-g", "g", None),
        "HTTP status client error (403 Forbidden) for url (https://api.example/x)",
    )];
    let (units, counts) = run(&rows, &ops);
    assert!(units.is_empty(), "{units:?}");
    assert_eq!(
        counts,
        Counts {
            replay: 1,
            replay_given_up: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_parent_404_is_read_before_the_url() {
    assert!(is_parent_404(NOT_FOUND));
    assert!(!is_parent_404(
        "HTTP status server error (500) for url (https://api.example/files/404a0000)"
    ));
    assert!(!is_parent_404("connection reset"));
}

/// The op's sync-root-relative key, as every create queues it.
fn at(op: Op, path: &str) -> Op {
    Op {
        target_path: Some(path.to_string()),
        ..op
    }
}

fn resolve(op_id: &str, file_id: &str, parent: &str) -> Unit {
    Unit::ResolveParent {
        op_id: op_id.to_string(),
        file_id: Some(file_id.to_string()),
        parent: parent.to_string(),
    }
}

/// D6: a create queued with no parent but a nested path gets the one server-known folder at its directory.
#[test]
fn a_root_level_upload_with_a_nested_path_gets_its_parent_by_path() {
    let rows = [
        folder("a", None, "A", true),
        folder("b", Some("a"), "A/B", true),
        folder("decoy", None, "B", true),
        file("f", None, "A/B/f.txt", false, 0),
    ];
    let ops = [at(paused(upload("op-f", "f", None), "auth"), "A/B/f.txt")];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![resolve("op-f", "f", "b")]);
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            ..Counts::default()
        }
    );
}

/// D6: two server-known folders at the directory (`≡`), or none at all: never a guess, the create is held.
#[test]
fn a_nested_path_that_names_no_single_folder_is_held() {
    let rows = [
        folder("a", None, "A", true),
        folder("b1", Some("a"), "A/B", true),
        folder("b2", Some("a"), "A/b", true),
        file("f", None, "A/B/f.txt", false, 0),
        file("g", None, "C/D/g.txt", false, 0),
    ];
    let ops = [
        at(upload("op-f", "f", None), "A/B/f.txt"),
        at(upload("op-g", "g", None), "C/D/g.txt"),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![hold("op-f"), hold("op-g")]);
    assert_eq!(
        counts,
        Counts {
            parent_unresolved: 2,
            ..Counts::default()
        }
    );
    // Held already: no second Hold unit, still counted.
    let held: Vec<Op> = ops.iter().cloned().map(|op| paused(op, NEEDS_REVIEW)).collect();
    let (units, counts) = run(&rows, &held);
    assert!(units.is_empty(), "{units:?}");
    assert_eq!(
        counts,
        Counts {
            parent_unresolved: 2,
            ..Counts::default()
        }
    );
}

/// D6 then D3: the only folder at the directory is new (a ghost with no twin): the change goes under it and waits.
#[test]
fn a_nested_path_that_only_a_new_folder_matches_goes_under_it() {
    let rows = [
        folder("a", None, "A", true),
        folder("g", Some("a"), "A/B", false),
        file("f", None, "A/B/f.txt", false, 0),
    ];
    let ops = [
        create_folder("op-g", "g", Some("a")),
        at(upload("op-f", "f", None), "A/B/f.txt"),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![resolve("op-f", "f", "g")]);
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            replay: 1,
            ..Counts::default()
        }
    );
}

/// D6 then D2, in ONE plan: the parent found by path is a ghost, so the ghost's remap (after the resolve) re-points
/// the change to the twin. The twin is found by parent and leaf (§3); its path here does not match, so the ghost is
/// the only folder at the directory.
#[test]
fn a_parent_found_by_path_that_is_a_ghost_is_remapped_in_the_same_plan() {
    let rows = [
        folder("a", None, "A", true),
        folder("g", Some("a"), "A/B", false),
        folder("t", Some("a"), "Old/B", true),
        file("f", None, "A/B/f.txt", false, 0),
        file("s", Some("t"), "Old/B/f.txt", true, 2),
    ];
    let ops = [
        create_folder("op-g", "g", Some("a")),
        at(upload("op-f", "f", None), "A/B/f.txt"),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![resolve("op-f", "f", "g"), remap("g", "t", "op-g", &[]), hold("op-f")]
    );
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            remapped: 1,
            file_twin: 1,
            ..Counts::default()
        },
        "the change meets the twin's file through the resolved parent and the remap"
    );
}

/// D6 before D2: a nested ghost the scan wrote without a parent is matched under the folder at its directory, never
/// at the top level.
#[test]
fn a_nested_ghost_without_a_parent_id_gets_its_parent_by_path_and_is_remapped() {
    let rows = [
        folder("a", None, "A", true),
        folder("t", Some("a"), "A/Photos", true),
        folder("decoy", None, "Photos", true),
        folder("g", None, "A/photos", false),
    ];
    let ops = [at(create_folder("op-g", "g", None), "A/photos")];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![resolve("op-g", "g", "a"), remap("g", "t", "op-g", &[])]);
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            remapped: 1,
            ..Counts::default()
        }
    );
}

/// D6 is for creates: a move with no parent is a move to the top level, and a hydrate names no parent at all.
#[test]
fn only_a_create_gets_its_parent_by_path() {
    let rows = [
        folder("a", None, "A", true),
        folder("b", Some("a"), "A/B", true),
        file("m", Some("b"), "A/B/m.txt", true, 1),
        file("h", Some("b"), "A/B/h.txt", true, 1),
        folder("c", None, "A/B/C", false),
    ];
    let op = |op_id: &str, kind: &str, file_id: &str| Op {
        op_id: op_id.to_string(),
        kind: kind.to_string(),
        file_id: Some(file_id.to_string()),
        ..Op::default()
    };
    let ops = [
        at(op("op-move", "move_file", "m"), "A/B/m.txt"),
        at(op("op-hydrate", "hydrate_file", "h"), "A/B/h.txt"),
        at(create_folder("op-c", "c", None), "A/B/C"),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![resolve("op-c", "c", "b")],
        "the move and the hydrate are untouched"
    );
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            replay: 1,
            ..Counts::default()
        }
    );
}

/// D6 for a change held for the person (2267): its parent is set, its pause stays; unresolved, it is left as it is.
#[test]
fn a_change_held_for_the_person_gets_its_parent_by_path_and_keeps_its_pause() {
    let rows = [
        folder("a", None, "A", true),
        folder("b", Some("a"), "A/B", true),
        file("f", None, "A/B/f.txt", false, 0),
        file("g", None, "C/D/g.txt", false, 0),
    ];
    let ops = [
        paused(at(upload("op-f", "f", None), "A/B/f.txt"), "found_while_paused"),
        paused(at(upload("op-g", "g", None), "C/D/g.txt"), "found_while_paused"),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![resolve("op-f", "f", "b")]);
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            ..Counts::default()
        }
    );
}

/// D6 re-evaluated: a change held because no single folder matched, once one does, goes under it and is released.
#[test]
fn a_held_change_whose_parent_is_found_later_is_released_under_it() {
    let rows = [
        folder("a", None, "A", true),
        folder("b", Some("a"), "A/B", true),
        file("f", None, "A/B/f.txt", false, 0),
    ];
    let ops = [paused(at(upload("op-f", "f", None), "A/B/f.txt"), NEEDS_REVIEW)];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![
            resolve("op-f", "f", "b"),
            Unit::Release {
                op_id: "op-f".to_string()
            }
        ]
    );
    assert_eq!(
        counts,
        Counts {
            parent_resolved: 1,
            released: 1,
            ..Counts::default()
        }
    );
}

/// D10: a new file meets a server file of the same name (`≡`) in the same folder: held, never compared or sent.
#[test]
fn a_new_file_that_meets_a_server_file_of_the_same_name_is_held() {
    let rows = [
        folder("p", None, "Docs", true),
        file("s", Some("p"), "Docs/report.txt", true, 2),
        file("n", Some("p"), "Docs/Report.txt", false, 0),
    ];
    let ops = [paused(upload("op-n", "n", Some("p")), "auth")];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![hold("op-n")]);
    assert_eq!(
        counts,
        Counts {
            file_twin: 1,
            ..Counts::default()
        }
    );

    let (units, counts) = run(&[rows[0].clone(), rows[2].clone()], &ops);
    assert!(units.is_empty(), "without the twin: {units:?}");
    assert_eq!(counts, Counts::default());
}

/// D10 under a remapped ghost: the effective parent is the twin folder, so the new file meets the twin's file. It
/// is held, not re-armed, though it gave up on the ghost's 404.
#[test]
fn a_new_file_under_a_remapped_ghost_that_meets_a_server_file_is_held_not_rearmed() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        file("s", Some("t"), "Photos/1.jpg", true, 1),
        file("n", Some("g"), "Photos/1.jpg", false, 0),
    ];
    let ops = [
        create_folder("op-g", "g", None),
        gave_up(upload("op-n", "n", Some("g")), NOT_FOUND),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![remap("g", "t", "op-g", &[]), hold("op-n")]);
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            file_twin: 1,
            ..Counts::default()
        }
    );
}

/// Task 2271, the shape on the maintainer's PC (128 of 145 held new files): the file's row names no parent, its
/// create names the ghost. The create is sent under the ghost's twin, so D10 must look there: the twin's file of the
/// same name makes it a file twin, held, never sent as a second file of that name.
#[test]
fn a_new_file_whose_row_names_no_parent_is_judged_under_the_folder_its_create_is_sent_to() {
    let rows = [
        folder("a", None, "A", true),
        folder("t", Some("a"), "A/Photos", true),
        folder("g", Some("a"), "A/Photos", false),
        file("s", Some("t"), "A/Photos/1.jpg", true, 1),
        file("n", None, "A/Photos/1.jpg", false, 0),
    ];
    let ops = [
        create_folder("op-g", "g", Some("a")),
        paused(upload("op-n", "n", Some("g")), "auth"),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![remap("g", "t", "op-g", &[]), hold("op-n")]);
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            file_twin: 1,
            ..Counts::default()
        }
    );
}

/// Task 2271: a row that names no parent names no folder, so a top-level server file of the same leaf is no twin;
/// a row that names another folder than its create is checked there too, and a twin under either holds it.
#[test]
fn a_row_parent_counts_only_when_it_names_a_folder() {
    let rows = [
        folder("t", None, "Photos", true),
        folder("g", None, "Photos", false),
        file("top", None, "1.jpg", true, 1),
        file("n", None, "Photos/1.jpg", false, 0),
    ];
    let ops = [
        create_folder("op-g", "g", None),
        paused(upload("op-n", "n", Some("g")), "auth"),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![remap("g", "t", "op-g", &[])],
        "the top-level 1.jpg is not in the folder it is sent to"
    );
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            ..Counts::default()
        }
    );

    let rows = [
        folder("p", None, "Docs", true),
        folder("q", None, "Other", true),
        file("s", Some("q"), "Other/a.txt", true, 2),
        file("n", Some("q"), "Docs/a.txt", false, 0),
    ];
    let ops = [paused(upload("op-n", "n", Some("p")), "auth")];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![hold("op-n")],
        "the row's own folder holds a file of that name"
    );
    assert_eq!(
        counts,
        Counts {
            file_twin: 1,
            ..Counts::default()
        }
    );
}

/// D10: only a server FILE that no op names is a twin. A server file with a pending change is in flux (the person
/// may be replacing it); a folder of the same name is not a file.
#[test]
fn only_an_idle_server_file_is_a_file_twin() {
    let rows = [
        folder("p", None, "Docs", true),
        file("s", Some("p"), "Docs/a.txt", true, 2),
        file("n", Some("p"), "Docs/a.txt", false, 0),
        folder("q", Some("p"), "Docs/b", true),
        file("m", Some("p"), "Docs/b", false, 0),
        file("idle", Some("p"), "Docs/c.txt", true, 1),
        file("k", Some("p"), "Docs/C.txt", false, 0),
    ];
    let trash = Op {
        op_id: "op-trash".to_string(),
        kind: "trash_file".to_string(),
        file_id: Some("s".to_string()),
        ..Op::default()
    };
    let ops = [
        trash,
        upload("op-n", "n", Some("p")),
        upload("op-m", "m", Some("p")),
        upload("op-k", "k", Some("p")),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(units, vec![hold("op-k")], "only the idle file is a twin");
    assert_eq!(
        counts,
        Counts {
            file_twin: 1,
            ..Counts::default()
        }
    );
}

/// D10 re-evaluated: a held new file whose twin is gone is released; one whose twin is still there stays held.
#[test]
fn a_held_file_whose_twin_is_gone_is_released() {
    let rows = [
        folder("p", None, "Docs", true),
        file("s", Some("p"), "Docs/kept.txt", true, 2),
        file("n", Some("p"), "Docs/kept.txt", false, 0),
        file("m", Some("p"), "Docs/gone.txt", false, 0),
    ];
    let ops = [
        paused(upload("op-n", "n", Some("p")), NEEDS_REVIEW),
        paused(upload("op-m", "m", Some("p")), NEEDS_REVIEW),
    ];
    let (units, counts) = run(&rows, &ops);
    assert_eq!(
        units,
        vec![Unit::Release {
            op_id: "op-m".to_string()
        }]
    );
    assert_eq!(
        counts,
        Counts {
            file_twin: 1,
            released: 1,
            ..Counts::default()
        }
    );
}

fn never(_: &str) -> bool {
    false
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn drop_scaffold(op_ids: &[&str], file_ids: &[&str], dirs: &[&str]) -> Unit {
    Unit::DropScaffold {
        op_ids: strings(op_ids),
        file_ids: strings(file_ids),
        dirs: strings(dirs),
    }
}

/// The 28 Oct shape (D5): the backup mirror could not read this PC's name and scaffolded `Backups/unknown` (D9)
/// beside this PC's own `Backups/PC-1`, three empty folders deep.
fn backups_unknown() -> (Vec<Row>, Vec<Op>) {
    let rows = vec![
        folder("b", None, "Backups", true),
        folder("pc", Some("b"), "Backups/PC-1", true),
        folder("u", Some("b"), "Backups/unknown", false),
        folder("d", Some("u"), "Backups/unknown/Documents", false),
        folder("o", Some("d"), "Backups/unknown/Documents/Old", false),
    ];
    let ops = vec![
        paused(create_folder("op-u", "u", Some("b")), "auth"),
        paused(create_folder("op-d", "d", Some("u")), "auth"),
        gave_up(create_folder("op-o", "o", Some("d")), NOT_FOUND),
    ];
    (rows, ops)
}

/// D5: the whole subtree goes in one unit, its directories listed deepest first for the caller to remove after the
/// commit. The disk is asked once, about the subtree's top, which covers everything under it.
#[test]
fn empty_backups_unknown_scaffolding_is_dropped_and_its_directories_removed() {
    let (rows, ops) = backups_unknown();
    let asked = std::cell::RefCell::new(Vec::new());
    let disk_has_file = |dir: &str| {
        asked.borrow_mut().push(dir.to_string());
        false
    };
    let (units, counts) = plan(&rows, &ops, &opts(), &always, &disk_has_file);
    let dirs = [
        "Backups/unknown/Documents/Old",
        "Backups/unknown/Documents",
        "Backups/unknown",
    ];
    assert_eq!(
        units,
        vec![drop_scaffold(&["op-u", "op-d", "op-o"], &["u", "d", "o"], &dirs)]
    );
    assert_eq!(
        counts,
        Counts {
            scaffold_dropped: 3,
            scaffold_dirs: strings(&dirs),
            ..Counts::default()
        }
    );
    assert_eq!(asked.into_inner(), strings(&["Backups/unknown"]));
}

/// D5, anti-goal 3: a file on disk under it keeps the subtree, held as D4; the folders under the held one wait.
#[test]
fn backups_unknown_with_a_file_on_disk_is_kept_and_held() {
    let (rows, ops) = backups_unknown();
    let (units, counts) = plan(&rows, &ops, &opts(), &always, &always);
    assert_eq!(units, vec![hold("op-u")], "nothing dropped, nothing re-armed");
    assert_eq!(
        counts,
        Counts {
            backup_unknown_kept: 1,
            waiting_under_held: 2,
            ..Counts::default()
        }
    );
}

/// D5: "only if the whole subtree is scaffolding". Anything else there keeps it, even with no file on disk: a
/// change queued under it, a row that is not a ghost, an op acting on one of its folders, a change moving in, or a
/// folder's create held for the person.
#[test]
fn backups_unknown_with_more_than_scaffolding_under_it_is_kept_and_held() {
    let upload_under_it = |rows: &mut Vec<Row>, ops: &mut Vec<Op>| {
        rows.push(file("f", Some("d"), "Backups/unknown/Documents/a.txt", false, 0));
        ops.push(upload("op-f", "f", Some("d")));
    };
    let server_file_under_it = |rows: &mut Vec<Row>, _: &mut Vec<Op>| {
        rows.push(file("s", Some("d"), "Backups/unknown/Documents/b.txt", true, 2));
    };
    let rename_of_a_folder = |_: &mut Vec<Row>, ops: &mut Vec<Op>| {
        let rename = Op {
            op_id: "op-rename".to_string(),
            kind: "rename_file".to_string(),
            ..Op::default()
        };
        ops.push(Op {
            file_id: Some("d".to_string()),
            ..rename
        });
    };
    let move_into_it = |rows: &mut Vec<Row>, ops: &mut Vec<Op>| {
        rows.push(file("m", None, "Notes.txt", true, 1));
        ops.push(Op {
            op_id: "op-move".to_string(),
            kind: "move_file".to_string(),
            file_id: Some("m".to_string()),
            target_path: Some("Backups/unknown/Documents/Notes.txt".to_string()),
            ..Op::default()
        });
    };
    let held_for_the_person = |_: &mut Vec<Row>, ops: &mut Vec<Op>| {
        ops[1].paused_reason = Some("found_while_paused".to_string());
    };
    let cases: [(&str, &dyn Fn(&mut Vec<Row>, &mut Vec<Op>)); 5] = [
        ("upload under it", &upload_under_it),
        ("server file under it", &server_file_under_it),
        ("rename of a folder", &rename_of_a_folder),
        ("move into it", &move_into_it),
        ("held for the person", &held_for_the_person),
    ];
    for (case, add) in cases {
        let (mut rows, mut ops) = backups_unknown();
        add(&mut rows, &mut ops);
        let (units, counts) = plan(&rows, &ops, &opts(), &always, &never);
        assert_eq!(units, vec![hold("op-u")], "{case}");
        assert_eq!((counts.scaffold_dropped, counts.backup_unknown_kept), (0, 1), "{case}");
    }
}

/// D5, D9: when this PC's own backup folder could be `Backups/unknown` (its name is `unknown`, under `≡`) or its
/// name cannot be read, D5 does not apply and the folders replay as before (D3). On Linux `Unknown` is another
/// folder than `unknown`, so there D5 still applies.
#[test]
fn backups_unknown_is_never_dropped_when_this_device_is_unknown_or_unreadable() {
    let (rows, ops) = backups_unknown();
    for device in [Some("unknown"), Some("UNKNOWN"), Some(" unknown."), None] {
        let opts = Options {
            device_segment: device.map(str::to_string),
            case_insensitive: true,
        };
        let (units, counts) = plan(&rows, &ops, &opts, &always, &never);
        assert_eq!(
            units,
            vec![Unit::Rearm {
                op_id: "op-o".to_string()
            }],
            "{device:?}"
        );
        assert_eq!(
            counts,
            Counts {
                replay: 3,
                rearmed: 1,
                ..Counts::default()
            },
            "{device:?}"
        );
    }
    let linux = Options {
        device_segment: Some("Unknown".to_string()),
        case_insensitive: false,
    };
    assert_eq!(plan(&rows, &ops, &linux, &always, &never).1.scaffold_dropped, 3);
}

/// D5 before D8: scaffolding whose create gave up on the 404 of a folder the pass folds into its twin is dropped,
/// never re-armed, so it can never be sent.
#[test]
fn scaffolding_that_gave_up_on_its_parents_404_is_dropped_not_rearmed() {
    let rows = [
        folder("bt", None, "Backups", true),
        folder("bg", None, "Backups", false),
        folder("u", Some("bg"), "Backups/unknown", false),
    ];
    let ops = [
        create_folder("op-bg", "bg", None),
        gave_up(create_folder("op-u", "u", Some("bg")), NOT_FOUND),
    ];
    let (units, counts) = plan(&rows, &ops, &opts(), &always, &never);
    assert_eq!(
        units,
        vec![
            remap("bg", "bt", "op-bg", &[]),
            drop_scaffold(&["op-u"], &["u"], &["Backups/unknown"])
        ]
    );
    assert_eq!(
        counts,
        Counts {
            remapped: 1,
            scaffold_dropped: 1,
            scaffold_dirs: strings(&["Backups/unknown"]),
            ..Counts::default()
        }
    );
}

/// D5 matches `Backups/unknown` by whole path segments, under `≡`: a folder that only looks like it replays.
#[test]
fn backups_unknown_is_matched_by_whole_segments() {
    let rows = [
        folder("b", None, "Backups", true),
        folder("a", None, "Archive", true),
        folder("g1", Some("b"), "Backups/unknownish", false),
        folder("g2", Some("a"), "Archive/unknown", false),
        folder("g3", Some("b"), "backups/UNKNOWN", false),
    ];
    let ops = [
        create_folder("op-g1", "g1", Some("b")),
        create_folder("op-g2", "g2", Some("a")),
        create_folder("op-g3", "g3", Some("b")),
    ];
    let (units, counts) = plan(&rows, &ops, &opts(), &always, &never);
    assert_eq!(
        units,
        vec![drop_scaffold(&["op-g3"], &["g3"], &["backups/UNKNOWN"])],
        "Windows: NTFS ignores case"
    );
    assert_eq!(counts.replay, 2);
    let linux = Options {
        case_insensitive: false,
        ..opts()
    };
    let (units, counts) = plan(&rows, &ops, &linux, &always, &never);
    assert!(units.is_empty(), "{units:?}");
    assert_eq!(
        counts,
        Counts {
            replay: 3,
            ..Counts::default()
        }
    );
}
