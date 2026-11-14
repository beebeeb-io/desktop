//! The passkey calls on a real handle against the engine's fake account on loopback, with the passkey routes answered
//! here in the server's shape and a platform call that is a plain struct. Every name, id and address is invented.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use beebeeb_core::passkey::PrfOutput;
use beebeeb_core::passkey::ceremony::{CreationOptions, NativeAssertion, NativeAttestation, RequestOptions};
use beebeeb_core::passkey::flow::{Assertion, Registration};
use serde_json::{Value, json};

use super::super::{Engine, EngineConfig};
use super::*;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::signin::gate::GateState;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{ConfirmTokens, FakeApi, OpaqueAccount, Reply, Request};

const PASSWORD: &str = "an invented password for passkeys";
const PASSKEY: &str = "c0000000-0000-4000-8000-000000000001";
const OTHER: &str = "c0000000-0000-4000-8000-000000000002";
const CREDENTIAL: [u8; 3] = [1, 2, 3];
const PRF: [u8; 32] = [7; 32];

struct Fake {
    api: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    /// The status the list answers with (200: the rows).
    list_status: Arc<AtomicU16>,
    /// The ids the server deleted, and the escrow blobs it stored.
    deleted: Arc<Mutex<Vec<String>>>,
    escrow: Arc<Mutex<Vec<Value>>>,
}

fn fake(tag: &str) -> Fake {
    let (account, phrase) = FakeAccount::register(&format!("{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let step = Arc::new(OpaqueAccount::register(&format!("{tag}-step@example.eu"), PASSWORD));
    let tokens = Arc::new(ConfirmTokens::default());
    let list_status = Arc::new(AtomicU16::new(200));
    let deleted = Arc::new(Mutex::new(Vec::new()));
    let escrow = Arc::new(Mutex::new(Vec::new()));
    let blob = beebeeb_core::passkey::wrap_vault_key(
        &PrfOutput::from_slice(&PRF).unwrap(),
        &beebeeb_core::kdf::MasterKey::from_bytes(account.master_key),
    )
    .unwrap();
    let (account_in, status_in, deleted_in, escrow_in) =
        (account.clone(), list_status.clone(), deleted.clone(), escrow.clone());
    let escrow_path = beebeeb_core::passkey::ceremony::escrow_path(&CREDENTIAL);
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let signed = req.bearer() == Some(account_in.session_token.as_str());
        match route.as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => step.login_start(&req.json()),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if step.login_finish(&req.json()) {
                    Reply::ok(json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            "GET /api/v1/auth/passkeys" => match status_in.load(Ordering::SeqCst) {
                200 if signed => Reply::ok(json!({ "passkeys": [
                    { "id": PASSKEY, "name": "An invented key", "created_at": "2026-10-28T09:00:00Z", "prf": true },
                    { "id": OTHER, "name": "Another invented key", "created_at": "2026-10-29T09:00:00Z" },
                ] })),
                200 => Reply::error(401, "unauthorized"),
                status => Reply::error(status, "refused"),
            },
            "POST /api/v1/auth/passkey/register-start" if tokens.spend(req) => Reply::ok(json!({
                "reg_id": "a-registration",
                "publicKey": {
                    "challenge": "AAAA",
                    "rp": { "id": "beebeeb.io", "name": "Beebeeb" },
                    "user": { "id": "AAAA", "name": "a@example.eu", "displayName": "A" },
                    "pubKeyCredParams": [{ "alg": -7 }],
                },
            })),
            "POST /api/v1/auth/passkey/register-start" => Reply::error(403, "confirmation_required"),
            "POST /api/v1/auth/passkey/register-finish" => Reply::ok(json!({ "id": PASSKEY, "name": "n" })),
            "POST /api/v1/auth/vault-key-escrow" => {
                escrow_in.lock().unwrap().push(req.json());
                Reply::ok(json!({}))
            }
            "POST /api/v1/auth/passkey/login-start" => Reply::ok(json!({
                "auth_state": "a-state",
                "user_id": "a-user",
                "publicKey": { "challenge": "AAAA", "rpId": "beebeeb.io", "allowCredentials": [] },
            })),
            "POST /api/v1/auth/passkey/login-finish" => Reply::ok(json!({
                "user_id": account_in.user_id,
                "session_token": account_in.session_token,
            })),
            other if other == format!("GET {escrow_path}") && signed => {
                Reply::ok(json!({ "encrypted_vault_key": base64::engine::general_purpose::STANDARD.encode(&blob) }))
            }
            other if other.starts_with("DELETE /api/v1/auth/passkeys/") => {
                if tokens.spend(req) {
                    deleted_in
                        .lock()
                        .unwrap()
                        .push(other.trim_start_matches("DELETE /api/v1/auth/passkeys/").to_string());
                    Reply::empty(204)
                } else {
                    Reply::error(403, "confirmation_required")
                }
            }
            _ => rest(req),
        }
    }));
    Fake {
        api,
        account,
        phrase,
        list_status,
        deleted,
        escrow,
    }
}

fn engine(fake: &Fake, signed_in: bool) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "passkeys-test-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: fake.api.base.clone(),
    };
    let engine = Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    if signed_in {
        engine
            .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
            .expect("the fake accepts the password");
        engine
            .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
            .expect("the phrase unlocks");
    }
    engine
}

fn passkey_routes(fake: &Fake) -> Vec<String> {
    fake.api
        .routes()
        .into_iter()
        .filter(|route| route.contains("passkey") || route.contains("escrow") || route.contains("confirm-opaque"))
        .collect()
}

/// A platform call that answers at once, in the shape webauthn.dll answers.
struct Platform {
    cancel: bool,
    prf: bool,
}

impl PasskeyAuthenticator for Platform {
    fn make_credential(
        &self,
        options: &CreationOptions,
    ) -> impl Future<Output = Result<Registration, PasskeyError>> + Send {
        let answer = if self.cancel {
            Err(PasskeyError::Cancelled)
        } else {
            Ok(Registration {
                attestation: NativeAttestation {
                    credential_id: CREDENTIAL.to_vec(),
                    attestation_object: vec![9],
                    client_data_json: options.client_data_json(),
                },
                prf_output: self.prf.then(|| PrfOutput::from_slice(&PRF).unwrap()),
                prf_enabled: Some(self.prf),
            })
        };
        std::future::ready(answer)
    }

    fn get_assertion(&self, options: &RequestOptions) -> impl Future<Output = Result<Assertion, PasskeyError>> + Send {
        let answer = if self.cancel {
            Err(PasskeyError::Cancelled)
        } else {
            Ok(Assertion {
                assertion: NativeAssertion {
                    credential_id: CREDENTIAL.to_vec(),
                    authenticator_data: vec![1],
                    client_data_json: options.client_data_json(),
                    signature: vec![2],
                    user_handle: None,
                },
                prf_output: self.prf.then(|| PrfOutput::from_slice(&PRF).unwrap()),
            })
        };
        std::future::ready(answer)
    }
}

fn password(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_string())
}

#[test]
fn a_name_is_one_to_sixty_four_characters_and_the_relying_party_is_the_builds_own() {
    assert_eq!(clean_name("  Laptop key "), Some("Laptop key".to_string()));
    assert_eq!(clean_name("   "), None);
    assert_eq!(clean_name(&"x".repeat(64)).map(|n| n.len()), Some(64));
    assert_eq!(clean_name(&"x".repeat(65)), None);
    // A release build never claims localhost, whatever the environment says.
    assert_eq!(rp_id_for(false, Some("localhost")), "beebeeb.io");
    assert_eq!(rp_id_for(true, Some("localhost")), "localhost");
    assert_eq!(rp_id_for(true, None), "beebeeb.io");
}

#[test]
fn core_errors_become_kinds_with_fixed_words() {
    assert_eq!(problem_of(&PasskeyError::Cancelled), PasskeyProblem::Cancelled);
    assert_eq!(problem_of(&PasskeyError::Unavailable), PasskeyProblem::Unavailable);
    assert_eq!(problem_of(&PasskeyError::Network), PasskeyProblem::Offline);
    assert_eq!(problem_of(&PasskeyError::Unauthorized), PasskeyProblem::SessionEnded);
    assert_eq!(
        problem_of(&PasskeyError::StepUp {
            code: "invalid_credentials"
        }),
        PasskeyProblem::WrongPassword
    );
    assert_eq!(
        problem_of(&PasskeyError::Server {
            status: 429,
            code: None
        }),
        PasskeyProblem::RateLimited
    );
    assert_eq!(problem_of(&PasskeyError::RpMismatch), PasskeyProblem::Refused);
}

#[test]
fn signed_out_nothing_is_asked_and_nothing_is_sent() {
    let fake = fake("signed-out");
    let engine = engine(&fake, false);
    assert_eq!(engine.passkeys().err(), Some(PasskeyProblem::NotSignedIn));
    let stale = crate::account::SessionGeneration::for_test(1);
    assert_eq!(
        engine.remove_passkey(stale, password(PASSWORD), PASSKEY),
        Err(PasskeyProblem::NotSignedIn)
    );
    let platform = Platform {
        cancel: false,
        prf: true,
    };
    assert_eq!(
        engine
            .add_passkey_with(&platform, stale, password(PASSWORD), "A key")
            .err(),
        Some(PasskeyProblem::NotSignedIn)
    );
    assert_eq!(passkey_routes(&fake), Vec::<String>::new());
}

#[test]
fn the_list_is_the_servers_rows_for_the_session_and_prints_no_name() {
    let fake = fake("list");
    let engine = engine(&fake, true);
    let rows = engine.passkeys().expect("the list answers");
    assert_eq!(rows.session, engine.session_turn());
    assert_eq!(rows.value.len(), 2);
    assert_eq!(rows.value[0].id, PASSKEY);
    assert_eq!(rows.value[0].name, "An invented key");
    assert_eq!(rows.value[0].prf, Some(true));
    assert_eq!(rows.value[1].prf, None);
    assert!(!format!("{:?}", rows.value[0]).contains("invented"));
    assert_eq!(passkey_routes(&fake), ["GET /api/v1/auth/passkeys"]);
}

#[test]
fn a_refused_list_is_a_kind_and_a_401_is_the_session_ending() {
    let fake = fake("refused");
    let engine = engine(&fake, true);
    fake.list_status.store(500, Ordering::SeqCst);
    assert_eq!(engine.passkeys().err(), Some(PasskeyProblem::Refused));
    fake.list_status.store(401, Ordering::SeqCst);
    assert_eq!(engine.passkeys().err(), Some(PasskeyProblem::SessionEnded));
}

#[test]
fn removing_proves_the_password_and_spends_one_token_on_one_delete() {
    let fake = fake("remove");
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    assert_eq!(engine.remove_passkey(session, password(PASSWORD), PASSKEY), Ok(()));
    assert_eq!(*fake.deleted.lock().unwrap(), [PASSKEY]);
    assert_eq!(
        passkey_routes(&fake),
        [
            "POST /api/v1/auth/confirm-opaque-start".to_string(),
            "POST /api/v1/auth/confirm-opaque-finish".to_string(),
            format!("DELETE /api/v1/auth/passkeys/{PASSKEY}"),
        ]
    );
}

#[test]
fn a_wrong_password_removes_nothing_and_an_empty_one_sends_nothing() {
    let fake = fake("wrong");
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    assert_eq!(
        engine.remove_passkey(session, password("not the password"), PASSKEY),
        Err(PasskeyProblem::WrongPassword)
    );
    assert_eq!(
        engine.remove_passkey(session, password(""), PASSKEY),
        Err(PasskeyProblem::InvalidInput)
    );
    assert!(fake.deleted.lock().unwrap().is_empty());
    assert!(!passkey_routes(&fake).iter().any(|route| route.starts_with("DELETE")));
}

#[test]
fn a_press_made_under_another_session_sends_nothing() {
    let fake = fake("stale");
    let engine = engine(&fake, true);
    let before = engine.session_turn();
    // A lock and an unlock again: the same account, another session.
    engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
    assert_ne!(engine.session_turn(), before);
    assert_eq!(
        engine.remove_passkey(before, password(PASSWORD), PASSKEY),
        Err(PasskeyProblem::NotSignedIn)
    );
    assert_eq!(passkey_routes(&fake), Vec::<String>::new());
}

#[test]
fn adding_with_prf_stores_the_escrow_and_says_it_can_unlock() {
    let fake = fake("add");
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    let platform = Platform {
        cancel: false,
        prf: true,
    };
    let added = engine
        .add_passkey_with(&platform, session, password(PASSWORD), "  A new key ")
        .expect("the passkey is added");
    assert!(added.can_unlock);
    assert_eq!(fake.escrow.lock().unwrap().len(), 1);
    let routes = passkey_routes(&fake);
    assert_eq!(
        routes.last().map(String::as_str),
        Some("POST /api/v1/auth/vault-key-escrow")
    );
    assert!(routes.contains(&"POST /api/v1/auth/passkey/register-finish".to_string()));
}

#[test]
fn adding_without_prf_signs_in_only_and_a_closed_dialog_adds_nothing() {
    let fake = fake("add-no-prf");
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    let added = engine
        .add_passkey_with(
            &Platform {
                cancel: false,
                prf: false,
            },
            session,
            password(PASSWORD),
            "A key",
        )
        .expect("the passkey is added");
    assert!(!added.can_unlock);
    assert!(fake.escrow.lock().unwrap().is_empty());
    let cancelled = engine.add_passkey_with(
        &Platform {
            cancel: true,
            prf: true,
        },
        session,
        password(PASSWORD),
        "A key",
    );
    assert_eq!(cancelled.err(), Some(PasskeyProblem::Cancelled));
    let finishes = |fake: &Fake| {
        fake.api
            .routes()
            .iter()
            .filter(|r| r.ends_with("register-finish"))
            .count()
    };
    // The first add finished once; the closed dialog never reached the server's finish.
    assert_eq!(finishes(&fake), 1);
}

#[test]
fn an_empty_or_long_name_and_an_empty_password_send_nothing() {
    let fake = fake("add-bad");
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    let platform = Platform {
        cancel: false,
        prf: true,
    };
    for (pass, name) in [(PASSWORD, "  "), (PASSWORD, &*"x".repeat(65)), ("", "A key")] {
        assert_eq!(
            engine.add_passkey_with(&platform, session, password(pass), name).err(),
            Some(PasskeyProblem::InvalidInput)
        );
    }
    assert_eq!(passkey_routes(&fake), Vec::<String>::new());
}

#[test]
fn a_passkey_sign_in_with_prf_opens_the_vault_and_without_it_asks_for_the_words() {
    let fake = fake("signin-prf");
    let engine = engine(&fake, false);
    assert_eq!(engine.gate(), GateState::SignedOut);
    let next = engine
        .sign_in_passkey_with(
            &Platform {
                cancel: false,
                prf: true,
            },
            &fake.account.email,
        )
        .expect("the passkey signs in");
    assert_eq!(next, NextAfterSignIn::Drive);
    assert_eq!(engine.gate(), GateState::Ready);

    let fake = self::fake("signin-no-prf");
    let engine = self::engine(&fake, false);
    let next = engine
        .sign_in_passkey_with(
            &Platform {
                cancel: false,
                prf: false,
            },
            &fake.account.email,
        )
        .expect("the passkey signs in");
    // The passkey proved the person; this computer holds no key and the passkey opened none.
    assert_eq!(next, NextAfterSignIn::RecoveryPhrase);
    assert_eq!(engine.gate(), GateState::NeedsRecoveryPhrase);
}

#[test]
fn a_closed_dialog_signs_nobody_in_and_an_empty_address_sends_nothing() {
    let fake = fake("signin-cancel");
    let engine = engine(&fake, false);
    assert_eq!(
        engine
            .sign_in_passkey_with(
                &Platform {
                    cancel: true,
                    prf: true
                },
                &fake.account.email
            )
            .err(),
        Some(PasskeyProblem::Cancelled)
    );
    assert_eq!(engine.gate(), GateState::SignedOut);
    assert!(!fake.api.routes().iter().any(|route| route.ends_with("login-finish")));
    let before = fake.api.routes().len();
    assert_eq!(
        engine
            .sign_in_passkey_with(
                &Platform {
                    cancel: false,
                    prf: true
                },
                "  "
            )
            .err(),
        Some(PasskeyProblem::InvalidInput)
    );
    assert_eq!(fake.api.routes().len(), before);
}

#[test]
fn a_passkey_sign_in_over_a_session_that_is_there_sends_nothing() {
    let fake = fake("signin-present");
    let engine = engine(&fake, true);
    let before = fake.api.routes().len();
    assert_eq!(
        engine
            .sign_in_passkey_with(
                &Platform {
                    cancel: false,
                    prf: true
                },
                &fake.account.email
            )
            .err(),
        Some(PasskeyProblem::Refused)
    );
    assert_eq!(fake.api.routes().len(), before);
}

#[test]
fn off_windows_the_platform_is_not_supported() {
    let fake = fake("support");
    let engine = engine(&fake, false);
    #[cfg(not(target_os = "windows"))]
    assert!(!engine.passkeys_supported());
    #[cfg(target_os = "windows")]
    let _ = engine.passkeys_supported();
}
