//! `Engine::share_change`'s answer to a 403 (ported from the Tauri shell's
//! `share_v2/reads_tests.rs::a_403_on_set_access_is_not_a_signed_out_session`): a 403 is Beebeeb refusing this
//! account the change (a frozen account, sharing paused), never a session that ended. Against the fake API on
//! loopback, every share route answering the status under test; the session is put in memory as a sign-in and an
//! unlock leave it. Every id is invented.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use serde_json::json;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::{FakeApi, Reply, Request};

const INVITE: &str = "b0000000-0000-4000-8000-000000000501";
const REQUEST: &str = "b0000000-0000-4000-8000-000000000502";

fn engine_at(base: &str) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-shares-403-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens");
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: "an-invented-session-token".into(),
        master_key: [5; 32],
        email: Some("owner@example.eu".into()),
    });
    drop(_writes);
    engine.remember_session_user_id(Some("d0000000-0000-4000-8000-000000000501".into()));
    engine.set_auth_present(true);
    engine
}

#[test]
fn a_403_on_set_access_is_not_a_signed_out_session() {
    let status = Arc::new(AtomicU16::new(200));
    let code = status.clone();
    let api = FakeApi::start(Arc::new(move |_req: &Request| {
        match code.load(Ordering::SeqCst) {
            200 => Reply::ok(json!({})),
            // The body an account-frozen refusal carries.
            403 => Reply::json(403, json!({ "error": "account_frozen" })),
            other => Reply::json(other, json!({ "error": "a server body" })),
        }
    }));
    let engine = engine_at(&api.base);
    let session = engine.session_turn();
    let changes = [
        ShareChange::SetAccess {
            invite_id: INVITE.into(),
            can_edit: true,
        },
        ShareChange::RemoveMember {
            invite_id: INVITE.into(),
        },
        ShareChange::Accept {
            invite_id: INVITE.into(),
        },
        ShareChange::CloseRequest {
            request_id: REQUEST.into(),
        },
    ];
    let mut checked = 0;
    for (answer, want) in [
        (200, Ok(())),
        (403, Err(ShareChangeError::Refused)),
        (401, Err(ShareChangeError::SessionEnded)),
        (500, Err(ShareChangeError::Refused)),
    ] {
        status.store(answer, Ordering::SeqCst);
        for change in &changes {
            let sent_before = api.routes().len();
            let got = engine.share_change(session, change);
            assert_eq!(got, want, "{change:?} answered {answer}");
            assert_eq!(api.routes().len(), sent_before + 1, "one request for {change:?}");
            checked += 1;
        }
    }
    assert_eq!(checked, 16);
    // The case the Tauri test named, once more by itself: a frozen account is not a signed-out session.
    status.store(403, Ordering::SeqCst);
    let set_access = engine.share_change(session, &changes[0]);
    assert_ne!(set_access, Err(ShareChangeError::SessionEnded));
    assert_eq!(set_access, Err(ShareChangeError::Refused));
    assert!(
        api.routes()
            .iter()
            .any(|route| route == &format!("PATCH /api/v1/shares/invites/{INVITE}")),
        "set access is the PATCH of the invite"
    );
}
