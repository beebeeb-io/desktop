//! The wait for the launch lock before a helper starts (review of #467, P3-3). Windows' app chooser holds that lock
//! for as long as it is open; the fake's slow start stands in for the wait. The wait is not the helper's time, and a
//! close during it drops the document's decrypted section at once, not when the start returns.

use std::thread;
use std::time::{Duration, Instant};

use beebeeb_office_protocol::office;
use beebeeb_office_sandbox::{Access, Granted, SandboxError};
use zeroize::Zeroizing;

use super::fake_helper_tests::*;
use crate::engine_host::office_host::*;

/// The start waits 500 ms, longer than the tests' 300 ms open timer: the helper that then starts and answers at once
/// opens the document. Its open timer counts from its start.
#[test]
fn an_open_that_waited_for_the_launch_lock_is_timed_from_the_helpers_start() {
    let registry = Registry::new();
    let sizes = [LETTER];
    let fake = Fake::new(move |link| serve(link, &sizes)).slow_start(Duration::from_millis(500));
    let started = Instant::now();
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "the instrument: the start waited past the open timer ({:?})",
        started.elapsed()
    );
    assert_eq!(
        &host.state(),
        &OfficeState::Pages {
            pages: 1,
            sizes: sizes.to_vec()
        },
        "not timed out by the wait"
    );
    assert_eq!((log.starts(), log.kills()), (1, 0));
}

/// A lock while the start waits: the close itself drops the document's section, before the start has returned. The
/// helper that starts after it is ended, and is sent nothing.
#[test]
fn a_close_while_the_start_waits_drops_the_document_at_once() {
    let registry = Registry::new();
    let sizes = [LETTER];
    let fake = Fake::new(move |link| serve(link, &sizes)).slow_start(Duration::from_millis(800));
    let (mut host, log, _) = open_fake(fake, &registry, DocKind::Docx);
    assert!(
        wait_for(|| log.live() == 1),
        "the document's section is made before the start"
    );
    // The section is filled and the worker is in the start (a copy of a few bytes takes far less).
    thread::sleep(Duration::from_millis(150));
    host.close(CloseWhy::Locked);
    assert_eq!(log.live(), 0, "the close dropped the document's section");
    assert_eq!(log.starts(), 0, "while the start still waited");
    assert!(wait_for(|| log.starts() == 1), "the start returned after the close");
    assert!(wait_for(|| log.kills() == 1), "and its helper is ended");
    assert!(wait_for(|| host.worker_ended()), "the worker ends");
    assert_eq!(&host.state(), &OfficeState::Closed(CloseWhy::Locked));
    let opens = lock(&log.sent)
        .iter()
        .filter(|(kind, _)| *kind == office::KIND_OPEN)
        .count();
    assert_eq!(opens, 0, "nothing was sent to it");
    assert_eq!(log.live(), 0);
}

/// The fake starter whose start panics (inside the wait for the launch lock): every other step is the fake's.
struct PanicsOnStart(Fake);

impl Starter for PanicsOnStart {
    type Section = FakeSection;
    type Process = FakeHelper;

    fn installed(&self) -> bool {
        self.0.installed()
    }

    fn file_name(&self) -> Option<String> {
        self.0.file_name()
    }

    fn exclude_from_crash_reports(&mut self, exe_name: &str) -> Result<(), u32> {
        self.0.exclude_from_crash_reports(exe_name)
    }

    fn section(&mut self, len: usize) -> Result<FakeSection, SandboxError> {
        self.0.section(len)
    }

    fn section_len(&self, section: &FakeSection) -> usize {
        self.0.section_len(section)
    }

    fn fill(&self, section: &mut FakeSection, bytes: &[u8]) {
        self.0.fill(section, bytes);
    }

    fn copy_out(&self, section: &FakeSection, out: &mut [u8]) -> bool {
        self.0.copy_out(section, out)
    }

    fn start(&mut self) -> Result<Link<FakeHelper>, SandboxError> {
        panic!("the fake start panics");
    }

    fn grant(&self, helper: &FakeHelper, section: &FakeSection, access: Access) -> Result<Granted, SandboxError> {
        self.0.grant(helper, section, access)
    }
}

/// A panic while the start waits: the parked document goes with the worker, with the host still open (nothing closed
/// it), and the window says "Renderer stopped" with no Try again.
#[test]
fn a_start_that_panics_leaves_no_document_behind() {
    let registry = Registry::new();
    let fake = Fake::new(|_| {});
    let log = fake.log.clone();
    let mut host = open_with(
        PanicsOnStart(fake),
        &registry,
        TEST_TIMEOUTS,
        DocKind::Docx,
        Zeroizing::new(doc()),
        noop_wake(),
    );
    assert!(wait_for(|| host.worker_ended()), "the worker ends");
    assert_eq!(log.live(), 0, "the document's section went with the panicked worker");
    let mut events = Vec::new();
    assert!(pump(&mut host, &mut events, |host, _| host.state() != OfficeState::Opening));
    assert_eq!(
        &host.state(),
        &OfficeState::Stopped {
            why: Stop::Failed,
            can_retry: false
        }
    );
}
