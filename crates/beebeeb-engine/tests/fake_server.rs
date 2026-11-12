//! Task 2475: the fake server (`bb-fake-server`, `test_api::server`).
//!
//! 1. Its handler signs a real engine in with the fixture account and nobody else, keeps no request, and writes one
//!    access line per answer: method, path, status, never a token, a password or a phrase.
//! 2. It never ships: `fake-server` is not a default feature, the one bin requires it, no app's manifest names it on
//!    any edge, and the Windows packaging packs exactly one file, the app's own exe.
//!
//! The TEST identity is refused on macOS (`EngineIdentity::refusal_on`), so the engine test is not compiled there.

use std::path::Path;

#[cfg(not(target_os = "macos"))]
#[test]
fn the_fixture_signs_in_a_real_engine_and_nobody_else_does() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use beebeeb_engine::app::{Engine, EngineConfig};
    use beebeeb_engine::host::ChannelHost;
    use beebeeb_engine::identity::TEST;
    use beebeeb_engine::signin::gate::{GateState, NextAfterSignIn};
    use beebeeb_engine::signin::memory_store::MemoryCredentials;
    use beebeeb_engine::state_paths;
    use beebeeb_engine::test_api::FakeApi;
    use beebeeb_engine::test_api::fake_account::TEST_PHRASE;
    use beebeeb_engine::test_api::server::{self, FIXTURE_EMAIL, FIXTURE_PASSWORD, FIXTURE_USER_ID};
    use zeroize::Zeroizing;

    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = lines.clone();
    let served = Arc::new(AtomicU64::new(0));
    let account = server::fixture_account();
    let token = account.session_token.clone();
    let listener = server::bind(server::loopback_listen("127.0.0.1:0").unwrap()).unwrap();
    let handler = server::fixture_handler(
        account,
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
        served.clone(),
    );
    let fake = FakeApi::start_on(listener, handler).unwrap();

    let local_data = tempfile::tempdir().unwrap();
    state_paths::init_from_app_local_data_dir(&TEST, local_data.path()).unwrap();
    let config = EngineConfig {
        account_id: beebeeb_engine::config::DesktopConfig::ensure_account_id_for(&TEST).unwrap(),
        state_dir: state_paths::beebeeb_state_dir_for(&TEST).unwrap(),
        api_base: fake.base.clone(),
    };
    let (host, _messages) = ChannelHost::new(|| {});
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("engine opens");
    assert_eq!(engine.restore(), GateState::SignedOut);

    // Another email with the fixture's password: answered as a wrong password before OPAQUE runs.
    let other = engine.sign_in_password("someone@example.invalid", Zeroizing::new(FIXTURE_PASSWORD.into()));
    // The fixture's email with another password: OPAQUE answers, the client finds the password wrong and stops.
    let wrong = engine.sign_in_password(FIXTURE_EMAIL, Zeroizing::new("not the fixture password".into()));
    assert!(other.is_err() && wrong.is_err());
    assert_eq!(engine.gate(), GateState::SignedOut);
    // The fixture: signed in; this computer holds no key of the account yet, so the phrase step comes next (DW-A23).
    let next = engine
        .sign_in_password(FIXTURE_EMAIL, Zeroizing::new(FIXTURE_PASSWORD.into()))
        .expect("the fixture signs in");
    assert_eq!(next, NextAfterSignIn::RecoveryPhrase);
    assert_eq!(engine.gate(), GateState::NeedsRecoveryPhrase);
    assert_eq!(engine.user_id().as_deref(), Some(FIXTURE_USER_ID));
    engine
        .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.into()))
        .expect("the fixture's phrase is the account's");
    assert_eq!(engine.gate(), GateState::Ready);

    let written = lines.lock().unwrap().clone();
    assert_eq!(served.load(Ordering::SeqCst), 6);
    assert_eq!(fake.requests().len(), 0, "the server keeps no request");
    // The secrets check runs before the exact-match assertion so it guards against a future change to the access-line
    // format that leaks a secret (P2-4 of the PR #338 review).
    let secrets = [token.as_str(), FIXTURE_PASSWORD, "abandon", "Bearer"];
    let leaking = written
        .iter()
        .filter(|line| secrets.iter().any(|s| line.contains(s)))
        .count();
    assert_eq!(leaking, 0, "a secret leaked into the access lines: {written:?}");
    assert_eq!(
        written,
        [
            "POST /api/v1/opaque/login-start -> 401",
            "POST /api/v1/opaque/login-start -> 200",
            "POST /api/v1/opaque/login-start -> 200",
            "POST /api/v1/opaque/login-finish -> 200",
            "GET /api/v1/auth/me -> 200",
            "POST /api/v1/auth/verify-recovery-check -> 200",
        ]
    );
}

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn strings(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Every dependency table of a manifest: the three kinds, at the top and under each `[target.<cfg>]`.
fn dependency_tables(manifest: &toml::Value) -> Vec<(String, &toml::value::Table)> {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut tables = Vec::new();
    for kind in KINDS {
        if let Some(table) = manifest.get(kind).and_then(toml::Value::as_table) {
            tables.push((kind.to_string(), table));
        }
    }
    for (cfg, target) in manifest
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
    {
        for kind in KINDS {
            if let Some(table) = target.get(kind).and_then(toml::Value::as_table) {
                tables.push((format!("target.{cfg}.{kind}"), table));
            }
        }
    }
    tables
}

#[test]
fn the_fake_server_is_built_only_when_asked_for_and_no_app_or_package_holds_it() {
    // Every way it could ship, collected: one run names each one it finds.
    let mut wrong: Vec<String> = Vec::new();

    // 1. The engine: not a default feature; it implies the test helpers (a release build refuses them, lib.rs); every
    //    bin of the crate (there is one) requires it.
    let engine: toml::Value = toml::from_str(&read("Cargo.toml")).unwrap();
    let features = engine
        .get("features")
        .and_then(toml::Value::as_table)
        .expect("[features]");
    if strings(features.get("default")).iter().any(|f| f == "fake-server") {
        wrong.push("fake-server is a default feature".into());
    }
    if strings(features.get("fake-server")) != ["test-support"] {
        wrong.push(format!(
            "fake-server implies {:?}, not test-support",
            strings(features.get("fake-server"))
        ));
    }
    let bins = engine
        .get("bin")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    for bin in &bins {
        if strings(bin.get("required-features")) != ["fake-server"] {
            wrong.push(format!("bin {:?} does not require fake-server", bin.get("name")));
        }
    }
    let named = bins
        .iter()
        .filter(|bin| bin.get("name").and_then(toml::Value::as_str) == Some("bb-fake-server"))
        .count();

    // 2. The apps: no manifest names the feature in any spelling; every engine edge is read (normal and dev, both
    //    apps), and only a dev edge carries the test helpers.
    let mut edges = 0;
    for app in [
        "../../native/beebeeb-drive-rune/Cargo.toml",
        "../../src-tauri/Cargo.toml",
    ] {
        let text = read(app);
        let mentions = text.matches("fake-server").count();
        if mentions > 0 {
            wrong.push(format!("{app} names fake-server {mentions} time(s)"));
        }
        let manifest: toml::Value = toml::from_str(&text).unwrap();
        for (kind, table) in dependency_tables(&manifest) {
            let Some(dependency) = table.get("beebeeb-engine") else {
                continue;
            };
            edges += 1;
            let on = strings(dependency.get("features"));
            if on.iter().any(|f| f == "fake-server") {
                wrong.push(format!("{app} [{kind}] turns on fake-server"));
            }
            if on.iter().any(|f| f == "test-support") && !kind.ends_with("dev-dependencies") {
                wrong.push(format!("{app} [{kind}] turns on test-support outside a dev edge"));
            }
        }
    }

    // 3. The Windows packaging (TEST and REAL alike) packs one file, the app's exe: one NSIS `File` command over every
    //    script in the folder, and one staged copy of the -Exe it was given.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/windows");
    let mut scripts: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".nsi") || name.ends_with(".nsh"))
        .collect();
    scripts.sort();
    let file_commands: Vec<String> = scripts
        .iter()
        .flat_map(|name| {
            read(&format!("../../packaging/windows/{name}"))
                .lines()
                .map(str::trim)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|line| {
            line.split_whitespace()
                .next()
                .is_some_and(|word| word.eq_ignore_ascii_case("File"))
        })
        .collect();
    // Case-insensitive: NSIS directives are case-insensitive (File, file, FILE all work).
    if file_commands.len() != 1 || !file_commands[0].eq_ignore_ascii_case(r#"File "${MAINBINARYSRCPATH}""#) {
        wrong.push(format!("the NSIS scripts pack {file_commands:?}"));
    }
    let packager = read("../../packaging/windows/build-installer.ps1");
    let code: Vec<&str> = packager
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect();
    let copies: Vec<&str> = code.iter().copied().filter(|line| line.contains("Copy-Item")).collect();
    if copies != ["Copy-Item -LiteralPath $Exe -Destination $staged"] {
        wrong.push(format!("build-installer.ps1 copies {copies:?}"));
    }
    let packed: Vec<&str> = code
        .iter()
        .copied()
        .filter(|line| line.contains("MAINBINARYSRCPATH"))
        .collect();
    if packed != [r#""/DMAINBINARYSRCPATH=$staged","#] {
        wrong.push(format!("build-installer.ps1 hands makensis {packed:?}"));
    }

    assert!(
        wrong.is_empty(),
        "{} way(s) the fake server could ship: {wrong:#?}",
        wrong.len()
    );
    // The instruments read something: the crate's bins, the bin by name, the engine edges of the two apps.
    assert_eq!(
        (bins.len(), named, edges),
        (1, 1, 4),
        "bins, bins named bb-fake-server, engine edges"
    );
    assert!(scripts.len() >= 4, "the NSIS scripts read: {scripts:?}");
}
