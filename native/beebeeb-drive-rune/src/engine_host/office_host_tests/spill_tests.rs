//! A large editing session keeps its working copy sealed on disk (`office_host/worker.rs` -> `held.rs` with the
//! session's scratch), restarts from it, and leaves no file after the session ends. The real path: an editing host
//! opened with a scratch store, a helper that dies, a restart that reads the sealed copy.

use std::time::Duration;

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::office_scratch::{end_for_test, store_for_test};
use beebeeb_office_protocol::office::{self, Message};
use zeroize::Zeroizing;

use super::fake_helper_tests::*;
use crate::engine_host::office_host::*;

const MARKER: &[u8] = b"TOP-SECRET-MARKER-9d3f";

fn files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir.join("office-scratch"))
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn mortal() -> Fake {
    Fake::new(move |link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::OpenEdit(_) => opened(&[LETTER]),
                Message::Edit(office::Edit {
                    op: EditOp::InsertText(text),
                }) if text == "die" => return,
                _ => Message::Error(office::Error {
                    code: office::ErrorCode::Internal,
                }),
            };
            link.answer(id, reply);
        }
    })
}

fn big() -> Vec<u8> {
    // Just above the spill threshold, below the input limit.
    let mut v = MARKER.repeat(SPILL_LIMIT / MARKER.len() + 2);
    v.truncate(SPILL_LIMIT + 1);
    v
}

/// Debug builds seal and open 64 MiB slowly: the helper's timers are long here.
const SLOW: Timeouts = Timeouts {
    open: Duration::from_secs(120),
    request: Duration::from_secs(120),
    split: Duration::from_secs(120),
    outline: Duration::from_secs(120),
};

const LONG: Duration = Duration::from_secs(120);

const SPILL_LIMIT: usize = 64 * 1024 * 1024;

#[test]
fn a_large_edit_spills_sealed_restarts_from_it_and_leaves_no_file() {
    let tmp = tempfile::tempdir().unwrap();
    let scratch = store_for_test(tmp.path(), SessionGeneration::for_test(41));
    let registry = Registry::new();
    let doc = big();
    let mut host = open_mode_held(
        mortal(),
        &registry,
        SLOW,
        Cadence::PRODUCTION,
        Mode::Edit(EditKind::Docx),
        DocKind::Docx,
        Some(Zeroizing::new(doc.clone())),
        noop_wake(),
        Some(scratch),
    );
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());

    // The working copy is on disk, sealed.
    let on_disk = files(tmp.path());
    assert_eq!(on_disk.len(), 1, "one sealed copy of the opened bytes");
    let raw = std::fs::read(&on_disk[0]).unwrap();
    assert!(!raw.windows(MARKER.len()).any(|w| w == MARKER), "no plaintext on disk");

    // The helper dies: the restart opens what was sealed (the whole document).
    host.edit(EditOp::InsertText("die".to_owned())).expect("sent");
    let until = std::time::Instant::now() + LONG;
    let mut got = Vec::new();
    while !got.iter().any(|e| matches!(e, EditEvent::Restored { .. })) {
        assert!(std::time::Instant::now() < until, "restored in time: {got:?}");
        let _ = host.poll();
        got.extend(host.poll_edit());
        std::thread::sleep(Duration::from_millis(5));
    }

    // The session ends (a close): nothing stays.
    host.close(CloseWhy::Closed);
    let until = std::time::Instant::now() + LONG;
    while !files(tmp.path()).is_empty() {
        assert!(std::time::Instant::now() < until, "scratch removed after the session");
        std::thread::sleep(Duration::from_millis(5));
    }
    end_for_test(tmp.path());
}
