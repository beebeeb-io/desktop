//! Task 2475: the fake server (`bb-fake-server`, `test_api::server`).
//!
//! 1. Its handler signs a real engine in with the fixture account and nobody else, keeps no request, and writes one
//!    access line per answer: method, path, status, never a token, a password or a phrase.
//! 2. It never ships: `fake-server` is not a default feature, the one bin requires it, no app's manifest names it on
//!    any edge, and the Windows packaging packs exactly one file, the app's own exe.
//! 3. A device sign-in (DW-A07 to DW-A10) runs against it end to end (`fake_server/device_sign_in.rs`): the engine opens
//!    the socket on the fake's own address and is shown a code; the fake's second device approves it with ONE HTTP call
//!    and no token, as a check script on the build VM does:
//!    `POST /fake-control/device-approve` with `{"code": "XXXX-XXXX"}` (or `{}`: the one waiting request), and
//!    `"approver": "stranger"` for another account, which is refused; `POST /fake-control/device-deny` (DW-A09);
//!    `POST /fake-control/device-code-life` with `{"seconds": n}` for the codes issued after it (DW-A10);
//!    `GET /fake-control/device-requests` answers `{"waiting": n}`. The routes and the wire: `test_api::fake_device_signin`.
//!
//! The TEST identity is refused on macOS (`EngineIdentity::refusal_on`), so the engine tests are not compiled there.

use std::path::Path;

#[cfg(not(target_os = "macos"))]
#[path = "fake_server/device_sign_in.rs"]
mod device_sign_in;

/// The process's one state folder for the TEST identity, shared by every engine test of this binary (it is set once per
/// process; a second folder would refuse).
#[cfg(not(target_os = "macos"))]
fn test_state_dir() -> std::path::PathBuf {
    use beebeeb_engine::identity::TEST;
    use beebeeb_engine::state_paths;
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    state_paths::init_from_app_local_data_dir(&TEST, dir.path()).unwrap();
    state_paths::beebeeb_state_dir_for(&TEST).unwrap()
}

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
        server::Switches::default(),
    );
    let fake = FakeApi::start_on(listener, handler).unwrap();

    let state_dir = test_state_dir();
    let config = EngineConfig {
        account_id: beebeeb_engine::config::DesktopConfig::ensure_account_id_for(&TEST).unwrap(),
        state_dir,
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
    // Six sign-in requests and the one plan check at the unlock (the No plan gate).
    assert_eq!(served.load(Ordering::SeqCst), 7);
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
            "GET /api/v1/billing/subscription -> 200",
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
    for app in ["../../native/beebeeb-drive-rune/Cargo.toml"] {
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
    let packed_files = [
        r#"File "${MAINBINARYSRCPATH}""#,
        r#"File "${OFFICERENDERSRCPATH}""#,
        r#"File "${MEDIAHELPERSRCPATH}""#,
    ];
    if file_commands.len() != packed_files.len()
        || !file_commands.iter().zip(packed_files).all(|(a, b)| a.eq_ignore_ascii_case(b))
    {
        wrong.push(format!("the NSIS scripts pack {file_commands:?}"));
    }
    let packager = read("../../packaging/windows/build-installer.ps1");
    let code: Vec<&str> = packager
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect();
    let copies: Vec<&str> = code.iter().copied().filter(|line| line.contains("Copy-Item")).collect();
    if copies
        != [
            "Copy-Item -LiteralPath $Exe -Destination $staged",
            "Copy-Item -LiteralPath $OfficeRenderExe -Destination $stagedOffice",
            "Copy-Item -LiteralPath $MediaHelperExe -Destination $stagedMedia",
        ]
    {
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
    // The instruments read something: the crate's bins, the bin by name, the engine edges of the native app (its normal edge and its dev edge).
    assert_eq!(
        (bins.len(), named, edges),
        (1, 1, 2),
        "bins, bins named bb-fake-server, engine edges"
    );
    assert!(scripts.len() >= 4, "the NSIS scripts read: {scripts:?}");
}

// ── Wave 2b, slice S7: whole uploads, "another device saved a newer version", uploads refused on demand ─────────────

mod versions {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    use beebeeb_core::chunk_stream::{ChunkDecryptor, ChunkEncryptor, ExpectedChunkFormat};
    use beebeeb_core::encrypt::{NameMetadata, decrypt_file_metadata_with_key, encrypt_name_for_version};
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    use beebeeb_engine::api_client::{ApiClient, DesktopUploadInitRequest};
    use beebeeb_engine::test_api::FakeApi;
    use beebeeb_engine::test_api::fake_account::FakeAccount;
    use beebeeb_engine::test_api::server::{self, Switches};
    use serde_json::{Value, json};

    const CHUNK: u64 = 1024;
    const MADE: &str = "b0000000-0000-4000-8000-0000000000a1";
    const SEEDED: &str = "b0000000-0000-4000-8000-0000000000a2";

    fn start() -> (Arc<FakeAccount>, FakeApi, Switches) {
        let account = server::fixture_account();
        let switches = Switches::default();
        let listener = server::bind(server::loopback_listen("127.0.0.1:0").unwrap()).unwrap();
        let handler = server::fixture_handler(
            account.clone(),
            Arc::new(|_: &str| {}),
            Arc::new(AtomicU64::new(0)),
            switches.clone(),
        );
        (account, FakeApi::start_on(listener, handler).unwrap(), switches)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// One plain HTTP/1.1 call; the status is `0` when the server hung up without an answer.
    fn http(base: &str, method: &str, path: &str, bearer: Option<&str>, body: &[u8]) -> (u16, Value) {
        let host = base.trim_start_matches("http://");
        let mut stream = TcpStream::connect(host).unwrap();
        let auth = bearer
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer);
        let text = String::from_utf8_lossy(&answer).to_string();
        let Some(status) = text.split_whitespace().nth(1).and_then(|s| s.parse::<u16>().ok()) else {
            return (0, Value::Null);
        };
        let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    /// A control route of the server, called as a rung does: one HTTP call.
    fn control(base: &str, route: &str, body: Value) -> (u16, Value) {
        let (method, path) = route.split_once(' ').unwrap();
        http(base, method, path, None, body.to_string().as_bytes())
    }

    fn master(account: &FakeAccount) -> MasterKey {
        MasterKey::from_bytes(account.master_key)
    }

    /// What the engine's upload does: seal `plain` under the file's key, and the name with the commitment one above
    /// the current one's (none: the first).
    fn seal(
        account: &FakeAccount,
        id: &str,
        name: &str,
        current: Option<&NameMetadata>,
        plain: &[u8],
    ) -> (Vec<Vec<u8>>, String) {
        let key = derive_file_key(&master(account), id.as_bytes());
        let pin = current.map_or(ExpectedChunkFormat::Unrecorded, |meta| meta.chunk_format);
        let mut encryptor = ChunkEncryptor::for_push_with_file_key(key, id, plain.len() as u64, CHUNK)
            .unwrap()
            .with_recorded_pin(pin)
            .unwrap();
        let frames: Vec<Vec<u8>> = plain
            .chunks(CHUNK as usize)
            .map(|piece| encryptor.push_chunk(piece).unwrap().data)
            .collect();
        let commitment = encryptor
            .version_commitment_after(current.and_then(|meta| meta.version).map(|v| v.counter))
            .unwrap();
        encryptor.finish().unwrap();
        let key = derive_file_key(&master(account), id.as_bytes());
        (
            frames,
            encrypt_name_for_version(&key, name, None, pin, commitment).unwrap(),
        )
    }

    /// The stored content of `id` opened against the stored name, the way a reader does.
    fn open(account: &FakeAccount, id: &str) -> (Vec<u8>, NameMetadata) {
        let name = account.stored_name(id).unwrap();
        let key = derive_file_key(&master(account), id.as_bytes());
        let meta = decrypt_file_metadata_with_key(&key, &name).unwrap();
        let mut reader = ChunkDecryptor::for_current_version(&master(account), id, &meta);
        let mut out = Vec::new();
        for frame in account.stored_frames(id).unwrap() {
            out.extend_from_slice(&reader.push_frame(&frame).unwrap().data);
        }
        reader.finish().unwrap();
        (out, meta)
    }

    fn init_request(
        id: &str,
        sealed_name: &str,
        size: usize,
        base: Option<i32>,
        current_name: Option<String>,
    ) -> DesktopUploadInitRequest {
        DesktopUploadInitRequest {
            file_id: Some(id.to_string()),
            file_name: sealed_name.to_string(),
            file_size_bytes: size as u64,
            mime_type: Some("text/plain".into()),
            parent_id: None,
            profile: "standard".into(),
            is_media: false,
            chunk_size_bytes: Some(CHUNK),
            chunk_count: Some((size as u64).div_ceil(CHUNK).max(1)),
            base_version_number: base,
            version_metadata_encrypted: current_name,
        }
    }

    /// Run one whole upload through the engine's own client: init, every chunk, complete. Returns `complete`'s answer.
    async fn upload(
        api: &ApiClient,
        request: DesktopUploadInitRequest,
        frames: &[Vec<u8>],
        name: &str,
    ) -> (Value, String) {
        let session = api.init_upload(&request).await.expect("init");
        assert_eq!(
            session.chunk_count as usize,
            frames.len(),
            "the fake plans the chunks the client asked for"
        );
        for (index, frame) in frames.iter().enumerate() {
            api.upload_session_chunk(&session.upload_session_id, index as u32, frame)
                .await
                .expect("chunk");
        }
        let answer = api
            .complete_upload_session(&session.upload_session_id, Some(name))
            .await
            .expect("complete");
        (answer, session.upload_session_id)
    }

    #[test]
    fn a_whole_upload_of_a_new_file_and_of_a_new_version_completes_and_the_next_ops_show_it() {
        let (account, fake, _switches) = start();
        let api = ApiClient::new(fake.base.clone(), account.session_token.clone(), account.master_key);
        let runtime = runtime();
        let first: Vec<u8> = (0..2500u32).map(|n| (n % 251) as u8).collect();
        let second: Vec<u8> = (0..1500u32).map(|n| (n % 241) as u8).collect();

        // A NEW file: three chunks, the name the client seals for it.
        let (frames1, name1) = seal(&account, MADE, "made-here.txt", None, &first);
        assert_eq!(frames1.len(), 3);
        let head_before = runtime.block_on(api.sync_ops(0)).unwrap().head_seq.unwrap();
        let (answer, session1) = runtime.block_on(upload(
            &api,
            init_request(MADE, &name1, first.len(), None, None),
            &frames1,
            &name1,
        ));
        assert_eq!(answer["version_number"], 1);
        assert_eq!(
            answer["name_encrypted"],
            name1.as_str(),
            "the name is published as it was sent"
        );
        assert_eq!(
            account.stored_frames(MADE).unwrap(),
            frames1,
            "the fake keeps the ciphertext as it came"
        );
        let (state1, updated1) = (account.item_state(MADE).unwrap(), account.item_state(MADE).unwrap().1);
        assert_eq!((state1.0, state1.2), (1, first.len() as u64));
        let (plain, meta1) = open(&account, MADE);
        assert_eq!(
            plain, first,
            "the content opens with the account's own key against the stored name"
        );
        assert_eq!(meta1.version.unwrap().counter, 1);
        // The next sync ops and the snapshot show it.
        let ops = runtime.block_on(api.sync_ops(head_before)).unwrap();
        let kinds: Vec<(&str, &str)> = ops
            .ops
            .iter()
            .map(|op| (op.op_type.as_str(), op.payload["id"].as_str().unwrap()))
            .collect();
        assert_eq!(kinds, [("file_create", MADE)]);
        assert_eq!(ops.ops[0].payload["version_number"], 1);
        let snapshot = runtime.block_on(api.sync_snapshot()).unwrap();
        let node = snapshot
            .nodes
            .iter()
            .find(|n| n["id"] == MADE)
            .expect("the new file is in the snapshot");
        assert_eq!(
            (node["version_number"].as_i64(), node["size_bytes"].as_u64()),
            (Some(1), Some(first.len() as u64))
        );

        // A NEW VERSION of it: the base is checked, the counter follows the current name's.
        let (frames2, name2) = seal(&account, MADE, "made-here.txt", Some(&meta1), &second);
        let stale =
            runtime.block_on(api.init_upload(&init_request(MADE, &name2, second.len(), Some(7), Some(name1.clone()))));
        assert!(stale.is_err(), "a base that is not the current version is refused");
        let cursor = ops.head_seq.unwrap();
        let (answer, _) = runtime.block_on(upload(
            &api,
            init_request(MADE, &name2, second.len(), Some(1), Some(name1.clone())),
            &frames2,
            &name2,
        ));
        assert_eq!(answer["version_number"], 2);
        let (state2, updated2) = (account.item_state(MADE).unwrap(), account.item_state(MADE).unwrap().1);
        assert_eq!((state2.0, state2.2), (2, second.len() as u64));
        assert!(updated2 > updated1, "updated_at moved: {updated1} -> {updated2}");
        assert_eq!(account.stored_frames(MADE).unwrap(), frames2);
        let (plain, meta2) = open(&account, MADE);
        assert_eq!(plain, second);
        assert_eq!(meta2.version.unwrap().counter, 2);
        let ops = runtime.block_on(api.sync_ops(cursor)).unwrap();
        let kinds: Vec<&str> = ops.ops.iter().map(|op| op.op_type.as_str()).collect();
        assert_eq!(
            kinds,
            ["file_rename", "file_update"],
            "a version upload emits both, as the server does"
        );
        assert_eq!(ops.ops[1].payload["version_number"], 2);
        let row = runtime.block_on(api.get_file(MADE)).unwrap();
        assert_eq!(
            (row["version_number"].as_i64(), row["size_bytes"].as_u64()),
            (Some(2), Some(second.len() as u64))
        );
        // A repeated `complete` (the answer was lost) is told it was done, and changes nothing.
        let again = runtime
            .block_on(api.complete_upload_session(&session1, Some(&name1)))
            .unwrap();
        assert_eq!(again["already_completed"], true);
        assert_eq!(account.item_state(MADE).unwrap().0, 2);
    }

    #[test]
    fn another_device_saves_a_newer_version_with_one_http_call_and_the_next_ops_report_it() {
        let (account, fake, _switches) = start();
        let api = ApiClient::new(fake.base.clone(), account.session_token.clone(), account.master_key);
        let runtime = runtime();
        account.add_file_with_content(SEEDED, None, "notes.txt", b"version one");
        runtime
            .block_on(api.upload_thumbnail(SEEDED, "medium", b"sealed picture of version one", None))
            .unwrap();
        assert!(account.has_picture(SEEDED));
        let before = account.item_state(SEEDED).unwrap();
        let cursor = runtime.block_on(api.sync_ops(0)).unwrap().head_seq.unwrap();
        assert!(
            runtime.block_on(api.sync_ops(cursor)).unwrap().ops.is_empty(),
            "nothing has changed yet"
        );

        // One HTTP call, by path.
        let (status, made) = control(
            &fake.base,
            server::NEWER_VERSION_ROUTE,
            json!({ "path": "notes.txt", "content_text": "version two, saved on another device" }),
        );
        assert_eq!(status, 200, "{made}");
        assert_eq!(made["version_number"], 2);
        let after = account.item_state(SEEDED).unwrap();
        assert_eq!(
            (after.0, after.2),
            (2, "version two, saved on another device".len() as u64)
        );
        assert!(after.1 > before.1, "updated_at moved: {} -> {}", before.1, after.1);
        let (plain, meta) = open(&account, SEEDED);
        assert_eq!(plain, b"version two, saved on another device");
        assert_eq!(
            meta.version.unwrap().counter,
            2,
            "the commitment counter is one above the old name's"
        );
        assert!(!account.has_picture(SEEDED), "the old version's picture is cleared");
        let gone = http(
            &fake.base,
            "GET",
            &format!("/api/v1/files/{SEEDED}/thumbnail/medium"),
            Some(&account.session_token),
            b"",
        );
        assert_ne!(gone.0, 200);
        // The next sync ops report it, with the version and the new object version.
        let ops = runtime.block_on(api.sync_ops(cursor)).unwrap();
        let kinds: Vec<&str> = ops.ops.iter().map(|op| op.op_type.as_str()).collect();
        assert_eq!(kinds, ["file_rename", "file_update"]);
        assert_eq!(ops.ops[1].payload["version_number"], 2);
        let row = runtime.block_on(api.get_file(SEEDED)).unwrap();
        assert_eq!(row["version_number"], 2);
        assert_eq!(
            ops.ops[1].payload["current_object_version_id"], row["current_object_version_id"],
            "the op and the row name the same object version"
        );
        assert!(row["current_object_version_id"].is_string());
        // By id, and the refusals.
        let (status, _) = control(
            &fake.base,
            server::NEWER_VERSION_ROUTE,
            json!({ "file_id": SEEDED, "content_text": "three" }),
        );
        assert_eq!(status, 200);
        assert_eq!(account.item_state(SEEDED).unwrap().0, 3);
        assert_eq!(
            control(
                &fake.base,
                server::NEWER_VERSION_ROUTE,
                json!({ "path": "nothing.txt", "content_text": "x" })
            )
            .0,
            404
        );
        assert_eq!(
            control(&fake.base, server::NEWER_VERSION_ROUTE, json!({ "path": "notes.txt" })).0,
            400
        );
    }

    #[test]
    fn uploads_are_refused_on_demand_and_not_by_default() {
        let (account, fake, switches) = start();
        let call = |method: &str, path: &str| http(&fake.base, method, path, Some(&account.session_token), b"{}").0;
        // Off by default: an upload route is reached and answers.
        assert_eq!(call("POST", "/api/v1/uploads/init"), 200);
        // On: the upload routes hang up, other routes are untouched.
        assert_eq!(
            control(&fake.base, server::UPLOADS_REFUSED_ROUTE, json!({ "refused": true })).0,
            200
        );
        assert_eq!(call("POST", "/api/v1/uploads/init"), 0);
        assert_eq!(call("PUT", "/api/v1/uploads/upload-1/chunks/0"), 0);
        assert_eq!(call("POST", "/api/v1/uploads/upload-1/complete"), 0);
        assert_eq!(
            call("GET", "/api/v1/sync/snapshot"),
            200,
            "the rest of the API answers as before"
        );
        // Off again.
        assert_eq!(
            control(&fake.base, server::UPLOADS_REFUSED_ROUTE, json!({ "refused": false })).0,
            200
        );
        assert_eq!(call("POST", "/api/v1/uploads/init"), 200);
        // A line on standard input sets the same switch.
        let stop = server::wait_for_stop(
            std::io::Cursor::new(format!("{}\nstop\n", server::UPLOADS_OFF_LINE)),
            std::time::Duration::from_secs(10),
            switches.clone(),
        )
        .unwrap();
        assert_eq!(stop, server::Stop::StopLine);
        assert_eq!(call("POST", "/api/v1/uploads/init"), 0);
    }
}
