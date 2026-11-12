//! The running app's flyout from what the engine knows (`live.rs`): each state from its facts, in the
//! board's order, the rows and their times. The engine-backed source that gathers the facts is
//! `engine_host::tray`'s, tested there on real engines.

use chrono::{DateTime, Local as LocalTime, TimeZone};

use super::live::{self, Facts, Gate, Local, Progress, Report, Row, RowKind};
use super::source::{Action, Change, Changes, Hidden, MAX_ROWS, Origin, Snapshot, Status, Tile, What};
use crate::frames::global::source::size_words;

/// An invented app name: the sentences follow the build's identity, never a spelled-out one.
const NAME: &str = "Hive";

/// Thursday 12 November 2026, 15:30 on this PC's clock.
fn now() -> DateTime<LocalTime> {
    LocalTime
        .with_ymd_and_hms(2026, 11, 12, 15, 30, 0)
        .single()
        .expect("a time that exists")
}

fn ago(seconds: i64) -> i64 {
    now().timestamp() - seconds
}

const MIN: i64 = 60;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;

/// Signed in, unlocked, sync runs and its last check found nothing to do; nothing in the database.
fn ready() -> Facts {
    Facts {
        gate: Gate::Ready,
        session_ended: false,
        running: true,
        paused: false,
        report: Report::Idle,
        progress: Progress::default(),
        explorer_ready: false,
        local: Some(Local::default()),
        storage: None,
    }
}

fn with_local(facts: Facts, local: Local) -> Facts {
    Facts {
        local: Some(local),
        ..facts
    }
}

fn snap(facts: &Facts) -> Snapshot {
    live::snapshot(facts, NAME, now())
}

fn row(path: &str, at: i64, kind: RowKind) -> Row {
    Row {
        id: Some(format!("id-{path}")),
        path: path.to_string(),
        at,
        kind,
    }
}

/// Rows that name files: none of them may show while the gate is not Ready.
fn named_rows() -> Local {
    Local {
        rows: vec![row("/Plans/2027/ledger.ods", ago(2 * MIN), RowKind::Uploaded)],
        ..Local::default()
    }
}

/// DW-1212: "never a guess such as Up to date". Running, before the engine's first report (and after
/// a resume, which is no report), the flyout says Checking, also over what the database holds; the
/// first finished check makes it up to date.
#[test]
fn before_the_engines_first_report_the_flyout_says_checking_never_up_to_date() {
    let conflicts = Local {
        conflicts: 2,
        ..Local::default()
    };
    for report in [Report::Unknown, Report::Starting] {
        assert_eq!(
            snap(&Facts { report, ..ready() }).status,
            Status::Checking,
            "{report:?}"
        );
        assert_eq!(
            snap(&with_local(Facts { report, ..ready() }, conflicts.clone())).status,
            Status::Checking,
            "{report:?}: the database's conflicts wait for the check"
        );
    }
    assert_eq!(
        snap(&ready()).status,
        Status::UpToDate {
            files: None,
            last_change: None
        }
    );
    // A report that does not fit the switches (stopped, while running and not paused): not known.
    assert_eq!(
        snap(&Facts {
            report: Report::Quiet,
            ..ready()
        })
        .status,
        Status::Checking
    );
}

/// The session gate decides first. Only a signed-in, unlocked account shows file names; the
/// buttons that would do nothing (Unlock with Windows Hello: not wired) are not drawn.
#[test]
fn the_gate_decides_before_anything_the_engine_reports_and_hides_every_name() {
    let empty_since = |name: &str| Changes::Empty {
        text: format!("Changes show here once {name} syncs this PC."),
    };
    let cases = [
        (
            Gate::SignedOut,
            false,
            Status::SignedOut,
            Changes::Hidden(Hidden::SignedOut),
            Some((Action::SignIn, true)),
        ),
        (
            Gate::Locked,
            true,
            Status::Locked,
            Changes::Hidden(Hidden::Locked),
            None,
        ),
        (
            Gate::Locked,
            false,
            Status::LockedSyncOff,
            Changes::Hidden(Hidden::Locked),
            None,
        ),
        (
            Gate::NeedsRecoveryPhrase,
            false,
            Status::NeedsRecoveryPhrase,
            Changes::Hidden(Hidden::NoKeys),
            None,
        ),
        (
            Gate::KeyStoreUnavailable,
            false,
            Status::KeysUnreadable,
            Changes::Hidden(Hidden::NoKeys),
            None,
        ),
        (
            Gate::NoPlan,
            false,
            Status::NoPlan { waiting: false },
            empty_since(NAME),
            None,
        ),
    ];
    for (gate, running, status, changes, button) in cases {
        let facts = with_local(
            Facts {
                gate,
                running,
                ..ready()
            },
            named_rows(),
        );
        let snapshot = snap(&facts);
        assert_eq!(
            (&snapshot.status, &snapshot.changes, snapshot.button),
            (&status, &changes, button),
            "{gate:?}, running {running}"
        );
    }
    // The hidden list's sentence names the build's app.
    assert_eq!(
        Hidden::Locked.sentence(NAME),
        "Unlock Hive to see file names. Nothing shows while it's locked."
    );
}

/// DW-1201's order for a ready account, top to bottom: each state holds until the one above it
/// is gone. Every condition holds at the start; one is taken away per step.
#[test]
fn a_ready_accounts_states_follow_the_boards_order() {
    let mut facts = Facts {
        session_ended: true,
        paused: true,
        report: Report::Offline,
        storage: Some((95, 100)),
        ..ready()
    };
    let mut local = Local {
        unsent: 4,
        held_for_quota: 5,
        gave_up: 3,
        conflicts: 2,
        ..Local::default()
    };
    let step = |facts: &Facts, local: &Local| snap(&with_local(facts.clone(), local.clone()));

    let s = step(&facts, &local);
    assert_eq!((s.status, s.button), (Status::SignedOut, None), "session ended");
    facts.session_ended = false;
    let s = step(&facts, &local);
    assert_eq!(
        (s.status, s.button),
        (Status::Paused { waiting: 4 }, Some((Action::Resume, true)))
    );
    facts.paused = false;
    facts.running = false;
    let s = step(&facts, &local);
    assert_eq!((s.status, s.button), (Status::NotSyncing, None));
    facts.running = true;
    let s = step(&facts, &local);
    // Add storage waits for an engine call (FRAMES.md, "Not wired"): no button.
    assert_eq!((s.status, s.button), (Status::StorageFull { waiting: 5 }, None));
    local.held_for_quota = 0;
    let s = step(&facts, &local);
    // Try again too.
    assert_eq!((s.status, s.button), (Status::Offline, None));
    facts.report = Report::ServerError;
    assert_eq!(step(&facts, &local).status, Status::Offline, "Beebeeb did not answer");
    facts.report = Report::Idle;
    let s = step(&facts, &local);
    assert_eq!(
        (s.status, s.button),
        (Status::Stopped { files: 3 }, Some((Action::ShowProblems, true)))
    );
    local.gave_up = 0;
    let s = step(&facts, &local);
    assert_eq!(
        (s.status, s.button),
        (Status::NeedsDecision { files: 2 }, Some((Action::Review, true)))
    );
    local.conflicts = 0;
    let s = step(&facts, &local);
    assert_eq!(
        (s.status, s.button),
        (
            Status::StorageAlmostFull {
                used: size_words(95),
                plan: size_words(100)
            },
            None
        )
    );
    facts.storage = Some((89, 100));
    assert_eq!(
        step(&facts, &local).status,
        Status::UpToDate {
            files: None,
            last_change: None
        },
        "under 90% of the plan says nothing"
    );
}

/// An error on this PC that gave nothing up stops sync without a count: the title alone.
#[test]
fn an_error_on_this_pc_stops_sync_and_counts_nothing_it_does_not_know() {
    let s = snap(&Facts {
        report: Report::Failed,
        ..ready()
    });
    assert_eq!(s.status, Status::Stopped { files: 0 });
    assert_eq!(s.status.title(NAME), "Sync stopped");
    assert_eq!(s.status.line(NAME), None);
    assert_eq!(
        Status::Stopped { files: 3 }.line(NAME).as_deref(),
        Some("3 files couldn't upload. Nothing was lost.")
    );
}

/// DW-1203 from the engine's numbers: what is left and how far the batch is, each only when the
/// engine said it; no time estimate and no speed (the engine reports no rate).
#[test]
fn syncing_shows_the_engines_numbers_and_nothing_it_did_not_report() {
    let progress = Progress {
        files_remaining: Some(6),
        bytes_total: Some(4_000_000),
        bytes_done: Some(1_000_000),
    };
    let s = snap(&Facts {
        report: Report::Syncing,
        progress,
        ..ready()
    });
    assert_eq!(
        (s.status.clone(), s.button),
        (
            Status::Syncing {
                files: 6,
                left: Some(size_words(3_000_000)),
                about: None,
                fraction: Some(0.25),
                speed: None,
            },
            Some((Action::Pause, false))
        )
    );
    assert_eq!(s.status.title(NAME), "Syncing 6 files");
    assert_eq!(s.status.line(NAME), Some(format!("{} left", size_words(3_000_000))));
    // Files moving under an older "idle" report: syncing, the count from the database, no numbers.
    let s = snap(&with_local(
        ready(),
        Local {
            moving: 2,
            ..Local::default()
        },
    ));
    assert_eq!(
        s.status,
        Status::Syncing {
            files: 2,
            left: None,
            about: None,
            fraction: None,
            speed: None,
        }
    );
    assert_eq!(s.status.line(NAME), None);
    // The numbers of an "idle" report are not a sync's.
    let s = snap(&Facts { progress, ..ready() });
    assert!(matches!(s.status, Status::UpToDate { .. }), "{:?}", s.status);
}

/// The rows in the board's words: what moves, what failed, what happened, each with its tile and
/// its time; a click hands back the engine's id, and a trashed file opens Trash.
#[test]
fn rows_say_what_happened_in_the_boards_words() {
    let dot = "\u{00B7}";
    let local = Local {
        rows: vec![
            row(
                "/Plans/2027/ledger.ods",
                ago(0),
                RowKind::Uploading { bytes: 8_100_000 },
            ),
            row("/clip.mov", ago(0), RowKind::Downloading { bytes: 0 }),
            row("/Plans/notes.txt", ago(MIN), RowKind::Failed),
            row("/Plans/2027/photo.heic", ago(2 * MIN), RowKind::Uploaded),
            row("/song.m4a", ago(3 * HOUR), RowKind::Downloaded),
            Row {
                id: None,
                path: "/Plans/old.md".into(),
                at: ago(26 * HOUR),
                kind: RowKind::MovedToTrash,
            },
        ],
        ..Local::default()
    };
    let rows = match snap(&with_local(ready(), local)).changes {
        Changes::Rows { rows, complete } => {
            assert!(!complete, "six rows are not the last 50");
            rows
        }
        other => panic!("rows, not {other:?}"),
    };
    let change = |id: &str, name: &str, tile, what, when: &str, in_trash| Change {
        id: id.to_string(),
        name: name.to_string(),
        tile,
        what,
        when: when.to_string(),
        in_trash,
    };
    let done = |text: String| What::Done {
        origin: Origin::ThisPc,
        text,
    };
    assert_eq!(
        rows,
        [
            change(
                "id-/Plans/2027/ledger.ods",
                "ledger.ods",
                Tile::Sheet,
                What::Moving {
                    text: format!("Uploading {dot} {}", size_words(8_100_000)),
                    fraction: None,
                    upload: true
                },
                "now",
                false
            ),
            change(
                "id-/clip.mov",
                "clip.mov",
                Tile::Video,
                What::Moving {
                    text: "Downloading".into(),
                    fraction: None,
                    upload: false
                },
                "now",
                false
            ),
            change(
                "id-/Plans/notes.txt",
                "notes.txt",
                Tile::Document,
                What::Failed {
                    text: "Couldn't sync this file".into()
                },
                "1 min",
                false
            ),
            change(
                "id-/Plans/2027/photo.heic",
                "photo.heic",
                Tile::Image,
                done(format!("Uploaded from this PC {dot} 2027")),
                "2 min",
                false
            ),
            change(
                "id-/song.m4a",
                "song.m4a",
                Tile::Audio,
                done(format!("Downloaded to this PC {dot} Files")),
                "3 h",
                false
            ),
            change(
                "",
                "old.md",
                Tile::Trashed,
                done("Moved to Trash on this PC".into()),
                "Yesterday",
                true
            ),
        ]
    );
}

/// DW-1220: "up to 50 rows, newest first", and "That's the last 50 changes." only when the list
/// holds 50.
#[test]
fn the_list_holds_50_rows_at_most_and_ends_only_when_it_is_full() {
    let rows = |n: usize| Local {
        rows: (0..n)
            .map(|i| row(&format!("/Plans/f{i}.txt"), ago(i as i64 * MIN), RowKind::Uploaded))
            .collect(),
        ..Local::default()
    };
    for (n, shown, complete) in [(1, 1, false), (49, 49, false), (50, 50, true), (61, 50, true)] {
        match snap(&with_local(ready(), rows(n))).changes {
            Changes::Rows { rows, complete: ends } => {
                assert_eq!((rows.len(), ends), (shown, complete), "{n} rows");
                assert_eq!(rows[0].name, "f0.txt", "newest first");
            }
            other => panic!("rows, not {other:?}"),
        }
    }
    assert_eq!(
        snap(&ready()).changes,
        Changes::Empty {
            text: "Changes show here once Hive syncs this PC.".into()
        },
        "no history yet"
    );
    assert_eq!(MAX_ROWS, 50);
}

/// DW-1202's line: the newest row's time after "last change".
#[test]
fn up_to_date_says_when_the_last_change_was() {
    let local = Local {
        rows: vec![
            row("/Plans/a.txt", ago(3 * HOUR), RowKind::Uploaded),
            row("/Plans/b.txt", ago(2 * MIN), RowKind::Downloaded),
        ],
        ..Local::default()
    };
    let s = snap(&with_local(ready(), local));
    assert_eq!(
        s.status,
        Status::UpToDate {
            files: None,
            last_change: Some("2 min ago".into())
        }
    );
    assert_eq!(s.status.line(NAME).as_deref(), Some("Last change 2 min ago"));
    assert_eq!(s.button, None);
}

/// The board's relative times (DW-1201), on this PC's clock.
#[test]
fn times_read_as_the_board_writes_them() {
    let year = 365 * DAY;
    for (at, when, last) in [
        (ago(0), "now", "just now"),
        (ago(59), "now", "just now"),
        (ago(MIN), "1 min", "1 min ago"),
        (ago(44 * MIN), "44 min", "44 min ago"),
        (ago(2 * HOUR), "2 h", "2 h ago"),
        (ago(DAY), "Yesterday", "yesterday"),
        (ago(3 * DAY), "Monday", "on Monday"),
        (ago(10 * DAY), "2 Nov", "on 2 Nov"),
        (ago(year), "12 Nov 2025", "on 12 Nov 2025"),
    ] {
        assert_eq!(live::when(at, now()), when);
        assert_eq!(live::last_change(at, now()), last);
    }
}

/// A file's tile from its name; an image is a glyph, never one of the board's sample photos.
#[test]
fn a_rows_tile_follows_the_files_kind() {
    for (name, tile) in [
        ("IMG_0001.HEIC", Tile::Image),
        ("plan.png", Tile::Image),
        ("clip.MOV", Tile::Video),
        ("call.m4a", Tile::Audio),
        ("rates.xlsx", Tile::Sheet),
        ("talk.key", Tile::Deck),
        ("site.tar.gz", Tile::Archive),
        ("notes.txt", Tile::Document),
        (".hidden", Tile::Document),
        ("README", Tile::Document),
    ] {
        assert_eq!(live::tile_for(name), tile, "{name}");
    }
}
