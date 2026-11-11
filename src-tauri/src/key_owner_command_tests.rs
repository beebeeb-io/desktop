//! Task 2447: the commands themselves, run. `key_owner_tests` drives the rules and the loader over a store of its
//! own, and `key_owner_wiring_tests` pins each command's calls by reading the source; here the real
//! `restore_stored_session`, `store_first_sign_in` and `store_recovered_vault_key` run on the test build's in-memory
//! platform store (`keychain::TestKeychainStore`, every OS) against a loopback server that names an account.
//! Tokens and keys are filler bytes, not credentials. Every test has an account id of its own; a test that needs the
//! flat layout takes a flat layout of its own (`keychain::test_flat_layout_scope`) and stays on its thread.

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::account::{AccountId, AccountRuntime, synthesize_single_account};
use crate::key_owner::{FOREIGN_KEY_NOT_REMOVED, KEY_NOT_THIS_ACCOUNTS};
use keychain::{AuthSecretStore as _, PlatformKeychainStore, SecretBytes, SessionToken};

const KEY_A: [u8; 32] = [0xA1; 32];
const KEY_B: [u8; 32] = [0xB2; 32];
const A: &str = "a@beebeeb.io";
const B: &str = "b@beebeeb.io";

/// One start of the app on this PC: a fresh process state over the credential store that outlives it.
struct Pc {
    state: AppState,
    acct: Arc<AccountRuntime>,
}

impl Pc {
    fn start(id: &str) -> Self {
        let state = AppState::default();
        synthesize_single_account(&state, AccountId(id.to_string()));
        let acct = state.active_account().unwrap();
        Self { state, acct }
    }

    /// The startup restore, as `app_run` calls it. On macOS and Linux it may complete the owner record in the test
    /// process's shared state directory, so it takes that directory's turn like the other restore tests do.
    async fn restore(&self, base_url: &str) -> bool {
        #[cfg(not(target_os = "windows"))]
        {
            with_the_shared_state_dir(restore_stored_session(&self.state, &self.acct, base_url)).await
        }
        #[cfg(target_os = "windows")]
        {
            restore_stored_session(&self.state, &self.acct, base_url).await
        }
    }

    /// What is in memory: the token and the first byte of the key (which names the key in these tests).
    fn in_memory(&self) -> Option<(String, u8)> {
        self.acct
            .session
            .lock()
            .unwrap()
            .as_ref()
            .map(|session| (session.token.clone(), session.master_key[0]))
    }

    fn signed_in_flag(&self) -> bool {
        *self.state.auth_present.lock().unwrap()
    }

    /// A password sign-in's last step, as `desktop_login` calls it.
    fn sign_in(&self, token: &str, user_id: &str, email: &str) -> Result<(), String> {
        let mut turn = self.acct.session_generation();
        store_first_sign_in(
            &self.state,
            &self.acct,
            &mut turn,
            token,
            email,
            profile(user_id, email),
        )
    }

    /// The recovery phrase proved `key` for the stored `token`: the store step of the unlock.
    fn recover(&self, token: &str, email: &str, key: &[u8; 32]) -> Result<(), String> {
        let mut turn = self.acct.session_generation();
        store_recovered_vault_key(&self.acct, &mut turn, token, Some(email), key).map(drop)
    }
}

fn profile_json(user_id: &str, email: &str) -> String {
    format!(r#"{{"user_id":"{user_id}","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#)
}

fn profile(user_id: &str, email: &str) -> account_dto::AccountProfile {
    serde_json::from_str(&profile_json(user_id, email)).unwrap()
}

/// A loopback server that answers every request with the account record of one user, and counts the requests.
struct Server {
    base_url: String,
    requests: Arc<AtomicUsize>,
}

impl Server {
    fn naming(user_id: &str, email: &str) -> Self {
        let body = profile_json(user_id, email);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                seen.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self { base_url, requests }
    }

    fn asked(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// No server: a port that refuses the connection (the probe is inconclusive, as when the PC is offline).
fn offline() -> String {
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", dead.local_addr().unwrap());
    drop(dead);
    url
}

/// What one layout of the credential store holds. The key is named by its first byte.
#[derive(Debug, Default, PartialEq, Eq)]
struct Held {
    token: Option<String>,
    key: Option<u8>,
    email: Option<String>,
    key_owner: Option<String>,
    session_owner: Option<String>,
}

fn held(store: &PlatformKeychainStore) -> Held {
    Held {
        token: store
            .load_session_token()
            .unwrap()
            .map(|token| token.expose_for_request().to_string()),
        key: store
            .load_wrapped_master_key()
            .unwrap()
            .map(|key| key.expose_for_crypto()[0]),
        email: store.load_account_email().unwrap(),
        key_owner: store.load_vault_key_owner().unwrap(),
        session_owner: store.load_session_owner().unwrap(),
    }
}

/// Put items in one layout as an earlier build (or an earlier run) left them: raw writes, no rule applied.
fn seed(store: &PlatformKeychainStore, items: Held) {
    if let Some(token) = &items.token {
        store
            .save_session_token(&SessionToken::new(token.clone()).unwrap())
            .unwrap();
    }
    if let Some(key) = items.key {
        store
            .save_wrapped_master_key(SecretBytes::new_master_key(&[key; 32]))
            .unwrap();
    }
    if let Some(email) = &items.email {
        store.save_account_email(email).unwrap();
    }
    if let Some(owner) = &items.key_owner {
        store.save_vault_key_owner(owner).unwrap();
    }
    if let Some(owner) = &items.session_owner {
        store.save_session_owner(owner).unwrap();
    }
}

fn some(text: &str) -> Option<String> {
    Some(text.to_string())
}

fn id_keyed(id: &str) -> PlatformKeychainStore {
    keychain::platform_keychain_store_for(id)
}

fn flat() -> PlatformKeychainStore {
    keychain::legacy_platform_keychain_store()
}

/// What 0.8.24 leaves for a signed-in, unlocked account A: token, key and email, no tags.
fn as_0_8_24_left_a() -> Held {
    Held {
        token: some("tok-a"),
        key: Some(KEY_A[0]),
        email: some(A),
        ..Held::default()
    }
}

// ── the browser handoff into a signed-in session (review P2-1) ─────────────────────────────

/// Lead ruling on review P2-1: Windows refuses a sign-in that arrives while a session is present, unconditionally, as
/// 0.8.24 did. Locked or unlocked, and whatever the owner tags say (here they name the arriving account itself).
#[test]
fn a_sign_in_into_a_signed_in_session_is_refused_whatever_the_tags_say() {
    const REFUSAL: &str = "Sign out of the current account before signing in again.";
    for unlocked in [false, true] {
        for tags in [None, Some("user-a"), Some("user-b")] {
            let id = format!("h2b-handoff-{unlocked}-{}", tags.unwrap_or("untagged"));
            let pc = Pc::start(&id);
            assert_eq!(
                refuse_sign_in_while_signed_in(&pc.state),
                Ok(()),
                "the premise: nobody is signed in, so a sign-in is not refused"
            );
            seed(
                &id_keyed(&id),
                Held {
                    token: some("tok-a"),
                    key: unlocked.then_some(KEY_A[0]),
                    email: some(A),
                    key_owner: tags.filter(|_| unlocked).map(str::to_string),
                    session_owner: tags.map(str::to_string),
                },
            );
            set_auth_present(&pc.state, true);
            if unlocked {
                *pc.acct.session.lock().unwrap() = Some(Session {
                    token: "tok-a".into(),
                    master_key: KEY_A,
                    email: some(A),
                });
            }
            assert_eq!(
                refuse_sign_in_while_signed_in(&pc.state),
                Err(REFUSAL.to_string()),
                "unlocked: {unlocked}, tags: {tags:?}"
            );
        }
    }
}

// ── the startup restore ────────────────────────────────────────────────────────────────────

/// Review test 1: the stored token is another account's than the tagged key's. The tags are equal (so the loader's own
/// check passes), and the server names the token's real account: nothing reaches memory.
#[tokio::test]
async fn the_restore_installs_nothing_when_the_server_names_another_account_than_the_keys() {
    let id = "h2b-restore-foreign";
    let stale = || Held {
        token: some("tok-b"),
        key: Some(KEY_A[0]),
        email: some(B),
        key_owner: some("user-a"),
        session_owner: some("user-a"),
    };
    seed(&id_keyed(id), stale());
    let server = Server::naming("user-b", B);
    let pc = Pc::start(id);
    assert!(!pc.restore(&server.base_url).await, "no engine start follows");
    assert_eq!((pc.in_memory(), pc.signed_in_flag(), server.asked()), (None, false, 1));
    assert_eq!(
        held(&id_keyed(id)),
        Held {
            session_owner: some("user-b"),
            ..stale()
        },
        "the session is named as the server named it; the key stays stored under its own owner"
    );
    // Tags that already differ are refused by the loader, before the server is asked, online and offline.
    for base_url in [server.base_url.clone(), offline()] {
        let pc = Pc::start(id);
        assert!(!pc.restore(&base_url).await);
        assert_eq!((pc.in_memory(), server.asked()), (None, 1));
    }
    assert_eq!(
        load_session_from_keychain(id, None).map(|session| session.is_some()),
        Err(KEY_NOT_THIS_ACCOUNTS.to_string())
    );
    // The same store when the server names the key's own account: it is installed (the refusal above is the id's).
    let id = "h2b-restore-same";
    seed(
        &id_keyed(id),
        Held {
            token: some("tok-a"),
            email: some(A),
            ..stale()
        },
    );
    let pc = Pc::start(id);
    assert!(pc.restore(&Server::naming("user-a", A).base_url).await);
    assert_eq!(pc.in_memory(), Some(("tok-a".to_string(), KEY_A[0])));
}

/// Review test 2: a store as 0.8.24 leaves it, in the id-keyed layout, in the flat layout read through the fallback,
/// and in the flat layout after the migration `app_run` runs first. Online: installed, the session is named, the key
/// is NOT tagged (nothing proved it). Then offline: installed again. No recovery phrase at any point.
#[tokio::test]
async fn an_upgraded_0_8_24_store_restores_online_then_offline_without_the_phrase() {
    for layout in ["id-keyed", "flat-fallback", "flat-migrated"] {
        let id = format!("h2b-upgrade-{layout}");
        let _flat = keychain::test_flat_layout_scope(&id);
        let holder = match layout {
            "id-keyed" => {
                seed(&id_keyed(&id), as_0_8_24_left_a());
                id_keyed(&id)
            }
            "flat-fallback" => {
                seed(&flat(), as_0_8_24_left_a());
                flat()
            }
            _ => {
                seed(&flat(), as_0_8_24_left_a());
                keychain::migrate_legacy_keychain_to_account(&id).unwrap();
                assert_eq!(held(&flat()), Held::default(), "the migration emptied the flat layout");
                id_keyed(&id)
            }
        };
        let named = Held {
            session_owner: some("user-a"),
            ..as_0_8_24_left_a()
        };

        let server = Server::naming("user-a", A);
        let pc = Pc::start(&id);
        assert!(pc.restore(&server.base_url).await, "{layout}: online");
        assert_eq!(
            (pc.in_memory(), pc.signed_in_flag(), server.asked()),
            (Some(("tok-a".to_string(), KEY_A[0])), true, 1),
            "{layout}: online"
        );
        assert_eq!(
            held(&holder),
            named,
            "{layout}: session-owner written, no vault-key-owner"
        );

        // The next start, offline.
        let pc = Pc::start(&id);
        assert!(pc.restore(&offline()).await, "{layout}: offline");
        assert_eq!(
            pc.in_memory(),
            Some(("tok-a".to_string(), KEY_A[0])),
            "{layout}: offline"
        );
        assert_eq!(held(&holder), named, "{layout}: an offline start writes nothing");

        // Review P3-2: a session served from the flat layout got its owner tag there. When the migration moves it,
        // nothing of it stays behind (a leftover tag would later be reported as a vault key left behind).
        if layout == "flat-fallback" {
            keychain::migrate_legacy_keychain_to_account(&id).unwrap();
            assert_eq!(
                held(&flat()),
                Held::default(),
                "the flat layout is empty, tags included"
            );
            assert_eq!(
                held(&id_keyed(&id)),
                as_0_8_24_left_a(),
                "moved whole; its owner is unknown again"
            );
            let pc = Pc::start(&id);
            assert!(pc.restore(&offline()).await, "after the migration, offline");
            assert_eq!(pc.in_memory(), Some(("tok-a".to_string(), KEY_A[0])));
        }
    }
}

/// Review test 5: a clear that failed earlier left a key and tags with no token, in both layouts. On Windows the
/// restore finishes that clear before it reads anything. macOS and Linux keep a key after a rejected token on purpose
/// (R9) and prove it at the next sign-in, so there the store is left as it is.
#[tokio::test]
async fn a_start_on_a_key_and_tags_without_a_token_finishes_the_clear_on_windows_only() {
    let id = "h2b-pending-clear";
    let _flat = keychain::test_flat_layout_scope(id);
    let leftover = || Held {
        key: Some(KEY_A[0]),
        email: some(A),
        key_owner: some("user-a"),
        session_owner: some("user-a"),
        ..Held::default()
    };
    seed(&id_keyed(id), leftover());
    seed(&flat(), leftover());
    let server = Server::naming("user-a", A);
    let pc = Pc::start(id);
    assert!(!pc.restore(&server.base_url).await);
    assert_eq!(
        (pc.in_memory(), pc.signed_in_flag(), server.asked()),
        (None, false, 0),
        "no token: the server is not asked"
    );
    #[cfg(target_os = "windows")]
    assert_eq!(
        (held(&id_keyed(id)), held(&flat())),
        (Held::default(), Held::default()),
        "Windows: both layouts are cleared, tags included"
    );
    #[cfg(not(target_os = "windows"))]
    assert_eq!(
        (held(&id_keyed(id)), held(&flat())),
        (leftover(), leftover()),
        "macOS and Linux: nothing is removed at startup"
    );
}

// ── a token-only sign-in over a leftover key ───────────────────────────────────────────────

/// Review test 3: `store_first_sign_in` itself, on a store that still holds a key from before.
#[test]
fn a_password_sign_in_removes_a_leftover_key_that_is_not_its_accounts_before_it_stores_its_token() {
    let signed_in_as_b = Held {
        token: some("tok-b"),
        email: some(B),
        session_owner: some("user-b"),
        ..Held::default()
    };
    for (case, leftover) in [
        (
            "tagged-other-id",
            Held {
                key: Some(KEY_A[0]),
                key_owner: some("user-a"),
                session_owner: some("user-a"),
                ..Held::default()
            },
        ),
        (
            "untagged-other-email",
            Held {
                key: Some(KEY_A[0]),
                email: some(A),
                ..Held::default()
            },
        ),
        (
            "untagged-no-email",
            Held {
                key: Some(KEY_A[0]),
                ..Held::default()
            },
        ),
    ] {
        let id = format!("h2b-sign-in-{case}");
        seed(&id_keyed(&id), leftover);
        let pc = Pc::start(&id);
        assert_eq!(pc.sign_in("tok-b", "user-b", B), Ok(()), "{case}");
        assert_eq!(
            held(&id_keyed(&id)),
            signed_in_as_b,
            "{case}: the key is gone; token and owner stored"
        );
        assert!(pc.signed_in_flag() && pc.in_memory().is_none(), "{case}");
        assert!(
            load_session_from_keychain(&id, None).is_err_and(|error| error.contains("secret not found")),
            "{case}: the loader finds no key, so the recovery phrase is asked for"
        );
    }

    // The same account's own key stays, tagged or (with its own email beside it) untagged, and loads.
    for (case, key_owner) in [("tagged", some("user-a")), ("untagged", None)] {
        let id = format!("h2b-sign-in-own-{case}");
        seed(
            &id_keyed(&id),
            Held {
                key: Some(KEY_A[0]),
                email: some(A),
                key_owner: key_owner.clone(),
                ..Held::default()
            },
        );
        let pc = Pc::start(&id);
        assert_eq!(pc.sign_in("tok-a2", "user-a", A), Ok(()), "{case}");
        assert_eq!(
            held(&id_keyed(&id)),
            Held {
                token: some("tok-a2"),
                key: Some(KEY_A[0]),
                email: some(A),
                key_owner,
                session_owner: some("user-a"),
            },
            "{case}"
        );
        let loaded = load_session_from_keychain(&id, None).unwrap().expect("loads");
        assert_eq!((loaded.token.as_str(), loaded.master_key), ("tok-a2", KEY_A), "{case}");
    }

    // A key that cannot be removed: the sign-in is refused and nothing of it is stored.
    let id = "h2b-sign-in-stuck";
    let stuck = || Held {
        key: Some(KEY_A[0]),
        key_owner: some("user-a"),
        ..Held::default()
    };
    seed(&id_keyed(id), stuck());
    let pc = Pc::start(id);
    keychain::test_fail_deletes(Some(id), true);
    let refused = pc.sign_in("tok-b", "user-b", B);
    keychain::test_fail_deletes(Some(id), false);
    assert_eq!(refused, Err(FOREIGN_KEY_NOT_REMOVED.to_string()));
    assert_eq!(held(&id_keyed(id)), stuck(), "no token, no email, no owner of B's");
    assert!(!pc.signed_in_flag() && pc.acct.cached_profile.lock().unwrap().is_none());
}

// ── the recovery phrase ────────────────────────────────────────────────────────────────────

/// Review test 4: `store_recovered_vault_key`, then the loader. The key takes the stored session's owner; a session
/// nobody named leaves the key untagged. Both load.
#[test]
fn a_recovered_key_takes_the_sessions_owner_and_loads() {
    let id = "h2b-recover-named";
    let pc = Pc::start(id);
    pc.sign_in("tok-a", "user-a", A).unwrap();
    assert_eq!(pc.recover("tok-a", A, &KEY_A), Ok(()));
    assert_eq!(
        held(&id_keyed(id)),
        Held {
            token: some("tok-a"),
            key: Some(KEY_A[0]),
            email: some(A),
            key_owner: some("user-a"),
            session_owner: some("user-a"),
        }
    );
    let loaded = load_session_from_keychain(id, None).unwrap().expect("loads");
    assert_eq!((loaded.token.as_str(), loaded.master_key), ("tok-a", KEY_A));

    // A session stored by a build that knew no tags (0.8.24's password sign-in), not named since.
    let id = "h2b-recover-unnamed";
    seed(
        &id_keyed(id),
        Held {
            token: some("tok-a"),
            email: some(A),
            ..Held::default()
        },
    );
    let pc = Pc::start(id);
    set_auth_email(&pc.state, some(A));
    assert_eq!(pc.recover("tok-a", A, &KEY_A), Ok(()));
    assert_eq!(
        held(&id_keyed(id)),
        as_0_8_24_left_a(),
        "untagged: nobody named the session"
    );
    let loaded = load_session_from_keychain(id, None).unwrap().expect("loads");
    assert_eq!((loaded.token.as_str(), loaded.master_key), ("tok-a", KEY_A));
}

// ── back to 0.8.24 and forward again (review P2-4): today's behaviour, pinned ──────────────
//
// 0.8.24 knows no tags: its sign-out removes token, key and email and leaves both tags; its sign-in writes a token
// (and later a key) under them. So after an interlude on 0.8.24 with ANOTHER account, 0.8.25 finds stale tags beside a
// new token. These tests pin what happens then. They are known limits, stated in the release notes, not goals.

/// P2-4 (a): B signed in and unlocked under 0.8.24; both stale tags name A. The first ONLINE start refuses B's own
/// key once (a false refusal: acceptable, it fails closed and costs one recovery phrase); after the phrase it loads.
#[tokio::test]
async fn after_a_0_8_24_interlude_another_accounts_own_key_is_refused_once_then_loads() {
    let id = "h2b-interlude-a";
    seed(
        &id_keyed(id),
        Held {
            token: some("tok-b"),
            key: Some(KEY_B[0]),
            email: some(B),
            key_owner: some("user-a"),
            session_owner: some("user-a"),
        },
    );
    let server = Server::naming("user-b", B);
    let pc = Pc::start(id);
    assert!(
        !pc.restore(&server.base_url).await,
        "the false refusal, once: B's key under A's stale tag"
    );
    assert_eq!((pc.in_memory(), server.asked()), (None, 1));
    // The window asks for the recovery phrase (a stored session, not unlocked); B enters it.
    set_auth_email(&pc.state, some(B));
    assert_eq!(pc.recover("tok-b", B, &KEY_B), Ok(()));
    let healed = Held {
        token: some("tok-b"),
        key: Some(KEY_B[0]),
        email: some(B),
        key_owner: some("user-b"),
        session_owner: some("user-b"),
    };
    assert_eq!(held(&id_keyed(id)), healed);
    for base_url in [server.base_url.clone(), offline()] {
        let pc = Pc::start(id);
        assert!(pc.restore(&base_url).await, "every later start loads");
        assert_eq!(pc.in_memory(), Some(("tok-b".to_string(), KEY_B[0])));
    }
}

/// P2-4 (b): B signed in under 0.8.24 and had NOT entered the phrase yet. 0.8.25's first start finds no key and stops
/// before it asks the server, so the session keeps A's stale tag; the recovered key takes that tag, and the next
/// online start refuses it: the phrase is asked a SECOND time (acceptable: fails closed; stated in the notes). After
/// the second phrase it loads.
#[tokio::test]
async fn after_a_0_8_24_interlude_a_not_yet_unlocked_account_is_asked_for_the_phrase_twice() {
    let id = "h2b-interlude-b";
    seed(
        &id_keyed(id),
        Held {
            token: some("tok-b"),
            email: some(B),
            key_owner: some("user-a"),
            session_owner: some("user-a"),
            ..Held::default()
        },
    );
    let server = Server::naming("user-b", B);
    let pc = Pc::start(id);
    assert!(!pc.restore(&server.base_url).await, "no key yet");
    assert_eq!(
        (pc.in_memory(), server.asked()),
        (None, 0),
        "the restore stops before the probe"
    );
    set_auth_email(&pc.state, some(B));
    assert_eq!(pc.recover("tok-b", B, &KEY_B), Ok(()), "the first phrase");
    assert_eq!(
        held(&id_keyed(id)).key_owner,
        some("user-a"),
        "the key took the stale session owner"
    );

    let pc = Pc::start(id);
    assert!(
        !pc.restore(&server.base_url).await,
        "the next online start refuses B's own key"
    );
    assert_eq!((pc.in_memory(), server.asked()), (None, 1));
    set_auth_email(&pc.state, some(B));
    assert_eq!(pc.recover("tok-b", B, &KEY_B), Ok(()), "the second phrase");
    assert_eq!(
        held(&id_keyed(id)),
        Held {
            token: some("tok-b"),
            key: Some(KEY_B[0]),
            email: some(B),
            key_owner: some("user-b"),
            session_owner: some("user-b"),
        }
    );
    let pc = Pc::start(id);
    assert!(pc.restore(&server.base_url).await, "and from then on it loads");
    assert_eq!(pc.in_memory(), Some(("tok-b".to_string(), KEY_B[0])));
}

/// P2-4 (c): 0.8.24 itself left A's key beside B's token during the interlude (see task 2447), under A's stale, equal
/// tags. An OFFLINE start takes the equal tags for a match and installs it (no worse than 0.8.24, which always did;
/// acceptable for 0.8.25, stated in the notes); the first ONLINE start refuses it.
#[tokio::test]
async fn after_a_0_8_24_interlude_stale_equal_tags_match_offline_and_are_refused_online() {
    let id = "h2b-interlude-c";
    seed(
        &id_keyed(id),
        Held {
            token: some("tok-b"),
            key: Some(KEY_A[0]),
            email: some(B),
            key_owner: some("user-a"),
            session_owner: some("user-a"),
        },
    );
    let pc = Pc::start(id);
    assert!(pc.restore(&offline()).await, "offline: the known limit (a false match)");
    assert_eq!(pc.in_memory(), Some(("tok-b".to_string(), KEY_A[0])));

    let server = Server::naming("user-b", B);
    let pc = Pc::start(id);
    assert!(!pc.restore(&server.base_url).await, "online: refused");
    assert_eq!((pc.in_memory(), server.asked()), (None, 1));
    // And once the server named the session, an offline start refuses it too.
    let pc = Pc::start(id);
    assert!(!pc.restore(&offline()).await);
    assert_eq!(pc.in_memory(), None);
}
