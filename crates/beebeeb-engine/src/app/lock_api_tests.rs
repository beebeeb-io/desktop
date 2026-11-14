//! The choke point of a person's call (`Engine::person_session`) and the census that keeps it the only one. The
//! behaviour behind a locked screen is in `tests/host_contract.rs`, against a session a real sign-in installed.

use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use super::PersonRefused;
use crate::account::SessionTransition;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;

fn engine_with_a_session() -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "an-invented-token".into(),
            master_key: [7; 32],
            email: None,
        });
    }
    engine
}

#[test]
fn a_person_call_takes_the_token_in_the_same_hold_as_the_locks_answer() {
    let engine = engine_with_a_session();
    let held = engine.writes.write().unwrap();
    let asker = engine.clone();
    let answer = std::thread::spawn(move || asker.person_session(|session, _| session.token.len()))
        .join()
        .unwrap();
    assert_eq!(
        answer,
        Err(PersonRefused::NoSession),
        "a session change under way is never waited for"
    );
    drop(held);
    assert_eq!(
        engine.person_session(|session, _| session.token.len()),
        Ok("an-invented-token".len())
    );
}

/// Lines of one source file that are not comments, with every whitespace character removed.
fn code_of(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(|line| line.chars())
        .filter(|c| !c.is_whitespace())
        .collect()
}

#[test]
fn every_person_accessor_goes_through_the_choke_point() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("app.rs")];
    for entry in std::fs::read_dir(src.join("app")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".rs") && !name.ends_with("_tests.rs") {
            files.push(path);
        }
    }
    assert!(files.len() >= 15, "{} source files scanned", files.len());
    let table: [(&str, &[(&str, usize)]); 5] = [
        (
            "acct.session.lock()",
            &[
                ("app.rs", 5),
                ("owned_state.rs", 1),
                ("lock_api.rs", 1),
                ("viewer_api.rs", 2),
            ],
        ),
        (
            "while_session_is(",
            &[
                ("owned_state.rs", 1),
                ("guide_mark.rs", 2),
                ("trash_api.rs", 1),
                ("photos_api.rs", 1),
                ("account_api.rs", 2),
            ],
        ),
        ("call_may_run(", &[("lock_api.rs", 2), ("owned_state.rs", 1)]),
        ("Call::Work", &[("owned_state.rs", 2)]),
        ("screen_refuses_a_person", &[]),
    ];
    let mut failures = Vec::new();
    for (needle, allowed) in table {
        for file in &files {
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            let count = code_of(file).matches(needle).count();
            let expected = allowed.iter().find(|(n, _)| *n == name).map_or(0, |(_, c)| *c);
            if count != expected {
                failures.push(format!("{needle} in {name}: {count}, expected {expected}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
