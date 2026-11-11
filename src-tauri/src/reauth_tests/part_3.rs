use super::super::*;
use crate::account::{AccountId, AccountRuntime, synthesize_single_account};
use crate::account_binding::Identity;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) fn profile(user_id: &str, email: &str) -> account_dto::AccountProfile {
    serde_json::from_str(&format!(
        r#"{{"user_id":"{user_id}","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#
    ))
    .unwrap()
}

pub(super) struct Fixture {
    pub state: AppState,
    pub acct: Arc<AccountRuntime>,
    pub dir: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let state = AppState::default();
        synthesize_single_account(&state, AccountId::new_v4());
        let acct = state.active_account().unwrap();
        Self {
            state,
            acct,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn id(&self) -> &str {
        self.acct.id.as_str()
    }

    /// Where this computer's local data is: a throwaway state dir.
    pub fn sources(&self) -> LocalSources {
        LocalSources::for_test(self.dir.path())
    }

    pub fn db(&self) -> state_db::StateDb {
        state_db::StateDb::open(self.dir.path().join("state.db")).unwrap()
    }

    /// The session in memory (and the flags that go with it).
    pub fn install_session(&self, token: &str, key: [u8; 32], email: Option<&str>) {
        *self.acct.session.lock().unwrap() = Some(Session {
            token: token.into(),
            master_key: key,
            email: email.map(str::to_string),
        });
        set_auth_present(&self.state, true);
    }

    pub fn classify(&self, profile: &account_dto::AccountProfile) -> (reauth::SignInKind, u64) {
        classify_sign_in(&self.state, &self.acct, &self.sources(), profile)
    }

    /// The decision with the server's proof, against the server at `base_url`.
    pub async fn decide(
        &self,
        profile: &account_dto::AccountProfile,
        token: &str,
        base_url: &str,
    ) -> (reauth::SignInKind, u64) {
        decide_sign_in(&self.state, &self.acct, &self.sources(), profile, token, base_url).await
    }

    /// Everything a sign-in could change on this Mac. `state.db` is looked at only if it exists (looking creates it).
    pub fn snapshot(&self) -> Snapshot {
        let id = self.id();
        let db = self.dir.path().join("state.db").exists().then(|| self.db());
        Snapshot {
            token: stored_token(id),
            key: stored_key(id),
            email: keychain_account_email(id),
            owner: db.as_ref().and_then(|db| db.owner().unwrap()),
            waiting: db.as_ref().map(|db| db.queued_or_staged_count().unwrap()),
            session: self
                .acct
                .session
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| (s.token.clone(), s.master_key, s.email.clone())),
            auth_present: *self.state.auth_present.lock().unwrap(),
            auth_email: self.acct.auth_email.lock().unwrap().clone(),
            cached: self
                .acct
                .cached_profile
                .lock()
                .unwrap()
                .as_ref()
                .map(|p| p.user_id.clone()),
            engine_state: self.acct.engine_state.lock().unwrap().clone(),
        }
    }
}

impl Fixture {
    /// [`Fixture::snapshot`] for a world whose `state.db` may be unreadable: the database is left out.
    pub fn snapshot_without_reading_state_db(&self) -> Snapshot {
        let id = self.id();
        Snapshot {
            token: stored_token(id),
            key: stored_key(id),
            email: keychain_account_email(id),
            owner: None,
            waiting: None,
            session: self
                .acct
                .session
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| (s.token.clone(), s.master_key, s.email.clone())),
            auth_present: *self.state.auth_present.lock().unwrap(),
            auth_email: self.acct.auth_email.lock().unwrap().clone(),
            cached: self
                .acct
                .cached_profile
                .lock()
                .unwrap()
                .as_ref()
                .map(|p| p.user_id.clone()),
            engine_state: self.acct.engine_state.lock().unwrap().clone(),
        }
    }
}

/// A Mac's whole account state: the Keychain, the local database, memory and the flags.
#[derive(Debug, PartialEq)]
pub(super) struct Snapshot {
    token: Option<String>,
    key: Option<[u8; 32]>,
    email: Option<String>,
    owner: Option<Identity>,
    waiting: Option<u64>,
    session: Option<(String, [u8; 32], Option<String>)>,
    auth_present: bool,
    auth_email: Option<String>,
    cached: Option<String>,
    engine_state: String,
}

/// A local HTTP server that answers every request with `status_line` and `body` and records each request it
/// received (request line, headers and body, as sent): a test sees what the server was asked, and with which token.
fn serving(status_line: &'static str, body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    serving_by(move |_| (status_line, body))
}

/// A server that confirms exactly one key: `200 {valid:true}` for that key's check, `invalid_recovery_phrase`
/// for any other request. Records each request, as [`serving`] does.
fn confirming_only(key: [u8; 32]) -> (String, Arc<Mutex<Vec<String>>>) {
    let check = recovery_check_of(key);
    serving_by(move |request| {
        if request.contains(&check) {
            KEY_IS_THIS_ACCOUNTS
        } else {
            KEY_IS_ANOTHER_ACCOUNTS
        }
    })
}

/// [`serving`], with the status line and body chosen per request by `respond` (given the request as received).
fn serving_by(
    respond: impl Fn(&str) -> (&'static str, &'static str) + Send + 'static,
) -> (String, Arc<Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let wanted = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + wanted {
                        break;
                    }
                }
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            let (status_line, body) = respond(&request);
            record.lock().unwrap().push(request);
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (base_url, seen)
}

const KEY_IS_THIS_ACCOUNTS: (&str, &str) = ("200 OK", r#"{"valid":true}"#);
const KEY_IS_ANOTHER_ACCOUNTS: (&str, &str) = ("400 Bad Request", r#"{"error":"invalid_recovery_phrase"}"#);
const SERVER_IS_BROKEN: (&str, &str) = ("500 Internal Server Error", r#"{"error":"boom"}"#);

/// The `recovery_check` the server is sent for a key.
fn recovery_check_of(key: [u8; 32]) -> String {
    encode_base64(&*beebeeb_core::opaque::compute_recovery_check(
        &beebeeb_core::kdf::MasterKey::from_bytes(key),
    ))
}

/// A base URL nothing listens on: the connection is refused at once.
fn nobody_listens() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    url
}

/// Poison a lock the way a panic while holding it does.
fn poison<T: Send>(mutex: &Mutex<T>) {
    std::thread::scope(|scope| {
        let _ = scope
            .spawn(|| {
                let _held = mutex.lock().unwrap();
                panic!("poisoned on purpose");
            })
            .join();
    });
}

pub(super) fn some_operation(db: &state_db::StateDb, op_id: &str) {
    db.enqueue_operation(&state_db::PendingOperation {
        op_id: op_id.into(),
        kind: state_db::OperationKind::UploadVersion,
        file_id: Some("file-r8".into()),
        parent_id: None,
        target_path: Some("/R8.txt".into()),
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    })
    .unwrap();
}

pub(super) fn stored_token(id: &str) -> Option<String> {
    load_session_token_from_keychain(id).unwrap()
}

pub(super) fn stored_key(id: &str) -> Option<[u8; 32]> {
    let mut vault = AuthVault::new(platform_keychain_store_for(id));
    vault.unlock().ok()?;
    let mut key = [0u8; 32];
    key.copy_from_slice(vault.master_key().ok()?);
    Some(key)
}

pub(super) fn put_email(id: &str, email: &str) {
    AuthVault::new(platform_keychain_store_for(id))
        .store_account_email(email)
        .unwrap();
}

pub(super) fn sam() -> account_dto::AccountProfile {
    profile("u-1", "sam@beebeeb.io")
}

pub(super) fn kim() -> account_dto::AccountProfile {
    profile("u-2", "kim@beebeeb.io")
}

// -- the token swap --

#[test]
fn a_new_token_replaces_only_the_token_in_memory() {
    let fx = Fixture::new();
    fx.install_session("old-token", [7u8; 32], Some("sam@beebeeb.io"));
    assert!(replace_session_token_in_memory(&SessionWrite::for_test(), &fx.acct, "new-token").unwrap());
    let guard = fx.acct.session.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert_eq!(session.token, "new-token");
    assert_eq!(session.master_key, [7u8; 32], "the keys stay");
    assert_eq!(session.email.as_deref(), Some("sam@beebeeb.io"));
}

#[test]
fn without_keys_in_memory_nothing_is_invented() {
    let fx = Fixture::new();
    assert!(!replace_session_token_in_memory(&SessionWrite::for_test(), &fx.acct, "new-token").unwrap());
    assert!(fx.acct.session.lock().unwrap().is_none());
    assert!(
        !name_session_in_memory(&SessionWrite::for_test(), &fx.acct, "sam@beebeeb.io").unwrap(),
        "no session, nobody to name"
    );
    assert!(fx.acct.session.lock().unwrap().is_none());
}

#[test]
fn a_re_sign_in_ends_the_revoked_streak_and_never_asks_finder_to_remove() {
    let fx = Fixture::new();
    fx.install_session("old-token", [7u8; 32], Some("sam@beebeeb.io"));
    for _ in 0..3 {
        fx.acct
            .auth_health
            .note_result(Some(&anyhow::anyhow!("HTTP 401 Unauthorized: session revoked")));
    }
    assert!(
        fx.acct.auth_health.is_expired(),
        "precondition: the session reads as revoked"
    );
    let (handle, mut rx) = finder_setup::driver::FinderSetupHandle::for_test(
        finder_setup::driver::FinderSetupView::initial(finder_setup::launch_location::LaunchLocation::Applications),
    );
    let _ = fx.state.finder_setup.set(handle);
    reauth_settle_flags(&SessionWrite::for_test(), &fx.state, &fx.acct, "sam@beebeeb.io");
    keys_arrived(&fx.state, KeysFrom::SignIn);
    assert!(!fx.acct.auth_health.is_expired());
    assert!(*fx.state.auth_present.lock().unwrap());
    let mut seen = 0;
    while let Ok(event) = rx.try_recv() {
        seen += 1;
        assert!(
            matches!(
                event,
                finder_setup::driver::Event::Trigger(finder_setup::core::Trigger::KeysArrived)
            ),
            "a re-sign-in only ever tells Finder that keys are here"
        );
    }
    assert_eq!(seen, 1, "and tells it once");
}

/// R8: the same account signs in again, with keys in memory and in the Keychain. Only the token changes, in
/// the Keychain and in memory; the key, the email, the queue, the owner and the cache all stay.
#[test]
fn a_same_account_re_sign_in_swaps_only_the_token_and_changes_nothing_else() {
    let fx = Fixture::new();
    let id = fx.id().to_string();
    persist_session_to_keychain(
        &SessionWrite::for_test(),
        &id,
        "tok-old",
        &[7u8; 32],
        Some("sam@beebeeb.io"),
    )
    .unwrap();
    fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
    let db = fx.db();
    db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
        .unwrap();
    some_operation(&db, "op-r8");
    let waiting = db.queued_or_staged_count().unwrap();
    assert_eq!(waiting, 1);

    // The ids match and a key is here, so the server proves the key first (spec §5.6); here it confirms it.
    assert_eq!(fx.classify(&sam()), (reauth::SignInKind::NeedsKeyProof, 1));
    assert_eq!(
        classify_sign_in_after(
            &fx.state,
            &fx.acct,
            &fx.sources(),
            &sam(),
            Some(reauth::KeyProof::Matches)
        ),
        (reauth::SignInKind::SameAccount, 1)
    );
    let unlocked = reauth_swap_token(
        &fx.state,
        &fx.acct,
        &fx.sources(),
        "tok-new",
        &sam(),
        None,
        &mut fx.acct.session_generation(),
    )
    .unwrap();

    assert!(unlocked, "the keys were here");
    assert_eq!(
        stored_token(&id).as_deref(),
        Some("tok-new"),
        "the Keychain holds the new token"
    );
    assert_eq!(stored_key(&id), Some([7u8; 32]), "and the same key");
    assert_eq!(keychain_account_email(&id).as_deref(), Some("sam@beebeeb.io"));
    {
        let guard = fx.acct.session.lock().unwrap();
        let session = guard.as_ref().unwrap();
        assert_eq!(
            (session.token.as_str(), session.master_key),
            ("tok-new", [7u8; 32]),
            "memory: the new token, the same keys"
        );
        assert_eq!(session.email.as_deref(), Some("sam@beebeeb.io"));
    }
    assert_eq!(
        db.queued_or_staged_count().unwrap(),
        waiting,
        "pending edits stay and upload afterwards"
    );
    assert_eq!(
        db.owner().unwrap(),
        Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io"))),
        "the same owner"
    );
    assert_eq!(
        fx.acct
            .cached_profile
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.user_id.clone())
            .as_deref(),
        Some("u-1")
    );
    assert!(*fx.state.auth_present.lock().unwrap());
    assert_eq!(fx.acct.auth_email.lock().unwrap().as_deref(), Some("sam@beebeeb.io"));
}

/// After a startup 401 (R9) the Keychain keeps the key and the email and no session is in memory: the same
/// account signing in again reloads that key, and nothing asks for the recovery phrase.
#[test]
fn a_same_account_re_sign_in_after_a_revoked_token_reloads_the_key_this_mac_kept() {
    let fx = Fixture::new();
    let id = fx.id().to_string();
    persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[5u8; 32]).unwrap();
    put_email(&id, "sam@beebeeb.io");
    fx.db()
        .set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
        .unwrap();
    assert!(fx.acct.session.lock().unwrap().is_none());

    // The ids match and the key is kept, so the server proves it first (spec §5.6); here it confirms it.
    assert_eq!(fx.classify(&sam()).0, reauth::SignInKind::NeedsKeyProof);
    assert_eq!(
        classify_sign_in_after(
            &fx.state,
            &fx.acct,
            &fx.sources(),
            &sam(),
            Some(reauth::KeyProof::Matches)
        )
        .0,
        reauth::SignInKind::SameAccount
    );
    let unlocked = reauth_swap_token(
        &fx.state,
        &fx.acct,
        &fx.sources(),
        "tok-new",
        &sam(),
        None,
        &mut fx.acct.session_generation(),
    )
    .unwrap();

    assert!(unlocked, "the kept key was loaded");
    let guard = fx.acct.session.lock().unwrap();
    let session = guard.as_ref().expect("the keys are in memory again");
    assert_eq!((session.token.as_str(), session.master_key), ("tok-new", [5u8; 32]));
    assert_eq!(session.email.as_deref(), Some("sam@beebeeb.io"));
    assert_eq!(stored_key(&id), Some([5u8; 32]), "the Keychain key is untouched");
}

/// Same account, no key on this Mac: the token is replaced and the person is sent to the recovery phrase.
#[test]
fn a_same_account_re_sign_in_without_a_key_here_asks_for_the_recovery_phrase() {
    let fx = Fixture::new();
    let id = fx.id().to_string();
    put_email(&id, "sam@beebeeb.io");
    fx.db()
        .set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
        .unwrap();

    let unlocked = reauth_swap_token(
        &fx.state,
        &fx.acct,
        &fx.sources(),
        "tok-new",
        &sam(),
        None,
        &mut fx.acct.session_generation(),
    )
    .unwrap();

    assert!(!unlocked, "no key here: the recovery phrase step follows");
    assert!(fx.acct.session.lock().unwrap().is_none(), "no keys are invented");
    assert_eq!(stored_token(&id).as_deref(), Some("tok-new"));
    assert_eq!(stored_key(&id), None);
    assert!(*fx.state.auth_present.lock().unwrap());
}

/// The session in memory is named by the server's spelling of the email, so the next engine start compares
/// the same identity as the owner record and keeps the data (a different spelling would read as another
/// account and reset the local data).
#[test]
fn a_re_sign_in_gives_the_session_the_servers_email_so_the_binding_keeps_the_data() {
    let fx = Fixture::new();
    let owner = Identity::new(Some("u-1"), Some("Sam@beebeeb.io"));
    fx.db().set_owner(&owner).unwrap();
    for (name, session_email) in [
        ("an unidentified session", None),
        ("an older spelling", Some("sam@beebeeb.io")),
    ] {
        fx.install_session("tok-old", [7u8; 32], session_email);
        let canonical = profile("u-1", "Sam@beebeeb.io");
        reauth_swap_token(
            &fx.state,
            &fx.acct,
            &fx.sources(),
            "tok-new",
            &canonical,
            None,
            &mut fx.acct.session_generation(),
        )
        .unwrap();
        let email = fx.acct.session.lock().unwrap().as_ref().unwrap().email.clone();
        assert_eq!(email.as_deref(), Some("Sam@beebeeb.io"), "{name}");
        let cached = fx.acct.cached_profile.lock().unwrap().clone();
        let identity = identity_of_session(email.as_deref(), cached.as_ref());
        assert_eq!(
            account_binding::same_account(&owner, &identity),
            Some(true),
            "{name}: the engine start keeps the data"
        );
    }
}

// -- the account check --

/// Against the real sources: each retained trace alone means the Mac is not fresh, and
/// another account never gets `Fresh`. An empty Mac is `Fresh`.
#[test]
fn the_account_check_reads_every_retained_trace_from_the_real_sources() {
    use reauth::SignInKind::{DifferentAccount, NeedsKeyProof};
    type Set = fn(&Fixture);
    // What each retained trace alone makes of another account signing in: a switch, except a vault key, stored
    // or in a session in memory, which the server is asked about (it names no account itself; spec §5.6).
    let cases: [(&str, Set, reauth::SignInKind); 9] = [
        (
            "a session in memory",
            |fx| {
                *fx.acct.session.lock().unwrap() = Some(Session {
                    token: "t".into(),
                    master_key: [1u8; 32],
                    email: None,
                })
            },
            NeedsKeyProof,
        ),
        ("auth present", |fx| set_auth_present(&fx.state, true), DifferentAccount),
        (
            "a Keychain token",
            |fx| persist_session_token_to_keychain(&SessionWrite::for_test(), fx.id(), "tok-a", None).unwrap(),
            DifferentAccount,
        ),
        (
            "a Keychain email",
            |fx| put_email(fx.id(), "a@beebeeb.io"),
            DifferentAccount,
        ),
        (
            "a vault key",
            |fx| persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[3u8; 32]).unwrap(),
            NeedsKeyProof,
        ),
        (
            "a recorded owner",
            |fx| fx.db().set_owner(&Identity::new(None, Some("a@beebeeb.io"))).unwrap(),
            DifferentAccount,
        ),
        (
            "a cached profile",
            |fx| *fx.acct.cached_profile.lock().unwrap() = Some(profile("u-a", "a@beebeeb.io")),
            DifferentAccount,
        ),
        (
            "a queued change",
            |fx| some_operation(&fx.db(), "op-trace"),
            DifferentAccount,
        ),
        (
            "a staged payload",
            |fx| {
                fx.db()
                    .track_staged_payload("/tmp/bb-trace-staged.bin", None, false)
                    .unwrap()
            },
            DifferentAccount,
        ),
    ];
    let empty = Fixture::new();
    assert_eq!(
        empty.classify(&kim()),
        (reauth::SignInKind::Fresh, 0),
        "nothing here: the first sign-in"
    );
    for (name, set, expected) in cases {
        let fx = Fixture::new();
        set(&fx);
        let (kind, _) = fx.classify(&kim());
        assert_ne!(
            kind,
            reauth::SignInKind::Fresh,
            "{name} alone is a trace: another account must not get Fresh"
        );
        assert_eq!(kind, expected, "{name}");
    }
}

/// An unreadable local database is a trace, never an absence, and it settles nothing: the sign-in is
/// `LocalDataUnreadable` (not `Fresh`, not a switch, not decided by the credentials; lead ruling 12 gave it its
/// own outcome, where Task 11 had `Unconfirmed`).
#[test]
fn an_unreadable_state_db_is_a_trace_and_settles_nothing() {
    let fx = Fixture::new();
    std::fs::write(
        fx.dir.path().join("state.db"),
        b"this is not a database, and it is long enough to be read",
    )
    .unwrap();
    assert_eq!(fx.classify(&kim()).0, reauth::SignInKind::LocalDataUnreadable);
    assert!(gather_local_facts(&fx.state, &fx.acct, &fx.sources()).unreadable);
    assert!(
        queued_or_staged_present(&fx.sources()),
        "unreadable counts as waiting data"
    );
    assert!(local_data_owner(&fx.sources()).is_err());
    // Each fail-closed answer is its own trace, so neither depends on the other.
    let facts = gather_local_facts(&fx.state, &fx.acct, &fx.sources());
    assert!(
        facts.traces.recorded_owner,
        "an owner that cannot be read counts as recorded"
    );
    assert!(
        facts.traces.queued_or_staged,
        "data that cannot be read counts as waiting"
    );
    assert_eq!(facts.owner, None, "and names nobody");
}

/// The state database file of a fixture, opened directly: to break and repair it as a transient fault would.
fn raw_state_db(fx: &Fixture) -> rusqlite::Connection {
    rusqlite::Connection::open(fx.dir.path().join("state.db")).unwrap()
}

/// A signed-in sam: credentials, a stored key, an owner record naming him, and one edit not uploaded yet.
fn sam_with_a_pending_edit() -> Fixture {
    let fx = Fixture::new();
    after_a_first_sign_in(&fx, true, true);
    let db = fx.db();
    db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
        .unwrap();
    some_operation(&db, "op-pending");
    fx
}

/// Lead ruling 12 (Task 12), end to end: a Mac whose owner record names sam by email only (another letter case)
/// and that stores a vault key asks the server to prove the key. When the server answers with exactly
/// `recovery_check_missing` (sam has no check on file), the email decides: sam signs in again in place, kim is a
/// switch. When it answers `invalid_recovery_phrase`, it is the switch, whatever the email says.
#[tokio::test]
async fn no_recovery_check_on_file_lets_the_email_decide_and_a_mismatch_is_still_the_switch() {
    use reauth::SignInKind::{DifferentAccount, NeedsKeyProof, SameAccount};
    let fx = Fixture::new();
    after_a_first_sign_in(&fx, false, true);
    fx.db().set_owner(&Identity::new(None, Some("Sam@Beebeeb.io"))).unwrap();
    assert_eq!(
        fx.classify(&sam()).0,
        NeedsKeyProof,
        "the premise: the server's proof is asked first"
    );
    let (missing, asked) = serving("400 Bad Request", r#"{"error":"recovery_check_missing"}"#);
    assert_eq!(
        fx.decide(&sam(), "tok-2", &missing).await.0,
        SameAccount,
        "no check on file: the same address decides"
    );
    assert_eq!(
        fx.decide(&kim(), "tok-2", &missing).await.0,
        DifferentAccount,
        "and another address is a switch"
    );
    assert_eq!(asked.lock().unwrap().len(), 2, "the server was asked both times");
    let (mismatch, _) = serving(KEY_IS_ANOTHER_ACCOUNTS.0, KEY_IS_ANOTHER_ACCOUNTS.1);
    assert_eq!(
        fx.decide(&sam(), "tok-2", &mismatch).await.0,
        DifferentAccount,
        "a key that is not sam's is the switch"
    );
}

/// Spec §5.6: a `400` decides only with one of the server's two exact codes. A `400` with any other body (no
/// JSON, another code, a near miss, a body the server's extractor rejected, an intermediary's page) says nothing
/// about the kept key, so a same-account sign-in with a stored key settles nothing, as for a server error: the
/// key is not called outdated, and nothing on this Mac changes. The same holds without a user id (rule 2).
#[tokio::test]
async fn a_400_without_one_of_the_two_codes_settles_nothing_and_the_kept_key_stays() {
    let not_an_answer: [&'static str; 5] = [
        "not json",
        r#"{"error":"bad_request"}"#,
        r#"{"error":"Invalid_Recovery_Phrase"}"#,
        "Failed to deserialize the JSON body into the target type: missing field `recovery_check`",
        "<html><body><h1>400 Bad Request</h1></body></html>",
    ];
    for body in not_an_answer {
        let fx = sam_with_a_pending_edit();
        let before = fx.snapshot();
        let (server, seen) = serving("400 Bad Request", body);
        assert_eq!(
            fx.decide(&sam(), "tok-new", &server).await,
            (reauth::SignInKind::Unconfirmed, 1),
            "the same id and a stored key: {body:?} is not a mismatch"
        );
        assert_eq!(seen.lock().unwrap().len(), 1, "{body:?}: the server was asked once");
        assert_eq!(fx.snapshot(), before, "{body:?}: nothing changed, the kept key stays");
        assert_eq!(
            stored_key(fx.id()),
            Some([5u8; 32]),
            "{body:?}: the key is still stored"
        );

        let fx = Fixture::new();
        persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[5u8; 32]).unwrap();
        fx.db().set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
        let (server, _) = serving("400 Bad Request", body);
        assert_eq!(
            fx.decide(&sam(), "tok-new", &server).await.0,
            reauth::SignInKind::Unconfirmed,
            "no user id here (rule 2): {body:?} is not a switch either"
        );
    }
}

/// An owner record or a `state.db` that cannot be read is neither "no owner" nor a switch: nothing is decided,
/// nothing changes (the pending edit is not purged, the credentials do not decide), the server is not asked, and
/// once the read works again the same account is signed in again in place.
#[tokio::test]
async fn an_unreadable_owner_record_or_state_db_settles_nothing_and_the_same_account_then_stays_in_place() {
    use reauth::SignInKind::{LocalDataUnreadable, SameAccount};
    let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
    let pending_edits = |fx: &Fixture| {
        raw_state_db(fx)
            .query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get::<_, i64>(0))
            .unwrap()
    };

    // (A) The owner record alone cannot be read: its value is not text. Everything else in the database is fine.
    let fx = sam_with_a_pending_edit();
    raw_state_db(&fx)
        .execute("UPDATE sync_state SET value = x'ff00' WHERE key = 'owner_user_id'", [])
        .unwrap();
    assert!(
        local_data_owner(&fx.sources()).is_err(),
        "the premise: the owner cannot be read, the database opens"
    );
    let before = fx.snapshot_without_reading_state_db();
    assert_eq!(
        fx.decide(&sam(), "tok-2", &server).await,
        (LocalDataUnreadable, 1),
        "the credentials name sam and a key is stored: still nothing is decided"
    );
    assert_eq!(
        fx.decide(&kim(), "tok-2", &server).await.0,
        LocalDataUnreadable,
        "and another account is not switched either"
    );
    assert!(seen.lock().unwrap().is_empty(), "the server is not asked");
    assert_eq!(
        fx.snapshot_without_reading_state_db(),
        before,
        "nothing changed in the Keychain, memory or the flags"
    );
    assert_eq!(pending_edits(&fx), 1, "the pending edit is still there");
    // The read works again: the same account stays in place.
    raw_state_db(&fx)
        .execute("UPDATE sync_state SET value = 'u-1' WHERE key = 'owner_user_id'", [])
        .unwrap();
    assert_eq!(fx.decide(&sam(), "tok-2", &server).await, (SameAccount, 1));
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "the ids match and a key is stored: the server proves it (spec §5.6)"
    );
    seen.lock().unwrap().clear();
    reauth_swap_token(
        &fx.state,
        &fx.acct,
        &fx.sources(),
        "tok-2",
        &sam(),
        None,
        &mut fx.acct.session_generation(),
    )
    .unwrap();
    assert_eq!(stored_token(fx.id()).as_deref(), Some("tok-2"));
    assert_eq!(stored_key(fx.id()), Some([5u8; 32]));
    assert_eq!(
        fx.db().owner().unwrap(),
        Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
    );
    assert_eq!(pending_edits(&fx), 1, "and still the edit");

    // (B) The whole database cannot be read: moved aside, a file that is not a database in its place.
    let fx = sam_with_a_pending_edit();
    let aside = |suffix: &str| {
        (
            fx.dir.path().join(format!("state.db{suffix}")),
            fx.dir.path().join(format!("state.db{suffix}.valid")),
        )
    };
    for suffix in ["", "-wal", "-shm"] {
        let (from, to) = aside(suffix);
        if from.exists() {
            std::fs::rename(from, to).unwrap();
        }
    }
    let garbage = b"this is not a database, and it is long enough to be read".to_vec();
    std::fs::write(fx.dir.path().join("state.db"), &garbage).unwrap();
    let before = fx.snapshot_without_reading_state_db();
    assert_eq!(fx.decide(&sam(), "tok-2", &server).await.0, LocalDataUnreadable);
    assert_eq!(fx.decide(&kim(), "tok-2", &server).await.0, LocalDataUnreadable);
    assert!(seen.lock().unwrap().is_empty(), "the server is not asked");
    assert_eq!(fx.snapshot_without_reading_state_db(), before);
    assert_eq!(
        std::fs::read(fx.dir.path().join("state.db")).unwrap(),
        garbage,
        "the unreadable file is not touched"
    );
    // The read works again: the real database is back, and nothing of it was purged.
    std::fs::remove_file(fx.dir.path().join("state.db")).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let (back, valid) = aside(suffix);
        if valid.exists() {
            std::fs::rename(valid, back).unwrap();
        }
    }
    assert_eq!(fx.decide(&sam(), "tok-2", &server).await, (SameAccount, 1));
    assert_eq!(pending_edits(&fx), 1, "the pending edit survived the whole episode");
}

/// A test that works in the shared scratch state directory holds the gate the sign-out purge takes, and the marker
/// that tells that sign-out so (otherwise its own sign-out would wait for the gate its test holds, for ever).
#[tokio::test]
async fn a_test_in_the_shared_state_dir_holds_the_purge_gate_and_says_so() {
    assert!(PURGE_GATE_HELD.try_with(|_| ()).is_err(), "no gate is held outside");
    with_the_shared_state_dir(async {
        assert!(
            SIGN_OUT_TEST_PURGE_GATE.try_lock().is_err(),
            "the gate is held for the whole body"
        );
        assert!(PURGE_GATE_HELD.try_with(|_| ()).is_ok(), "and the sign-out can tell");
    })
    .await;
    assert!(
        PURGE_GATE_HELD.try_with(|_| ()).is_err(),
        "the marker goes with the body"
    );
}

// -- the identity rule: ids, then the server's key proof, then the email in its canonical form (ruling A″) --

/// Rule 2, the legacy owner: an older build recorded the lowercase text of the address, the server's own spelling
/// has a capital, and the key is still stored on this Mac. The server proves the key is this account's, and the
/// sign-in is in place: nothing is purged, the key stays, and the record is completed with the user id and the
/// server's spelling. The proof is asked as the account that is signing in, about the key this Mac stores.
#[tokio::test]
async fn a_key_the_server_proves_signs_a_legacy_email_only_owner_in_again_in_place_whatever_the_letter_case() {
    let fx = Fixture::new();
    let id = fx.id().to_string();
    persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[5u8; 32]).unwrap();
    put_email(&id, "sam@beebeeb.io");
    let db = fx.db();
    db.set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
    some_operation(&db, "op-legacy");
    let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
    let signing_in = profile("u-1", "Sam@beebeeb.io");

    assert_eq!(
        fx.decide(&signing_in, "tok-new", &server).await,
        (reauth::SignInKind::SameAccount, 1)
    );
    {
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 1, "one question to the server");
        assert!(
            requests[0].starts_with("POST /api/v1/auth/verify-recovery-check "),
            "{}",
            requests[0]
        );
        assert!(
            requests[0].to_lowercase().contains("authorization: bearer tok-new"),
            "asked as the account that is signing in"
        );
        assert!(
            requests[0].contains(&recovery_check_of([5u8; 32])),
            "about the key this Mac stores"
        );
    }

    let unlocked = reauth_swap_token(
        &fx.state,
        &fx.acct,
        &fx.sources(),
        "tok-new",
        &signing_in,
        None,
        &mut fx.acct.session_generation(),
    )
    .unwrap();
    assert!(unlocked, "the stored key is loaded: no recovery phrase");
    assert_eq!(
        db.owner().unwrap(),
        Some(Identity::new(Some("u-1"), Some("Sam@beebeeb.io"))),
        "the record gets the id and the server's spelling"
    );
    assert_eq!(
        db.queued_or_staged_count().unwrap(),
        1,
        "no purge: the pending edit stays"
    );
    assert_eq!(stored_key(&id), Some([5u8; 32]), "the key stays");
    assert_eq!(stored_token(&id).as_deref(), Some("tok-new"));
    assert_eq!(keychain_account_email(&id).as_deref(), Some("Sam@beebeeb.io"));
    let session = fx.acct.session.lock().unwrap();
    let session = session.as_ref().expect("the keys are in memory again");
    assert_eq!((session.token.as_str(), session.master_key), ("tok-new", [5u8; 32]));
}

/// Rule 2, a key that is not this account's: a different account, and nothing local changes.
#[tokio::test]
async fn a_key_the_server_says_is_another_accounts_makes_the_sign_in_a_switch() {
    let fx = Fixture::new();
    persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[5u8; 32]).unwrap();
    put_email(fx.id(), "sam@beebeeb.io");
    let db = fx.db();
    db.set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
    some_operation(&db, "op-legacy");
    let before = fx.snapshot();
    let (server, seen) = serving(KEY_IS_ANOTHER_ACCOUNTS.0, KEY_IS_ANOTHER_ACCOUNTS.1);

    let decision = fx.decide(&profile("u-1", "Sam@beebeeb.io"), "tok-new", &server).await;

    assert_eq!(
        decision,
        (reauth::SignInKind::DifferentAccount, 1),
        "the same letters, but not this account's key"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(fx.snapshot(), before, "nothing changed on this Mac");
}

/// Rule 2, no answer: nothing is swapped and nothing is switched. A broken server, a refused connection and a
/// stored key that cannot be read are all "cannot confirm", and nothing local changes.
#[tokio::test]
async fn a_proof_that_cannot_complete_changes_nothing_and_decides_nothing() {
    let (broken, _) = serving(SERVER_IS_BROKEN.0, SERVER_IS_BROKEN.1);
    for (name, server) in [("a broken server", broken), ("a refused connection", nobody_listens())] {
        let fx = Fixture::new();
        persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[5u8; 32]).unwrap();
        put_email(fx.id(), "sam@beebeeb.io");
        let db = fx.db();
        db.set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
        some_operation(&db, "op-legacy");
        let before = fx.snapshot();
        let decision = fx.decide(&profile("u-1", "Sam@beebeeb.io"), "tok-new", &server).await;
        assert_eq!(decision.0, reauth::SignInKind::Unconfirmed, "{name}");
        assert_eq!(fx.snapshot(), before, "{name}: nothing changed on this Mac");
    }
    // A key item that cannot be read (here: not 32 bytes) cannot be proven either, and the server is not asked.
    let fx = Fixture::new();
    AuthVault::new(platform_keychain_store_for(fx.id()))
        .store_wrapped_master_key(keychain::SecretBytes::new(vec![1u8, 2, 3]).unwrap())
        .unwrap();
    fx.db().set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
    let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
    assert_eq!(
        fx.decide(&profile("u-1", "sam@beebeeb.io"), "tok-new", &server).await.0,
        reauth::SignInKind::Unconfirmed
    );
    assert!(seen.lock().unwrap().is_empty(), "an unreadable key is never sent");
}
