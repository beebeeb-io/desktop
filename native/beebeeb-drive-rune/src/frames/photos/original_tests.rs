//! The open photo's Original tab: progress moves the bar, a failure and Stop leave the preview.

use super::source::{FixturePhotos, OriginalAnswer, PhotoId, board};
use super::{Original, Toast, Ui};

fn harbour() -> PhotoId {
    PhotoId((board::HARBOUR.0 * 1000 + board::HARBOUR.1) as u64)
}

fn open_harbour() -> Ui {
    let mut ui = Ui::new(Box::new(FixturePhotos::new()));
    ui.open_tile(board::HARBOUR.0, board::HARBOUR.1);
    ui
}

#[test]
fn progress_answers_move_the_bar_and_the_last_byte_waits_for_the_decoded_picture() {
    let mut ui = open_harbour();
    ui.show_original();
    ui.original_answered(OriginalAnswer::Progress { id: harbour(), done: 12_400_000, total: 30_900_000 });
    assert_eq!(ui.viewer.as_ref().unwrap().original, Original::Downloading { done: 12_400_000 });
    ui.original_answered(OriginalAnswer::Progress { id: harbour(), done: 30_900_000, total: 30_900_000 });
    assert!(matches!(ui.viewer.as_ref().unwrap().original, Original::Downloading { .. }));
    ui.original_answered(OriginalAnswer::Done(harbour()));
    assert_eq!(ui.viewer.as_ref().unwrap().original, Original::Whole);
}

#[test]
fn a_failed_original_and_stop_leave_the_preview() {
    let mut ui = open_harbour();
    ui.show_original();
    ui.original_answered(OriginalAnswer::Failed(harbour()));
    assert_eq!(ui.viewer.as_ref().unwrap().original, Original::Preview);
    assert!(matches!(ui.toast, Some(Toast::Words(_))));

    ui.toast = None;
    ui.show_original();
    ui.show_preview();
    assert_eq!(ui.viewer.as_ref().unwrap().original, Original::Preview);
    // A progress answer that lands after Stop does not bring the bar back.
    ui.original_answered(OriginalAnswer::Progress { id: harbour(), done: 1, total: 10 });
    assert_eq!(ui.viewer.as_ref().unwrap().original, Original::Preview);
}
