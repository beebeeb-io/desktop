//! Two-factor and email change at the engine: over a real local socket (the bearer token goes with each request, a
//! refused code is a fixed kind, a wrong code does not end the session), and the session rule.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;

type TestEngine = Engine<MemoryCredentials>;

/// One request seen by the fake server: "POST /path" and whether it carried the session's bearer token.
type Seen = Arc<Mutex<Vec<(String, bool)>>>;

/// Answers each request by its path from `routes` ((path, status, body)); 404 for any other.
fn serve(routes: Vec<(&'static str, u16, &'static str)>) -> (String, Seen) {
    serve_after(routes, 0)
}

/// [`serve`], answering each request `delay_ms` after it was read (the request has left, the answer is on its way).
fn serve_after(routes: Vec<(&'static str, u16, &'static str)>, delay_ms: u64) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .to_lowercase()
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if raw.len() >= split + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&raw).to_string();
            let first = text.lines().next().unwrap_or("").to_string();
            let mut parts = first.split(' ');
            let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let bearer = text.to_lowercase().contains("authorization: bearer a-token");
            log.lock().unwrap().push((format!("{method} {path}"), bearer));
            let (status, body) = routes
                .iter()
                .find(|(p, _, _)| *p == path)
                .map(|(_, s, b)| (*s, *b))
                .unwrap_or((404, "{}"));
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (base, seen)
}

fn engine(api_base: &str) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: api_base.into(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

fn signed_in(engine: &TestEngine) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "a-token".into(),
            master_key: [7; 32],
            email: Some("a@example.eu".into()),
        });
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

const SETUP: &str = r#"{"secret":"JBSWY3DPEHPK3PXP","qr_uri":"otpauth://totp/x?secret=JBSWY3DPEHPK3PXP","backup_codes":["11111111","22222222"]}"#;

#[test]
fn a_set_up_starts_and_the_first_code_turns_it_on_with_the_session_s_token() {
    let (base, seen) = serve(vec![
        ("/api/v1/auth/2fa/setup", 200, SETUP),
        ("/api/v1/auth/2fa/enable", 200, "{}"),
    ]);
    let engine = engine(&base);
    let session = signed_in(&engine);
    // The owner a sign-in records (a session with none asks Beebeeb first: the last test of this file).
    engine.remember_session_user_id(Some("user-a".into()));
    let pending = engine.two_factor_begin(session, None).unwrap();
    assert_eq!(pending.key(), "JBSWY3DPEHPK3PXP");
    assert!(pending.otpauth_uri().starts_with("otpauth://"));
    let codes = engine.two_factor_turn_on(session, pending, "123456").unwrap();
    assert_eq!(codes.len(), 2);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter().all(|(_, bearer)| *bearer),
        "every request carried the session's token"
    );
}

#[test]
fn a_refused_code_is_a_fixed_kind_and_does_not_end_the_session() {
    let (base, _seen) = serve(vec![
        ("/api/v1/auth/2fa/setup", 200, SETUP),
        ("/api/v1/auth/2fa/enable", 401, r#"{"error":"invalid_code"}"#),
        ("/api/v1/auth/2fa/disable", 401, r#"{"error":"invalid_code"}"#),
    ]);
    let engine = engine(&base);
    let session = signed_in(&engine);
    let pending = engine.two_factor_begin(session, None).unwrap();
    assert_eq!(
        engine.two_factor_turn_on(session, pending, "000000").err(),
        Some(AccountFlowFailure::WrongCode)
    );
    for _ in 0..4 {
        assert_eq!(
            engine.two_factor_turn_off(session, "000000").err(),
            Some(AccountFlowFailure::WrongCode)
        );
    }
    assert!(
        !engine.account_session_refused(),
        "wrong codes are not a refused session"
    );
}

#[test]
fn nothing_is_sent_for_a_session_that_moved_on_or_without_one() {
    let (base, seen) = serve(vec![("/api/v1/auth/2fa/setup", 200, SETUP)]);
    let engine = engine(&base);
    assert_eq!(
        engine.two_factor_begin(SessionGeneration::for_test(1), None).err(),
        Some(AccountFlowFailure::NotSignedIn)
    );
    let first = signed_in(&engine);
    let second = signed_in(&engine);
    assert_ne!(first, second);
    assert_eq!(
        engine.two_factor_begin(first, None).err(),
        Some(AccountFlowFailure::NotSignedIn)
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "no request left for a session that is not the engine's"
    );
}

#[test]
fn an_unreachable_server_is_offline_not_a_wrong_code() {
    let engine = engine("http://127.0.0.1:9");
    let session = signed_in(&engine);
    assert_eq!(
        engine.two_factor_turn_off(session, "123456").err(),
        Some(AccountFlowFailure::Offline)
    );
    assert_eq!(
        engine.email_change_confirm(session, "12345678").err(),
        Some(AccountFlowFailure::Offline)
    );
}

#[test]
fn the_email_code_is_checked_before_anything_is_sent() {
    let (base, seen) = serve(vec![]);
    let engine = engine(&base);
    let session = signed_in(&engine);
    assert_eq!(
        engine.email_change_confirm(session, "12ab").err(),
        Some(AccountFlowFailure::WrongCode)
    );
    assert_eq!(
        engine.email_change_start(session, "not an address", "a password").err(),
        Some(AccountFlowFailure::NotAnAddress)
    );
    assert!(seen.lock().unwrap().is_empty());
}

/// Review of desktop#424: an answer that arrives after Beebeeb already changed two-factor reaches the person even
/// when the session's turn moved meanwhile (a screen lock does that) for the same account: the new backup codes are
/// not wiped unseen. A turn that moved because ANOTHER account signed in drops them.
#[test]
fn new_backup_codes_reach_the_person_when_the_turn_moves_while_the_request_is_out() {
    let codes = r#"{"backup_codes":["11111111","22222222"]}"#;
    let (base, seen) = serve_after(vec![("/api/v1/auth/2fa/backup-codes", 200, codes)], 600);
    let engine = engine(&base);
    let session = signed_in(&engine);
    *engine.acct.auth_email.lock().unwrap() = Some("a@example.eu".into());
    engine.remember_session_user_id(Some("user-a".into()));
    let moved = |engine: &TestEngine| {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    };
    let waited = |seen: &Seen| {
        for _ in 0..200 {
            if !seen.lock().unwrap().is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the request never left");
    };
    let answer = std::thread::scope(|scope| {
        let call = scope.spawn(|| engine.two_factor_new_backup_codes(session, "123456"));
        waited(&seen);
        moved(&engine);
        call.join().unwrap()
    });
    let codes = answer.expect("the codes Beebeeb made reach the person");
    assert_eq!(codes.len(), 2);

    // Another account signed in meanwhile: the answer is dropped.
    let session = engine.session_turn();
    seen.lock().unwrap().clear();
    let answer = std::thread::scope(|scope| {
        let call = scope.spawn(|| engine.two_factor_new_backup_codes(session, "123456"));
        waited(&seen);
        moved(&engine);
        // Another account signed in under the same address: told apart by its id, never by the email.
        engine.remember_session_user_id(Some("user-b".into()));
        call.join().unwrap()
    });
    assert_eq!(answer.err(), Some(AccountFlowFailure::NotSignedIn));

    // The session Beebeeb no longer accepts (401 again and again) while the request was out: dropped too.
    engine.remember_session_user_id(Some("user-a".into()));
    let session = engine.session_turn();
    seen.lock().unwrap().clear();
    let answer = std::thread::scope(|scope| {
        let call = scope.spawn(|| engine.two_factor_new_backup_codes(session, "123456"));
        waited(&seen);
        moved(&engine);
        for _ in 0..3 {
            engine.note_call_for_test(true);
        }
        call.join().unwrap()
    });
    assert!(engine.account_session_refused());
    assert_eq!(answer.err(), Some(AccountFlowFailure::NotSignedIn));
}

/// Review of desktop#424 (delta 2 P2): a session stored before the owner tag (task 2447) comes back from a restart
/// with no recorded owner id. A same-account turn move while "New backup codes" is out still delivers the codes
/// Beebeeb made: the account is known by its id (Beebeeb's answer for this session's token), never guessed.
#[test]
fn a_session_with_no_recorded_owner_still_gets_its_codes_after_a_same_account_move() {
    let codes = r#"{"backup_codes":["11111111","22222222"]}"#;
    let me = r#"{"user_id":"user-a","email":"a@example.eu","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}"#;
    let (base, seen) = serve_after(
        vec![
            ("/api/v1/auth/2fa/backup-codes", 200, codes),
            ("/api/v1/auth/me", 200, me),
        ],
        300,
    );
    let engine = engine(&base);
    let session = signed_in(&engine);
    *engine.acct.auth_email.lock().unwrap() = Some("a@example.eu".into());
    // No `remember_session_user_id`: what a restore of a session stored before the owner tag leaves.
    let answer = std::thread::scope(|scope| {
        let call = scope.spawn(|| engine.two_factor_new_backup_codes(session, "123456"));
        let mut left = false;
        for _ in 0..300 {
            if seen
                .lock()
                .unwrap()
                .iter()
                .any(|(route, _)| route.ends_with("/api/v1/auth/2fa/backup-codes"))
            {
                left = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(left, "the request never left");
        {
            let _writes = engine.writes.write().unwrap();
            engine
                .acct
                .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        }
        call.join().unwrap()
    });
    let codes = answer.expect("the codes Beebeeb made reach the person");
    assert_eq!(codes.len(), 2);
}
