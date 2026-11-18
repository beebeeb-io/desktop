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

/// A press in the viewer reaches the shell's viewer route: with no handler yet the router's fallback says so.
#[test]
fn a_viewer_share_press_reaches_the_viewer_route_and_says_not_built_while_nothing_takes_it() {
    use crate::frames::viewers;
    let mut rt = runtime(FixtureEngine::ready());
    rt.update(|s, tx| {
        assert!(viewers::open_from(s, Box::new(viewers::source::FixtureViewerSource::new()), crate::frames::viewers::source::board::AUDIO));
        viewers::ui(s).expect("open").intent(viewers::Intent::Share);
        assert!(drain_in_poll(s, tx), "the viewer's queue was taken");
        let ui = viewers::ui(s).expect("open");
        assert!(ui.take_intents().is_empty());
        assert_eq!(ui.toast.as_deref(), Some("Not built yet: Share"));
    });
}
