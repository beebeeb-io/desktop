//! This PC's transfer limits at the engine: merged into the settings file and nothing else, read back as written,
//! and never written for a session that moved on. The file is each test's own, except the one test of the session
//! rule, which writes speeds too high to slow any transfer and puts back what it found.

use std::sync::Arc;

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;

type TestEngine = Engine<MemoryCredentials>;

#[test]
fn a_change_is_merged_into_the_file_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop.toml");
    // No file yet: the defaults (no speed limit).
    let defaults = transfer_limits_at(&path).unwrap();
    assert_eq!((defaults.upload_kib_per_s, defaults.download_kib_per_s), (0, 0));

    // Another setting in the file, to see it stay.
    DesktopConfig::update_at(&path, |config| {
        config.pause_sync = true;
        (true, ())
    })
    .unwrap();

    let after = set_transfer_limits_at(
        &path,
        TransferLimitsChange {
            upload_kib_per_s: Some(244),
            ..TransferLimitsChange::default()
        },
    )
    .unwrap();
    assert_eq!(after.upload_kib_per_s, 244);
    assert_eq!(
        after.download_kib_per_s, 0,
        "a limit that was not named keeps its value"
    );
    assert_eq!(after.downloaded_copies_bytes, defaults.downloaded_copies_bytes);

    let after = set_transfer_limits_at(
        &path,
        TransferLimitsChange {
            download_kib_per_s: Some(1_220),
            downloaded_copies_bytes: Some(5_000_000_000),
            ..TransferLimitsChange::default()
        },
    )
    .unwrap();
    assert_eq!(
        after,
        TransferLimits {
            upload_kib_per_s: 244,
            download_kib_per_s: 1_220,
            downloaded_copies_bytes: 5_000_000_000,
        }
    );
    assert_eq!(transfer_limits_at(&path).unwrap(), after, "the file holds it");
    let config = DesktopConfig::load_from(&path).unwrap();
    assert!(config.pause_sync, "no other setting is touched");
    assert_eq!(config.local_cache_limit_for_eviction(), Some(5_000_000_000));

    // No limit: 0, which eviction reads as unlimited. A negative number is no limit too.
    let after = set_transfer_limits_at(
        &path,
        TransferLimitsChange {
            upload_kib_per_s: Some(0),
            downloaded_copies_bytes: Some(-4),
            ..TransferLimitsChange::default()
        },
    )
    .unwrap();
    assert_eq!((after.upload_kib_per_s, after.downloaded_copies_bytes), (0, 0));
    assert_eq!(
        DesktopConfig::load_from(&path)
            .unwrap()
            .local_cache_limit_for_eviction(),
        None
    );
}

#[test]
fn a_change_that_changes_nothing_does_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop.toml");
    let same = set_transfer_limits_at(&path, TransferLimitsChange::default()).unwrap();
    assert_eq!(same, transfer_limits_at(&path).unwrap());
    assert!(!path.exists(), "nothing to save: no file is made");
}

fn engine() -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

fn signed_in(engine: &TestEngine) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "an-invented-token".into(),
            master_key: [7; 32],
            email: Some("a@example.eu".into()),
        });
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

/// Speeds no transfer reaches: a test that runs beside this one is not slowed by them.
const HIGH: u64 = 4_000_000_000;

#[test]
fn only_the_session_of_the_press_changes_the_limits() {
    let engine = engine();
    let found = engine.transfer_limits().unwrap();
    let change = TransferLimitsChange {
        upload_kib_per_s: Some(HIGH),
        ..TransferLimitsChange::default()
    };

    // Nobody signed in: nothing is written.
    assert_eq!(
        engine.set_transfer_limits_for(engine.session_turn(), change),
        Err(TransferLimitsError::SessionChanged)
    );
    assert_eq!(engine.transfer_limits().unwrap(), found);

    // The session of the press: written, and read back.
    let press = signed_in(&engine);
    let after = engine.set_transfer_limits_for(press, change).unwrap();
    assert_eq!(after.upload_kib_per_s, HIGH);
    assert_eq!(engine.transfer_limits().unwrap().upload_kib_per_s, HIGH);

    // The session moved on (a lock, a sign-out): a change asked under the earlier one writes nothing.
    engine.forget_session(SessionTransition::SignOut);
    let later = TransferLimitsChange {
        upload_kib_per_s: Some(HIGH - 1),
        ..TransferLimitsChange::default()
    };
    assert_eq!(
        engine.set_transfer_limits_for(press, later),
        Err(TransferLimitsError::SessionChanged)
    );
    assert_eq!(engine.transfer_limits().unwrap().upload_kib_per_s, HIGH, "untouched");

    // A new session of its own may; and the file is put back as it was found.
    let again = signed_in(&engine);
    assert_eq!(
        engine.set_transfer_limits_for(press, later),
        Err(TransferLimitsError::SessionChanged),
        "the earlier press stays refused under a later session"
    );
    let restored = engine
        .set_transfer_limits_for(
            again,
            TransferLimitsChange {
                upload_kib_per_s: Some(found.upload_kib_per_s),
                ..TransferLimitsChange::default()
            },
        )
        .unwrap();
    assert_eq!(restored, found);
}
