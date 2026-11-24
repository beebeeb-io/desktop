//! Tests that Settings' Privacy and Export my data each ask the shell for their web page, and that no other target is taken.

use std::sync::Arc;

use rune::{Env, Runtime};

use super::super::actions::FixtureEngine;
use super::super::host::{Shell, poll};
use super::super::platform::fake::FakePlatform;
use super::super::{registry, state};
use crate::engine_host::signin::OPENED;
use crate::frames::settings::Intent;

fn runtime() -> Runtime<crate::State> {
    let state = state::init_with(Shell::fixture_on(
        Arc::new(FixtureEngine::ready()),
        Arc::new(FakePlatform::default()),
    ));
    let mut rt = Runtime::new(
        state,
        registry::view,
        (1280.0, 800.0),
        Env { system_dark: false, scale: 2.0 },
    );
    rt.poll = Some(poll);
    super::super::intents::poll_drains_for_this_test();
    rt.frame(0.0);
    rt
}

#[test]
fn privacy_and_export_my_data_each_ask_for_their_web_page() {
    OPENED.with(|o| o.borrow_mut().clear());
    let mut rt = runtime();
    rt.update(|s, _| {
        s.settings.ask(Intent::Open("privacy"));
        s.settings.ask(Intent::Open("export-my-data"));
    });
    rt.frame(1.0);
    assert_eq!(
        OPENED.with(|o| o.borrow().clone()),
        vec!["https://app.beebeeb.io/settings/privacy", "https://app.beebeeb.io/settings/privacy"]
    );
}

#[test]
fn another_target_is_not_taken() {
    let mut rt = runtime();
    rt.update(|s, tx| assert!(!super::open(s, tx, "open-source-licences")));
}
