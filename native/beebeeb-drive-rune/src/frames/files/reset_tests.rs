//! What a session end forgets of the Files window's starts (review of #467, P3-1): on a lock, a sign-out and another
//! account (`end_session`), and whenever the session gate closes (`session_closed`), the source drops the starts it
//! was asked for and has not begun. The shell calls both (`shell::host`).

use std::sync::atomic::Ordering::SeqCst;

use crate::frames::files::source::FilesFixture;

/// The board's window over a fixture; how often it was told to forget its starts.
fn window() -> (crate::State, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let mut s = crate::init();
    let fixture = FilesFixture::default();
    let forgotten = fixture.starts_forgotten.clone();
    s.actions.source = Box::new(fixture);
    (s, forgotten)
}

#[test]
fn a_session_end_forgets_the_starts_not_begun() {
    // A lock: the same account comes back, and still nothing it asked to start before the lock begins after it.
    let (mut s, forgotten) = window();
    assert_eq!(forgotten.load(SeqCst), 0, "nothing forgotten before the end");
    super::end_session(&mut s, false);
    assert_eq!(forgotten.load(SeqCst), 1, "a lock");
    // A sign-out or another account.
    let (mut s, forgotten) = window();
    super::end_session(&mut s, true);
    assert_eq!(forgotten.load(SeqCst), 1, "the account left");
    // The gate closed in any way (also a session the server ended while the gate stayed Ready).
    let (mut s, forgotten) = window();
    super::session_closed(&mut s);
    assert_eq!(forgotten.load(SeqCst), 1, "the gate closed");
}
