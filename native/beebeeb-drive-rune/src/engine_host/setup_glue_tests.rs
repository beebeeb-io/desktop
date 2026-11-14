//! The engine's own first-run calls, once, on a real handle against the fake API: the gate the shell reads from
//! the live engine, the settings each call writes, and the session rule of the writes. Every name is invented.

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::app::setup_api::decide_first_run;
use beebeeb_engine::config::{CONFIG_DIR_OVERRIDE, DesktopConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::LiveEngine;
use crate::engine_host::open::Opened;

const PASSWORD: &str = "an invented password for setup";

type Messages = std::sync::mpsc::Receiver<beebeeb_engine::host::HostMessage>;

/// A handle on a fake API of its own, signed in and unlocked.
fn signed_in() -> (Arc<Engine<MemoryCredentials>>, FakeApi, String) {
    let (engine, api, phrase, _messages) = signed_in_with_messages();
    (engine, api, phrase)
}

fn signed_in_with_messages() -> (Arc<Engine<MemoryCredentials>>, FakeApi, String, Messages) {
    let (account, phrase) = FakeAccount::register(FIRST, PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let api = FakeApi::start(account_handler(account.clone()));
    let (host, messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "setup-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("opens"));
    engine.restore();
    engine
        .sign_in_password(FIRST, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.clone()))
        .expect("the phrase unlocks");
    (engine, api, phrase, messages)
}

fn live_on(engine: &Arc<Engine<MemoryCredentials>>, messages: Messages) -> Arc<LiveEngine<MemoryCredentials>> {
    let opened = Opened {
        engine: engine.clone(),
        identity: &TEST,
        gate: engine.gate(),
    };
    Arc::new(LiveEngine::new(&opened, messages))
}

/// The settings file of this test alone (the process's is shared with every other test). Only this thread's
/// reads and writes go there: a worker's go to the process's file.
fn in_own_settings<T>(run: impl FnOnce() -> T) -> T {
    let dir = tempfile::tempdir().expect("a folder for the settings file");
    CONFIG_DIR_OVERRIDE.sync_scope(dir.path().to_path_buf(), run)
}

fn file() -> DesktopConfig {
    DesktopConfig::load_for(&TEST).expect("the settings file reads")
}

/// Contract S2: fresh settings and an engine that is Ready show the first run; after "Open my Beebeeb" the main
/// window; and the first run does not come back, for a lock and an unlock, or for the next start of the app.
#[test]
fn first_ready_on_a_new_pc_is_first_run() {
    use crate::shell::actions::EngineActions;
    let (engine, _api, phrase, messages) = signed_in_with_messages();
    let live = live_on(&engine, messages);
    assert_eq!(live.session().gate, ShellGate::Ready, "nobody wired a first run");
    in_own_settings(|| {
        assert_eq!(decide_first_run(&TEST), Ok(true), "as the app's open() does");
        live.wire_first_run();
    });
    assert_eq!(live.session().gate, ShellGate::FirstRun, "due, signed in and unlocked");
    // Locked: the lock screen, never the first run. Unlocked: the first run again.
    engine.lock();
    assert_eq!(live.session().gate, ShellGate::Locked);
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(live.session().gate, ShellGate::FirstRun);

    // "Open my Beebeeb", through the source the app makes from the live engine's own hooks: the gate that the
    // shell reads is the one the source ends.
    let hooks = live.setup_hooks(Arc::new(FakePlatform::default()));
    let mut source = EngineSetupSource::new(engine.clone(), hooks);
    source.finish();
    let began = Instant::now();
    while live.session().gate != ShellGate::Ready {
        assert!(began.elapsed() < Duration::from_secs(10), "the first run did not end");
        std::thread::sleep(Duration::from_millis(10));
        source.poll();
    }
    // And it stays ended for this run of the app: a lock and an unlock show the main window.
    engine.lock();
    assert_eq!(live.session().gate, ShellGate::Locked);
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    assert_eq!(live.session().gate, ShellGate::Ready);

    // The next start of the app, on settings that say the first run is done: none.
    in_own_settings(|| {
        assert_eq!(decide_first_run(&TEST), Ok(true));
        assert_eq!(engine.mark_first_run_done_for(engine.session_turn()), Ok(()));
        assert_eq!(decide_first_run(&TEST), Ok(false), "decided once: done stays done");
        let (_other_host, other_messages) = ChannelHost::new(|| {});
        let restarted = live_on(&engine, other_messages);
        restarted.wire_first_run();
        assert_eq!(restarted.session().gate, ShellGate::Ready);
    });
}

/// Contract S2: settings that name a sync folder and say nothing of the native first run are an install 0.8.x
/// set up. The app's open() marks it done before anything else writes, and its gate is Ready.
#[test]
fn an_08x_install_never_sees_the_first_run() {
    use crate::shell::actions::EngineActions;
    let (engine, _api, _phrase, messages) = signed_in_with_messages();
    let live = live_on(&engine, messages);
    in_own_settings(|| {
        DesktopConfig::update_for(&TEST, |config| {
            config.sync_root = Some(PathBuf::from("/invented/home/Beebeeb"));
            (true, ())
        })
        .expect("written");
        assert_eq!(file().native_setup_done, None);
        assert_eq!(decide_first_run(&TEST), Ok(false));
        assert_eq!(file().native_setup_done, Some(true), "open() wrote the flag");
        live.wire_first_run();
    });
    assert_eq!(live.session().gate, ShellGate::Ready);

    // The call is in open(), before the first write of the settings there (the account slot's id).
    let open = include_str!("open.rs");
    let body = &open[open.find("pub fn open<").expect("open()")..];
    let decided = body
        .find("setup_api::decide_first_run(identity)?")
        .expect("open() decides the first run");
    let first_write = body.find("ensure_account_id_for(identity)").expect("the account slot");
    assert!(
        decided < first_write,
        "decided before anything else writes the settings"
    );
}

/// Contract S2: "Open my Beebeeb" writes its flag and nothing else.
#[test]
fn finish_writes_only_its_flag() {
    let (engine, _api, _phrase) = signed_in();
    in_own_settings(|| {
        assert_eq!(decide_first_run(&TEST), Ok(true));
        let path = DesktopConfig::path_for(&TEST).expect("the settings file's place");
        let text = || std::fs::read_to_string(&path).expect("the settings file as text");
        let before = text();
        assert!(before.contains("native_setup_done = false"), "{before}");
        assert_eq!(SetupEngine::mark_done_for(&*engine, engine.session_turn()), Ok(()));
        let after = text();
        assert_eq!(
            after,
            before.replace("native_setup_done = false", "native_setup_done = true"),
            "one line differs"
        );
        assert_eq!(file().sync_root, None, "it names no sync folder");
        assert!(file().known_folder_backup.is_empty(), "and turns on no backup");
    });
}

/// The folder calls of the adapter are the engine's: the settings' folder, the suggestion, the rule, the one write.
#[test]
fn the_folder_calls_reach_the_engine() {
    let (engine, _api, _phrase) = signed_in();
    in_own_settings(|| {
        assert_eq!(SetupEngine::settings_folder(&*engine), Ok(None));
        let suggested = SetupEngine::suggested_folder(&*engine);
        assert!(
            suggested.ends_with(TEST.sync_folder_name()),
            "the identity's own folder name"
        );
        // The rule is the engine's: the settings' own folder is Beebeeb's data.
        let settings = DesktopConfig::path_for(&TEST).expect("the settings file's place");
        assert_eq!(
            SetupEngine::check_folder(&*engine, settings.parent().expect("its folder")),
            Err(SyncFolderRefused::OwnData)
        );
        assert_eq!(
            SetupEngine::check_folder(&*engine, Path::new("not-a-full-path")),
            Err(SyncFolderRefused::NotAFullPath)
        );
        // Written once: the first folder stays.
        let (first, second) = (PathBuf::from("/invented/first"), PathBuf::from("/invented/second"));
        assert_eq!(SetupEngine::keep_folder(&*engine, &first), Ok(true));
        assert_eq!(SetupEngine::keep_folder(&*engine, &second), Ok(false));
        assert_eq!(SetupEngine::settings_folder(&*engine), Ok(Some(first.clone())));
        assert_eq!(SetupEngine::sync_folder(&*engine), Some(first));
    });
}

#[test]
fn the_first_run_flag_and_the_notifications_switch_are_written_for_the_session_of_the_press_only() {
    let (engine, _api, phrase) = signed_in();
    in_own_settings(|| {
        let turn = engine.session_turn();
        // A PC that never had a first run: it is due, and nothing was written by asking.
        assert_eq!(Engine::first_run_due(&*engine), Ok(true));
        assert_eq!(file().native_setup_done, None);
        assert!(
            file().notify_allow,
            "precondition: notifications are allowed by default"
        );
        // The switch, both ways, for this session.
        assert_eq!(engine.set_notifications_allowed_for(turn, false), Ok(()));
        assert!(!file().notify_allow);
        assert_eq!(engine.set_notifications_allowed_for(turn, true), Ok(()));
        assert!(file().notify_allow);
        assert_eq!(
            Engine::first_run_due(&*engine),
            Ok(true),
            "a switch does not end the first run"
        );

        // Another session (a lock and an unlock): a press stamped with the old one writes nothing.
        engine.lock();
        assert_eq!(
            engine.set_notifications_allowed_for(turn, false),
            Err(SetupRefused::SessionMoved)
        );
        assert_eq!(engine.mark_first_run_done_for(turn), Err(SetupRefused::SessionMoved));
        engine
            .unlock_with_phrase(Zeroizing::new(phrase.clone()))
            .expect("the phrase unlocks");
        assert_eq!(
            engine.set_notifications_allowed_for(turn, false),
            Err(SetupRefused::SessionMoved)
        );
        assert_eq!(engine.mark_first_run_done_for(turn), Err(SetupRefused::SessionMoved));
        assert!(file().notify_allow, "nothing was written");
        assert_eq!(Engine::first_run_due(&*engine), Ok(true), "nothing was written");

        // The session of now: the first run is done, and stays done (also for the adapter's own glue).
        let now = engine.session_turn();
        assert_eq!(SetupEngine::mark_done_for(&*engine, now), Ok(()));
        assert_eq!(SetupEngine::first_run_due(&*engine), Ok(false));
        assert_eq!(file().native_setup_done, Some(true), "the native first run's own flag");
        assert!(
            !file().known_folder_onboarding_seen,
            "the 0.8.x app's show-once flag is not this one"
        );
        assert_eq!(engine.mark_first_run_done_for(now), Ok(()), "twice is the same");
        assert_eq!(SetupEngine::set_notifications_for(&*engine, now, false), Ok(()));
        assert!(!file().notify_allow);
        // The glue's stamp is the handle's session and account.
        assert_eq!(SetupEngine::stamp(&*engine), (now, Some(FIRST.to_string())));
        assert_eq!(SetupEngine::view(&*engine).gate, GateState::Ready);
    });
}
