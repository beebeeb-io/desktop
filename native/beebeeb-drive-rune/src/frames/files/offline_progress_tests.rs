//! Make available offline: the status bar names the running download, and the row shows its progress.

use super::keeping::{status_words, Download};
use super::source::FilesFixture;
use super::tests::*;
use crate::init;

fn dl(id: &str, done: u64, total: u64, kept: bool) -> Download {
    Download {
        id: id.into(),
        done,
        total,
        kept,
    }
}

#[test]
fn offline_progress_status_words_count_files_and_what_is_left() {
    const MB: u64 = 1024 * 1024;
    let three = [
        dl("a", 0, 100 * MB, true),
        dl("b", 10 * MB, 20 * MB, true),
        dl("c", 0, 10 * MB, true),
    ];
    assert_eq!(
        status_words(&three).as_deref(),
        Some("Downloading 3 files, 120 MB left")
    );
    assert_eq!(
        status_words(&[dl("a", 0, 367 * MB, true)]).as_deref(),
        Some("Downloading 1 file, 367 MB left")
    );
}

#[test]
fn offline_progress_says_nothing_when_nothing_kept_comes_down() {
    assert_eq!(status_words(&[]), None);
    // A player's read is no keep.
    assert_eq!(status_words(&[dl("a", 1, 2, false)]), None);
}

#[test]
fn offline_progress_shows_in_the_window_bar_and_the_row() {
    let mut state = init();
    let id = state.listing.items[0].id.clone();
    let mut fixture = FilesFixture::default();
    fixture.more.downloads = vec![dl(&id, 100, 400, true)];
    state.actions.source = Box::new(fixture);
    let mut rt = runtime_of(state);
    settle(&mut rt);
    let words = find(&rt, "status-details").and_then(|e| e.text).unwrap_or_default();
    assert!(words.starts_with("Downloading 1 file, "), "{words}");
    assert!(find(&rt, "row-0-bar").is_some(), "the row has its bar");
}
