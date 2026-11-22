//! The Office host's rules (`office_host.rs`), one test each; the mutant that must turn each red is in the lane's
//! notes. Every test but the last runs against the fake helper (`office_host_tests/fake_helper_tests.rs`) on every
//! operating system; the last starts the real confined helper on Windows (O-CP2 step 2, `--ignored`).

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{self, ErrorCode, Message};
use beebeeb_office_sandbox::SandboxError;
use zeroize::Zeroizing;

use super::office_host::*;

#[path = "office_host_tests/fake_helper_tests.rs"]
mod fake_helper_tests;
use fake_helper_tests::*;
// The rows of the two security reviews after O-CP2 (budgets, handles, wipes, the lock, a worker panic).
#[path = "office_host_tests/hardening_tests.rs"]
mod hardening_tests;
// Charts and pictures of a sheet: asked once seen, sized by the protocol.
#[path = "office_host_tests/anchor_tests.rs"]
mod anchor_tests;
// The wait for the launch lock: not the helper's time, and a close during it drops the document (review of #467).
#[path = "office_host_tests/launch_wait_tests.rs"]
mod launch_wait_tests;
// The editing session: checked answers, the save handshake.
#[path = "office_host_tests/edit_tests.rs"]
mod edit_tests;
// A helper that dies mid-edit comes back from the last snapshot; the snapshot timing.
#[path = "office_host_tests/restart_tests.rs"]
mod restart_tests;
#[path = "office_host_tests/spill_tests.rs"]
mod spill_tests;

/// 100 % on a 96 DPI screen: a Letter page is 816 x 1056.
fn scale_100() -> f32 {
    check::render_scale(1.0, 96.0)
}

fn stopped(host: &OfficeHost) -> bool {
    matches!(host.state(), OfficeState::Stopped { .. })
}

fn outline(pages: &[u32], next: Option<u32>) -> Message {
    Message::OutlineData(office::OutlineData {
        entries: pages
            .iter()
            .map(|page| OutlineEntry {
                level: 1,
                text: format!("Heading {page}"),
                page: *page,
            })
            .collect(),
        next,
    })
}

fn no_cells(range: CellRange) -> CellData {
    CellData {
        cells: Vec::new(),
        col_widths: vec![48.0; range.cols as usize],
        row_heights: vec![15.0; range.rows as usize],
        merges: Vec::new(),
        frozen: office::Frozen { rows: 0, cols: 0 },
        anchors: Vec::new(),
        missing_content: false,
    }
}

fn cells_of(events: &[Event]) -> Vec<CellRange> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Cells { range, .. } => Some(*range),
            _ => None,
        })
        .collect()
}

#[test]
fn opening_becomes_pages_on_a_checked_answer() {
    let registry = Registry::new();
    let sizes = [LETTER, LETTER, A4];
    let (mut host, _, _) = open_fake(Fake::new(move |link| serve(link, &sizes)), &registry, DocKind::Docx);
    assert_eq!(
        &host.state(),
        &OfficeState::Opening,
        "nothing is shown as open before the helper answered"
    );
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "no answer within 2 s");
    assert_eq!(
        &host.state(),
        &OfficeState::Pages {
            pages: 3,
            sizes: sizes.to_vec()
        }
    );
    let changes = events.iter().filter(|event| matches!(event, Event::State)).count();
    assert_eq!(changes, 1, "{events:?}");

    // Three pages, two sizes: refused, and the helper killed.
    let fake = Fake::new(|link| {
        while let Some((id, _)) = link.next() {
            let wrong = office::Opened {
                pages: 3,
                sizes: vec![LETTER, LETTER],
            };
            link.answer(id, Message::Opened(wrong));
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    let refused = OfficeState::Stopped {
        why: Stop::Protocol,
        can_retry: true,
    };
    assert_eq!(&host.state(), &refused);
    assert_eq!(log.kills(), 1, "the helper is killed");
}

#[test]
fn a_rendered_page_of_another_size_kills() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => opened(&[LETTER]),
                // Asked 816 x 1056: one pixel wider (within the protocol's 1 px, but not the section's length).
                _ => Message::Rendered(office::Rendered {
                    w: 817,
                    h: 1056,
                    missing_content: false,
                }),
            };
            link.answer(id, reply);
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert!(host.request_page(0, scale_100(), None).is_some());
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert!(
        matches!(
            host.state(),
            OfficeState::Stopped {
                why: Stop::Protocol,
                ..
            }
        ),
        "{:?}",
        host.state()
    );
    assert_eq!(
        lock(&log.section_sizes).last().copied(),
        Some(816 * 1056 * 4),
        "the ask was 816 x 1056"
    );
    assert_eq!(pages_of(&events), 0, "nothing of that page reached the window");
    assert_eq!(log.kills(), 1);
}

#[test]
fn an_answer_that_is_not_the_oldest_kills() {
    let registry = Registry::new();
    // Two pages asked (ids 2 and 4, after the Open's 0); the helper answers the id `pick` chooses.
    let cases: [Case; 3] = [
        ("an id never sent", |_, _| 1000),
        ("the second of two answered first", |_, second| second),
        ("an odd id", |first, _| first + 1),
    ];
    for (case, pick) in cases {
        let fake = Fake::new(move |link| {
            let mut renders = Vec::new();
            while let Some((id, message)) = link.next() {
                match message {
                    Message::Open(_) => link.answer(id, opened(&[LETTER, LETTER])),
                    Message::Render(_) => {
                        renders.push(id);
                        if let [first, second] = renders[..] {
                            link.answer(pick(first, second), rendered_letter());
                        }
                    }
                    _ => {}
                }
            }
        });
        let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
        let mut events = Vec::new();
        assert!(until_open(&mut host, &mut events), "{case}");
        assert!(host.request_page(0, scale_100(), None).is_some(), "{case}");
        assert!(host.request_page(1, scale_100(), None).is_some(), "{case}");
        assert!(
            pump(&mut host, &mut events, |host, _| stopped(host)),
            "{case}: {:?}",
            host.state()
        );
        assert!(
            matches!(
                host.state(),
                OfficeState::Stopped {
                    why: Stop::Protocol,
                    ..
                }
            ),
            "{case}: {:?}",
            host.state()
        );
        assert_eq!(pages_of(&events), 0, "{case}");
        assert_eq!(log.kills(), 1, "{case}");
    }
}

#[test]
fn a_wrong_kind_of_answer_kills() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => opened(&[LETTER]),
                // A render answered with counts.
                _ => Message::StatsData(office::StatsData { words: 7 }),
            };
            link.answer(id, reply);
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert!(host.request_page(0, scale_100(), None).is_some());
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert!(
        matches!(
            host.state(),
            OfficeState::Stopped {
                why: Stop::Protocol,
                ..
            }
        ),
        "{:?}",
        host.state()
    );
    let reached = events
        .iter()
        .any(|event| matches!(event, Event::Stats { .. } | Event::Page { .. }));
    assert!(!reached, "{events:?}");
    assert_eq!(log.kills(), 1);
}

#[test]
fn error_codes_become_their_states() {
    let registry = Registry::new();
    let cases = [
        (ErrorCode::Unsupported, OfficeState::NoPreview(NoPreview::ThisType)),
        (ErrorCode::Password, OfficeState::NoPreview(NoPreview::Password)),
        (ErrorCode::Damaged, OfficeState::CannotOpen(CannotOpen::Damaged)),
        (ErrorCode::TooLarge, OfficeState::CannotOpen(CannotOpen::TooLarge)),
        (
            ErrorCode::Internal,
            OfficeState::Stopped {
                why: Stop::Failed,
                can_retry: true,
            },
        ),
        (
            ErrorCode::RangeTooLarge,
            OfficeState::Stopped {
                why: Stop::Protocol,
                can_retry: true,
            },
        ),
    ];
    for (code, want) in cases {
        let fake = Fake::new(move |link| {
            while let Some((id, _)) = link.next() {
                link.answer(id, Message::Error(office::Error { code }));
            }
        });
        let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
        let mut events = Vec::new();
        assert!(until_open(&mut host, &mut events), "{code:?}: no state within 2 s");
        assert_eq!(&host.state(), &want, "{code:?}");
        assert!(wait_for(|| log.kills() == 1), "{code:?}: the helper is killed");
        if !matches!(want, OfficeState::Stopped { .. }) {
            assert!(!host.retry(), "{code:?}: no Try again for this state");
            assert_eq!(log.starts(), 1, "{code:?}");
        }
    }
}

#[test]
fn a_range_too_large_is_asked_again_in_halves() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => Message::OpenedSheets(office::OpenedSheets {
                    sheets: vec![SheetInfo {
                        name: "Sheet1".to_owned(),
                        rows: 1000,
                        cols: 100,
                    }],
                    more: 0,
                }),
                // Over 1,000 cells does not fit; one cell "does not fit" either (a hostile answer).
                Message::Cells(cells) if cells.range.area() > 1000 || cells.range.area() == 1 => {
                    Message::Error(office::Error {
                        code: ErrorCode::RangeTooLarge,
                    })
                }
                Message::Cells(cells) => Message::CellData(no_cells(cells.range)),
                _ => Message::Error(office::Error {
                    code: ErrorCode::Internal,
                }),
            };
            link.answer(id, reply);
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Xlsx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert!(matches!(host.state(), OfficeState::Sheets { .. }), "{:?}", host.state());
    let whole = CellRange {
        row: 0,
        col: 0,
        rows: 100,
        cols: 20,
    };
    assert!(host.request_cells(0, whole));
    assert!(
        pump(&mut host, &mut events, |_, events| cells_of(events).len() == 2),
        "{:?}",
        host.state()
    );
    let halves = vec![
        CellRange { rows: 50, ..whole },
        CellRange {
            row: 50,
            rows: 50,
            ..whole
        },
    ];
    assert_eq!(cells_of(&events), halves, "each half arrives as its own answer");
    assert!(matches!(host.state(), OfficeState::Sheets { .. }), "{:?}", host.state());
    assert_eq!(log.kills(), 0);

    // One cell answered "too large": nothing to halve, so the helper is wrong.
    let one = CellRange {
        row: 3,
        col: 3,
        rows: 1,
        cols: 1,
    };
    assert!(host.request_cells(0, one));
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert!(
        matches!(
            host.state(),
            OfficeState::Stopped {
                why: Stop::Protocol,
                ..
            }
        ),
        "{:?}",
        host.state()
    );
    assert_eq!(log.kills(), 1);
}

#[test]
fn the_outline_is_asked_until_next_is_none() {
    let registry = Registry::new();
    let froms = Arc::new(Mutex::new(Vec::new()));
    let seen = froms.clone();
    let fake = Fake::new(move |link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => opened(&[LETTER; 5]),
                Message::Outline(asked) => {
                    lock(&seen).push(asked.from);
                    if asked.from == 0 {
                        outline(&[0, 1, 2], Some(3))
                    } else {
                        outline(&[3, 4], None)
                    }
                }
                _ => Message::Error(office::Error {
                    code: ErrorCode::Internal,
                }),
            };
            link.answer(id, reply);
        }
    });
    let (mut host, _, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert!(host.request_outline());
    let outlines = |events: &[Event]| -> Vec<Vec<u32>> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Outline(entries) => Some(entries.iter().map(|entry| entry.page).collect()),
                _ => None,
            })
            .collect()
    };
    assert!(
        pump(&mut host, &mut events, |_, events| !outlines(events).is_empty()),
        "{:?}",
        host.state()
    );
    // Give a second (wrong) delivery the time to show.
    pump_for(&mut host, &mut events, Duration::from_millis(100), |_, _| false);
    assert_eq!(
        outlines(&events),
        vec![vec![0, 1, 2, 3, 4]],
        "one event with every entry"
    );
    assert_eq!(*lock(&froms), vec![0, 3], "asked from 0, then from each `next`");
}

#[test]
fn an_outline_that_does_not_advance_kills() {
    let registry = Registry::new();
    let cases = [
        ("next == from", outline(&[0], Some(0))),
        ("next with no entries", outline(&[], Some(0))),
    ];
    for (case, page) in cases {
        let fake = Fake::new(move |link| {
            while let Some((id, message)) = link.next() {
                let reply = match message {
                    Message::Open(_) => opened(&[LETTER; 2]),
                    // The same page every time: a host that follows it asks for ever.
                    _ => page.clone(),
                };
                link.answer(id, reply);
            }
        });
        let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
        let mut events = Vec::new();
        assert!(until_open(&mut host, &mut events), "{case}");
        assert!(host.request_outline(), "{case}");
        // The 2 s bound turns an endless asking into a red, never a hang.
        assert!(
            pump(&mut host, &mut events, |host, _| stopped(host)),
            "{case}: {:?}",
            host.state()
        );
        assert!(
            matches!(
                host.state(),
                OfficeState::Stopped {
                    why: Stop::Protocol,
                    ..
                }
            ),
            "{case}: {:?}",
            host.state()
        );
        assert!(!events.iter().any(|event| matches!(event, Event::Outline(_))), "{case}");
        assert_eq!(log.kills(), 1, "{case}");
    }
}

#[test]
fn the_open_timer_ends_a_silent_helper() {
    let registry = Registry::new();
    let started = Instant::now();
    let (mut host, log, _) = open_fake(
        Fake::new(|link| while link.next().is_some() {}),
        &registry,
        DocKind::Docx,
    );
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    let timed_out = OfficeState::Stopped {
        why: Stop::TimedOut,
        can_retry: true,
    };
    assert_eq!(&host.state(), &timed_out);
    assert!(
        started.elapsed() >= Duration::from_millis(290),
        "not before the 300 ms timer: {:?}",
        started.elapsed()
    );
    assert_eq!(log.kills(), 1);
}

#[test]
fn the_request_timer_runs_on_the_oldest_only() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        let mut renders = Vec::new();
        while let Some((id, message)) = link.next() {
            match message {
                Message::Open(_) => link.answer(id, opened(&[LETTER; 3])),
                Message::Render(_) if renders.len() < 2 => {
                    renders.push(id);
                    if renders.len() == 2 {
                        // One at a time, 200 ms each: the second waits 400 ms since its send, 200 ms as the oldest.
                        for answered in renders.clone() {
                            thread::sleep(Duration::from_millis(200));
                            link.answer(answered, rendered_letter());
                        }
                    }
                }
                // The third is never answered.
                _ => {}
            }
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert!(host.request_page(0, scale_100(), None).is_some());
    assert!(host.request_page(1, scale_100(), None).is_some());
    assert!(
        pump(&mut host, &mut events, |_, events| pages_of(events) == 2),
        "{:?}",
        host.state()
    );
    assert!(
        matches!(host.state(), OfficeState::Pages { .. }),
        "no kill: {:?}",
        host.state()
    );
    assert_eq!(log.kills(), 0);

    assert!(host.request_page(2, scale_100(), None).is_some());
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert!(
        matches!(
            host.state(),
            OfficeState::Stopped {
                why: Stop::TimedOut,
                ..
            }
        ),
        "{:?}",
        host.state()
    );
    assert_eq!(log.kills(), 1);
}

#[test]
fn the_window_never_waits_on_the_helper() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        let sizes = [LETTER; 2];
        while let Some((id, message)) = link.next() {
            if !matches!(message, Message::Open(_)) {
                thread::sleep(Duration::from_secs(2));
            }
            if let Some(reply) = well(link, message, &sizes) {
                link.answer(id, reply);
            }
        }
    });
    let fast = Duration::from_millis(50);
    let at = Instant::now();
    let (mut host, _, _) = open_fake(fake, &registry, DocKind::Docx);
    assert!(at.elapsed() < fast, "open: {:?}", at.elapsed());
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));

    let at = Instant::now();
    let id = host.request_page(0, scale_100(), None).expect("page 1 can be asked");
    assert!(at.elapsed() < fast, "request_page: {:?}", at.elapsed());
    let at = Instant::now();
    let _ = host.poll();
    assert!(at.elapsed() < fast, "poll: {:?}", at.elapsed());
    let at = Instant::now();
    host.cancel(id);
    assert!(at.elapsed() < fast, "cancel: {:?}", at.elapsed());
    let at = Instant::now();
    host.close(CloseWhy::Closed);
    assert!(at.elapsed() < fast, "close: {:?}", at.elapsed());
}

#[test]
fn a_cancelled_page_is_discarded() {
    let registry = Registry::new();
    // A page asked and cancelled: the helper had started it, so it answers it anyway.
    let answered = Arc::new(AtomicBool::new(false));
    let flag = answered.clone();
    let fake = Fake::new(move |link| {
        while let Some((id, message)) = link.next() {
            match message {
                Message::Open(_) => link.answer(id, opened(&[LETTER; 8])),
                Message::Render(render) => {
                    // The Cancel comes next; then the page is answered in its turn.
                    let _cancel = link.next();
                    link.paint(render.out, 0x80);
                    link.answer(id, rendered_letter());
                    flag.store(true, SeqCst);
                }
                _ => {}
            }
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    let id = host.request_page(0, scale_100(), None).expect("asked");
    assert!(wait_for(|| log.renders_sent() == 1), "the page was asked");
    host.cancel(id);
    assert!(
        wait_for(|| answered.load(SeqCst)),
        "the helper answered the cancelled page"
    );
    assert!(
        wait_for(|| log.live() == 1),
        "its section is dropped; only the document's is left"
    );
    pump_for(&mut host, &mut events, Duration::from_millis(100), |_, _| false);
    assert_eq!(pages_of(&events), 0, "a cancelled page never reaches the window");
    assert!(
        matches!(host.state(), OfficeState::Pages { .. }),
        "no kill: {:?}",
        host.state()
    );
    assert_eq!(log.kills(), 0);
    // Open is id 0, the page id 2: the Cancel names the page.
    assert!(
        lock(&log.sent).contains(&(office::KIND_CANCEL, 2)),
        "{:?}",
        lock(&log.sent)
    );

    // A page still waiting in the host (six are out) is never asked once cancelled.
    let gate = Gate::default();
    let opens = gate.clone();
    let fake = Fake::new(move |link| {
        let sizes = [LETTER; 8];
        while let Some((id, message)) = link.next() {
            if !matches!(message, Message::Open(_)) && !opens.wait() {
                return;
            }
            if let Some(reply) = well(link, message, &sizes) {
                link.answer(id, reply);
            }
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    let ids: Vec<RequestId> = (0..7)
        .map(|page| host.request_page(page, scale_100(), None).expect("asked"))
        .collect();
    host.cancel(ids[6]);
    gate.open();
    assert!(
        pump(&mut host, &mut events, |_, events| pages_of(events) == 6),
        "{:?}",
        host.state()
    );
    pump_for(&mut host, &mut events, Duration::from_millis(150), |_, _| false);
    assert_eq!(pages_of(&events), 6);
    assert_eq!(log.renders_sent(), 6, "no Render frame for the cancelled page");
    assert!(!lock(&log.rendered_pages).contains(&6));
}

#[test]
fn close_kills_and_wipes() {
    let registry = Registry::new();
    let sizes = [LETTER];
    let (mut host, log, wakes) = open_fake(Fake::new(move |link| serve(link, &sizes)), &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    let before = wakes.load(SeqCst);
    assert!(host.request_page(0, scale_100(), None).is_some());
    // The page is queued for the window (the worker woke it), and nobody polls.
    assert!(wait_for(|| wakes.load(SeqCst) > before), "the page arrived");
    host.close(CloseWhy::Closed);
    assert_eq!(&host.state(), &OfficeState::Closed(CloseWhy::Closed));
    assert_eq!(log.kills(), 1, "close itself killed the helper");
    assert!(wait_for(|| log.live() == 0), "every section dropped within 2 s");
    assert_eq!(host.wiped_bytes(), 816 * 1056 * 4, "the page nobody took was wiped");
    assert!(host.poll().is_empty(), "nothing is handed out after a close");
    assert_eq!(host.helper_pid(), None);
}

#[test]
fn close_while_starting_kills_the_late_helper() {
    let registry = Registry::new();
    let sizes = [LETTER];
    let fake = Fake::new(move |link| serve(link, &sizes)).slow_start(Duration::from_millis(200));
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    thread::sleep(Duration::from_millis(10));
    host.close(CloseWhy::Closed);
    assert_eq!(log.starts(), 0, "the start had not returned yet");
    assert!(
        wait_for(|| log.kills() == 1),
        "the helper whose start returned after the close is killed"
    );
    assert_eq!(log.starts(), 1);
    assert!(wait_for(|| host.worker_ended()), "the worker ends");
    assert!(host.poll().is_empty());
    assert_eq!(&host.state(), &OfficeState::Closed(CloseWhy::Closed));
    let opens = lock(&log.sent)
        .iter()
        .filter(|(kind, _)| *kind == office::KIND_OPEN)
        .count();
    assert_eq!(opens, 0, "nothing was sent to it");
}

#[test]
fn kill_all_ends_every_helper_of_its_registry() {
    let registry = Registry::new();
    let sizes = [LETTER];
    let silent = |link: &mut FakeLink| while link.next().is_some() {};
    let (mut open, open_log, _) = open_fake(Fake::new(move |link| serve(link, &sizes)), &registry, DocKind::Docx);
    let (mut opening, opening_log, _) = open_fake(Fake::new(silent), &registry, DocKind::Docx);
    let late = Fake::new(silent).slow_start(Duration::from_millis(300));
    let (mut starting, starting_log, _) = open_fake(late, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut open, &mut events));
    assert!(wait_for(|| opening_log.starts() == 1), "the second helper started");
    assert_eq!(starting_log.starts(), 0, "the third is still starting");

    kill_all_in(&registry, CloseWhy::Locked);

    let hosts = [
        ("open", &mut open, &open_log),
        ("opening", &mut opening, &opening_log),
        ("starting", &mut starting, &starting_log),
    ];
    for (name, host, log) in hosts {
        let mut events = Vec::new();
        let locked = |host: &OfficeHost, _: &[Event]| host.state() == OfficeState::Closed(CloseWhy::Locked);
        assert!(pump(host, &mut events, locked), "{name}: {:?}", host.state());
        assert!(wait_for(|| log.kills() == 1), "{name}: its helper is killed");
    }
}

#[test]
fn drop_kills_the_helper() {
    let registry = Registry::new();
    let sizes = [LETTER];
    let (mut host, log, _) = open_fake(Fake::new(move |link| serve(link, &sizes)), &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert_eq!(log.kills(), 0);
    drop(host);
    assert!(wait_for(|| log.kills() == 1), "dropping the host kills its helper");
    assert!(wait_for(|| log.live() == 0), "and drops every section");
}

#[test]
fn try_again_starts_one_new_helper_once() {
    let registry = Registry::new();
    // Each helper reads the Open and exits.
    let fake = Fake::new(|link| {
        let _ = link.next();
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    let first = OfficeState::Stopped {
        why: Stop::Exited,
        can_retry: true,
    };
    assert_eq!(&host.state(), &first);
    assert!(host.retry(), "Try again starts a new helper");
    assert_eq!(&host.state(), &OfficeState::Opening);
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    let second = OfficeState::Stopped {
        why: Stop::Exited,
        can_retry: false,
    };
    assert_eq!(&host.state(), &second);
    assert!(!host.retry(), "once per open");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(log.starts(), 2, "the starter was called exactly twice");
    assert!(
        wait_for(|| log.live() == 0),
        "without a Try again left, the document's section goes"
    );
}

#[test]
fn sections_are_sized_by_the_protocol() {
    let registry = Registry::new();
    let sizes = [LETTER, A4];
    let (mut host, log, _) = open_fake(Fake::new(move |link| serve(link, &sizes)), &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    let tile = Rect {
        x: 0,
        y: 0,
        w: 512,
        h: 512,
    };
    // Letter at 100 % on 96 DPI: 816 x 1056 (in f64, 817 x 1057). A4 at scale 1.0: 596 x 842 (rounded: 595).
    assert!(host.request_page(0, scale_100(), None).is_some());
    assert!(host.request_page(1, 1.0, None).is_some());
    assert!(
        host.request_page(0, check::render_scale(2.0, 96.0), Some(tile))
            .is_some()
    );
    assert!(
        pump(&mut host, &mut events, |_, events| pages_of(events) == 3),
        "{:?}",
        host.state()
    );
    let made = lock(&log.section_sizes).clone();
    assert_eq!(made[0], doc().len(), "the document's section first");
    assert_eq!(made[1..], [816 * 1056 * 4, 596 * 842 * 4, 512 * 512 * 4]);
    let shown: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|event| match event {
            Event::Page { w, h, .. } => Some((*w, *h)),
            _ => None,
        })
        .collect();
    assert_eq!(shown, vec![(816, 1056), (596, 842), (512, 512)]);
}

#[test]
fn at_most_six_renders_are_out() {
    let registry = Registry::new();
    let gate = Gate::default();
    let opens = gate.clone();
    let fake = Fake::new(move |link| {
        let sizes = [LETTER; 10];
        while let Some((id, message)) = link.next() {
            if !matches!(message, Message::Open(_)) && !opens.wait() {
                return;
            }
            if let Some(reply) = well(link, message, &sizes) {
                link.answer(id, reply);
            }
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    for page in 0..10 {
        assert!(host.request_page(page, scale_100(), None).is_some(), "page {page}");
    }
    assert!(wait_for(|| log.renders_sent() == 6), "six asked");
    thread::sleep(Duration::from_millis(150));
    assert_eq!(log.renders_sent(), 6, "six out, four waiting in the host");
    gate.open();
    assert!(
        pump(&mut host, &mut events, |_, events| pages_of(events) == 10),
        "{:?}",
        host.state()
    );
    assert_eq!(
        *lock(&log.rendered_pages),
        (0..10).collect::<Vec<u32>>(),
        "the rest went in order"
    );
}

#[test]
fn a_page_over_the_engine_side_is_not_asked() {
    let registry = Registry::new();
    let over = PageSize {
        w_pt: 16_001.0,
        h_pt: 600.0,
    };
    let at_the_side = PageSize {
        w_pt: 16_000.0,
        h_pt: 600.0,
    };
    let sizes = [over, at_the_side];
    let (mut host, log, _) = open_fake(Fake::new(move |link| serve(link, &sizes)), &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    // A 512 px tile is within the protocol's 4,096 px; the page it is cut from is not within the engine's side.
    let tile = Some(Rect {
        x: 0,
        y: 0,
        w: 512,
        h: 512,
    });
    assert_eq!(
        host.request_page(0, 1.0, tile),
        None,
        "a tile of a 16,001 px page is not asked"
    );
    assert!(
        host.request_page(1, 1.0, tile).is_some(),
        "16,000 px is the engine's side: asked"
    );
    assert!(
        pump(&mut host, &mut events, |_, events| pages_of(events) == 1),
        "{:?}",
        host.state()
    );
    assert_eq!(log.renders_sent(), 1, "no frame for the refused page");
    assert_eq!(*lock(&log.rendered_pages), vec![1]);
}

#[test]
fn over_100_mib_or_empty_starts_nothing() {
    let registry = Registry::new();
    let big = Fake::new(|link| serve(link, &[LETTER]));
    let big_log = big.log.clone();
    let too_large = Zeroizing::new(vec![0u8; check::MAX_INPUT_BYTES as usize + 1]);
    let host = open_with(big, &registry, TEST_TIMEOUTS, DocKind::Docx, too_large, noop_wake());
    assert_eq!(&host.state(), &OfficeState::CannotOpen(CannotOpen::TooLarge));

    let empty = Fake::new(|link| serve(link, &[LETTER]));
    let empty_log = empty.log.clone();
    let nothing = open_with(
        empty,
        &registry,
        TEST_TIMEOUTS,
        DocKind::Docx,
        Zeroizing::new(Vec::new()),
        noop_wake(),
    );
    assert_eq!(&nothing.state(), &OfficeState::CannotOpen(CannotOpen::Damaged));

    thread::sleep(Duration::from_millis(100));
    for (case, log) in [("over 100 MiB", &big_log), ("empty", &empty_log)] {
        assert_eq!(log.starts(), 0, "{case}: no helper");
        assert!(lock(&log.calls).is_empty(), "{case}: the starter was never called");
        assert!(lock(&log.section_sizes).is_empty(), "{case}: no section");
    }
    assert!(host.worker_ended() && nothing.worker_ended(), "no worker ran");
}

#[test]
fn a_missing_helper_says_not_installed() {
    let missing = std::env::temp_dir().join("beebeeb-office-render-that-is-not-there");
    assert!(!missing.exists());
    let mut host = OfficeHost::open(&missing, DocKind::Docx, Zeroizing::new(doc()), noop_wake());
    let mut events = Vec::new();
    let ended = |host: &OfficeHost, _: &[Event]| host.worker_ended() && !matches!(host.state(), OfficeState::Opening);
    assert!(pump(&mut host, &mut events, ended), "{:?}", host.state());
    assert_eq!(&host.state(), &OfficeState::NoPreview(NoPreview::NotInstalled));
    assert_eq!(host.helper_pid(), None, "no helper");
}

/// F4: a Windows older than the sandbox needs says so, not "Renderer stopped" (red first: it mapped to `Stopped`).
#[test]
fn a_too_old_windows_needs_a_windows_update() {
    let registry = Registry::new();
    let fake = Fake::new(|link| serve(link, &[LETTER])).failing_start(SandboxError::NeedsNewerWindows);
    let mut host = open_with(
        fake,
        &registry,
        TEST_TIMEOUTS,
        DocKind::Docx,
        Zeroizing::new(doc()),
        noop_wake(),
    );
    let mut events = Vec::new();
    let ended = |host: &OfficeHost, _: &[Event]| host.worker_ended() && !matches!(host.state(), OfficeState::Opening);
    assert!(pump(&mut host, &mut events, ended), "{:?}", host.state());
    assert_eq!(&host.state(), &OfficeState::NoPreview(NoPreview::NeedsWindowsUpdate));
    assert_eq!(host.helper_pid(), None);
}

#[cfg(not(windows))]
#[test]
fn no_sandbox_no_helper() {
    // An existing, runnable file of the helper's name: were there an unconfined way, this would start it.
    let exe = runnable_helper("no-sandbox");
    let registry = Registry::new();
    let mut host = OfficeHost::open_in(&registry, &exe, DocKind::Docx, Zeroizing::new(doc()), noop_wake());
    let mut events = Vec::new();
    let ended = |host: &OfficeHost, _: &[Event]| host.worker_ended() && !matches!(host.state(), OfficeState::Opening);
    assert!(pump(&mut host, &mut events, ended), "{:?}", host.state());
    assert_eq!(&host.state(), &OfficeState::NoPreview(NoPreview::NoSandbox));
    assert_eq!(host.helper_pid(), None);
    remove_runnable_helper(&exe);
}

/// The lead's answer 4 (2026-11-15 19:03): Windows Error Reporting is told to keep no dump of the helper BEFORE the
/// first helper starts.
#[test]
fn crash_reports_are_off_before_the_first_helper_starts() {
    let registry = Registry::new();
    let sizes = [LETTER];
    let (mut host, log, _) = open_fake(Fake::new(move |link| serve(link, &sizes)), &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert_eq!(*lock(&log.calls), vec!["exclude", "start"]);
}

/// O-CP2 step 2: the real helper, confined, on Windows. Asked to run and unable to (no helper, no fixtures, a refused
/// system call) is a red with the reason, never a pass.
#[cfg(windows)]
#[test]
#[ignore = "O-CP2: real confined helper"]
fn a_real_confined_helper_renders_page_one() {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::{BOOL, CloseHandle, GetLastError, HANDLE, STILL_ACTIVE};
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TokenIsAppContainer};
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        TerminateProcess,
    };

    let helper = std::env::var_os("BB_OFFICE_HELPER")
        .map(PathBuf::from)
        .expect("BB_OFFICE_HELPER is not set: this measurement was asked to run and could not");
    let fixtures = std::env::var_os("BB_OFFICE_FIXTURES")
        .map(PathBuf::from)
        .expect("BB_OFFICE_FIXTURES is not set: this measurement was asked to run and could not");
    let read = |name: &str| {
        let bytes = std::fs::read(fixtures.join(name));
        Zeroizing::new(bytes.unwrap_or_else(|error| panic!("the fixture {name} could not be read ({})", error.kind())))
    };
    let real = Duration::from_secs(30);
    let page_of = |events: &[Event]| -> Option<(RequestId, u32, u32, usize)> {
        events.iter().find_map(|event| match event {
            Event::Page { id, w, h, rgba, .. } => {
                let colours: HashSet<[u8; 4]> = rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
                Some((*id, *w, *h, colours.len()))
            }
            _ => None,
        })
    };

    // letter-1.docx, page 1 at 100 % on 96 DPI: 816 x 1056 on the real path (row 10), and drawn.
    let mut host = OfficeHost::open(&helper, DocKind::Docx, read("letter-1.docx"), noop_wake());
    let mut events = Vec::new();
    assert!(pump_for(&mut host, &mut events, real, |host, _| !matches!(
        host.state(),
        OfficeState::Opening
    )));
    assert!(
        matches!(host.state(), OfficeState::Pages { .. }),
        "letter-1.docx: {:?}",
        host.state()
    );
    let first = host.request_page(0, scale_100(), None).expect("page 1 can be asked");
    assert!(
        pump_for(&mut host, &mut events, real, |_, events| pages_of(events) == 1),
        "{:?}",
        host.state()
    );
    let (id, w, h, colours) = page_of(&events).expect("page 1 arrived");
    assert_eq!((id, w, h), (first, 816, 1056));
    assert!(colours >= 10, "page 1 is drawn, not flat: {colours} colours");
    // The handle bound reads the real helper's count (a `None` here leaves a helper that keeps its sections unbounded).
    let handles = host.helper_handle_count();
    assert!(
        handles.is_some_and(|count| count > 0),
        "the host reads the helper's open handles: {handles:?}"
    );

    // spike.docx, page 1 at 150 %.
    let mut spike = OfficeHost::open(&helper, DocKind::Docx, read("spike.docx"), noop_wake());
    let mut spike_events = Vec::new();
    assert!(pump_for(&mut spike, &mut spike_events, real, |host, _| !matches!(
        host.state(),
        OfficeState::Opening
    )));
    let OfficeState::Pages { sizes, .. } = spike.state().clone() else {
        panic!("spike.docx: {:?}", spike.state());
    };
    let scale = check::render_scale(1.5, 96.0);
    let (want_w, want_h) = check::page_pixels(&sizes[0], scale).expect("page 1 has a size");
    spike
        .request_page(0, scale, None)
        .expect("page 1 of spike.docx can be asked");
    assert!(
        pump_for(&mut spike, &mut spike_events, real, |_, events| pages_of(events) == 1),
        "{:?}",
        spike.state()
    );
    let (_, w, h, _) = page_of(&spike_events).expect("spike.docx page 1 arrived");
    assert_eq!((w, h), (want_w, want_h));
    drop(spike);

    // The helper's process: alive while the document is open, an AppContainer token, inside a job. Then our own
    // kill, from outside the host.
    let pid = host
        .helper_pid()
        .expect("while the document is open, its helper has a process id");
    // SAFETY: Win32 calls on handles this test opens and closes itself; every out pointer is a local of its type.
    unsafe {
        let process: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE, 0, pid);
        assert!(
            !process.is_null(),
            "OpenProcess on the helper failed with system code {}",
            GetLastError()
        );
        let mut code: u32 = 0;
        let read = GetExitCodeProcess(process, &mut code);
        assert!(
            read != 0,
            "GetExitCodeProcess failed with system code {}",
            GetLastError()
        );
        assert_eq!(
            code, STILL_ACTIVE as u32,
            "the helper process runs while the document is open"
        );
        let mut token: HANDLE = std::ptr::null_mut();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        assert!(
            opened != 0,
            "OpenProcessToken failed with system code {}",
            GetLastError()
        );
        let mut is_container: u32 = 0;
        let mut returned: u32 = 0;
        let asked = GetTokenInformation(
            token,
            TokenIsAppContainer,
            (&mut is_container as *mut u32).cast(),
            4,
            &mut returned,
        );
        assert!(
            asked != 0,
            "GetTokenInformation(TokenIsAppContainer) failed with system code {}",
            GetLastError()
        );
        assert_eq!(is_container, 1, "the helper's token is an AppContainer token");
        let mut in_job: BOOL = 0;
        let checked = IsProcessInJob(process, std::ptr::null_mut(), &mut in_job);
        assert!(
            checked != 0,
            "IsProcessInJob failed with system code {}",
            GetLastError()
        );
        assert!(in_job != 0, "the helper runs inside a job");
        CloseHandle(token);
        let ended = TerminateProcess(process, 1);
        assert!(
            ended != 0,
            "TerminateProcess failed with system code {}",
            GetLastError()
        );
        CloseHandle(process);
    }
    let exited = OfficeState::Stopped {
        why: Stop::Exited,
        can_retry: true,
    };
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert_eq!(&host.state(), &exited);
    assert!(host.retry(), "Try again");
    assert!(pump_for(&mut host, &mut events, real, |host, _| !matches!(
        host.state(),
        OfficeState::Opening
    )));
    assert!(
        matches!(host.state(), OfficeState::Pages { .. }),
        "after Try again: {:?}",
        host.state()
    );
}

/// A named rule from two sizes to one.
type Case = (&'static str, fn(u32, u32) -> u32);
