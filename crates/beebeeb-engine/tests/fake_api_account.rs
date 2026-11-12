//! Integration tests for the fake Beebeeb API: one process, one invented account, the real OPAQUE and recovery
//! protocol, and every assertion proved to fail on a mutant (broken client).
//!
//! Tests the path: sign-in -> recovery phrase -> gate Ready -> start_sync -> the engine pulls the snapshot, decrypts
//! the names with the real master key, and state.db has COUNT 7 entries with the right decrypted names. Nothing
//! touches the OS credential store or the network: everything is in-memory and on loopback.
//!
//! ## macOS
//!
//! The TEST identity is deliberately refused on macOS (`EngineIdentity::refusal_on`): it would share the real
//! app's data there. The 4 tests that open an Engine are gated with `#[cfg(not(target_os = "macos"))]`; one
//! macOS-only test asserts the refusal.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
#[cfg(not(target_os = "macos"))]
use beebeeb_engine::state_db::StateDb;
use beebeeb_engine::state_paths;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, assert_loopback, require_loopback, test_account};

use zeroize::Zeroizing;

// ── Shared setup (once per process) ─────────────────────────────────────────────

struct TestHarness {
    account: Arc<FakeAccount>,
    phrase: String,
    fake: FakeApi,
    state_dir: PathBuf,
}

fn harness() -> &'static TestHarness {
    static HARNESS: OnceLock<TestHarness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let (account, phrase, fake) = test_account();
        let base = tempfile::tempdir().expect("temp dir for engine");
        let identity = &TEST;
        // init_from_app_local_data_dir works on every OS (it just sets a global); Engine::open is
        // what refuses on macOS.
        state_paths::init_from_app_local_data_dir(identity, base.path()).expect("init state dir");
        let state_dir = state_paths::beebeeb_state_dir_for(identity).expect("state dir");
        std::mem::forget(base);
        TestHarness {
            account,
            phrase,
            fake,
            state_dir,
        }
    })
}

/// Open an Engine<MemoryCredentials> against the shared fake. Not available on macOS (the test build is refused).
#[cfg(not(target_os = "macos"))]
fn fresh_engine() -> Arc<Engine<MemoryCredentials>> {
    let h = harness();
    let identity = &TEST;
    let account_id = beebeeb_engine::config::DesktopConfig::ensure_account_id_for(identity).expect("account id");
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id,
        state_dir: h.state_dir.clone(),
        api_base: h.fake.base.clone(),
    };
    let stores = MemoryCredentials::default();
    Arc::new(Engine::open(identity, config, host, stores).expect("engine opens"))
}

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

// ═══════════════════════════════════════════════════════════════════════════════
// macOS: the test build is refused (deliberate product guard)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    let h = harness();
    let identity = &TEST;
    let account_id = beebeeb_engine::config::DesktopConfig::ensure_account_id_for(identity).expect("account id");
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id,
        state_dir: h.state_dir.clone(),
        api_base: h.fake.base.clone(),
    };
    let stores = MemoryCredentials::default();
    let result = Engine::open(identity, config, host, stores);
    assert!(
        result.is_err_and(|words| words.contains("macOS")),
        "the test build must be refused on macOS"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 1. Loopback rule
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn the_fake_is_loopback_and_refuses_non_loopback() {
    let h = harness();
    assert_loopback(&h.fake.base);
    assert!(require_loopback(&h.fake.base).is_ok());
    assert!(require_loopback("http://10.0.0.1:3000").is_err());
    assert!(require_loopback("https://api.beebeeb.io").is_err());
    assert!(require_loopback("http://example.com:9").is_err());
}

// ═══════════════════════════════════════════════════════════════════════════════
// 2. Wrong password -> refused (real OPAQUE)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(not(target_os = "macos"))]
#[test]
fn wrong_password_refused_by_real_opaque() {
    let engine = fresh_engine();
    engine.restore();
    let result = engine.sign_in_password("test@example.eu", Zeroizing::new("wrong password".into()));
    assert!(
        result.is_err(),
        "wrong password is refused by the real OPAQUE protocol, not a string compare"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. Wrong recovery phrase -> gate stays NeedsRecoveryPhrase
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(not(target_os = "macos"))]
#[test]
fn wrong_phrase_not_ready() {
    let engine = fresh_engine();
    engine.restore();
    engine
        .sign_in_password("test@example.eu", Zeroizing::new("correct horse battery".into()))
        .expect("sign-in");
    let wrong = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong";
    let result = engine.unlock_with_phrase(Zeroizing::new(wrong.into()));
    assert!(result.is_err(), "wrong phrase is refused");
    assert_eq!(engine.gate(), GateState::NeedsRecoveryPhrase);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. Happy path: sign in -> recovery phrase -> gate Ready
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(not(target_os = "macos"))]
#[test]
fn happy_path_signin_phrase_gate_ready() {
    let h = harness();
    let engine = fresh_engine();
    engine.restore();
    assert_eq!(engine.gate(), GateState::SignedOut);

    let next = engine
        .sign_in_password("test@example.eu", Zeroizing::new("correct horse battery".into()))
        .expect("sign-in succeeds");
    assert_eq!(next, beebeeb_engine::signin::gate::NextAfterSignIn::RecoveryPhrase);
    assert_eq!(engine.gate(), GateState::NeedsRecoveryPhrase);

    engine
        .unlock_with_phrase(Zeroizing::new(h.phrase.clone()))
        .expect("phrase unlock succeeds");
    assert_eq!(engine.gate(), GateState::Ready, "gate is Ready after correct phrase");

    let routes = h.fake.routes();
    let opaque_start = routes
        .iter()
        .filter(|r| r.as_str() == "POST /api/v1/opaque/login-start")
        .count();
    let opaque_finish = routes
        .iter()
        .filter(|r| r.as_str() == "POST /api/v1/opaque/login-finish")
        .count();
    let profile = routes.iter().filter(|r| r.as_str() == "GET /api/v1/auth/me").count();
    let recovery = routes
        .iter()
        .filter(|r| r.as_str() == "POST /api/v1/auth/verify-recovery-check")
        .count();
    assert!(opaque_start >= 1, "OPAQUE login-start calls: {opaque_start}");
    assert!(opaque_finish >= 1, "OPAQUE login-finish calls: {opaque_finish}");
    assert!(profile >= 1, "profile fetch calls: {profile}");
    assert!(recovery >= 1, "recovery-check calls: {recovery}");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 5. Listing after pull: start_sync -> snapshot -> state.db has 7 decrypted names
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(not(target_os = "macos"))]
#[test]
fn listing_after_pull_has_7_decrypted_names() {
    let h = harness();
    let engine = fresh_engine();
    engine.restore();
    engine
        .sign_in_password("test@example.eu", Zeroizing::new("correct horse battery".into()))
        .expect("sign-in");
    engine
        .unlock_with_phrase(Zeroizing::new(h.phrase.clone()))
        .expect("phrase unlock");
    assert_eq!(engine.gate(), GateState::Ready);

    let sync_root = h.state_dir.join("sync-test-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let start = engine.start_sync(sync_root.clone());
    assert!(
        matches!(start, Ok(beebeeb_engine::app::SyncStart::Started)),
        "start_sync result: {start:?}"
    );

    let db_path = state_paths::state_db_path_from_state_dir(&h.state_dir);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut count = 0;
    while std::time::Instant::now() < deadline {
        if let Ok(db) = StateDb::open(&db_path) {
            if let Ok(files) = db.list_files() {
                count = files.len();
                if count >= 7 {
                    break;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    engine.stop();

    assert_eq!(count, 7, "state.db has 7 entries after the first sync pull");

    let db = StateDb::open(&db_path).expect("open state.db for assertions");
    let files = db.list_files().expect("list files");
    let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let expected = [
        "Documents",
        "Photos",
        "readme.txt",
        "notes.md",
        "report.pdf",
        "sunset.jpg",
        "portrait.png",
    ];
    for name in &expected {
        assert!(
            names.iter().any(|path| path.contains(name)),
            "expected decrypted name '{name}' in state.db, got: {names:?}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. Snapshot has 7 items with encrypted names (wire check, every OS)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn the_snapshot_has_7_entries_with_encrypted_names() {
    let (account, _phrase, fake) = test_account();
    let (status, body) = http_get(&fake.base, "/api/v1/sync/snapshot", &account.session_token);
    assert_eq!(status, 200);
    let nodes = body["nodes"].as_array().expect("nodes is an array");
    assert_eq!(nodes.len(), 7, "the tree has 7 items");
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
        let enc = node["name_encrypted"].as_str().expect("name_encrypted is a string");
        for pt in &plaintext_names {
            assert_ne!(enc, *pt, "node {} has plaintext name '{pt}'", node["id"]);
        }
        assert!(
            enc.len() > 20,
            "encrypted name suspiciously short ({} chars)",
            enc.len()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 7. The fake catches a plaintext name sent by the CLIENT (every OS)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn the_fake_catches_a_plaintext_name_in_create_folder() {
    let (account, _phrase, fake) = test_account();

    let (status, _) = http_send(
        &fake.base,
        "POST",
        "/api/v1/files/folder",
        &account.session_token,
        Some(serde_json::json!({
            "parent_id": null,
            "name_encrypted": "Documents",
        })),
    );
    assert_eq!(status, 200);

    let requests = fake.requests();
    let folder_req = requests
        .iter()
        .find(|r| r.route() == "POST /api/v1/files/folder")
        .expect("the create-folder request was logged");
    let body = folder_req.json();
    let name = body["name_encrypted"].as_str().unwrap();
    assert!(
        !account.is_not_plaintext(name),
        "the fake catches the plaintext name '{name}'"
    );

    let (_, snap) = http_get(&fake.base, "/api/v1/sync/snapshot", &account.session_token);
    let real_enc = snap["nodes"][0]["name_encrypted"].as_str().unwrap();
    assert!(account.is_not_plaintext(real_enc), "an encrypted name passes the check");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 8. Request counts after a scripted flow (every OS)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn request_counts_after_scripted_flow() {
    let (account, _phrase, fake) = test_account();

    let readme_id = "a0000000-0000-4000-8000-000000000003";
    let (s, _) = http_send(
        &fake.base,
        "DELETE",
        &format!("/api/v1/files/{readme_id}"),
        &account.session_token,
        None,
    );
    assert_eq!(s, 200);
    let (s, _) = http_send(
        &fake.base,
        "POST",
        &format!("/api/v1/files/{readme_id}/restore"),
        &account.session_token,
        None,
    );
    assert_eq!(s, 200);
    let (s, _) = http_send(
        &fake.base,
        "POST",
        "/api/v1/files/folder",
        &account.session_token,
        Some(serde_json::json!({"parent_id": null, "name_encrypted": "enc"})),
    );
    assert_eq!(s, 200);

    let routes = fake.routes();
    let trash = routes.iter().filter(|r| r.starts_with("DELETE /api/v1/files/")).count();
    let restore = routes
        .iter()
        .filter(|r| r.ends_with("/restore") && r.starts_with("POST /api/v1/files/"))
        .count();
    let create_folder = routes
        .iter()
        .filter(|r| r.as_str() == "POST /api/v1/files/folder")
        .count();
    let permanent_delete = routes.iter().filter(|r| r.ends_with("/permanent")).count();
    assert_eq!(trash, 1, "trash calls = 1");
    assert_eq!(restore, 1, "restore calls = 1");
    assert_eq!(create_folder, 1, "create-folder calls = 1");
    assert_eq!(permanent_delete, 0, "permanent-delete calls = 0");
}
