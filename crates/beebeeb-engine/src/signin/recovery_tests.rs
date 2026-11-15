//! The recovery-phrase proof's rules, against a local fake of the server's check: an account with no check on file is
//! refused like a wrong phrase, and the phrase is normalized to 12 lower-case words before anything is sent. Ported from
//! the old shell's `recovery_phrase_unlock_tests.rs` and `tests.rs`.

use super::*;
use crate::test_api::{FakeApi, Handler, Reply};
use serde_json::{Value, json};
use std::sync::Arc;

const CHECK_ROUTE: &str = "POST /api/v1/auth/verify-recovery-check";
const TOKEN: &str = "sess-tok";

/// The server's check as the fake answers it: `200 {valid:true}` when the submitted check is the one on file, a `400`
/// with the mismatch code when it is another one, and a `400` with `when_none` when the account has no check on file.
fn verifier(on_file: Option<String>, when_none: &'static str) -> FakeApi {
    let handler: Handler = Arc::new(move |request| {
        if request.route() != CHECK_ROUTE {
            return Reply::error(404, "not_found");
        }
        let submitted = request
            .json()
            .get("recovery_check")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match &on_file {
            None => Reply::error(400, when_none),
            Some(stored) if submitted.as_deref() == Some(stored.as_str()) => Reply::ok(json!({ "valid": true })),
            Some(_) => Reply::error(400, INVALID_RECOVERY_PHRASE_CODE),
        }
    });
    FakeApi::start(handler)
}

/// The check the server keeps for the account whose phrase this is.
fn check_of(phrase: &str) -> String {
    let key = beebeeb_core::recovery::recover_from_phrase(phrase).expect("a valid phrase");
    encode_base64(&*beebeeb_core::opaque::compute_recovery_check(&key))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap()
}

/// Port of `account_without_stored_check_is_refused_like_web`. An account with no recovery check on file proves
/// nothing about the key, so the phrase is refused with the wrong-phrase sentence (web does the same), whichever way
/// the server says it: the mismatch code (a server without a distinct code) or `recovery_check_missing`. One request
/// each, the check, with the session's token. The same fake with the check on file opens, so the refusal is the rule's,
/// not the fake's. Mutant: `recovered_key_matches_account` takes every answer but `Mismatch` as a match; the
/// `recovery_check_missing` case goes red.
///
/// The old test also asserted that nothing was stored. The engine's proof stores nothing itself (`Engine::
/// unlock_with_phrase` stores only after it returns `Ok`); that fence is pinned by the integration test
/// `a_wrong_phrase_writes_nothing`.
#[tokio::test]
async fn account_without_stored_check_is_refused_like_web() {
    let (phrase, _) = beebeeb_core::recovery::generate_recovery_phrase().unwrap();
    let client = client();

    let control = verifier(Some(check_of(&phrase)), INVALID_RECOVERY_PHRASE_CODE);
    let key = verify_vault_key_from_phrase(&client, &control.base, TOKEN, &phrase)
        .await
        .expect("the account's own phrase opens when its check is on file");
    let expected = beebeeb_core::recovery::recover_from_phrase(&phrase).unwrap().to_bytes();
    assert_eq!(*key, *expected);
    assert_eq!(control.routes(), [CHECK_ROUTE]);

    let master_key = beebeeb_core::recovery::recover_from_phrase(&phrase).unwrap();
    for (shape, code, answer) in [
        (
            "the mismatch code",
            INVALID_RECOVERY_PHRASE_CODE,
            RecoveryCheckAnswer::Mismatch,
        ),
        (
            "recovery_check_missing",
            RECOVERY_CHECK_MISSING,
            RecoveryCheckAnswer::NoCheckOnFile,
        ),
    ] {
        let server = verifier(None, code);
        let refused = verify_vault_key_from_phrase(&client, &server.base, TOKEN, &phrase).await;
        assert_eq!(
            refused.err().as_deref(),
            Some(INCORRECT_RECOVERY_PHRASE),
            "{shape}: refused like a wrong phrase"
        );
        let requests = server.requests();
        assert_eq!(requests.len(), 1, "{shape}: exactly one request: {requests:?}");
        assert_eq!(requests[0].route(), CHECK_ROUTE, "{shape}");
        assert_eq!(requests[0].bearer(), Some(TOKEN), "{shape}: with the session's token");

        // Underneath: the answer is read exactly, and it is not a match.
        assert_eq!(
            recovery_check_answer(&client, &server.base, TOKEN, &master_key).await,
            Ok(answer),
            "{shape}"
        );
        assert_eq!(
            recovered_key_matches_account(&client, &server.base, TOKEN, &master_key).await,
            Ok(false),
            "{shape}"
        );
    }
}

/// Port of `normalizes_recovery_phrase_in_rust`. The 12 words come out lower case with one space between, and the
/// proof's own normalization (`normalized_phrase`) gives the same text. A pasted phrase with numbering, punctuation,
/// tabs and line breaks normalizes to the same 12 words. Mutant: drop the lower-casing; the first assert goes red.
#[test]
fn normalizes_recovery_phrase_in_rust() {
    let input = "Alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";
    let normalized = normalize_recovery_phrase_input(input).expect("phrase should normalize");

    assert_eq!(
        normalized,
        "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima"
    );
    assert_eq!(
        normalized_phrase(input).expect("the proof's normalization").as_str(),
        normalized
    );

    let pasted = "  1. Alpha,\n2. BRAVO\tcharlie  delta; echo - foxtrot golf hotel india juliet kilo lima. ";
    assert_eq!(
        normalize_recovery_phrase_input(pasted).as_deref(),
        Ok(normalized.as_str())
    );
}

/// Port of `rejects_non_twelve_word_recovery_phrase`. Fewer or more than 12 words is refused with the board's
/// sentence, and the proof refuses it before it sends anything. Mutant: drop the `words != 12` check; the sentence
/// changes (the derivation's refusal instead) and the asserts go red.
#[tokio::test]
async fn rejects_non_twelve_word_recovery_phrase() {
    const SENTENCE: &str = "Recovery phrase must contain exactly 12 words.";
    let error = normalize_recovery_phrase_input("alpha bravo charlie").expect_err("phrase should be incomplete");
    assert_eq!(error, SENTENCE);

    let eleven = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo";
    let thirteen = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike";
    for phrase in [eleven, thirteen] {
        assert_eq!(
            normalize_recovery_phrase_input(phrase).err().as_deref(),
            Some(SENTENCE),
            "{phrase}"
        );
    }

    let server = verifier(None, RECOVERY_CHECK_MISSING);
    let refused = verify_vault_key_from_phrase(&client(), &server.base, TOKEN, "alpha bravo charlie").await;
    assert_eq!(refused.err().as_deref(), Some(SENTENCE));
    assert!(server.requests().is_empty(), "nothing is sent for an incomplete phrase");
}
