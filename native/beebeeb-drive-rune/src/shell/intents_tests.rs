//! The shell takes what Settings and sign-in asked for: web pages (from the table only), the clipboard.

use std::sync::Arc;

use rune::{Env, Runtime};

use super::super::actions::FixtureEngine;
use super::super::host::{Shell, poll};
use super::super::platform::fake::FakePlatform;
use super::super::{registry, state};
use super::*;
use crate::engine_host::signin::OPENED;

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

fn runtime(engine: FixtureEngine) -> Runtime<State> {
    let state = state::init_with(Shell::fixture_on(Arc::new(engine), Arc::new(FakePlatform::default())));
    let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
    rt.poll = Some(poll);
    super::poll_drains_for_this_test();
    rt.frame(0.0);
    rt
}

fn opened() -> Vec<String> {
    OPENED.with(|o| o.borrow().clone())
}

fn clear() {
    OPENED.with(|o| o.borrow_mut().clear());
}

#[test]
fn web_pages_open_by_name_from_the_table_and_an_unknown_name_opens_nothing() {
    clear();
    let mut rt = runtime(FixtureEngine::ready());
    rt.update(|s, _| {
        s.settings.ask(Intent::Web("plan"));
        s.settings.ask(Intent::Web("about-audits"));
        s.settings.ask(Intent::OpenWindowsNotificationSettings);
    });
    rt.frame(1.0);
    assert_eq!(
        opened(),
        vec!["https://app.beebeeb.io/settings/billing", "ms-settings:notifications"]
    );
    rt.update(|s, _| assert!(s.settings.take_intents().is_empty(), "the queue is empty"));
}

#[test]
fn drain_empties_the_queue_and_every_unwired_variant_is_taken_not_dropped() {
    clear();
    let mut rt = runtime(FixtureEngine::ready());
    rt.update(|s, tx| {
        for intent in [
            Intent::Show("DW-A51"),
            Intent::Open("x"),
            Intent::RestartToUpdate,
            Intent::SaveBundleAndWriteEmail,
            Intent::CheckForUpdates,
            Intent::ShowInFiles("a".into()),
        ] {
            s.settings.ask(intent);
        }
        assert!(drain_settings_intents(s, tx), "something was taken");
        assert!(s.settings.take_intents().is_empty());
        assert!(!drain_settings_intents(s, tx), "nothing left");
    });
    assert!(opened().is_empty(), "an unwired variant opens nothing");
}

#[test]
fn copy_puts_exactly_the_text_on_the_clipboard_and_says_copied() {
    let mut rt = runtime(FixtureEngine::ready());
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    rt.update(|s, tx| {
        s.settings.ask(Intent::Copy("Beebeeb 0.9.0\nline two".into()));
        assert!(drain_settings_intents(s, tx));
        assert_eq!(s.settings.toast, Some("Copied"));
    });
    rt.frame(1.0);
    assert_eq!(
        clipboard.text().map(|t| t.to_string()).as_deref(),
        Some("Beebeeb 0.9.0\nline two")
    );
}

#[test]
fn sign_in_terms_privacy_and_support_reach_the_opener_with_the_tables_address() {
    use crate::frames::signin::Intent as SignIn;
    clear();
    let mut rt = runtime(FixtureEngine::signed_out());
    rt.update(|s, _| {
        let ui = s.signin.as_mut().expect("the sign-in screens");
        ui.leave(SignIn::OpenTerms);
        ui.leave(SignIn::OpenPrivacy);
        ui.leave(SignIn::WriteToSupport);
    });
    rt.frame(1.0);
    assert_eq!(
        opened(),
        vec![
            web_pages::TERMS_PAGE.to_string(),
            web_pages::PRIVACY_PAGE.to_string(),
            "mailto:support@beebeeb.io?subject=Help%20signing%20in".to_string(),
        ]
    );
}

/// The frames' queue (Trash, Shared, Search) is taken whole: what is not wired yet is taken and logged, never left
/// to grow.
#[test]
fn drain_takes_every_frame_intent() {
    use crate::frames::Intent as Frame;
    let mut rt = runtime(FixtureEngine::ready());
    rt.update(|s, tx| {
        s.frames.intents = vec![
            Frame::Frame("DW-0504"),
            Frame::Do("copy-link", "an-invented-id".into()),
            Frame::About("DW-0402", "an-invented-id".into()),
        ];
        assert!(drain_frame_intents(s, tx), "something was taken");
        assert!(
            s.frames.intents.is_empty(),
            "every intent was taken: {:?}",
            s.frames.intents
        );
        assert!(!drain_frame_intents(s, tx), "nothing left");
    });
}

/// The shell's poll itself drains the frames' queue (not only the function called by hand).
#[test]
fn the_poll_line_drains_frame_intents() {
    use crate::frames::Intent as Frame;
    let mut rt = runtime(FixtureEngine::ready());
    rt.update(|s, tx| {
        s.frames.intents.push(Frame::Frame("DW-0404"));
        s.frames.intents.push(Frame::OpenItem("an-invented-id".into()));
        poll(s, 0.0, tx);
        assert!(
            s.frames.intents.is_empty(),
            "the poll took them: {:?}",
            s.frames.intents
        );
    });
}

/// A press in the viewer reaches the shell's viewer route: Share opens Files' dialog for the open file.
#[test]
fn a_viewer_share_press_reaches_the_viewer_route_and_opens_files_share_dialog() {
    use crate::frames::viewers;
    let mut rt = runtime(FixtureEngine::ready());
    rt.update(|s, tx| {
        assert!(viewers::open_from(s, Box::new(viewers::source::FixtureViewerSource::new()), crate::frames::viewers::source::board::AUDIO));
        viewers::ui(s).expect("open").intent(viewers::Intent::Share);
        assert!(drain_in_poll(s, tx), "the viewer's queue was taken");
        let ui = viewers::ui(s).expect("open");
        assert!(ui.take_intents().is_empty());
        assert_eq!(ui.toast, None);
        assert!(matches!(&s.actions.dialog, Some(crate::frames::files::dialogs::Dialog::Share(sh)) if sh.items[0].id == crate::frames::viewers::source::board::AUDIO));
    });
}

// ---------------------------------------------------------------------------------------------------------------
// The office editor's lines in the shell (w05-wire-shell)
// ---------------------------------------------------------------------------------------------------------------

fn texts(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Files' New menu, "New document…": the press queues `Frame("DW-1558")`, the shell's poll takes it and the office
/// editor's dialog is over Files, on Docs, for the folder shown.
#[test]
fn new_document_from_the_files_menu_opens_dw_1558() {
    use crate::frames::files::MenuKind;
    use crate::frames::viewers::new_file::{Kind, Step};
    let mut rt = runtime(FixtureEngine::ready());
    rt.update(|s, tx| {
        s.toggle_menu(MenuKind::New);
        let at = s
            .menu_entries(MenuKind::New)
            .iter()
            .position(|entry| entry.id() == Some("new-document"))
            .expect("New document is in the New menu");
        s.pick_menu(at, tx);
        assert!(
            s.actions.intents.contains(&crate::frames::files::Intent::Frame("DW-1558")),
            "the press asks for DW-1558"
        );
    });
    rt.frame(1.0);
    rt.frame(1.0);
    rt.update(|s, _| {
        assert!(
            !s.actions.intents.contains(&crate::frames::files::Intent::Frame("DW-1558")),
            "the poll took it"
        );
        let dialog = s.viewer.new_file.as_ref().expect("New document is open");
        assert_eq!(dialog.step, Step::Document);
        assert_eq!(dialog.kind, Kind::Word);
        assert_eq!(dialog.folder, s.listing.folder.id);
    });
    let drawn = texts(&rt);
    assert!(drawn.contains(crate::frames::office::edit::new_document::TITLE), "{drawn}");
    assert!(drawn.contains("Meeting notes"), "Docs' templates are offered: {drawn}");
}

/// New file's Continue on an office type goes on to New document on that app.
#[test]
fn new_files_continue_on_an_office_type_opens_new_document_on_that_app() {
    use crate::frames::viewers::new_file::{self, Kind, Step};
    let mut rt = runtime(FixtureEngine::ready());
    rt.update(|s, _| {
        assert!(new_file::begin(s));
        new_file::choose(s, Kind::Excel);
        new_file::go_on(s);
        assert!(s.viewer.new_file.is_none(), "New file closed");
    });
    rt.frame(1.0);
    rt.frame(1.0);
    rt.update(|s, _| {
        let dialog = s.viewer.new_file.as_ref().expect("New document is open");
        assert_eq!((dialog.step, dialog.kind), (Step::Document, Kind::Excel));
    });
}

/// A source that says which files have a kept editing.
struct Kept(crate::frames::viewers::source::FixtureViewerSource, Vec<(String, String)>);

impl crate::frames::viewers::source::ViewerSource for Kept {
    fn open(
        &mut self,
        id: &str,
    ) -> Result<crate::frames::viewers::source::OpenFile, crate::frames::viewers::source::OpenError> {
        self.0.open(id)
    }
    fn poll(&mut self, id: &str) -> Option<crate::frames::viewers::source::Body> {
        self.0.poll(id)
    }
    fn cancel(&mut self, id: &str) {
        self.0.cancel(id)
    }
    fn retry(
        &mut self,
        id: &str,
    ) -> Result<crate::frames::viewers::source::OpenFile, crate::frames::viewers::source::OpenError> {
        self.0.retry(id)
    }
    fn recovery_pending(&self) -> Vec<(String, String)> {
        self.1.clone()
    }
}

fn runtime_keeping(kept: usize) -> Runtime<State> {
    let mut state = state::init_with(Shell::fixture_on(
        Arc::new(FixtureEngine::ready()),
        Arc::new(FakePlatform::default()),
    ));
    crate::frames::viewers::wire(
        &mut state,
        Box::new(move || {
            let files = (0..kept).map(|n| (format!("invented-{n}"), "14:32".to_string())).collect();
            Box::new(Kept(crate::frames::viewers::source::FixtureViewerSource::new(), files))
        }),
    );
    let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
    rt.poll = Some(poll);
    super::poll_drains_for_this_test();
    rt.frame(0.0);
    rt.frame(0.016);
    rt
}

/// The first poll that sees the session gate open says what unsaved editing this PC kept: Files' notice, in the
/// editor's words, for as many files as the source lists. Nothing kept: nothing said.
#[test]
fn the_gate_opening_says_what_unsaved_editing_this_pc_kept() {
    use crate::frames::files::Toast;
    let rt = runtime_keeping(2);
    assert_eq!(rt.state.actions.toast, Some(Toast::KeptEdits(2)));
    let drawn = texts(&rt);
    assert!(drawn.contains("Unsaved changes to 2 files were kept on this PC."), "{drawn}");

    let rt = runtime_keeping(1);
    assert_eq!(rt.state.actions.toast, Some(Toast::KeptEdits(1)));
    assert!(texts(&rt).contains("Unsaved changes to a file were kept on this PC."));
}

#[test]
fn nothing_kept_says_nothing_and_the_notice_is_said_once() {
    let rt = runtime_keeping(0);
    assert_eq!(rt.state.actions.toast, None);

    let mut rt = runtime_keeping(1);
    rt.update(|s, _| s.actions.toast = None);
    for _ in 0..3 {
        rt.frame(0.016);
    }
    assert_eq!(rt.state.actions.toast, None, "once per opening of the gate, not every poll");
}
