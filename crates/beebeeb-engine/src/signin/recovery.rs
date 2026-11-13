//! The recovery-phrase unlock's proof: is the key derived from the typed phrase the signed-in account's? Moved
//! out of the Tauri shell (`session_apply.rs`, `login_commands.rs`) as text.
//!
//! Must not: log the phrase, the key or its check; treat an unreachable or unreadable verifier as "valid".

/// The server's answer to "is this key the signed-in account's?" (`POST /api/v1/auth/verify-recovery-check`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryCheckAnswer {
    Matches,
    /// A `400` whose code is exactly [`INVALID_RECOVERY_PHRASE_CODE`]: the key is not the account's. A server that has
    /// no distinct code for it sends the same answer for an account with no recovery check on file (spec §5.6).
    Mismatch,
    /// A `400` whose code is exactly [`RECOVERY_CHECK_MISSING`]: the account has no recovery check on file.
    NoCheckOnFile,
}

/// The `error` code the server sends when the key's check is not the account's. Matched exactly; nothing else is.
pub const INVALID_RECOVERY_PHRASE_CODE: &str = "invalid_recovery_phrase";

/// The `error` code the server sends when the account has no recovery check on file. Matched exactly; nothing else is.
pub const RECOVERY_CHECK_MISSING: &str = "recovery_check_missing";

/// Ask the server whether `master_key` is the signed-in account's key by comparing its `recovery_check` (HKDF of the
/// key; never the key itself) against the stored one. Only a completed answer is `Ok`: a `200` that says valid, or a
/// `400` with one of the two exact codes above. A network error, a 401, a 5xx, an unreadable success body, and any other
/// `400` (no JSON, another code, a body the server's extractor rejected, an intermediary's page) are `Err`: they say
/// nothing about the key, and an unreachable verifier is never read as "valid" (the caller fails closed).
pub async fn recovery_check_answer(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    master_key: &beebeeb_core::kdf::MasterKey,
) -> Result<RecoveryCheckAnswer, String> {
    // The check stands in for the key at the server, so its text is wiped after the request (fix round 1, item 10),
    // and the body is serialized from a borrow of it (no second copy in a JSON value).
    let recovery_check = zeroize::Zeroizing::new(encode_base64(&*beebeeb_core::opaque::compute_recovery_check(
        master_key,
    )));
    #[derive(serde::Serialize)]
    struct RecoveryCheckBody<'a> {
        recovery_check: &'a str,
    }
    let resp = client
        .post(format!("{base_url}/api/v1/auth/verify-recovery-check"))
        .bearer_auth(session_token)
        .json(&RecoveryCheckBody {
            recovery_check: &recovery_check,
        })
        .send()
        .await
        .map_err(|e| format!("Could not verify the recovery phrase (network error: {e}). Try again."))?;
    let status = resp.status();
    if status == reqwest::StatusCode::BAD_REQUEST {
        let body = resp.text().await.unwrap_or_default();
        let code = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value.get("error").and_then(|code| code.as_str()).map(str::to_string));
        return match code.as_deref() {
            Some(INVALID_RECOVERY_PHRASE_CODE) => Ok(RecoveryCheckAnswer::Mismatch),
            Some(RECOVERY_CHECK_MISSING) => Ok(RecoveryCheckAnswer::NoCheckOnFile),
            // Any other 400 says nothing about the key: answered like a server error, never as a mismatch.
            _ => Err(format!(
                "Could not verify the recovery phrase (server returned {status}). Try again."
            )),
        };
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Your session expired. Sign in again before unlocking the vault.".to_string());
    }
    if !status.is_success() {
        return Err(format!(
            "Could not verify the recovery phrase (server returned {status}). Try again."
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Could not verify the recovery phrase (unreadable response: {e}). Try again."))?;
    if body.get("valid").and_then(|v| v.as_bool()) == Some(true) {
        Ok(RecoveryCheckAnswer::Matches)
    } else {
        Err("Could not verify the recovery phrase (unexpected response). Try again.".to_string())
    }
}

/// The recovery-phrase unlock's question: is the key from this phrase the signed-in account's? `Ok(true)` only on a
/// match. An account with no check on file proves nothing here either, so it stays `Ok(false)` (web applies the same
/// policy); see [`recovery_check_answer`] for what is `Err`.
pub async fn recovered_key_matches_account(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    master_key: &beebeeb_core::kdf::MasterKey,
) -> Result<bool, String> {
    recovery_check_answer(client, base_url, session_token, master_key)
        .await
        .map(|answer| answer == RecoveryCheckAnswer::Matches)
}

/// The 12 words as the phrase's derivation reads them: lower case, letters only, one space between. One wiping
/// buffer, sized up front so it never moves (a moved buffer would leave a copy of the words behind); no word is
/// copied anywhere else (invariant I2, task 2470).
pub fn normalized_phrase(input: &str) -> Result<zeroize::Zeroizing<String>, String> {
    // Never longer than the input: each kept letter is one byte of it, each space stands for at least one.
    let mut phrase = zeroize::Zeroizing::new(String::with_capacity(input.len()));
    let mut words = 0;
    for word in input.split_whitespace() {
        let mut letters = word.chars().filter(char::is_ascii_alphabetic).peekable();
        if letters.peek().is_none() {
            continue;
        }
        if words > 0 {
            phrase.push(' ');
        }
        phrase.extend(letters.map(|letter| letter.to_ascii_lowercase()));
        words += 1;
    }
    if words != 12 {
        return Err("Recovery phrase must contain exactly 12 words.".to_string());
    }
    Ok(phrase)
}

/// [`normalized_phrase`] for a caller that compares the text (the old shell's tests); no phrase path calls it.
pub fn normalize_recovery_phrase_input(input: &str) -> Result<String, String> {
    normalized_phrase(input).map(|phrase| phrase.as_str().to_owned())
}

/// Decode a base64 string (standard alphabet, with or without padding).
pub fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| e.to_string())
}

pub fn encode_base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub const INCORRECT_RECOVERY_PHRASE: &str = "Incorrect recovery phrase. Check your words and try again.";

/// Verification has no persistent side effects; callers fence installation.
pub async fn verify_vault_key_from_phrase(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    recovery_phrase: &str,
) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    let recovery_phrase = normalized_phrase(recovery_phrase)?;
    let master_key_struct = beebeeb_core::recovery::recover_from_phrase(&recovery_phrase)
        .map_err(|_| "Recovery phrase does not match a valid 12-word Beebeeb phrase.".to_string())?;
    if !recovered_key_matches_account(client, base_url, session_token, &master_key_struct).await? {
        // `master_key_struct` zeroizes on drop.
        return Err(INCORRECT_RECOVERY_PHRASE.to_string());
    }
    Ok(master_key_struct.to_bytes())
}
