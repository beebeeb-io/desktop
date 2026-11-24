//! S1, invariant I1 (contract §6): the recovery phrase, the key it derives and the session token appear in no log
//! line and nothing on stderr or stdout, over the wrong-phrase path and the whole happy path (sign-in, phrase, start,
//! the first pull, stop).
//!
//! The test runs itself again as a child process with a `tracing` subscriber at TRACE that writes to stderr, and reads
//! everything the child printed. The child writes the secrets to compare with into a file of the parent's, never to
//! its output. Mutant: log the phrase when the check refuses it.

mod common;

#[cfg(not(target_os = "macos"))]
const CHILD: &str = "BEEBEEB_LOG_CAPTURE_CHILD";

#[cfg(not(target_os = "macos"))]
#[test]
fn the_phrase_and_key_are_in_no_log() {
    use base64::Engine as _;

    if let Some(secrets) = std::env::var_os(CHILD) {
        return child(std::path::Path::new(&secrets));
    }
    let scratch = tempfile::tempdir().unwrap();
    let secrets = scratch.path().join("secrets.json");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "the_phrase_and_key_are_in_no_log",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(CHILD, &secrets)
        .output()
        .expect("the child runs");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "the child's scenario failed:\n{}",
        tail(&printed)
    );
    // The capture can see what the engine logs: the child's subscriber and the engine's own lines reached it.
    assert!(
        printed.contains("log capture: on"),
        "the child's marker is in the capture"
    );
    assert!(
        printed.contains(" INFO ") && printed.contains("beebeeb_engine"),
        "engine log lines were captured"
    );

    let secrets: serde_json::Value = serde_json::from_slice(&std::fs::read(&secrets).unwrap()).unwrap();
    let key: Vec<u8> = serde_json::from_value(secrets["key"].clone()).unwrap();
    assert_eq!(key.len(), 32);
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    let token = secrets["token"].as_str().unwrap();
    let phrase = secrets["phrase"].as_str().unwrap();
    let base64 = base64::engine::general_purpose::STANDARD;
    let base64_url = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let forbidden = [
        ("the phrase", phrase.to_string()),
        ("the wrong phrase", common::WRONG_PHRASE.to_string()),
        ("the key in hex", hex.clone()),
        ("the key in upper-case hex", hex.to_uppercase()),
        ("the key in base64", base64.encode(&key)),
        ("the key in url-safe base64", base64_url.encode(&key)),
        ("the token", token.to_string()),
    ];
    for (what, secret) in &forbidden {
        assert_eq!(printed.matches(secret.as_str()).count(), 0, "{what} is in the output");
    }
}

/// The scenario, in the child: a wrong phrase, then the whole happy path.
#[cfg(not(target_os = "macos"))]
fn child(secrets: &std::path::Path) {
    use beebeeb_engine::app::SyncStart;
    use zeroize::Zeroizing;

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    tracing::info!("log capture: on");
    let s = common::setup();
    s.sign_in();
    assert!(
        s.engine
            .unlock_with_phrase(Zeroizing::new(common::WRONG_PHRASE.into()))
            .is_err()
    );
    s.unlock();
    assert_eq!(s.engine.start_sync(s.home.join("Beebeeb Test")), Ok(SyncStart::Started));
    assert_eq!(common::wait_for_rows(&s.state_db(), 7), 7);
    s.engine.stop();
    let written = serde_json::json!({
        "key": s.account.master_key.to_vec(),
        "token": s.account.session_token,
        "phrase": s.phrase,
    });
    std::fs::write(secrets, written.to_string()).unwrap();
}

#[cfg(not(target_os = "macos"))]
fn tail(text: &str) -> String {
    text.lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
