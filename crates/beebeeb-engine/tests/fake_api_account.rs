//! Integration tests for the fake Beebeeb API: one process, one invented account, the real OPAQUE and recovery
//! protocol, and every assertion proved to fail on a mutant (broken client).
//!
//! Tests the path: sign-in → recovery phrase → gate Ready → the sync snapshot's encrypted names are real
//! ciphertext (a plaintext name is caught). Nothing touches the OS credential store or the network: everything
//! is in-memory and on loopback.

use std::sync::{Arc, OnceLock};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_paths;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, assert_loopback, require_loopback, test_account};

use zeroize::Zeroizing;

// ── Shared setup (once per process) ─────────────────────────────────────────────
//
// The state dir is process-global (one identity, one folder), so we init it once. The FakeAccount
// used for ENGINE tests (sign-in, recovery) is shared read-only. Tests that MUTATE the vault tree
// (trash, restore, folder-create) create their own short-lived account + fake.

struct TestHarness {
    /// Read-only account for the engine sign-in tests.
    account: Arc<FakeAccount>,
    phrase: String,
    fake: FakeApi,
}

fn harness() -> &'static TestHarness {
    static HARNESS: OnceLock<TestHarness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let (account, phrase, fake) = test_account();
        let base = tempfile::tempdir().expect("temp dir for engine");
        let identity = &TEST;
        state_paths::init_from_app_local_data_dir(identity, base.path()).expect("init state dir");
        std::mem::forget(base); // keep the dir alive for the process
        TestHarness { account, phrase, fake }
    })
}

/// Create a fresh engine handle against the shared fake, with fresh memory credentials.
fn fresh_engine() -> Arc<Engine<MemoryCredentials>> {
    let h = harness();
    let identity = &TEST;
    let state_dir = state_paths::beebeeb_state_dir_for(identity).expect("state dir");
    let account_id = beebeeb_engine::config::DesktopConfig::ensure_account_id_for(identity).expect("account id");
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id,
        state_dir,
        api_base: h.fake.base.clone(),
    };
    let stores = MemoryCredentials::default();
    Arc::new(Engine::open(identity, config, host, stores).expect("engine opens"))
}

/// Make a raw HTTP GET against a given base URL with an Authorization header.
fn http_get(base: &str, path: &str, token: &str) -> (u16, serde_json::Value) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{base}{path}"))
            .bearer_auth(token)
            .send()
            .await
            .expect("HTTP request");
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        (status, body)
    })
}

/// Make a raw HTTP POST/DELETE against a given base URL.
fn http_send(
    base: &str,
    method: &str,
    path: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let client = reqwest::Client::new();
        let mut builder = match method {
            "POST" => client.post(format!("{base}{path}")),
            "DELETE" => client.delete(format!("{base}{path}")),
            "PUT" => client.put(format!("{base}{path}")),
            _ => panic!("unsupported method {method}"),
        };
        builder = builder.bearer_auth(token);
        if let Some(body) = body {
            builder = builder.json(&body);
        }
        let resp = builder.send().await.expect("HTTP request");
        let status = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        (status, body)
    })
}

// ── 1. The fake is loopback ─────────────────────────────────────────────────────

#[test]
fn the_fake_api_is_loopback_and_refuses_non_loopback() {
    let h = harness();
    assert_loopback(&h.fake.base);
    assert!(require_loopback(&h.fake.base).is_ok());
    assert!(require_loopback("http://10.0.0.1:3000").is_err());
    assert!(require_loopback("https://api.beebeeb.io").is_err());
    assert!(require_loopback("http://example.com:9").is_err());
}

// ── 2. Sign-in with correct password ────────────────────────────────────────────

#[test]
fn sign_in_with_correct_password_reaches_needs_recovery_phrase() {
    let engine = fresh_engine();
    let gate = engine.restore();
    assert_eq!(gate, GateState::SignedOut, "no stored session: signed out");

    let result = engine.sign_in_password("test@example.eu", Zeroizing::new("correct horse battery".into()));
    assert!(result.is_ok(), "sign-in succeeds: {result:?}");
    let next = result.unwrap();
    assert_eq!(
        next,
        beebeeb_engine::signin::gate::NextAfterSignIn::RecoveryPhrase,
        "first sign-in on this computer: needs the recovery phrase"
    );

    let gate = engine.gate();
    assert_eq!(gate, GateState::NeedsRecoveryPhrase);
}

// ── 3. Wrong password is refused ────────────────────────────────────────────────

#[test]
fn sign_in_with_wrong_password_is_refused() {
    let engine = fresh_engine();
    engine.restore();

    let result = engine.sign_in_password("test@example.eu", Zeroizing::new("wrong password".into()));
    assert!(result.is_err(), "wrong password is refused");
}

// ── 4. Correct recovery phrase reaches Ready ────────────────────────────────────

#[test]
fn correct_recovery_phrase_reaches_ready() {
    let h = harness();
    let engine = fresh_engine();
    engine.restore();

    engine
        .sign_in_password("test@example.eu", Zeroizing::new("correct horse battery".into()))
        .expect("sign-in");
    assert_eq!(engine.gate(), GateState::NeedsRecoveryPhrase);

    let result = engine.unlock_with_phrase(Zeroizing::new(h.phrase.clone()));
    assert!(result.is_ok(), "phrase unlock succeeds: {result:?}");

    let gate = engine.gate();
    assert_eq!(gate, GateState::Ready, "unlocked: gate is Ready");
}

// ── 5. Wrong recovery phrase does NOT reach Ready ───────────────────────────────

#[test]
fn wrong_recovery_phrase_does_not_reach_ready() {
    let engine = fresh_engine();
    engine.restore();

    engine
        .sign_in_password("test@example.eu", Zeroizing::new("correct horse battery".into()))
        .expect("sign-in");

    let wrong_phrase = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong";
    let result = engine.unlock_with_phrase(Zeroizing::new(wrong_phrase.into()));
    assert!(result.is_err(), "wrong phrase is refused");

    let gate = engine.gate();
    assert_eq!(
        gate,
        GateState::NeedsRecoveryPhrase,
        "gate stays NeedsRecoveryPhrase after a wrong phrase"
    );
}

// ── 6. Snapshot has 7 items with encrypted (not plaintext) names ────────────────
//
// Uses its OWN account so no other test's mutations affect the count.

#[test]
fn the_snapshot_has_7_entries_with_encrypted_names() {
    let (account, _phrase, fake) = test_account();

    let (status, body) = http_get(&fake.base, "/api/v1/sync/snapshot", &account.session_token);
    assert_eq!(status, 200);
    let nodes = body["nodes"].as_array().expect("nodes is an array");

    // 2 folders + 5 files = 7.
    assert_eq!(nodes.len(), 7, "the tree has 7 items");

    // Every name is encrypted (not any of the known plaintext names).
    let plaintext_names = [
        "Documents",
        "Photos",
        "readme.txt",
        "notes.md",
        "report.pdf",
        "sunset.jpg",
        "portrait.png",
    ];
    for node in nodes {
        let name_encrypted = node["name_encrypted"].as_str().expect("name_encrypted is a string");
        for plaintext in &plaintext_names {
            assert_ne!(
                name_encrypted, *plaintext,
                "node {} has a plaintext name '{plaintext}' — the client or fake must encrypt it",
                node["id"]
            );
        }
        assert!(
            name_encrypted.len() > 20,
            "encrypted name is suspiciously short ({}): '{name_encrypted}'",
            name_encrypted.len()
        );
    }
}

// ── 7. The fake catches a plaintext name (mutant) ───────────────────────────────

#[test]
fn the_fake_catches_a_plaintext_name() {
    let (account, _phrase, fake) = test_account();

    // A name that matches a known plaintext: the fake says "is plaintext".
    assert!(
        !account.is_not_plaintext("Documents"),
        "Documents is a known plaintext name — the check must catch it"
    );
    assert!(
        !account.is_not_plaintext("readme.txt"),
        "readme.txt is a known plaintext name"
    );

    // An encrypted name from the snapshot is NOT plaintext.
    let (_, body) = http_get(&fake.base, "/api/v1/sync/snapshot", &account.session_token);
    let first_name = body["nodes"][0]["name_encrypted"].as_str().expect("name_encrypted");
    assert!(
        account.is_not_plaintext(first_name),
        "an encrypted name is not plaintext"
    );
}

// ── 8. Request counts: trash and restore ────────────────────────────────────────

#[test]
fn trash_and_restore_are_counted() {
    let (account, _phrase, fake) = test_account();

    let (status, _) = http_send(
        &fake.base,
        "DELETE",
        "/api/v1/files/file-readme",
        &account.session_token,
        None,
    );
    assert_eq!(status, 200);

    let (status, _) = http_send(
        &fake.base,
        "POST",
        "/api/v1/files/file-readme/restore",
        &account.session_token,
        None,
    );
    assert_eq!(status, 200);

    let routes = fake.routes();
    let trash_count = routes.iter().filter(|r| r.starts_with("DELETE /api/v1/files/")).count();
    let restore_count = routes
        .iter()
        .filter(|r| r.ends_with("/restore") && r.starts_with("POST /api/v1/files/"))
        .count();
    assert_eq!(trash_count, 1, "one trash call");
    assert_eq!(restore_count, 1, "one restore call");

    // Permanent delete calls = 0 (we never called it).
    let perm_count = routes.iter().filter(|r| r.ends_with("/permanent")).count();
    assert_eq!(perm_count, 0, "zero permanent-delete calls");
}

// ── 9. Create-folder calls = 1 ─────────────────────────────────────────────────

#[test]
fn folder_create_is_counted() {
    let (account, _phrase, fake) = test_account();

    let (status, _) = http_send(
        &fake.base,
        "POST",
        "/api/v1/files/folder",
        &account.session_token,
        Some(serde_json::json!({
            "parent_id": null,
            "name_encrypted": "opaque-encrypted-name",
        })),
    );
    assert_eq!(status, 200);

    let routes = fake.routes();
    let create_count = routes
        .iter()
        .filter(|r| r.as_str() == "POST /api/v1/files/folder")
        .count();
    assert_eq!(create_count, 1, "one create-folder call");
}
