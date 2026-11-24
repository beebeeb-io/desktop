//! 079b: the thumbnail cache is empty after a sign-out and after an account switch. Every name is invented.

use super::*;
use crate::account::SessionTransition;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::thumb_cache::ThumbCache;

const FILE: &str = "0190d8a6-1b2c-7d3e-8f40-0123456789ab";

fn engine_with_a_session(account: &str) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = super::super::EngineConfig {
        account_id: account.into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens");
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() =
            Some(Session { token: "an-invented-token".into(), master_key: [7; 32], email: None });
    }
    engine
}

fn keep_one(engine: &Engine<MemoryCredentials>) {
    let master = MasterKey::from_bytes([7; 32]);
    let cache = ThumbCache::new(&engine.config.state_dir, &engine.config.account_id, &master, DEFAULT_CAP_MB);
    cache.put(FILE, 1, 2, 2, &[1u8; 16]).expect("kept");
    assert!(cache.get(FILE).is_some());
}

fn kept(engine: &Engine<MemoryCredentials>) -> bool {
    let master = MasterKey::from_bytes([7; 32]);
    ThumbCache::new(&engine.config.state_dir, &engine.config.account_id, &master, DEFAULT_CAP_MB)
        .get(FILE)
        .is_some()
}

fn folder_files(engine: &Engine<MemoryCredentials>) -> usize {
    fn count(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| if e.path().is_dir() { count(&e.path()) } else { 1 }).sum())
            .unwrap_or(0)
    }
    // This account's folder: the scratch state dir is the whole test process's, and other tests keep thumbnails of
    // their own accounts in it at the same time.
    count(&crate::thumb_cache::account_dir(
        &engine.config.state_dir,
        &engine.config.account_id,
    ))
}

#[test]
fn a_sign_out_leaves_the_thumb_cache_empty() {
    let engine = engine_with_a_session("an-invented-slot-thumb-out");
    keep_one(&engine);
    assert!(folder_files(&engine) > 0);
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(folder_files(&engine), 0, "no thumbnail file is left");
    assert!(!kept(&engine));
}
