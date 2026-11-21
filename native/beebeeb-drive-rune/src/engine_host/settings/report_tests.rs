//! Report a problem (DW-0782) on the real-engine settings source: General and Diagnostics both open its dialog
//! (a bundle the person reads first, an email they send themselves; nothing is sent by the app).

use std::sync::Arc;

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};

use super::*;
use crate::frames::settings::{Dialog, Pane, go};
use crate::{State, init};

fn state_on_real_source() -> (State, FakeApi) {
    let (account, _phrase) = FakeAccount::register("report-problem@example.eu", "an invented password", TEST_PHRASE);
    let fake = FakeApi::start(account_handler(Arc::new(account)));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "report-problem-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    let mut state = init();
    state.settings.source = Box::new(EngineSettingsSource::new(Arc::new(engine)));
    (state, fake)
}

#[test]
fn report_a_problem_opens_from_general_and_from_diagnostics() {
    let (mut state, _fake) = state_on_real_source();
    assert!(state.settings.source.opens("DW-0782"), "the real source opens the dialog");
    // General's row and Diagnostics' button both press `go(s, "DW-0782")`.
    for from in [Pane::General, Pane::Diagnostics] {
        state.settings.dialog = None;
        state.settings.pane = from;
        go(&mut state, "DW-0782");
        assert_eq!(state.settings.dialog, Some(Dialog::ReportProblem), "from {from:?}");
    }
}
