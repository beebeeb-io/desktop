//! The direct-download driver's tests: transport selection, pacing, the pin and
//! window rules, renewal, the pinned-route fallback and the two-version cases.
//!
//! All of them run against `test_support`'s loopback pair, never the network.
//! Moved out of `direct_download.rs` byte-identically; `super` is still
//! `crate::direct_download`, so every `use super::{…}` resolves as before.

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::test_support::*;
    use super::*;

    const FILE_ID: &str = "0b9f3c2e-7d41-4a8e-9c55-2173d0000001";
    const KEY: [u8; 32] = [21; 32];
    /// 11 chunks of 1 KiB (the last one 77 bytes): two grant windows of 8.
    const STRIDE: u64 = 1024;
    const SIZE: usize = 10 * 1024 + 77;

    #[derive(Default)]
    struct Recorder {
        wire: AtomicU64,
        decrypted: Mutex<Vec<u64>>,
    }

    impl Progress for Recorder {
        fn received(&self, wire_bytes: u64) {
            self.wire.fetch_add(wire_bytes, Ordering::Relaxed);
        }
        fn decrypted(&self, plaintext_bytes: u64) {
            self.decrypted.lock().unwrap().push(plaintext_bytes);
        }
    }

    fn new_pair() -> Pair {
        Pair::start(KEY, FILE_ID, STRIDE, SIZE)
    }

    fn client(pair: &Pair) -> ApiClient {
        ApiClient::new(pair.api.base.clone(), "token".into(), KEY)
    }

    async fn run_with(
        api: &ApiClient,
        pair: &Pair,
        transport: Transport,
        kbps: u64,
        progress: &Recorder,
    ) -> Result<Outcome, DirectError> {
        let key = file_key(KEY, FILE_ID);
        let chunk_count = pair.knobs().version.chunk_count();
        download(
            api,
            Request {
                file_id: FILE_ID,
                file_key: &key,
                expected_format: beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
                chunk_count,
                download_kbps_limit: kbps,
                approx_size: SIZE,
                pin: None,
            },
            transport,
            None,
            progress,
        )
        .await
    }

    async fn run(pair: &Pair) -> Result<Outcome, DirectError> {
        run_with(&client(pair), pair, Transport::Auto, 0, &Recorder::default()).await
    }

    fn done(outcome: Result<Outcome, DirectError>) -> (Vec<u8>, FileTransport) {
        match outcome {
            Ok(Outcome::Done { plaintext, transport }) => (plaintext.to_vec(), transport),
            Ok(Outcome::UseApi) => panic!("expected a direct download, got UseApi"),
            Err(e) => panic!("expected a direct download, got {e:?}"),
        }
    }

    fn uses_api(outcome: Result<Outcome, DirectError>) {
        match outcome {
            Ok(Outcome::UseApi) => {}
            Ok(Outcome::Done { transport, .. }) => panic!("expected UseApi, got a {transport:?} download"),
            Err(e) => panic!("expected UseApi, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn direct_is_byte_identical_and_never_touches_the_api_chunk_route() {
        let pair = new_pair();
        let progress = Recorder::default();
        let (bytes, transport) = done(run_with(&client(&pair), &pair, Transport::Auto, 0, &progress).await);
        assert_eq!(bytes, pair.plain(), "byte-identical");
        assert_eq!(transport, FileTransport::Direct);
        assert_eq!(pair.discovery_requests(), 1);
        assert_eq!(pair.grant_requests().len(), 2, "11 chunks in windows of 8");
        assert_eq!(pair.storage_requests(), 11, "one storage GET per chunk");
        assert!(pair.api_chunk_requests().is_empty(), "the API chunk route is never touched");
        // The same numbers the API path reports: every wire byte, then the plaintext so far.
        let encrypted = pair.knobs().version.encrypted_size();
        assert_eq!(progress.wire.load(Ordering::Relaxed), encrypted);
        let decrypted = progress.decrypted.lock().unwrap().clone();
        assert_eq!(decrypted.first(), Some(&0), "the start is reported");
        assert_eq!(decrypted.last(), Some(&(SIZE as u64)));
        assert_eq!(decrypted.len(), 12, "the start, then once per chunk");
        // The first window pins nothing; every later one pins the version the first returned.
        let grants = pair.grant_requests();
        let first: serde_json::Value = serde_json::from_slice(&grants[0].body).unwrap();
        assert_eq!(first["object_version_id"], serde_json::Value::Null);
        assert_eq!(first["start_index"], 0);
        let second: serde_json::Value = serde_json::from_slice(&grants[1].body).unwrap();
        assert_eq!(second["object_version_id"], pair.knobs().version.id.as_str());
        assert_eq!(second["start_index"], 8);
    }

    #[tokio::test]
    async fn the_capability_header_rides_on_grant_requests_only_and_storage_sees_no_account_headers() {
        let pair = new_pair();
        // One chunk via the pinned route too, so that request is checked as well.
        pair.knobs().storage.insert(3, vec![StorageScript::Status(503); 4].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Mixed));
        let api = pair.api.seen();
        assert!(api.len() >= 4, "discovery, two windows (+ renewals), one pinned chunk: {}", api.len());
        for request in &api {
            let capabilities = request.header("x-beebeeb-capabilities");
            if request.path().ends_with("/download-grants") {
                assert_eq!(capabilities, Some("object-download-v1"), "{}", request.path());
            } else {
                assert_eq!(capabilities, None, "only grant requests declare the token: {}", request.path());
            }
            assert_eq!(request.header("authorization"), Some("Bearer token"));
        }
        let storage = pair.storage.seen();
        assert_eq!(storage.len(), 10 + 4, "10 chunks once, chunk 3 once plus 3 renewals");
        for request in &storage {
            for (name, _) in &request.headers {
                let name = name.to_ascii_lowercase();
                assert!(
                    !(name == "authorization" || name == "cookie" || name.starts_with("x-beebeeb")),
                    "storage saw the header {name}"
                );
            }
        }
    }

    #[tokio::test]
    async fn discovery_is_asked_once_per_signed_in_client_and_a_new_client_asks_again() {
        let pair = new_pair();
        let api = client(&pair);
        for round in 0..3 {
            let (bytes, _) = done(run_with(&api, &pair, Transport::Auto, 0, &Recorder::default()).await);
            assert_eq!(bytes, pair.plain(), "round {round}");
        }
        assert_eq!(pair.discovery_requests(), 1, "one request, then the cache");
        // A sign-out or an account switch drops the client, and with it everything it learned.
        drop(api);
        done(run(&pair).await);
        assert_eq!(pair.discovery_requests(), 2, "a new session asks again");
    }

    #[tokio::test]
    async fn servers_without_direct_transport_are_served_over_the_api() {
        // (mode, discovery requests over three downloads with one client)
        let cases = [
            (DiscoveryMode::OfferNothing, 1),
            (DiscoveryMode::Json404, 1),
            (DiscoveryMode::Bare404, 2),
            (DiscoveryMode::Bare405, 2),
            (DiscoveryMode::Status500, 2),
        ];
        for (mode, asked) in cases {
            let pair = new_pair();
            pair.knobs().discovery = mode;
            let api = client(&pair);
            for _ in 0..3 {
                uses_api(run_with(&api, &pair, Transport::Auto, 0, &Recorder::default()).await);
            }
            assert_eq!(pair.discovery_requests(), asked, "{mode:?}");
            assert!(pair.grant_requests().is_empty(), "{mode:?}: no grant without an offer");
            assert_eq!(pair.storage_requests(), 0, "{mode:?}");
        }
    }

    #[tokio::test]
    async fn forcing_the_api_transport_asks_nothing() {
        let pair = new_pair();
        uses_api(run_with(&client(&pair), &pair, Transport::Api, 0, &Recorder::default()).await);
        assert!(pair.api.seen().is_empty(), "no discovery, no grant");
        assert_eq!(pair.storage_requests(), 0);
    }

    #[test]
    fn the_environment_beats_the_config_file_and_anything_else_means_auto() {
        use Transport::{Api, Auto};
        let cases = [
            (None, None, Auto),
            (None, Some("api"), Api),
            (Some("api"), None, Api),
            (Some(" API "), None, Api),
            (Some("auto"), Some("api"), Auto),
            (Some("api"), Some("auto"), Api),
            (Some("direct"), Some("api"), Api),
            (Some("nonsense"), None, Auto),
            (None, Some("nonsense"), Auto),
        ];
        for (env, config, want) in cases {
            assert_eq!(Transport::resolve(env, config), want, "env {env:?} config {config:?}");
        }
    }

    #[tokio::test]
    async fn a_refusal_to_serve_this_file_directly_falls_back_to_the_api() {
        for reason in ["disabled", "speed_cap", "pool_ineligible", "not_eligible"] {
            let pair = new_pair();
            pair.knobs().grants.push_back(GrantScript::Json(
                409,
                json!({"error": "direct_unavailable", "reason": reason}),
            ));
            uses_api(run(&pair).await);
            assert_eq!(pair.storage_requests(), 0, "{reason}");
            assert!(pair.api_chunk_requests().is_empty(), "{reason}: the caller's API path takes it");
        }
        // The server answers with another transport than the one asked for: not direct.
        let pair = new_pair();
        pair.knobs().transport_answer = Some("object-download-v2".into());
        uses_api(run(&pair).await);
        assert_eq!(pair.storage_requests(), 0);
        // A gateway page instead of the contract's JSON.
        let pair = new_pair();
        pair.knobs().grants.push_back(GrantScript::Garbage);
        uses_api(run(&pair).await);
        // A node without the route (mid-rollout).
        let pair = new_pair();
        pair.knobs().grants.push_back(GrantScript::Bare(404));
        uses_api(run(&pair).await);
    }

    #[tokio::test]
    async fn the_manifest_must_agree_with_the_metadata() {
        let pair = new_pair();
        let key = file_key(KEY, FILE_ID);
        let outcome = download(
            &client(&pair),
            Request {
                file_id: FILE_ID,
                file_key: &key,
                expected_format: beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
                chunk_count: 12,
                download_kbps_limit: 0,
                approx_size: SIZE,
                pin: None,
            },
            Transport::Auto,
            None,
            &Recorder::default(),
        )
        .await;
        uses_api(outcome);
        assert_eq!(pair.storage_requests(), 0, "nothing fetched for a file the metadata describes otherwise");
    }

    #[tokio::test]
    async fn an_authorization_refusal_is_terminal_and_never_falls_back() {
        let cases: [(GrantScript, u16); 3] = [
            (GrantScript::Json(401, json!({"error": "unauthorized"})), 401),
            (GrantScript::Json(403, json!({"error": "forbidden"})), 403),
            (GrantScript::Json(404, json!({"error": "not found"})), 404),
        ];
        for (script, status) in cases {
            let pair = new_pair();
            pair.knobs().grants.push_back(script);
            match run(&pair).await {
                Err(DirectError::Refused(got)) => assert_eq!(got, status),
                Ok(Outcome::UseApi) => panic!("{status}: fell back to the API"),
                Ok(Outcome::Done { .. }) => panic!("{status}: downloaded"),
                Err(e) => panic!("{status}: {e:?}"),
            }
            assert_eq!(pair.storage_requests(), 0);
            assert!(pair.api_chunk_requests().is_empty());
        }
        // The pinned route refuses too (a revoked invite between windows): terminal as well.
        let pair = new_pair();
        pair.knobs().storage.insert(0, vec![StorageScript::Status(503); 4].into());
        pair.knobs().api_chunks.insert(0, vec![ApiScript::Status(403)].into());
        assert_eq!(run(&pair).await.err(), Some(DirectError::Refused(403)));
    }

    #[tokio::test]
    async fn a_version_that_goes_away_mid_transfer_returns_nothing() {
        let pair = new_pair();
        {
            let mut k = pair.knobs();
            let replacement = Version::new(&file_key(KEY, FILE_ID), "99999999-2222-4333-8444-555555555555", STRIDE, content(SIZE, 8));
            k.replacement = Some(replacement);
            // The first window is served; the second finds the file replaced.
            k.grants.push_back(GrantScript::Serve);
            k.grants.push_back(GrantScript::Replace);
        }
        assert_eq!(run(&pair).await.err(), Some(DirectError::VersionChanged));
        assert!(pair.api_chunk_requests().is_empty(), "never the other version through the API");
    }

    #[tokio::test]
    async fn an_expired_link_is_renewed_and_the_file_stays_direct() {
        let pair = new_pair();
        pair.knobs().storage.insert(1, vec![StorageScript::Status(403)].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Direct));
        let renewals: Vec<serde_json::Value> = pair
            .grant_requests()
            .iter()
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .filter(|b: &serde_json::Value| b["renewal"] == true)
            .collect();
        assert_eq!(renewals.len(), 1, "one renewal");
        assert_eq!((renewals[0]["start_index"].clone(), renewals[0]["count"].clone()), (json!(1), json!(1)));
        assert_eq!(pair.storage_requests(), 12);
        assert!(pair.api_chunk_requests().is_empty());
    }

    #[tokio::test]
    async fn a_chunk_storage_never_serves_goes_through_the_pinned_api_route() {
        let pair = new_pair();
        pair.knobs().storage.insert(2, vec![StorageScript::Status(503); 10].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!(bytes, pair.plain(), "byte-identical");
        assert_eq!(transport, FileTransport::Mixed);
        let via_api = pair.api_chunk_requests();
        assert_eq!(via_api.len(), 1, "only the chunk storage would not give");
        assert_eq!(via_api[0].path(), format!("/api/v1/files/{FILE_ID}/chunks/2"));
        let pinned = pair.knobs().version.id.clone();
        assert_eq!(via_api[0].query(), format!("object_version_id={pinned}"), "the pinned version");
        let to_storage_for_2 = pair.storage.count(|s| s.path().ends_with("/2.bin"));
        assert_eq!(to_storage_for_2, 4, "one try and three renewals, no more");
    }

    #[tokio::test]
    async fn a_window_refused_after_the_first_moves_the_rest_to_the_pinned_route() {
        let pair = new_pair();
        {
            let mut k = pair.knobs();
            k.grants.push_back(GrantScript::Serve);
            k.grants
                .push_back(GrantScript::Json(409, json!({"error": "direct_unavailable", "reason": "disabled"})));
        }
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Mixed));
        assert_eq!(pair.storage_requests(), 8, "the first window came from storage");
        assert_eq!(pair.api_chunk_requests().len(), 3, "the rest through the pinned route");
        assert!(pair.api_chunk_requests().iter().all(|r| r.query().starts_with("object_version_id=")));
    }

    #[tokio::test]
    async fn a_storage_redirect_is_never_followed() {
        let pair = new_pair();
        let elsewhere = Mock::start(std::sync::Arc::new(|_: &test_support::Seen| Reply::bytes(vec![0; 1052])));
        pair.knobs()
            .storage
            .insert(0, vec![StorageScript::Redirect(format!("{}/{BUCKET}/t/x/0.bin", elsewhere.base))].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Direct));
        assert!(elsewhere.seen().is_empty(), "the redirect target was never asked");
    }

    #[tokio::test]
    async fn a_body_shorter_than_the_grant_is_a_failed_fetch() {
        let pair = new_pair();
        pair.knobs().storage.insert(4, vec![StorageScript::Short].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Direct));
        assert_eq!(pair.storage.count(|s| s.path().ends_with("/4.bin")), 2, "renewed once");
    }

    #[tokio::test]
    async fn a_tampered_storage_chunk_is_refetched_through_the_api_and_a_double_tamper_is_fatal() {
        let pair = new_pair();
        pair.knobs().storage.insert(5, vec![StorageScript::Tamper].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Mixed));
        assert_eq!(pair.api_chunk_requests().len(), 1, "one fresh copy through the API");

        let pair = new_pair();
        pair.knobs().storage.insert(5, vec![StorageScript::Tamper].into());
        pair.knobs().api_chunks.insert(5, vec![ApiScript::Tamper].into());
        assert_eq!(run(&pair).await.err(), Some(DirectError::Corrupt), "never partial plaintext");
    }

    #[tokio::test]
    async fn a_link_to_a_host_discovery_did_not_list_is_never_fetched() {
        let pair = new_pair();
        let unlisted = Mock::start(std::sync::Arc::new(|_: &test_support::Seen| Reply::bytes(vec![0; 1052])));
        pair.knobs().link_authority = Some(unlisted.authority.clone());
        uses_api(run(&pair).await);
        assert!(unlisted.seen().is_empty(), "the unlisted host was never asked");
        assert_eq!(pair.storage_requests(), 0);
    }

    #[tokio::test]
    async fn no_url_or_signature_reaches_a_log_line_or_an_error() {
        // Every storage failure there is, then the pinned route, then a terminal refusal.
        let (results, logs) = capture_logs(async {
            let pair = new_pair();
            {
                let mut k = pair.knobs();
                k.storage.insert(0, vec![StorageScript::HangUp, StorageScript::Status(403)].into());
                k.storage.insert(1, vec![StorageScript::Redirect("https://elsewhere.test/x".into())].into());
                k.storage.insert(2, vec![StorageScript::Short; 4].into());
                k.storage.insert(3, vec![StorageScript::Tamper].into());
            }
            let mixed = run(&pair).await.map(|o| matches!(o, Outcome::Done { .. }));
            // The pinned route itself drops the connection once (reqwest's error names the API URL);
            // its retry gets the chunk.
            let dropped = new_pair();
            dropped.knobs().storage.insert(6, vec![StorageScript::Status(503); 4].into());
            dropped.knobs().api_chunks.insert(6, vec![ApiScript::HangUp].into());
            let (_, transport) = done(run(&dropped).await);
            assert_eq!(transport, FileTransport::Mixed);
            assert_eq!(dropped.api_chunk_requests().len(), 2, "the dropped request and its retry");
            let refused = new_pair();
            refused
                .knobs()
                .grants
                .push_back(GrantScript::Json(403, json!({"error": "forbidden"})));
            let terminal = run(&refused).await.map(|_| ()).map_err(|e| e.to_string());
            (mixed, terminal)
        })
        .await;
        assert_eq!(results.0, Ok(true), "the mixed download completed");
        let error = results.1.unwrap_err();
        assert!(logs.contains("direct download:"), "the capture saw the driver's lines:\n{logs}");
        for needle in [SIGNATURE, SECRET, "AKIDSECRET", BUCKET] {
            assert_eq!(logs.matches(needle).count(), 0, "{needle} in the log:\n{logs}");
            assert!(!error.contains(needle), "{needle} in the error: {error}");
        }
        assert!(!logs.contains("/t/"), "no object path in the log:\n{logs}");
        assert!(!logs.contains("object_version_id="), "no API chunk URL in the log either:\n{logs}");
        assert!(logs.contains("pinned API route: trying again"), "the dropped pinned request was logged:\n{logs}");
    }

    #[test]
    fn the_ceiling_holds_requests_back_by_the_bytes_received() {
        assert_eq!(paced_until_ms(1_000, 10_000, 0), i64::MIN, "no ceiling");
        assert_eq!(paced_until_ms(1_000, 0, 8), 1_000);
        assert_eq!(paced_until_ms(1_000, 8 * 1024, 8), 2_000, "8 KiB at 8 KB/s is one second");
        assert_eq!(paced_until_ms(0, u64::MAX, 1), i64::try_from(u128::from(u64::MAX) * 1000 / 1024).unwrap_or(i64::MAX));
    }

    #[tokio::test]
    async fn the_download_ceiling_applies_to_the_direct_path() {
        let pair = new_pair();
        let started = std::time::Instant::now();
        let (bytes, _) = done(run_with(&client(&pair), &pair, Transport::Auto, 8, &Recorder::default()).await);
        assert_eq!(bytes, pair.plain());
        // 11 chunks of about 1 KiB at 8 KB/s: the eleventh request cannot start before seven chunks
        // (the reorder window is four) have been received, i.e. about 0.9 s.
        let elapsed = started.elapsed();
        assert!(elapsed >= std::time::Duration::from_millis(800), "took {elapsed:?}");
    }

    async fn run_pinned(pair: &Pair, pin: &crate::download_pin::PinnedVersion) -> Result<Outcome, DirectError> {
        let key = file_key(KEY, FILE_ID);
        download(
            &client(pair),
            Request {
                file_id: FILE_ID,
                file_key: &key,
                expected_format: beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
                chunk_count: pin.chunk_count,
                download_kbps_limit: 0,
                approx_size: SIZE,
                pin: Some(pin),
            },
            Transport::Auto,
            None,
            &Recorder::default(),
        )
        .await
    }

    #[tokio::test]
    async fn a_pinned_route_409_is_a_restart_whatever_its_body() {
        // The pinned per-chunk route answers a gone version with its own sentence (server
        // `version_download::unavailable`), not the grant route's `version_unavailable`.
        let pair = new_pair();
        pair.knobs().storage.insert(0, vec![StorageScript::Status(503); 4].into());
        pair.knobs().api_chunks.insert(0, vec![ApiScript::Gone].into());
        assert_eq!(run(&pair).await.err(), Some(DirectError::VersionChanged));
        assert_eq!(pair.api_chunk_requests().len(), 1, "never the file again without its pin");
    }

    #[tokio::test]
    async fn the_pinned_route_rides_out_a_rate_limit_and_a_server_hiccup() {
        let pair = new_pair();
        pair.knobs().storage.insert(0, vec![StorageScript::Status(503); 4].into());
        // Retry-After 2 s is longer than the 1 s the retry would wait on its own: it must be honoured.
        pair.knobs().api_chunks.insert(0, vec![ApiScript::RateLimited(2), ApiScript::Status(503)].into());
        let (bytes, transport) = done(run(&pair).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Mixed));
        let pinned = pair.api_chunk_requests();
        assert_eq!(pinned.len(), 3, "429, 503, then the chunk");
        // Measured between the requests the server saw: the storage renewals before them do not count.
        let after_429 = pinned[1].at.duration_since(pinned[0].at);
        assert!(after_429 >= std::time::Duration::from_millis(1950), "Retry-After 2 s was honoured: {after_429:?}");
        let after_503 = pinned[2].at.duration_since(pinned[1].at);
        assert!(after_503 >= std::time::Duration::from_millis(1950), "then the doubled 2 s: {after_503:?}");
    }

    #[tokio::test]
    async fn a_transfer_that_lands_on_another_version_than_the_metadata_named_restarts() {
        // The metadata named A; B (with more chunks) completed before the first grant.
        let pair = new_pair();
        let named = pair.knobs().version.pin();
        let more = Version::new(&file_key(KEY, FILE_ID), "99999999-2222-4333-8444-555555555555", STRIDE, content(SIZE + 5000, 8));
        pair.knobs().replacement = Some(more);
        pair.knobs().grants.push_back(GrantScript::Replace);
        assert_eq!(run_pinned(&pair, &named).await.err(), Some(DirectError::VersionChanged));
        assert_eq!(pair.storage_requests(), 0, "nothing of the other version was fetched");
        // The version the metadata named: direct, as before.
        let pair = new_pair();
        let named = pair.knobs().version.pin();
        let (bytes, transport) = done(run_pinned(&pair, &named).await);
        assert_eq!((bytes, transport), (pair.plain(), FileTransport::Direct));
    }

    #[tokio::test]
    async fn a_grant_whose_sizes_are_not_the_manifests_is_left_to_the_api_path() {
        // Task 1643: the same version id, but the grant states other sizes than the metadata's
        // manifest: the API path, which checks every chunk against the manifest, serves it.
        let pair = new_pair();
        let mut pin = pair.knobs().version.pin();
        pin.plaintext_size_bytes += 1;
        pin.ciphertext_size_bytes += 1;
        uses_api(run_pinned(&pair, &pin).await);
        assert_eq!(pair.storage_requests(), 0);
    }
