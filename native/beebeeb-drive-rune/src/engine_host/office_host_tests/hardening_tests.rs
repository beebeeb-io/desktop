//! The host after two security reviews: the crash-report exclusion per start, a lock that wipes at once, frames
//! nobody handles wiped, a worker panic said, budgets on the operations that make many requests, the handle bound and
//! the raw page copy. One test each, against the fake helper; the mutant that must turn each red is in the lane's
//! notes.

use std::cell::Cell;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{self, ErrorCode, Message};
use beebeeb_office_sandbox::SandboxError;

use super::fake_helper_tests::*;
use super::{cells_of, no_cells, scale_100, stopped};
use crate::engine_host::office_host::*;

/// The length of `message`'s encoded payload: what a frame of it carries.
fn payload_len(message: &Message) -> usize {
    message.encode().expect("a test message encodes").len()
}

/// One outline entry with text, as a page of the outline from `from`, and the next page asked.
fn outline_from(from: u32) -> Message {
    Message::OutlineData(office::OutlineData {
        entries: vec![OutlineEntry {
            level: 1,
            text: format!("Heading {from} of a confidential plan"),
            page: 0,
        }],
        next: Some(from + 1),
    })
}

fn sheets(sheets: &[(&str, u32, u32)], more: u32) -> Message {
    Message::OpenedSheets(office::OpenedSheets {
        sheets: sheets
            .iter()
            .map(|(name, rows, cols)| SheetInfo {
                name: (*name).to_owned(),
                rows: *rows,
                cols: *cols,
            })
            .collect(),
        more,
    })
}

fn range(row: u32, col: u32, rows: u32, cols: u32) -> CellRange {
    CellRange { row, col, rows, cols }
}

fn stopped_for(host: &OfficeHost, why: Stop) -> bool {
    matches!(host.state(), OfficeState::Stopped { why: got, .. } if got == why)
}

/// The wait of the test below: it starts the test binary as a helper beside a full suite, which the 2 s of `WITHIN`
/// is too short for. A pass returns at once; only a real hang waits this long.
const LOADED: std::time::Duration = std::time::Duration::from_secs(60);

#[test]
fn the_exclusion_names_the_started_file_and_is_written_every_start() {
    let registry = Registry::new();
    // Each helper reads the Open and exits: Try again starts a second one.
    let fake = Fake::new(|link| {
        let _ = link.next();
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(
        pump_for(&mut host, &mut events, LOADED, |host, _| !matches!(
            host.state(),
            OfficeState::Opening
        )),
        "{:?}",
        host.state()
    );
    assert!(host.retry(), "Try again: {:?}", host.state());
    let second = OfficeState::Stopped {
        why: Stop::Exited,
        can_retry: false,
    };
    assert!(
        pump_for(&mut host, &mut events, LOADED, |host, _| host.state() == second),
        "{:?}",
        host.state()
    );
    assert_eq!(
        *lock(&log.calls),
        vec!["exclude", "start", "exclude", "start"],
        "written before every start, not once"
    );
    assert_eq!(
        *lock(&log.excluded),
        vec![HELPER_EXE.to_owned(); 2],
        "the started file's name"
    );

    // A helper file of another name: nothing is excluded, made or started.
    let other = Fake::new(|link| serve(link, &[LETTER])).named("other.exe");
    let (mut host, log, _) = open_fake(other, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(
        pump_for(&mut host, &mut events, LOADED, |host, _| !matches!(
            host.state(),
            OfficeState::Opening
        )),
        "{:?}",
        host.state()
    );
    assert_eq!(host.state(), OfficeState::NoPreview(NoPreview::NotInstalled));
    assert_eq!(log.starts(), 0);
    assert!(lock(&log.calls).is_empty(), "{:?}", lock(&log.calls));
    assert!(lock(&log.section_sizes).is_empty(), "no section for it");

    // The production starter writes on every start too (off Windows the write itself is empty, and the sandbox then
    // says there is none).
    #[cfg(not(windows))]
    {
        let exe = runnable_helper("exclusion");
        let before = crash_report_writes();
        for start in 0..2 {
            let bytes = zeroize::Zeroizing::new(doc());
            let mut host = OfficeHost::open(&exe, DocKind::Docx, bytes, noop_wake());
            let mut events = Vec::new();
            assert!(
                pump_for(&mut host, &mut events, LOADED, |host, _| host.worker_ended()),
                "start {start}: {:?}",
                host.state()
            );
            assert_eq!(
                host.state(),
                OfficeState::NoPreview(NoPreview::NoSandbox),
                "start {start}"
            );
        }
        // Other tests of this process write as well: at least these two, where once per process gives at most one.
        let writes = crash_report_writes() - before;
        assert!(writes >= 2, "{writes} writes for two starts");
        remove_runnable_helper(&exe);
    }
}

#[test]
fn a_lock_wipes_the_sheet_names_at_once() {
    let registry = Registry::new();
    let names = [("Budget", 100, 10), ("Q3 forecast", 100, 10), ("Salaries", 100, 10)];
    let fake = Fake::new(move |link| {
        while let Some((id, message)) = link.next() {
            if let Message::Open(_) = message {
                link.answer(id, sheets(&names, 0));
            }
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Xlsx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(
        matches!(host.state(), OfficeState::Sheets { ref sheets, .. } if sheets.len() == 3),
        "{:?}",
        host.state()
    );
    let before = host.wiped_bytes();

    kill_all_in(&registry, CloseWhy::Locked);

    // Before any poll: closed, and the names wiped by the lock itself.
    assert_eq!(host.state(), OfficeState::Closed(CloseWhy::Locked));
    let names_len: usize = names.iter().map(|(name, _, _)| name.len()).sum();
    assert_eq!(host.wiped_bytes() - before, names_len, "the three names");
    assert_eq!(log.kills(), 1);
    // The window hears of it once.
    let told = host.poll();
    assert_eq!(
        told.iter().filter(|event| matches!(event, Event::State)).count(),
        1,
        "{told:?}"
    );
    assert!(host.poll().is_empty());
}

#[test]
fn unhandled_frames_are_wiped() {
    let registry = Registry::new();
    let late = outline_from(7);
    let late_len = payload_len(&late);

    // After a stop, then from an earlier helper. The first helper answers a page with the wrong kind (stopped), and
    // its output stays open after the kill: it writes one frame after the stop, one after Try again started another.
    let starts = Arc::new(AtomicUsize::new(0));
    let (after_stop, after_retry) = (Gate::default(), Gate::default());
    let (stop_gate, retry_gate, frame) = (after_stop.clone(), after_retry.clone(), late.clone());
    let fake = Fake::new(move |link| {
        if starts.fetch_add(1, SeqCst) > 0 {
            return serve(link, &[LETTER]);
        }
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => opened(&[LETTER]),
                _ => Message::StatsData(office::StatsData { words: 1 }),
            };
            link.answer(id, reply);
        }
        if stop_gate.wait() {
            link.answer(1_000, frame.clone());
        }
        if retry_gate.wait() {
            link.answer(1_002, frame.clone());
        }
    })
    .lingering();
    let (mut host, _, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(host.request_page(0, scale_100(), None).is_some());
    assert!(
        pump(&mut host, &mut events, |host, _| stopped_for(host, Stop::Protocol)),
        "{:?}",
        host.state()
    );
    after_stop.open();
    assert!(
        wait_for(|| host.wiped_frame_bytes() == late_len),
        "after a stop: {} of {late_len} bytes",
        host.wiped_frame_bytes()
    );
    assert!(host.retry());
    assert!(
        pump(&mut host, &mut events, |host, _| matches!(
            host.state(),
            OfficeState::Pages { .. }
        )),
        "{:?}",
        host.state()
    );
    after_retry.open();
    assert!(
        wait_for(|| host.wiped_frame_bytes() == 2 * late_len),
        "an earlier helper's: {} of {} bytes",
        host.wiped_frame_bytes(),
        2 * late_len
    );

    // Queued at a close. The worker waits in its second section (a page's) while the helper writes two frames
    // nobody asked for; then the window closes, and the worker goes without handling them.
    let section = Gate::default();
    let write = Gate::default();
    let written = Arc::new(AtomicBool::new(false));
    let (writes, flag, frame) = (write.clone(), written.clone(), late.clone());
    let fake = Fake::new(move |link| {
        let Some((id, _open)) = link.next() else {
            return;
        };
        link.answer(id, opened(&[LETTER]));
        if writes.wait() {
            link.answer(2_000, frame.clone());
            link.answer(2_002, frame.clone());
            flag.store(true, SeqCst);
        }
        while link.next().is_some() {}
    })
    .block_section(2, section.clone());
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(host.request_page(0, scale_100(), None).is_some());
    assert!(
        wait_for(|| lock(&log.section_sizes).len() == 2),
        "the worker is in the page's section"
    );
    write.open();
    assert!(wait_for(|| written.load(SeqCst)), "the helper wrote both frames");
    // The reader hands each frame on as soon as it is read.
    thread::sleep(Duration::from_millis(150));
    host.close(CloseWhy::Closed);
    section.open();
    assert!(wait_for(|| host.worker_ended()), "the worker ends");
    // A reader that had not handed a frame on yet wipes it itself, after the worker ended: counted all the same.
    assert!(
        wait_for(|| host.wiped_frame_bytes() == 2 * late_len),
        "both queued frames: {} of {} bytes",
        host.wiped_frame_bytes(),
        2 * late_len
    );

    // The reader's, once the worker is gone: the helper's output outlives the close, and it writes one more frame.
    let write = Gate::default();
    let (writes, frame) = (write.clone(), late.clone());
    let fake = Fake::new(move |link| {
        let Some((id, _open)) = link.next() else {
            return;
        };
        link.answer(id, opened(&[LETTER]));
        while link.next().is_some() {}
        if writes.wait() {
            link.answer(3_000, frame.clone());
        }
    })
    .lingering();
    let (mut host, _, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    host.close(CloseWhy::Closed);
    assert!(wait_for(|| host.worker_ended()), "the worker ends");
    write.open();
    assert!(
        wait_for(|| host.wiped_frame_bytes() == late_len),
        "the reader's: {} of {late_len} bytes",
        host.wiped_frame_bytes()
    );
}

#[test]
fn a_worker_panic_stops_the_helper_and_says_so() {
    let registry = Registry::new();
    let fake = Fake::new(|link| serve(link, &[LETTER])).panic_on_section(2);
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    // The page's section is the starter's second: it panics on the worker thread.
    assert!(host.request_page(0, scale_100(), None).is_some());
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    let failed = OfficeState::Stopped {
        why: Stop::Failed,
        can_retry: false,
    };
    assert_eq!(host.state(), failed);
    assert!(wait_for(|| log.kills() == 1), "the helper is ended");
    assert!(wait_for(|| host.worker_ended()), "the worker counts as ended");
    assert_eq!(host.helper_pid(), None);
    assert!(!host.retry(), "no Try again after a panic");
}

#[test]
fn a_range_over_the_cap_or_outside_the_sheet_is_not_asked() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => sheets(&[("Sheet1", 1000, 100)], 1),
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
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    let refused = [
        ("over the cap: 65,600 cells", 0, range(0, 0, 656, 100)),
        ("past the last row", 0, range(990, 0, 20, 10)),
        ("past the last column", 0, range(0, 95, 10, 10)),
        ("an empty range", 0, range(0, 0, 0, 5)),
        ("no such sheet", 2, range(0, 0, 1, 1)),
        (
            "past the protocol's largest sheet",
            1,
            range(check::MAX_SHEET_ROWS - 1, 0, 2, 1),
        ),
    ];
    for (case, sheet, asked) in refused {
        assert!(!host.request_cells(sheet, asked), "{case}");
    }
    let asked = [
        // 65,500 cells: within the cap.
        (0, range(0, 0, 655, 100)),
        // To the last row and the last column.
        (0, range(900, 90, 100, 10)),
        // One of the sheets without a name here: the protocol's largest sheet bounds it.
        (1, range(0, 0, 10, 10)),
    ];
    for (sheet, cells) in asked {
        assert!(host.request_cells(sheet, cells), "sheet {sheet} {cells:?}");
    }
    assert!(
        pump(&mut host, &mut events, |_, events| cells_of(events).len() == 3),
        "{:?}",
        host.state()
    );
    pump_for(&mut host, &mut events, Duration::from_millis(100), |_, _| false);
    let frames = lock(&log.sent)
        .iter()
        .filter(|(kind, _)| *kind == office::KIND_CELLS)
        .count();
    assert_eq!(frames, 3, "no frame for a refused range");
    assert_eq!(log.kills(), 0);
}

#[test]
fn a_helper_that_refuses_every_range_is_stopped_by_the_budget() {
    let registry = Registry::new();
    // The helper refuses every range as too large, all at once: it gathers the requests until the host has gone quiet,
    // then answers them together. At that moment the host's requests out are exactly those written and not answered.
    let peak = Arc::new(AtomicUsize::new(0));
    let seen = peak.clone();
    let fake = Fake::new(move |link| {
        let mut gathered = Vec::new();
        let mut answered = 0;
        loop {
            if link.input_within(Duration::from_millis(50)) {
                let Some((id, message)) = link.next() else {
                    return;
                };
                match message {
                    Message::Open(_) => link.answer(id, sheets(&[("Sheet1", 300, 300)], 0)),
                    Message::Cells(_) => gathered.push(id),
                    _ => {}
                }
            } else if !gathered.is_empty() {
                seen.fetch_max(link.sent_of(office::KIND_CELLS) - answered, SeqCst);
                for id in gathered.drain(..) {
                    link.answer(
                        id,
                        Message::Error(office::Error {
                            code: ErrorCode::RangeTooLarge,
                        }),
                    );
                    answered += 1;
                }
            }
        }
    });
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Xlsx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    // 256 x 256: exactly the cap, inside the sheet.
    assert!(host.request_cells(0, range(0, 0, 256, 256)));
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert!(stopped_for(&host, Stop::Protocol), "{:?}", host.state());
    let peak = peak.load(SeqCst);
    assert!(peak <= MAX_SPLIT_OUTSTANDING as usize, "{peak} requests out at once");
    // The instrument counts (a loaded machine may answer in smaller batches, so the floor is low).
    assert!(peak >= 8, "the count moved: {peak} at most");
    assert!(cells_of(&events).is_empty());
    assert_eq!(log.kills(), 1);
}

#[test]
fn an_outline_past_its_time_budget_is_stopped() {
    let registry = Registry::new();
    let fake = Fake::new(|link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) => opened(&[LETTER; 5]),
                // One entry a page, each well within the request timer: the outline moves on and does not end.
                Message::Outline(asked) => {
                    thread::sleep(Duration::from_millis(150));
                    outline_from(asked.from)
                }
                _ => Message::Error(office::Error {
                    code: ErrorCode::Internal,
                }),
            };
            link.answer(id, reply);
        }
    });
    let timeouts = Timeouts {
        outline: Duration::from_millis(500),
        ..TEST_TIMEOUTS
    };
    let (mut host, log, _) = open_fake_with(fake, &registry, DocKind::Docx, timeouts);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    let asked_at = Instant::now();
    assert!(host.request_outline());
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert!(stopped_for(&host, Stop::Protocol), "{:?}", host.state());
    assert!(
        asked_at.elapsed() >= Duration::from_millis(450),
        "not before the budget: {:?}",
        asked_at.elapsed()
    );
    let pages = lock(&log.sent)
        .iter()
        .filter(|(kind, _)| *kind == office::KIND_OUTLINE)
        .count();
    assert!(pages >= 3, "each page came in time: {pages} asked");
    assert!(!events.iter().any(|event| matches!(event, Event::Outline(_))));
    assert_eq!(log.kills(), 1);
}

#[test]
fn a_helper_that_keeps_its_handles_is_stopped() {
    let registry = Registry::new();
    let sizes = [LETTER; 40];
    let fake = Fake::new(move |link| serve(link, &sizes)).leaking_handles();
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    // One page at a time, so the count the host reads after an answer is the helper's at that answer.
    for page in 0..40 {
        if stopped(&host) {
            break;
        }
        let before = pages_of(&events);
        assert!(host.request_page(page, scale_100(), None).is_some(), "page {page}");
        assert!(
            pump(&mut host, &mut events, |host, events| pages_of(events) > before
                || stopped(host)),
            "page {page}: {:?}",
            host.state()
        );
    }
    assert!(stopped_for(&host, Stop::Protocol), "{:?}", host.state());
    // The ceiling counts from the open answer (the baseline holds its handle already).
    let allowed = MAX_IN_FLIGHT + HANDLE_SLACK as usize;
    assert_eq!(
        pages_of(&events),
        allowed,
        "pages up to the ceiling: the requests out plus the slack, from the count at the first answer"
    );
    assert_eq!(log.kills(), 1);
}

/// The ceiling counts from the first checked answer (Opened/Sheets), not from the start: the loader's handles
/// (the helper's DLLs, taken between `ResumeThread` and its first answer) are part of the baseline. A helper that
/// holds many handles by its first answer (a burst past base + in-flight + slack) is not stopped for them.
#[test]
fn handles_taken_before_the_first_answer_are_part_of_the_baseline() {
    let registry = Registry::new();
    let sizes = [LETTER; 4];
    let burst = MAX_IN_FLIGHT as u32 + HANDLE_SLACK + 1;
    let fake = Fake::new(move |link| serve(link, &sizes)).bursting_handles(burst);
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(!stopped(&host), "{:?}", host.state());
    assert_eq!(log.kills(), 0);
}

/// After the first answer the ceiling is absolute: a burst before it moves the baseline, handles kept after it
/// (one more per answer) still run into the ceiling.
#[test]
fn a_helper_that_leaks_after_a_loader_burst_is_still_stopped() {
    let registry = Registry::new();
    let sizes = [LETTER; 40];
    let burst = MAX_IN_FLIGHT as u32 + HANDLE_SLACK + 1;
    let fake = Fake::new(move |link| serve(link, &sizes))
        .bursting_handles(burst)
        .leaking_handles();
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(!stopped(&host), "the burst is in the baseline: {:?}", host.state());
    for page in 0..40 {
        if stopped(&host) {
            break;
        }
        let before = pages_of(&events);
        assert!(host.request_page(page, scale_100(), None).is_some(), "page {page}");
        assert!(
            pump(&mut host, &mut events, |host, events| pages_of(events) > before
                || stopped(host)),
            "page {page}: {:?}",
            host.state()
        );
    }
    assert!(stopped_for(&host, Stop::Protocol), "{:?}", host.state());
    assert_eq!(log.kills(), 1);
}

/// A section that says which way it was read.
struct Recording {
    bytes: Vec<u8>,
    read: Cell<Option<&'static str>>,
}

impl Recording {
    fn new(bytes: Vec<u8>) -> Recording {
        Recording {
            bytes,
            read: Cell::new(None),
        }
    }
}

impl SectionRead for Recording {
    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn copy_to(&self, out: &mut [u8]) -> Result<(), SandboxError> {
        self.read.set(Some("copy_to"));
        if out.len() != self.bytes.len() {
            return Err(SandboxError::Limits("not the section's length"));
        }
        out.copy_from_slice(&self.bytes);
        Ok(())
    }

    fn as_slice(&self) -> &[u8] {
        self.read.set(Some("as_slice"));
        &self.bytes
    }
}

#[test]
fn page_pixels_are_copied_without_a_shared_slice() {
    let section = Recording::new((0..=255).collect());
    let mut out = vec![0u8; 256];
    assert!(copy_section_out(&section, &mut out));
    assert_eq!(
        section.read.get(),
        Some("copy_to"),
        "the raw copy, never the shared slice"
    );
    assert_eq!(out, section.bytes);
    // Another length: refused before anything is read.
    let other = Recording::new(vec![7; 256]);
    let mut short = vec![0u8; 255];
    assert!(!copy_section_out(&other, &mut short));
    assert_eq!(other.read.get(), None);
    assert!(short.iter().all(|byte| *byte == 0));

    // Through the host: a copy that fails delivers no page (never one of zeros) and stops the renderer.
    let registry = Registry::new();
    let fake = Fake::new(|link| serve(link, &[LETTER])).failing_copy();
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(host.request_page(0, scale_100(), None).is_some());
    assert!(
        pump(&mut host, &mut events, |host, _| stopped(host)),
        "{:?}",
        host.state()
    );
    assert!(stopped_for(&host, Stop::Failed), "{:?}", host.state());
    assert_eq!(pages_of(&events), 0);
    assert_eq!(*lock(&log.copies), vec!["refused"]);
}
