//! Tests of Thumbnails and previews (DW-0744), its confirm (DW-0744b) and the compare view
//! (DW-0745): every drawn control does what the board says, on the fixture's content.
//!
//! Input is scripted: keys and presses go through Rune's `Runtime`, as a person's would.

use super::drive::{BoardDrive, DriveSettings, Insights, Kept, Storage, SyncPane};
use super::previews::{CompareMode, PhotoPreviews, PreviewFormat, PreviewSize, REBUILDING};
use super::{Dialog, Intent, Pane};
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

fn start(state: State) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 998.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    rt
}

fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    start(state)
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    find(rt, key)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .text
        .unwrap_or_default()
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

fn tab_to(rt: &mut Runtime<State>, target: &str) {
    for _ in 0..80 {
        key(rt, Key::Tab);
        if focused(rt).as_deref() == Some(target) {
            return;
        }
    }
    panic!("Tab never reaches {target}");
}

fn press(rt: &mut Runtime<State>, target: &str) {
    tab_to(rt, target);
    key(rt, Key::Enter);
}

fn stored(rt: &Runtime<State>) -> (PreviewSize, PreviewFormat, bool) {
    match rt.state.settings.drive.photo_previews() {
        PhotoPreviews::Ready(p) => (p.size, p.format, p.on),
        PhotoPreviews::Unavailable => panic!("the fixture answers"),
    }
}

fn says(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| !e.exiting && e.text.as_deref().is_some_and(|t| t.contains(words)))
}

#[test]
fn the_sidebar_opens_thumbnails_and_previews_as_the_scene_draws_it() {
    let mut rt = runtime("DW-0712");
    press(&mut rt, "Thumbnails and previews-row");
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::ThumbnailsAndPreviews)
    );
    assert_eq!(rt.state.settings.pane, Pane::Previews);
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(text(&rt, "set-title"), "Thumbnails and previews");
    assert_eq!(text(&rt, "card-0-sub"), "4K HEIC \u{b7} 1.0 MB, original 30.9 MB");
    assert_eq!(text(&rt, "item-3-0-value"), "2.4 GB");
    assert_eq!(text(&rt, "item-4-0-title"), "3.1 GB of 20 GB limit");
    let scene = runtime("DW-0744");
    assert_eq!(scene.state.shell.app.screen(), rt.state.shell.app.screen());
    assert_eq!(scene.state.settings.pane, rt.state.settings.pane);
    assert_eq!(scene.state.settings.dialog, rt.state.settings.dialog);
}

#[test]
fn the_switch_turns_photo_previews_off_and_on() {
    let mut rt = runtime("DW-0744");
    assert!(stored(&rt).2);
    press(&mut rt, "item-0-0-switch");
    assert!(!stored(&rt).2);
    key(&mut rt, Key::Enter);
    assert!(stored(&rt).2);
}

#[test]
fn a_size_asks_with_its_cost_and_nothing_changes_until_apply() {
    let mut rt = runtime("DW-0744");
    press(&mut rt, "seg-size-2-tab");
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ApplyPreviews));
    assert_eq!(text(&rt, "dlg-title"), "Apply 2K HEIC to 2,431 photos?");
    assert_eq!(
        text(&rt, "dlg-sub"),
        "Previews go from about 3.1 GB to about 1.6 GB of your storage. The old ones stay until the new ones have uploaded."
    );
    assert_eq!(text(&rt, "btn-1-label"), "Apply to 2,431 photos");
    // Nothing changed yet: the stored choice, and the pane under the question, say 4K.
    assert_eq!(stored(&rt).0, PreviewSize::K4);
    assert_eq!(text(&rt, "seg-size-3"), "4K");
    assert!(find(&rt, "seg-size-3-tab").is_none(), "4K is still the raised segment");
    // The same state as the scene.
    let scene = runtime("DW-0744b");
    assert_eq!(scene.state.settings.dialog, rt.state.settings.dialog);
    assert_eq!(scene.state.settings.previews.draft, rt.state.settings.previews.draft);

    press(&mut rt, "btn-2");
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(rt.state.settings.previews.draft, None);
    assert_eq!(stored(&rt).0, PreviewSize::K4);
    assert_eq!(rt.state.settings.toast, None);
}

#[test]
fn apply_stores_the_choice_and_says_previews_are_being_made() {
    let mut rt = runtime("DW-0744b");
    press(&mut rt, "btn-1");
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!((stored(&rt).0, stored(&rt).1), (PreviewSize::K2, PreviewFormat::Heic));
    assert_eq!(rt.state.settings.toast, Some(REBUILDING));
    assert_eq!(text(&rt, "toast-0-text"), REBUILDING);
    assert_eq!(text(&rt, "card-0-sub"), "2K HEIC \u{b7} 640 KB, original 30.9 MB");
    assert!(find(&rt, "seg-size-2-tab").is_none(), "2K is the raised segment now");
}

#[test]
fn a_format_asks_too_and_the_stored_choice_asks_nothing() {
    let mut rt = runtime("DW-0744");
    press(&mut rt, "seg-size-on");
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(rt.state.settings.previews.draft, None);
    press(&mut rt, "seg-format-1-tab");
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ApplyPreviews));
    assert_eq!(text(&rt, "dlg-title"), "Apply 4K JPEG to 2,431 photos?");
    // Esc is Cancel.
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(stored(&rt).1, PreviewFormat::Heic);
}

#[test]
fn compare_opens_and_done_without_a_change_just_goes_back() {
    let mut rt = runtime("DW-0744");
    press(&mut rt, "btn-0");
    assert_eq!(rt.state.settings.dialog, Some(Dialog::Compare));
    assert_eq!(rt.state.settings.previews.compare.mode, CompareMode::Wipe);
    assert_eq!(text(&rt, "cmp-tag-0"), "Original \u{b7} 30.9 MB");
    assert_eq!(text(&rt, "cmp-tag-1"), "4K HEIC \u{b7} 1.0 MB");
    assert_eq!(text(&rt, "cmp-opt-0-bytes"), "213 KB");
    assert_eq!(text(&rt, "cmp-opt-5-bytes"), "2.1 MB");
    let scene = runtime("DW-0745");
    assert_eq!(scene.state.settings.dialog, rt.state.settings.dialog);
    press(&mut rt, "btn-1");
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(rt.state.settings.pane, Pane::Previews);
    assert!(find(&rt, "cmp").is_none());
}

#[test]
fn done_with_another_choice_goes_back_with_it_and_asks_before_anything_changes() {
    let mut rt = runtime("DW-0745");
    press(&mut rt, "cmp-opt-1-tab");
    assert_eq!(
        rt.state.settings.previews.draft,
        Some((PreviewSize::P1080, PreviewFormat::Heic))
    );
    assert_eq!(text(&rt, "cmp-tag-1"), "1080p HEIC \u{b7} 420 KB");
    assert_eq!(stored(&rt).0, PreviewSize::K4);
    press(&mut rt, "btn-1");
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ApplyPreviews));
    assert_eq!(text(&rt, "dlg-title"), "Apply 1080p HEIC to 2,431 photos?");
    assert_eq!(stored(&rt).0, PreviewSize::K4);
}

#[test]
fn esc_in_the_compare_view_is_done() {
    let mut rt = runtime("DW-0745");
    press(&mut rt, "cmp-opt-0-tab");
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.settings.dialog, Some(Dialog::ApplyPreviews));
    assert_eq!(
        rt.state.settings.previews.draft,
        Some((PreviewSize::P720, PreviewFormat::Heic))
    );
    // And with nothing chosen it only closes the view; settings stays open.
    let mut rt = runtime("DW-0745");
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.settings.dialog, None);
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::ThumbnailsAndPreviews)
    );
}

#[test]
fn the_compare_view_stacks_wipes_and_flips() {
    let mut rt = runtime("DW-0745");
    tab_to(&mut rt, "cmp-photo");
    key(&mut rt, Key::Right);
    let moved = rt.state.settings.previews.compare.wipe;
    assert!((moved - 0.5).abs() > 0.01, "an arrow key moves the wipe line: {moved}");
    press(&mut rt, "cmp-seg-0-tab");
    assert_eq!(rt.state.settings.previews.compare.mode, CompareMode::Stacked);
    assert!(find(&rt, "cmp-tag-0-pic").is_some() && find(&rt, "cmp-tag-1-pic").is_some());
    press(&mut rt, "cmp-seg-2-tab");
    assert_eq!(rt.state.settings.previews.compare.mode, CompareMode::Flip);
    assert!(find(&rt, "cmp-tag-1").is_some() && find(&rt, "cmp-tag-0").is_none());
    press(&mut rt, "cmp-photo");
    assert!(rt.state.settings.previews.compare.original_showing);
    assert!(find(&rt, "cmp-tag-0").is_some() && find(&rt, "cmp-tag-1").is_none());
}

#[test]
fn choose_photo_asks_the_shell_for_the_picker() {
    let mut rt = runtime("DW-0745");
    press(&mut rt, "btn-2");
    assert_eq!(rt.state.settings.take_intents(), vec![Intent::Open("choose-photo")]);
    assert_eq!(rt.state.settings.dialog, Some(Dialog::Compare), "the view stays open");
}

#[test]
fn on_this_pc_opens_storage_and_rebuild_says_so() {
    let mut rt = runtime("DW-0744");
    press(&mut rt, "item-6-0");
    assert_eq!(rt.state.settings.toast, Some(REBUILDING));
    press(&mut rt, "item-4-0");
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Storage));
    assert_eq!(rt.state.settings.pane, Pane::Storage);
}

/// A source that knows nothing of this pane: only the trait's default bodies answer.
struct Bare(BoardDrive);

impl DriveSettings for Bare {
    fn storage(&self) -> Storage {
        self.0.storage()
    }
    fn set_limit(&mut self, which: Kept, gb: u32) {
        self.0.set_limit(which, gb);
    }
    fn clear(&mut self, which: Kept) {
        self.0.clear(which);
    }
    fn sync(&self) -> SyncPane {
        self.0.sync()
    }
    fn set_status_on_files(&mut self, on: bool) {
        self.0.set_status_on_files(on);
    }
    fn set_open_at_login(&mut self, on: bool) {
        self.0.set_open_at_login(on);
    }
    fn set_paused(&mut self, paused: bool) {
        self.0.set_paused(paused);
    }
    fn sync_anyway(&mut self) {
        self.0.sync_anyway();
    }
    fn repair(&mut self) {
        self.0.repair();
    }
    fn turn_off_explorer(&mut self) {
        self.0.turn_off_explorer();
    }
    fn insights(&self) -> &Insights {
        self.0.insights()
    }
    fn reload_insights(&mut self) {
        self.0.reload_insights();
    }
}

#[test]
fn a_source_that_does_not_answer_draws_no_sample_value() {
    let mut state = init();
    assert!(super::scene("DW-0744", &mut state));
    state.settings.drive = Box::new(Bare(BoardDrive::new()));
    let mut rt = start(state);
    assert_eq!(text(&rt, "set-title"), "Thumbnails and previews");
    for sample in ["2,431", "30.9 MB", "3.1 GB", "4K HEIC"] {
        assert!(!says(&rt, sample), "no sample value: {sample}");
    }
    assert!(find(&rt, "btn-0").is_none() && find(&rt, "item-0-0-switch").is_none());
    // And its controls' entry points do nothing.
    rt.state.settings.open_compare();
    rt.state.settings.choose_previews(Some(PreviewSize::K2), None);
    settle(&mut rt);
    assert_eq!(rt.state.settings.dialog, None);
}

#[test]
fn a_build_that_ships_draws_no_sample_value_on_thumbnails_and_previews() {
    let mut state = init();
    assert!(super::scene("DW-0744", &mut state));
    state.settings.drive = super::drive::first(false);
    let rt = start(state);
    assert!(find(&rt, "set-main").is_none(), "the shell's not-built-yet frame");
    for sample in ["2,431", "30.9 MB", "3.1 GB"] {
        assert!(!says(&rt, sample), "no sample value: {sample}");
    }
}
