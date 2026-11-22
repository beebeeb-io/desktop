//! Every Settings sidebar pane on the app's own engine sources (`state::init_on_engine`, a real engine handle on the
//! fake API): none draws the shell's "not built yet" frame while its source is bound. Every name is invented.

use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::app::lock_api::LockState;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use rune::{Env, Runtime};
use zeroize::Zeroizing;

use crate::State;
use crate::engine_host::LiveEngine;
use crate::engine_host::open::Opened;
use crate::shell::actions::HostActions;
use crate::shell::app::{Event, SettingsPane};
use crate::shell::host::{self, Shell, Wiring};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, state};

const EMAIL: &str = "settings-walk@example.eu";
const PASSWORD: &str = "an invented password for the settings walk";

#[test]
fn no_settings_pane_draws_the_not_built_frame_on_the_real_sources() {
    let (account, phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    let api = FakeApi::start(account_handler(Arc::new(account)));
    let (engine_host, messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "settings-walk-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, engine_host, MemoryCredentials::default()).expect("opens"));
    engine.restore();
    engine
        .sign_in_password(EMAIL, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    assert_eq!(engine.lock_state(), LockState::Open);
    let opened = Opened {
        engine: engine.clone(),
        identity: &TEST,
        gate: engine.gate(),
    };
    let live_engine = Arc::new(LiveEngine::new(&opened, messages));
    let platform = Arc::new(FakePlatform::default());
    let shell = Shell::new(
        Arc::new(HostActions::new(live_engine, platform.clone())),
        platform,
        Wiring::default(),
    );
    let mut rt = Runtime::new(
        state::init_on_engine(shell, engine.clone()),
        registry::view,
        (1280.0, 800.0),
        Env {
            system_dark: false,
            scale: 1.0,
        },
    );
    rt.poll = Some(host::poll);
    rt.on_key = Some(host::on_key);
    rt.frame(0.0);

    let mut placeholders = Vec::new();
    for pane in SettingsPane::ALL {
        host::dispatch(&mut rt.state, Event::OpenSettings(pane));
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            rt.frame(0.016);
            std::thread::sleep(Duration::from_millis(10));
        }
        let built_not = rt
            .elements()
            .into_iter()
            .any(|e| e.key.as_deref() == Some("not-built") && !e.exiting);
        if built_not {
            placeholders.push(pane.label());
        }
    }
    let _state: &State = &rt.state;
    assert!(placeholders.is_empty(), "panes drawing the not-built frame: {placeholders:?}");
}
