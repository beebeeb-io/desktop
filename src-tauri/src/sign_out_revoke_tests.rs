    use super::loopback_api_for_tests::LoopbackApi;
    use super::*;
    use std::time::{Duration, Instant};

    /// A fixed address nothing listens on (the test build's closed default), so no parallel test can take it over.
    const NOTHING_LISTENS: &str = runner::TEST_API_BASE;

    /// One revoke against `base`, by its closed code.
    async fn revoke_one(token: &str, base: &str) -> &'static str {
        let outcomes = revoke_sessions_for_sign_out(&[token], base).await;
        assert_eq!(outcomes.len(), 1);
        outcomes[0].code()
    }

    /// The bearer token of a raw request, lowercased (every token here is lowercase).
    fn bearer_of(request: &str) -> String {
        request
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("authorization: bearer ")
                    .map(str::to_string)
            })
            .unwrap_or_default()
    }

    /// Every request is the logout; the tokens it carried, sorted.
    fn ended_tokens(api: &LoopbackApi) -> Vec<String> {
        let requests = api.requests();
        for request in &requests {
            assert!(
                request.to_ascii_lowercase().starts_with("post /api/v1/auth/logout http/1.1\r\n"),
                "only the logout reaches the API: {request}"
            );
        }
        let mut tokens: Vec<String> = requests.iter().map(|request| bearer_of(request)).collect();
        tokens.sort();
        tokens
    }

    /// Signed in on this computer: the auth flag and, with `in_memory`, the session (token and keys) in memory, as after
    /// a sign-in (`None`: no session in memory, as on a locked computer). On a Mac the Finder reconciler's stand-in
    /// confirms the sign-out's removal.
    fn signed_in(state: &AppState, id: &str, in_memory: Option<&str>) -> Arc<AccountRuntime> {
        synthesize_single_account(state, AccountId(id.to_string()));
        let acct = state.active_account().expect("synthesized account resolves");
        *acct.session.lock().unwrap() = in_memory.map(|token| Session {
            token: token.to_string(),
            master_key: [7u8; 32],
            email: None,
        });
        set_auth_present(state, true);
        #[cfg(target_os = "macos")]
        {
            let (handle, rx) = crate::finder_setup::driver::FinderSetupHandle::for_test(
                crate::finder_setup::driver::FinderSetupView::initial(
                    crate::finder_setup::launch_location::LaunchLocation::Applications,
                ),
            );
            let _ = state.finder_setup.set(handle);
            tokio::spawn(confirm_every_finder_event(rx));
        }
        acct
    }

    /// The real sign-out of `state`, its revoke sent to `api`, with `seam` (if any) between its steps. It works in the
    /// scratch state directory every sign-out test shares, so it holds that directory's gate throughout. The 30 s bound
    /// and the time it reports start once it has the gate, so a wait for another test's purge counts in neither.
    async fn sign_out(
        state: &AppState,
        api: &str,
        seam: Option<SignOutSeam>,
    ) -> (Result<SignedOut, SignOutFailure>, Duration) {
        crate::state_paths::init_for_test();
        with_the_shared_state_dir(async {
            let started = Instant::now();
            let sign_out = with_sign_out_api_for_test(
                api.to_string(),
                clear_session_impl(state, false, None, UnknownFilesArg::Keep, None),
            );
            let run = async {
                match seam {
                    Some(seam) => SIGN_OUT_SEAM.scope(seam, sign_out).await,
                    None => sign_out.await,
                }
            };
            let result = tokio::time::timeout(Duration::from_secs(30), run)
                .await
                .expect("the sign-out finished within 30 s of having the shared state directory");
            (result, started.elapsed())
        })
        .await
    }

    /// [`sign_out`] of a fresh account `id`, signed in with `in_memory` as the session in memory (`None`: none).
    async fn sign_out_of(
        id: &str,
        in_memory: Option<&str>,
        api: &str,
    ) -> (Result<SignedOut, SignOutFailure>, Arc<AccountRuntime>, Duration) {
        let state = AppState::default();
        let acct = signed_in(&state, id, in_memory);
        let (result, took) = sign_out(&state, api, None).await;
        (result, acct, took)
    }

    /// Every tracing event `body` emits on this thread, as text (a current-thread runtime polls the sign-out here).
    fn logs_of<T>(body: impl FnOnce() -> T) -> (T, String) {
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = Capture(buffer.clone());
        // tracing caches each callsite's interest for the whole process. While only ONE dispatcher is alive,
        // tracing-core takes that interest from the registering thread's own default, so a callsite first hit by another
        // test thread, which has no subscriber, is cached as "never" and this capture misses its events. A second live
        // dispatcher keeps registration on the all-dispatchers path.
        let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let out = tracing::subscriber::with_default(subscriber, body);
        let logs = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        (out, logs)
    }

    fn current_thread_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    /// The fix: a signed-in sign-out sends `POST /api/v1/auth/logout` once, with the session's own token, and clears
    /// the session on this computer.
    #[tokio::test]
    async fn a_sign_out_ends_its_session_on_the_server() {
        let api = LoopbackApi::start("200 OK", "{}");
        let (result, acct, _) =
            sign_out_of("signout-1886-revokes", Some("tok-1886-revoked-on-sign-out"), &api.base).await;
        assert_eq!(ended_tokens(&api), ["tok-1886-revoked-on-sign-out"], "{result:?}");
        assert!(acct.session.lock().unwrap().is_none(), "the session is gone from memory");
        #[cfg(target_os = "macos")]
        assert_eq!(result.map(|done| done.outcome), Ok(SignOutOutcome::Completed));
    }

    /// Review finding 1: a session written while the sign-out runs (a same-account re-sign-in that swaps the token in
    /// memory; on a Mac also a first sign-in that stores one in the Keychain) is cleared by the sign-out, so it is ended
    /// on the server too, as is the session the sign-out began with.
    #[tokio::test]
    async fn a_session_written_while_the_sign_out_runs_is_ended_on_the_server_too() {
        let api = LoopbackApi::start("200 OK", "{}");
        let state = AppState::default();
        let acct = signed_in(&state, "signout-1886-written-during", Some("tok-1886-a-at-the-start"));
        let during = acct.clone();
        let seam: SignOutSeam = Arc::new(move |point| {
            if point == "after_purge" {
                let swapped = replace_session_token_in_memory(&SessionWrite::for_test(), &during, "tok-1886-b-in-memory");
                assert_eq!(swapped, Ok(true), "the premise: a session is in memory to swap");
                #[cfg(target_os = "macos")]
                persist_session_token_to_keychain(
                    &SessionWrite::for_test(),
                    during.id.as_str(),
                    "tok-1886-c-in-the-keychain",
                    None,
                )
                .unwrap();
            }
            Ok(())
        });
        let (result, _) = sign_out(&state, &api.base, Some(seam)).await;
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut expected = vec!["tok-1886-a-at-the-start", "tok-1886-b-in-memory"];
        #[cfg(target_os = "macos")]
        expected.push("tok-1886-c-in-the-keychain");
        expected.sort();
        assert_eq!(ended_tokens(&api), expected, "every session the sign-out took is ended ({result:?})");
        assert!(acct.session.lock().unwrap().is_none(), "none is left in memory");
        #[cfg(target_os = "macos")]
        {
            assert_eq!(result.map(|done| done.outcome), Ok(SignOutOutcome::Completed));
            assert!(!keychain_session_present(acct.id.as_str()), "none is left in the Keychain");
        }
    }

    /// Review findings 2 and 3: by the time the server hears the logout, the sign-out is done on this computer (the
    /// session is cleared; on a Mac it reports signed out) and the engine slot is free, so `sync_status`, Lock, Repair
    /// and an engine start do not wait for a slow server.
    #[tokio::test]
    async fn the_server_hears_the_logout_only_after_the_local_clear_with_the_engine_slot_free() {
        let api = LoopbackApi::never_answering();
        let state = AppState::default();
        let acct = signed_in(&state, "signout-1886-after-the-clear", Some("tok-1886-after-the-clear"));
        let watch = async {
            let deadline = Instant::now() + Duration::from_secs(25);
            while api.requests().is_empty() {
                assert!(Instant::now() < deadline, "the logout never reached the server");
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            (
                acct.engine.try_lock().is_ok(),
                acct.session.lock().unwrap().is_none(),
                *state.auth_present.lock().unwrap(),
            )
        };
        let ((result, took), (slot_free, session_cleared, auth_present)) =
            tokio::join!(sign_out(&state, &api.base, None), watch);
        assert!(slot_free, "the engine slot is free while the logout waits ({result:?})");
        assert!(session_cleared, "the session is cleared before the logout is sent");
        assert!(took >= Duration::from_secs(3), "the unanswered logout was given its 3 s: {took:?}");
        #[cfg(target_os = "macos")]
        {
            assert!(!auth_present, "a Mac reports signed out before the logout is sent");
            assert_eq!(result.map(|done| done.outcome), Ok(SignOutOutcome::Completed));
        }
        #[cfg(not(target_os = "macos"))]
        let _ = auth_present;
    }

    /// Review finding 2, the failure: a Keychain that refuses the removal leaves the token on this Mac, so the sign-out
    /// pauses and ends no session the Mac still holds. Trying again, with the Keychain working, ends it.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_sign_out_whose_keychain_clear_fails_ends_no_session_the_mac_still_holds() {
        let api = LoopbackApi::start("200 OK", "{}");
        let id = "signout-1886-clear-fails";
        persist_session_token_to_keychain(&SessionWrite::for_test(), id, "tok-1886-still-held", None).unwrap();
        let state = AppState::default();
        let acct = signed_in(&state, id, Some("tok-1886-still-held"));
        keychain::test_fail_deletes(Some(id), true);
        let (paused, _) = sign_out(&state, &api.base, None).await;
        keychain::test_fail_deletes(Some(id), false);
        assert!(paused.is_err(), "the sign-out paused: {paused:?}");
        assert!(keychain_session_present(id), "the premise: the token is still in the Keychain");
        assert!(api.requests().is_empty(), "no session the Mac still holds is ended: {:?}", api.requests());
        let (again, _) = sign_out(&state, &api.base, None).await;
        assert_eq!(again.map(|done| done.outcome), Ok(SignOutOutcome::Completed));
        assert_eq!(ended_tokens(&api), ["tok-1886-still-held"], "the second sign-out ends it");
        assert!(acct.session.lock().unwrap().is_none());
    }

    /// Best effort and bounded, as on Windows: a revoke the server refuses, one that cannot reach it, and one the server
    /// never answers each leave the sign-out's result exactly what it is when the revoke succeeds, and the session is
    /// cleared; the one that is never answered is given up within the bound. Each is a warning that names what
    /// happened, and no log line holds the token.
    #[test]
    fn a_revoke_that_fails_or_never_answers_changes_nothing_of_the_sign_out_and_logs_no_token() {
        let runtime = current_thread_runtime();
        let id = "signout-1886-best-effort";
        let ok = LoopbackApi::start("200 OK", "{}");
        let ((expected, _, _), baseline_logs) =
            logs_of(|| runtime.block_on(sign_out_of(id, Some("tok-1886-baseline"), &ok.base)));
        assert_eq!(ok.requests().len(), 1, "the premise: the baseline's revoke reached its API");
        assert!(!baseline_logs.contains("tok-1886-baseline"), "{baseline_logs}");
        #[cfg(target_os = "macos")]
        assert_eq!(expected.as_ref().map(|done| done.outcome), Ok(SignOutOutcome::Completed));

        let refused = LoopbackApi::start("500 Internal Server Error", "{}");
        let silent = LoopbackApi::never_answering();
        for (case, api, token, outcome) in [
            ("refused", refused.base.clone(), "tok-1886-refused", "refused"),
            ("unreachable", NOTHING_LISTENS.to_string(), "tok-1886-unreachable", "unreachable"),
            ("never answers", silent.base.clone(), "tok-1886-never-answers", "timed_out"),
        ] {
            let ((result, acct, took), logs) = logs_of(|| runtime.block_on(sign_out_of(id, Some(token), &api)));
            assert_eq!(result, expected, "{case}: the sign-out's result is the one a revoke that succeeds gives");
            assert!(acct.session.lock().unwrap().is_none(), "{case}: the session is gone from memory");
            assert!(
                logs.lines()
                    .any(|line| line.contains("WARN") && line.contains("outcome") && line.contains(outcome)),
                "{case}: a warning says what happened ({outcome}):\n{logs}"
            );
            assert!(!logs.contains(token), "{case}: no log line holds the token:\n{logs}");
            if case == "never answers" {
                assert!(took >= Duration::from_secs(3), "{case}: it waited the full 3 s, took {took:?}");
            }
        }
        assert_eq!(refused.requests().len(), 1, "the refused revoke reached its API");
        assert_eq!(silent.requests().len(), 1, "the unanswered revoke reached its API");
    }

    /// Review finding 5: a 401 says the server had already ended the session. That is no warning.
    #[test]
    fn a_session_the_server_had_already_ended_is_no_warning() {
        let runtime = current_thread_runtime();
        let api = LoopbackApi::start("401 Unauthorized", "{}");
        let ((result, _, _), logs) = logs_of(|| {
            runtime.block_on(sign_out_of("signout-1886-already-ended", Some("tok-1886-already-ended"), &api.base))
        });
        assert_eq!(api.requests().len(), 1, "the logout was sent ({result:?})");
        assert!(
            !logs.lines().any(|line| line.contains("WARN") && line.contains("sign-out: the server")),
            "no warning about the server:\n{logs}"
        );
        assert!(
            logs.lines()
                .any(|line| line.contains("INFO") && line.contains("outcome") && line.contains("already_ended")),
            "it says the session had already ended:\n{logs}"
        );
        assert!(!logs.contains("tok-1886-already-ended"), "{logs}");
    }

    /// Review finding 9: a logout request that cannot even be built (the API address is not a URL) says so, instead of
    /// claiming the server was unreachable.
    #[test]
    fn a_logout_that_cannot_be_built_says_so() {
        let runtime = current_thread_runtime();
        let ((result, _, _), logs) = logs_of(|| {
            runtime.block_on(sign_out_of("signout-1886-misconfigured", Some("tok-1886-misconfigured"), "not a url"))
        });
        assert!(
            logs.lines()
                .any(|line| line.contains("WARN") && line.contains("outcome") && line.contains("misconfigured")),
            "a warning says the request could not be built ({result:?}):\n{logs}"
        );
        assert!(!logs.contains("tok-1886-misconfigured"), "{logs}");
    }

    /// The revoke reads the server's answer into its closed code: 2xx ended the session, 401 says it had ended
    /// already, any other status is `refused`, and no server is `unreachable`. Each request carries the token.
    #[tokio::test]
    async fn the_revoke_reads_each_answer_of_the_server() {
        for (status, expected) in [
            ("200 OK", "revoked"),
            ("204 No Content", "revoked"),
            ("401 Unauthorized", "already_ended"),
            ("500 Internal Server Error", "refused"),
        ] {
            let api = LoopbackApi::start(status, "{}");
            assert_eq!(revoke_one("tok-1886-answers", &api.base).await, expected, "{status}");
            assert_eq!(ended_tokens(&api), ["tok-1886-answers"], "{status}");
        }
        assert_eq!(revoke_one("tok-1886-unreachable", NOTHING_LISTENS).await, "unreachable");
        assert_eq!(revoke_one("tok-1886-not-a-url", "not a url").await, "misconfigured");
    }

    /// Bounded: a server that never answers is given up after 3 s, not later.
    #[tokio::test]
    async fn the_revoke_gives_up_after_three_seconds() {
        let api = LoopbackApi::never_answering();
        let started = Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(20), revoke_one("tok-1886-bound", &api.base))
            .await
            .expect("the revoke is bounded");
        let took = started.elapsed();
        assert_eq!(outcome, "timed_out");
        assert!(
            took >= Duration::from_secs(3) && took < Duration::from_secs(6),
            "given up at the 3 s bound: took {took:?}"
        );
        assert_eq!(api.requests().len(), 1, "the request did reach the server");
    }

    /// Signed in with no token in memory and none in the Keychain: the sign-out has nothing to end and sends nothing.
    #[tokio::test]
    async fn a_sign_out_with_no_token_anywhere_sends_nothing() {
        let api = LoopbackApi::start("200 OK", "{}");
        let (result, _acct, _) = sign_out_of("signout-1886-no-token", None, &api.base).await;
        assert!(api.requests().is_empty(), "{:?} ({result:?})", api.requests());
    }

    /// A locked Mac keeps the token only in the Keychain: a sign-out from there ends that session (the test build's
    /// Keychain is the in-memory store). Linux has no credential store to hold one.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_locked_mac_ends_the_session_the_keychain_holds() {
        let api = LoopbackApi::start("200 OK", "{}");
        let id = "signout-1886-locked";
        persist_session_token_to_keychain(&SessionWrite::for_test(), id, "tok-1886-in-the-keychain", None).unwrap();
        let (result, acct, _) = sign_out_of(id, None, &api.base).await;
        assert_eq!(ended_tokens(&api), ["tok-1886-in-the-keychain"], "{result:?}");
        assert_eq!(result.map(|done| done.outcome), Ok(SignOutOutcome::Completed));
        assert!(!keychain_session_present(acct.id.as_str()), "and the Keychain's token is cleared");
    }
