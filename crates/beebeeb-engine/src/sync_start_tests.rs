//! The S1 unit tests of the sync start (contract `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md`,
//! §8 S1) that need the handle's private parts or read the source: the view never waits, a failed connect ends the
//! engine task before an API client exists, and the phrase path keeps no plain copy of the words.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use super::{Engine, EngineConfig};
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::signin::memory_store::MemoryCredentials;
use crate::sync_view::Start;

fn source(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

/// The text of the function that starts at `from` in `source`, up to the first line that closes it at the
/// indentation `from` opens at.
fn body<'a>(source: &'a str, from: &str) -> &'a str {
    let start = source.find(from).unwrap_or_else(|| panic!("no {from}"));
    let indent = source[..start].rsplit('\n').next().unwrap_or("").len();
    let close = format!("\n{}}}\n", " ".repeat(indent));
    let end = source[start..]
        .find(&close)
        .unwrap_or_else(|| panic!("{from} does not close"));
    &source[start..start + end]
}

/// `view()` answers while another thread holds the transition lock and the engine slot (a start that registers, a
/// stop that waits for the engine): within 50 ms, from the view's own lock only.
#[test]
fn view_never_waits() {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));

    let (held, holding) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let holder = {
        let engine = engine.clone();
        std::thread::spawn(move || {
            engine.runtime.block_on(async {
                let _step = engine.transition.lock().await;
                let _slot = engine.acct.engine.lock().await;
                held.send(()).unwrap();
                let _ = released.recv();
            })
        })
    };
    holding.recv().unwrap();
    let (answered, answer) = mpsc::channel();
    let reader = {
        let engine = engine.clone();
        std::thread::spawn(move || {
            let asked = Instant::now();
            let view = engine.view();
            let _ = answered.send((asked.elapsed(), view));
        })
    };
    // A view that waits for the slot would only answer after the release below: give it 2 s, then let go.
    let read = answer.recv_timeout(Duration::from_secs(2));
    release.send(()).unwrap();
    holder.join().unwrap();
    reader.join().unwrap();
    let (waited, view) = read.expect("view() answered while the lock and the slot were held");
    assert!(waited < Duration::from_millis(50), "view() took {waited:?}");
    assert_eq!(view.start, Start::NotAsked);
}

/// The runner's failed-connect arm ends the task before any API client is built (invariant I7): the engine task runs
/// only on a root that is registered and whose callbacks are connected.
#[test]
fn the_runner_ends_when_connect_fails() {
    let runner = source("runner.rs");
    let run = body(&runner, "async fn run(");
    let connect = run
        .find("match crate::windows_cf::connect_root(identity, &sync_root) {")
        .expect("run() asks connect_root for its outcome");
    assert!(
        run[..connect].trim_end().ends_with("#[cfg(target_os = \"windows\")]"),
        "the connect is Windows' (and compiled there)"
    );
    let failed = &run[connect..];
    let arm = &failed[failed.find("Err(code) => {").expect("a failed connect has its own arm")..];
    let arm = &arm[..arm.find("\n        }\n").expect("the arm closes")];
    assert!(arm.contains("return;"), "the failed-connect arm ends the task:\n{arm}");
    let client = run.find("ApiClient::new(").expect("run() builds its API client");
    assert!(connect < client, "the connect comes before the API client");
}

/// On the phrase path every buffer that holds the words is a wiping one (invariant I2, task 2470): the normalized
/// phrase is one `Zeroizing` buffer that never moves, the proof normalizes through it, and the handle passes the
/// typed words on without copying them.
#[test]
fn no_phrase_copy_outlives_the_call() {
    let recovery = source("signin/recovery.rs");
    let normalized = body(&recovery, "pub fn normalized_phrase(");
    assert!(
        normalized.contains("zeroize::Zeroizing::new(String::with_capacity(input.len()))"),
        "one wiping buffer, sized up front:\n{normalized}"
    );
    for copy in [
        ".collect",
        "String::new",
        ".to_string()",
        ".to_owned()",
        "format!",
        ".join(",
        "Vec<String>",
    ] {
        assert!(!normalized.contains(copy), "normalized_phrase makes a copy with {copy}");
    }
    let proof = body(&recovery, "pub async fn verify_vault_key_from_phrase(");
    assert!(
        proof.contains("let recovery_phrase = normalized_phrase(recovery_phrase)?;"),
        "{proof}"
    );
    assert!(
        !proof.contains("normalize_recovery_phrase_input("),
        "the proof normalizes through normalized_phrase"
    );

    let app = source("app.rs");
    let unlock = body(&app, "fn unlock_with_phrase_step(");
    assert!(unlock.contains("phrase: Zeroizing<String>"), "{unlock}");
    for copy in [
        "phrase.to_string()",
        "phrase.clone()",
        "phrase.as_str().to_owned()",
        "(*phrase).clone()",
    ] {
        assert!(
            !unlock.contains(copy),
            "unlock_with_phrase copies the words with {copy}"
        );
    }
    assert!(unlock.contains("&phrase)"), "the words go on by reference");
}
