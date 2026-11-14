//! `Engine::shares_overview` and `Engine::share_change` on a real handle against the engine's fake account on
//! loopback, with the share routes answered here in the server's shape. Every name and address is invented.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Reply, Request};

const PASSWORD: &str = "an invented password for shares";
const TRIP: &str = "a0000000-0000-4000-8000-000000000401";
const PLAN: &str = "a0000000-0000-4000-8000-000000000402";
const THEIRS: &str = "a0000000-0000-4000-8000-000000000403";
const INV_A: &str = "b0000000-0000-4000-8000-000000000001";
const INV_B: &str = "b0000000-0000-4000-8000-000000000002";
const INV_OVER: &str = "b0000000-0000-4000-8000-000000000003";
const INV_IN: &str = "b0000000-0000-4000-8000-000000000004";
const INV_MINE: &str = "b0000000-0000-4000-8000-000000000005";
const INV_CLAIMED: &str = "b0000000-0000-4000-8000-000000000006";
const SECRETS: [&str; 4] = [
    "A-LINK-TOKEN",
    "AN-OWNER-WRAPPED-KEY",
    "A-WRAPPED-PRIVATE-KEY",
    "A-SEALED-KEY",
];

fn encrypted(account: &FakeAccount, id: &str, name: &str) -> String {
    let master = MasterKey::from_bytes(account.master_key);
    let key = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
    beebeeb_core::encrypt::encrypt_name_for(
        &key,
        name,
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .expect("encrypt name")
}

fn bodies(account: &FakeAccount) -> [Value; 4] {
    let sent = |id: &str, item: &str, name: &str, email: &str, status: &str, permission: &str| {
        json!({ "id": id, "file_id": item, "recipient_email": email, "status": status, "permission": permission,
            "file_name_encrypted": encrypted(account, item, name), "is_folder_share": item == TRIP,
            "encrypted_owner_folder_key": SECRETS[1] })
    };
    [
        json!({ "invites": [
            { "id": INV_IN, "file_id": THEIRS, "status": "invited", "sender_email": "sender@example.eu",
              "file_name_encrypted": "a plaintext field of the server", "is_folder": true, "size_bytes": 0,
              "created_at": "2026-11-01T10:00:00Z" },
            { "id": INV_MINE, "file_id": PLAN, "status": "approved", "sender_email": "other@example.eu",
              "file_name_encrypted": "{\"not\":\"for this key\"}", "encrypted_file_key": SECRETS[3],
              "sender_public_key": "AAAA", "size_bytes": 2048, "approved_at": "2026-11-02T10:00:00Z" },
            { "id": INV_CLAIMED, "file_id": TRIP, "status": "claimed", "sender_email": "other@example.eu" },
        ] }),
        json!({ "invites": [
            sent(INV_A, TRIP, "Invented trip", "one@example.eu", "approved", "write"),
            sent(INV_B, TRIP, "Invented trip", "two@example.eu", "invited", "read"),
            sent(INV_OVER, PLAN, "Invented plan.pdf", "gone@example.eu", "revoked", "read"),
        ] }),
        json!({ "shares": [
            { "id": INV_OVER, "file_id": TRIP, "token": SECRETS[0], "owner_wrapped_key": SECRETS[1],
              "expires_at": "2026-12-01T00:00:00Z", "has_passphrase": true },
            { "id": INV_IN, "file_id": TRIP, "token": SECRETS[0] },
            { "id": INV_MINE, "file_id": PLAN, "token": SECRETS[0], "revoked": true },
            { "id": INV_A, "file_id": THEIRS, "token": SECRETS[0] },
        ] }),
        json!({ "file_requests": [
            { "id": INV_A, "title": "Invented request", "files_received": 3, "max_files": 50, "closed": false,
              "token": SECRETS[0], "wrapped_private_key": SECRETS[2], "expires_at": "2026-11-14T00:00:00Z" },
            { "id": INV_B, "title": "Closed one", "max_files": 0, "closed": true },
        ] }),
    ]
}

struct Fake {
    api: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    /// The status every share route answers with (200: the bodies).
    status: Arc<AtomicU16>,
}

fn fake(tag: &str, empty: bool) -> Fake {
    let (account, phrase) = FakeAccount::register(&format!("{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let status = Arc::new(AtomicU16::new(200));
    let (answers, code) = (bodies(&account), status.clone());
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let at = [INCOMING, SENT, MINE, FILE_REQUESTS_PATH]
            .iter()
            .position(|path| route == format!("GET {path}"));
        if at.is_none() && !route.contains("/api/v1/shares/") && !route.contains("/api/v1/file-requests/") {
            return rest(req);
        }
        match (code.load(Ordering::SeqCst), at) {
            (200, Some(_)) if empty => Reply::ok(json!({})),
            (200, Some(at)) => Reply::ok(answers[at].clone()),
            (200, None) => Reply::ok(json!({})),
            (status, _) => Reply::json(status, json!({ "error": "a server body" })),
        }
    }));
    Fake {
        api,
        account,
        phrase,
        status,
    }
}

fn engine(fake: &Fake, sign_in: bool) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "shares-test-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: fake.api.base.clone(),
    };
    let engine = Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    if sign_in {
        engine
            .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
            .expect("the fake accepts the password");
        engine
            .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
            .expect("the phrase unlocks");
    }
    engine
}

fn share_routes(fake: &Fake) -> Vec<String> {
    fake.api
        .routes()
        .into_iter()
        .filter(|route| route.contains("/shares/") || route.contains("/file-requests"))
        .collect()
}

#[test]
fn the_overview_is_the_servers_rows_with_names_opened_here_and_no_key_material() {
    let fake = fake("overview", false);
    let engine = engine(&fake, true);
    let overview = engine.shares_overview().expect("the overview");
    assert_eq!(overview.session, engine.session_turn());
    assert_eq!(overview.owner_email.as_deref(), Some(fake.account.email.as_str()));

    // Incoming: the invitation waits, the approved one is shared with the account, the claimed one is neither. A
    // name that is not an envelope, or does not open, is the fixed words.
    let incoming: Vec<_> = overview
        .incoming
        .iter()
        .map(|i| (i.invite_id.as_str(), i.name.as_str(), i.waiting, i.is_folder))
        .collect();
    assert_eq!(
        incoming,
        [
            (INV_IN, ENCRYPTED_ITEM, true, true),
            (INV_MINE, ENCRYPTED_ITEM, false, false)
        ]
    );
    assert_eq!(overview.incoming[0].from_email, "sender@example.eu");
    assert_eq!(overview.incoming[1].shared_at, "2026-11-02T10:00:00Z");

    // By me: one share, its two people in the server's order; the revoked invite is no person and no share.
    assert_eq!(overview.by_me.len(), 1);
    let trip = &overview.by_me[0];
    assert_eq!(
        (trip.item_id.as_str(), trip.name.as_str(), trip.is_folder),
        (TRIP, "Invented trip", true)
    );
    let members: Vec<_> = trip
        .members
        .iter()
        .map(|m| (m.invite_id.as_str(), m.email.as_str(), m.can_edit, m.accepted))
        .collect();
    assert_eq!(
        members,
        [
            (INV_A, "one@example.eu", true, true),
            (INV_B, "two@example.eu", false, false)
        ]
    );

    // Links: one per item, never a revoked one; the name is the share's, else the fixed words.
    let links: Vec<_> = overview
        .links
        .iter()
        .map(|l| {
            (
                l.item_id.as_str(),
                l.name.as_str(),
                l.has_password,
                l.expires_at.is_some(),
            )
        })
        .collect();
    assert_eq!(
        links,
        [(TRIP, "Invented trip", true, true), (THEIRS, SHARED_ITEM, false, false)]
    );

    assert_eq!(overview.requests.len(), 2);
    assert_eq!(
        (
            overview.requests[0].files_received,
            overview.requests[0].max_files,
            overview.requests[0].closed
        ),
        (3, Some(50), false)
    );
    assert_eq!(
        (overview.requests[1].max_files, overview.requests[1].closed),
        (None, true)
    );

    let shown = format!("{overview:?}");
    for secret in SECRETS.iter().chain(&["a plaintext field", "a server body"]) {
        assert!(
            !shown.contains(secret),
            "nothing of the server's key fields is handed out"
        );
    }
}

#[test]
fn an_account_that_shares_nothing_reads_as_empty() {
    let fake = fake("nothing", true);
    let overview = engine(&fake, true).shares_overview().expect("an empty overview");
    assert!(overview.incoming.is_empty() && overview.by_me.is_empty());
    assert!(overview.links.is_empty() && overview.requests.is_empty());
}

#[test]
fn a_failure_is_its_kind_and_signed_out_asks_nothing() {
    let fake = fake("failures", false);
    let signed_out = engine(&fake, false);
    assert_eq!(signed_out.shares_overview(), Err(AccountCallError::NotSignedIn));
    let change = ShareChange::Accept {
        invite_id: INV_IN.into(),
    };
    assert_eq!(
        signed_out.share_change(signed_out.session_turn(), &change),
        Err(ShareChangeError::NotSignedIn)
    );
    assert!(share_routes(&fake).is_empty(), "nothing is asked without a session");

    let engine = engine(&fake, true);
    fake.status.store(500, Ordering::SeqCst);
    assert_eq!(engine.shares_overview(), Err(AccountCallError::ServerError));
    assert_eq!(
        engine.share_change(engine.session_turn(), &change),
        Err(ShareChangeError::Refused)
    );
    fake.status.store(401, Ordering::SeqCst);
    assert_eq!(engine.shares_overview(), Err(AccountCallError::SessionEnded));
    assert_eq!(
        engine.share_change(engine.session_turn(), &change),
        Err(ShareChangeError::SessionEnded)
    );
}

#[test]
fn a_change_is_one_request_for_the_session_it_was_decided_under() {
    let fake = fake("changes", false);
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    let accept = ShareChange::Accept {
        invite_id: INV_IN.into(),
    };
    let edit = ShareChange::SetAccess {
        invite_id: INV_B.into(),
        can_edit: true,
    };
    let remove = ShareChange::RemoveMember {
        invite_id: INV_A.into(),
    };
    for change in [&accept, &edit, &remove] {
        assert_eq!(engine.share_change(session, change), Ok(()));
    }
    assert_eq!(
        share_routes(&fake),
        [
            format!("POST /api/v1/shares/invites/{INV_IN}/claim"),
            format!("PATCH /api/v1/shares/invites/{INV_B}"),
            format!("DELETE /api/v1/shares/invites/{INV_A}"),
        ]
    );
    let patch = fake
        .api
        .requests()
        .into_iter()
        .find(|r| r.method == "PATCH")
        .expect("the PATCH");
    assert_eq!(patch.json()["permission"], "write");
    assert_eq!(patch.bearer(), Some(fake.account.session_token.as_str()));

    // Gone already: taking a person off is done; an invitation or a person to change is not found.
    fake.status.store(404, Ordering::SeqCst);
    assert_eq!(engine.share_change(session, &remove), Ok(()));
    assert_eq!(engine.share_change(session, &accept), Err(ShareChangeError::NotFound));
    assert_eq!(engine.share_change(session, &edit), Err(ShareChangeError::NotFound));
    fake.status.store(200, Ordering::SeqCst);

    // Not an id, or not this session any more: nothing is sent.
    let sent = share_routes(&fake).len();
    for id in ["", "owner", "B0000000-0000-4000-8000-000000000001", "../sent"] {
        let change = ShareChange::RemoveMember { invite_id: id.into() };
        assert_eq!(engine.share_change(session, &change), Err(ShareChangeError::NotFound));
    }
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks again");
    assert_ne!(
        engine.session_turn(),
        session,
        "a lock and an unlock is another session"
    );
    assert_eq!(
        engine.share_change(session, &remove),
        Err(ShareChangeError::NotSignedIn)
    );
    assert_eq!(share_routes(&fake).len(), sent);
}

#[test]
fn ending_a_share_is_the_requests_core_builds_one_per_item_and_an_unlisted_one_is_done() {
    let fake = fake("ends", false);
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    let changes = [
        ShareChange::Decline {
            invite_id: INV_IN.into(),
        },
        ShareChange::CancelInvites {
            invite_ids: vec![INV_A.into(), INV_B.into()],
        },
        ShareChange::RevokeLinks {
            share_ids: vec![INV_MINE.into(), INV_OVER.into()],
        },
        ShareChange::CloseRequest {
            request_id: INV_A.into(),
        },
    ];
    for change in &changes {
        assert_eq!(engine.share_change(session, change), Ok(()));
    }
    assert_eq!(
        share_routes(&fake),
        [
            format!("POST /api/v1/shares/invites/{INV_IN}/hide"),
            format!("DELETE /api/v1/shares/invites/{INV_A}"),
            format!("DELETE /api/v1/shares/invites/{INV_B}"),
            format!("DELETE /api/v1/shares/{INV_MINE}"),
            format!("DELETE /api/v1/shares/{INV_OVER}"),
            format!("POST /api/v1/file-requests/{INV_A}/close"),
        ]
    );
    // Already gone: ending a share or a link is done; declining or closing what is not there is not found.
    fake.status.store(404, Ordering::SeqCst);
    assert_eq!(engine.share_change(session, &changes[1]), Ok(()));
    assert_eq!(engine.share_change(session, &changes[2]), Ok(()));
    assert_eq!(
        engine.share_change(session, &changes[0]),
        Err(ShareChangeError::NotFound)
    );
    assert_eq!(
        engine.share_change(session, &changes[3]),
        Err(ShareChangeError::NotFound)
    );
    fake.status.store(500, Ordering::SeqCst);
    assert_eq!(
        engine.share_change(session, &changes[1]),
        Err(ShareChangeError::Refused)
    );
    fake.status.store(200, Ordering::SeqCst);

    // One bad id among several: nothing at all is sent.
    let sent = share_routes(&fake).len();
    let mixed = ShareChange::CancelInvites {
        invite_ids: vec![INV_A.into(), "../sent".into()],
    };
    assert_eq!(engine.share_change(session, &mixed), Err(ShareChangeError::NotFound));
    assert_eq!(share_routes(&fake).len(), sent);

    // The listing names a link by its own id and keeps every live link of an item.
    let overview = engine.shares_overview().expect("the overview");
    let ids: Vec<_> = overview
        .links
        .iter()
        .map(|l| (l.share_id.as_str(), l.share_ids.len()))
        .collect();
    assert_eq!(ids, [(INV_OVER, 2), (INV_A, 1)]);
}
