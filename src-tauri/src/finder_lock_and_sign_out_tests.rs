    use super::*;
    use crate::account::{AccountId, AccountRuntime, synthesize_single_account};
    use crate::finder_setup::error::FpError;
    use crate::runner::EngineRunner;
    use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
    use finder_setup::launch_location::LaunchLocation;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    type Seen = Arc<Mutex<Vec<String>>>;

    /// Long enough for the stand-in to answer, short enough for a test: the override of the
    /// production waits (45 s and 60 s) for the tests that need a wait to run out.
    const SHORT_WAIT: Duration = Duration::from_millis(150);

    #[derive(Clone, Copy)]
    enum Reconciler {
        /// Acknowledges every Lock and removes successfully.
        Answers,
        /// Receives events and never answers them (a check stuck in a bridge call).
        Silent,
        /// Receives a Lock or Remove and drops its acknowledgement unanswered (the task died mid-event).
        DropsTheAck,
        /// Acknowledges a Lock; the removal fails with an OS error whose message names a path.
        RemovalFails,
        /// Task 1882: acknowledges a Lock; the removal works and macOS kept files that had not reached the
        /// server, in `KEPT_FOLDER`.
        KeepsFiles,
        /// Task 1882 review M2: the removal fails and macOS kept files all the same.
        KeepsFilesAndFails,
    }

    /// Task 1882: where the stand-in says macOS kept the files that had not reached the server.
    const KEPT_FOLDER: &str = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";

    fn kept_folder() -> finder_removal::KeptFolder {
        finder_removal::KeptFolder::Kept {
            path: KEPT_FOLDER.to_string(),
            contents_checked: true,
            empty: false,
        }
    }

    struct Fixture {
        app: tauri::App<tauri::test::MockRuntime>,
        acct: Arc<AccountRuntime>,
        seen: Seen,
        task: Option<tokio::task::JoinHandle<()>>,
    }

    impl Fixture {
        fn new(id: &str) -> Self {
            // A sign-out that completes runs the local-data purge, which resolves the state dir (a throwaway one).
            crate::state_paths::init_for_test();
            let app = tauri::test::mock_app();
            app.manage(AppState::default());
            let state = app.state::<AppState>();
            synthesize_single_account(&state, AccountId(id.to_string()));
            let acct = state.active_account().unwrap();
            Self {
                app,
                acct,
                seen: Arc::default(),
                task: None,
            }
        }

        fn state(&self) -> tauri::State<'_, AppState> {
            self.app.state::<AppState>()
        }

        /// Install the stand-in. `None` is a reconciler that has stopped: the receiving end is gone.
        /// Call it inside the runtime that runs the test.
        fn reconciler(&mut self, mode: Option<Reconciler>) {
            let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
            let _ = self.state().finder_setup.set(handle);
            let Some(mode) = mode else {
                drop(rx);
                return;
            };
            let (acct, seen) = (self.acct.clone(), self.seen.clone());
            self.task = Some(tokio::spawn(async move {
                let (mut held_removals, mut held_locks) = (Vec::new(), Vec::new());
                while let Some(event) = rx.recv().await {
                    // Where the engine is when the reconciler handles the event: it must still be
                    // tracked, because Lock and Sign-out tell the reconciler BEFORE they stop it.
                    let engine = match acct.engine.try_lock() {
                        Ok(slot) if slot.is_some() => "engine_tracked",
                        Ok(_) => "no_engine",
                        Err(_) => "slot_busy",
                    };
                    match event {
                        Event::Remove { trigger, ack } => {
                            seen.lock()
                                .unwrap()
                                .push(format!("remove:{}:{engine}", trigger.as_str()));
                            match mode {
                                Reconciler::Answers => {
                                    let _ = ack.send((Ok(()), finder_removal::KeptFolder::default()));
                                }
                                Reconciler::Silent => held_removals.push(ack),
                                Reconciler::DropsTheAck => drop(ack),
                                Reconciler::RemovalFails => {
                                    let message = "The folder \u{201c}/Users/sam/Secret Folder\u{201d} already exists.";
                                    let _ = ack.send((
                                        Err(FpError::new("NSCocoaErrorDomain", 516, message)),
                                        finder_removal::KeptFolder::default(),
                                    ));
                                }
                                Reconciler::KeepsFiles => {
                                    let _ = ack.send((Ok(()), kept_folder()));
                                }
                                Reconciler::KeepsFilesAndFails => {
                                    let _ = ack.send((
                                        Err(FpError::new("NSFileProviderErrorDomain", -1001, "busy")),
                                        kept_folder(),
                                    ));
                                }
                            }
                        }
                        Event::Lock { ack } => {
                            seen.lock().unwrap().push(format!("lock:{engine}"));
                            match mode {
                                Reconciler::Answers
                                | Reconciler::RemovalFails
                                | Reconciler::KeepsFiles
                                | Reconciler::KeepsFilesAndFails => {
                                    let _ = ack.send(());
                                }
                                Reconciler::Silent => held_locks.push(ack),
                                Reconciler::DropsTheAck => drop(ack),
                            }
                        }
                        Event::Trigger(trigger) => seen.lock().unwrap().push(format!("trigger:{}", trigger.as_str())),
                        Event::AppActivated => seen.lock().unwrap().push("app_activated".to_string()),
                    }
                }
            }));
        }

        fn install_keys(&self) {
            *self.acct.session.lock().unwrap() = Some(Session {
                token: "tok-abc".into(),
                master_key: [9u8; 32],
                email: Some("sam.keys@beebeeb.io".into()),
            });
            set_auth_present(&self.state(), true);
        }

        /// An engine whose task has already ended: stops at once.
        async fn install_finished_engine(&self) {
            let task = tokio::spawn(async {});
            tokio::task::yield_now().await;
            *self.acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));
        }

        /// An engine that ticks until it is stopped. It ignores the cancel signal, so `abort()`
        /// waits out its 3 s graceful window and then forces it down.
        async fn install_ticking_engine(&self) -> Arc<AtomicUsize> {
            let ticks = Arc::new(AtomicUsize::new(0));
            let counter = ticks.clone();
            let task = tokio::spawn(async move {
                loop {
                    counter.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            });
            *self.acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));
            ticks
        }

        /// What the stand-in saw, once it has handled everything sent before this call.
        async fn finish(self) -> Vec<String> {
            let Fixture {
                app,
                acct: _,
                seen,
                task,
            } = self;
            drop(app);
            if let Some(mut task) = task
                && tokio::time::timeout(Duration::from_secs(3), &mut task).await.is_err()
            {
                task.abort();
            }
            seen.lock().unwrap().clone()
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn multi_thread_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    /// With no reconciler at all, a sign-out's or a Repair's removal and a Lock's hold fail with the sentence a person
    /// sees for a reconciler that is not running, the same one the handle gives when its reconciler has stopped.
    #[test]
    fn with_no_reconciler_a_removal_and_a_lock_say_finder_setup_is_not_running() {
        let not_running =
            Err("Beebeeb’s Finder setup isn’t running. Quit and reopen Beebeeb to start it again.".to_string());
        let state = AppState::default();
        runtime().block_on(async {
            for trigger in [
                finder_setup::core::Trigger::SignOut,
                finder_setup::core::Trigger::Repair,
            ] {
                // (Task 1882: the removal's answer also carries what macOS kept; with no reconciler, nothing.)
                let removal = finder_remove_for(&state, trigger).await;
                assert_eq!(
                    removal.as_ref().err().map(|failure| failure.kept.clone()),
                    Some(finder_removal::KeptFolder::default()),
                    "{trigger:?}"
                );
                assert_eq!(
                    removal.map(|_| ()).map_err(|failure| failure.message),
                    not_running,
                    "{trigger:?}"
                );
            }
            assert_eq!(finder_lock_for(&state).await, not_running);
        });
    }

    // ---- Lock ----

    #[test]
    fn lock_is_acknowledged_before_the_engine_stops_and_then_clears_the_keys_and_the_engine() {
        let mut fx = Fixture::new("finder-lock-answers");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            let ticks = fx.install_ticking_engine().await;
            lock_vault(fx.state())
                .await
                .expect("a lock the reconciler acknowledged succeeds");
            assert!(fx.acct.session.lock().unwrap().is_none(), "the keys are cleared");
            assert!(fx.acct.engine.lock().await.is_none(), "the engine is out of its slot");
            let after = ticks.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(ticks.load(Ordering::SeqCst), after, "and it is stopped");
            assert!(!fx.acct.engine_stop_unconfirmed.load(Ordering::SeqCst));
            // Item D: this lock read the credential store through the in-memory test store.
            assert!(
                keychain::test_store_touched(Some("finder-lock-answers")),
                "the lock went through the in-memory Keychain store"
            );
            assert_eq!(
                fx.finish().await,
                vec!["lock:engine_tracked"],
                "the reconciler held while the engine still ran"
            );
        });
    }

    /// Lead ruling 1, the timeout path: no acknowledgement within the wait is not a plain success, and the
    /// lock still clears what it can (ruling 2): the keys and the engine. The lock happened, so it is `Ok`, with a
    /// warning that says what is unconfirmed (FB-24).
    #[test]
    fn lock_the_reconciler_never_acknowledges_clears_everything_and_warns_it_is_unconfirmed() {
        let mut fx = Fixture::new("finder-lock-silent");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Silent));
            fx.install_keys();
            fx.install_finished_engine().await;
            let started = Instant::now();
            let outcome = FINDER_WAIT_OVERRIDE
                .scope(SHORT_WAIT, lock_vault(fx.state()))
                .await
                .expect("the lock happened");
            assert!(started.elapsed() >= SHORT_WAIT, "it waited for the acknowledgement");
            assert_eq!(
                outcome.warning,
                Some(ActionWarning::FinderLockUnconfirmed),
                "a lock nobody acknowledged is not a plain success"
            );
            assert!(
                fx.acct.session.lock().unwrap().is_none(),
                "the keys are cleared all the same"
            );
            assert!(fx.acct.engine.lock().await.is_none(), "and the engine is stopped");
            assert_eq!(fx.finish().await, vec!["lock:engine_tracked"]);
        });
    }

    /// Lead rulings 1 and 5, the error paths: a reconciler that has stopped, and one that dies
    /// holding the acknowledgement. Both complete the lock's own work and report it, as a warning (FB-24).
    #[test]
    fn lock_with_a_dead_reconciler_or_a_dropped_acknowledgement_clears_everything_but_reports_it() {
        for (id, mode) in [
            ("finder-lock-dead", None),
            ("finder-lock-dropped", Some(Reconciler::DropsTheAck)),
        ] {
            let mut fx = Fixture::new(id);
            runtime().block_on(async {
                fx.reconciler(mode);
                fx.install_keys();
                fx.install_finished_engine().await;
                let outcome = lock_vault(fx.state()).await.expect("the lock happened");
                assert_eq!(
                    outcome.warning,
                    Some(ActionWarning::FinderLockUnconfirmed),
                    "{id}: a lock the reconciler cannot confirm is not a plain success"
                );
                assert!(fx.acct.session.lock().unwrap().is_none(), "{id}: the keys are cleared");
                assert!(fx.acct.engine.lock().await.is_none(), "{id}: the engine is stopped");
                let sentence = ActionWarning::FinderLockUnconfirmed.sentence();
                assert!(!sentence.contains('/') && !sentence.contains("NS"), "{id}: {sentence}");
                fx.finish().await;
            });
        }
    }

    /// Lead ruling 3: lock honours `engine_stop_unconfirmed` after taking the slot, as sign-out does.
    #[test]
    fn lock_after_an_unconfirmed_engine_stop_clears_the_keys_and_says_to_restart() {
        let mut fx = Fixture::new("finder-lock-flagged");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            fx.acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
            let outcome = lock_vault(fx.state()).await.expect("the lock happened");
            assert_eq!(
                outcome.warning,
                Some(ActionWarning::EngineStopUnconfirmed),
                "never plain success"
            );
            let sentence = ActionWarning::EngineStopUnconfirmed.sentence();
            assert!(sentence.contains("Restart Beebeeb"), "{sentence}");
            assert!(
                fx.acct.session.lock().unwrap().is_none(),
                "the keys are cleared all the same"
            );
            fx.finish().await;
        });
    }

    /// Lead ruling 3: a lock whose own stop cannot be confirmed closes the gate for every start.
    #[test]
    fn lock_whose_own_engine_stop_cannot_be_confirmed_closes_the_gate_and_says_to_restart() {
        let mut fx = Fixture::new("finder-lock-unconfirmed");
        multi_thread_runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            // No await point: tokio cannot force-abort it. A wall-clock deadline ends the thread.
            let deadline = Instant::now() + Duration::from_secs(7);
            let task = tokio::task::spawn_blocking(move || {
                while Instant::now() < deadline {
                    std::hint::black_box(());
                }
            });
            *fx.acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));

            let outcome = lock_vault(fx.state()).await.expect("the lock happened");
            assert_eq!(
                outcome.warning,
                Some(ActionWarning::EngineStopUnconfirmed),
                "an engine that may still run is not a plain success"
            );
            assert!(
                fx.acct.session.lock().unwrap().is_none(),
                "the keys are cleared in memory all the same"
            );
            assert!(
                fx.acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
                "no start may run beside it"
            );
            assert!(fx.acct.engine.lock().await.is_none());
            // The gate is closed for the reconciler's starts too, even with the keys back.
            fx.install_keys();
            let refused = keys_for_engine_start(&fx.acct).expect_err("a start refuses while the stop is unconfirmed");
            assert!(refused.to_string().contains("Restart Beebeeb"), "{refused}");
            fx.finish().await;
        });
    }

    /// Lead ruling 2, behaviourally and deterministically: the lock is frozen at the moment it is about
    /// to clear the session (this thread holds the session mutex, and the lock runs on its own), and the
    /// engine slot must still be held then. A lock that released the slot first would let a start that
    /// was waiting for it read the keys the lock is about to clear.
    #[test]
    fn lock_holds_the_engine_slot_until_the_session_is_cleared() {
        let mut fx = Fixture::new("finder-lock-freeze");
        let rt = multi_thread_runtime();
        rt.block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            // An engine that ends by itself after 300 ms, so the lock holds the slot for that long
            // while it waits for it, and then goes on to clear the session.
            let task = tokio::spawn(tokio::time::sleep(Duration::from_millis(300)));
            *fx.acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));
        });
        let acct = fx.acct.clone();
        let state = fx.state();
        let (slot_held_at_the_freeze, session_still_there) = std::thread::scope(|scope| {
            let freeze = acct.session.lock().unwrap(); // the lock cannot clear the session while this is held
            let locking = scope.spawn(move || runtime().block_on(lock_vault(state)));
            // The lock takes the slot (the engine is still stopping); then it stops the engine, which ends
            // the slot's 300 ms of work, and blocks on the session mutex.
            let deadline = Instant::now() + Duration::from_secs(10);
            while acct.engine.try_lock().is_ok() {
                assert!(Instant::now() < deadline, "the lock never took the engine slot");
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(1000)); // well past the engine's end
            let held = acct.engine.try_lock().is_err();
            let kept = freeze.is_some();
            drop(freeze);
            locking
                .join()
                .unwrap()
                .expect("the lock succeeds once the session mutex is free");
            (held, kept)
        });
        assert!(session_still_there, "the freeze held: the session was not cleared yet");
        assert!(
            slot_held_at_the_freeze,
            "the engine slot was released before the session was cleared"
        );
        assert!(
            acct.session.lock().unwrap().is_none(),
            "and then the session is cleared"
        );
        assert!(acct.engine.try_lock().is_ok(), "and the slot is free again");
        rt.block_on(fx.finish());
    }

    /// Drives `command` (a lock, here) until it has the engine slot, and returns with `command` still pending, to be
    /// awaited next (task 1891). It waits for the command's own signal, not for a look at the slot: a look
    /// (`try_lock`) cannot tell the command from anyone else holding the slot for a moment, and the stand-in
    /// reconciler does exactly that when it handles the Lock, so a start released by a look could get the slot
    /// first, with the old engine still in it. A command that RETURNS before it signalled never took the slot (it
    /// failed first): the test fails at once with the command's own answer, not later with a message about a timeout.
    /// A command that does neither within 60 s fails the test instead of hanging it.
    async fn until_it_holds_the_slot<F>(what: &str, signal: &tokio::sync::Notify, mut command: std::pin::Pin<&mut F>)
    where
        F: std::future::Future,
        F::Output: std::fmt::Debug,
    {
        tokio::select! {
            biased;
            () = signal.notified() => {}
            answer = &mut command => panic!("{what} returned before it took the engine slot: {answer:?}"),
            () = tokio::time::sleep(Duration::from_secs(60)) => panic!("{what} neither took the engine slot nor returned in 60 s"),
        }
    }

    /// The same ordering as a race, end to end: starts that are waiting for the engine slot while a
    /// lock holds it find no keys when they get it. This is a smoke test, not the pin: it catches an
    /// early release most of the time on an idle machine and can miss it under load (a start has to
    /// wake inside the few microseconds between the release and the clear). The pins are the freeze
    /// test above and `lock_and_sign_out_clear_the_session_before_the_engine_slot_is_released`.
    #[test]
    fn a_start_queued_behind_a_lock_finds_no_keys() {
        let mut fx = Fixture::new("finder-lock-race");
        multi_thread_runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            fx.install_ticking_engine().await; // abort() holds the slot for its 3 s window
            let lock_holds_the_slot = Arc::new(tokio::sync::Notify::new());
            let (locked, starts) = {
                let lock = LOCK_HOLDS_THE_ENGINE_SLOT.scope(lock_holds_the_slot.clone(), lock_vault(fx.state()));
                tokio::pin!(lock);
                until_it_holds_the_slot("the lock", &lock_holds_the_slot, lock.as_mut()).await;
                let starts = (0..8)
                    .map(|_| {
                        let acct = fx.acct.clone();
                        tokio::spawn(async move {
                            let slot = acct.engine.lock().await;
                            let keys = keys_for_engine_start(&acct);
                            drop(slot);
                            keys.is_err()
                        })
                    })
                    .collect::<Vec<_>>();
                (lock.await, starts)
            };
            locked.expect("the lock succeeds");
            for start in starts {
                assert!(
                    start.await.unwrap(),
                    "a start that got the slot after the lock found the keys"
                );
            }
            fx.finish().await;
        });
    }

    /// Lead ruling T9-starts, behaviourally and through the real gate (`authorize_engine_start` is everything
    /// `spawn_bound_engine` does before its one spawn): starts that queued for the engine slot while a lock held
    /// it get no keys when they get it, so no engine is spawned after the lock's clear, and nothing is bound
    /// (no `state.db` is even created) for a vault that is locked. A start after the lock returned finds the same.
    /// The pins for the lock's own order are the freeze test above and the source tests; this is the start's side.
    #[test]
    fn an_unlock_racing_a_lock_spawns_no_engine_after_the_locks_clear() {
        let mut fx = Fixture::new("finder-lock-race-gate");
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_path_buf();
        multi_thread_runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            fx.install_ticking_engine().await; // abort() holds the slot for its 3 s window
            let handle = fx.app.handle().clone();
            let lock_holds_the_slot = Arc::new(tokio::sync::Notify::new());
            let (locked, starts) = {
                let lock = LOCK_HOLDS_THE_ENGINE_SLOT.scope(lock_holds_the_slot.clone(), lock_vault(fx.state()));
                tokio::pin!(lock);
                until_it_holds_the_slot("the lock", &lock_holds_the_slot, lock.as_mut()).await;
                let starts = (0..8)
                    .map(|_| {
                        let (acct, handle, state_dir) = (fx.acct.clone(), handle.clone(), state_dir.clone());
                        tokio::spawn(async move {
                            let slot = acct.engine.lock().await;
                            let state = handle.state::<AppState>();
                            let outcome = authorize_engine_start(
                                &state,
                                &acct,
                                &slot,
                                &LocalDataPaths::for_test(&state_dir, &state_dir),
                            );
                            (
                                outcome.map(|permit| matches!(permit, StartPermit::Go(_))),
                                slot.is_none(),
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                (lock.await, starts)
            };
            locked.expect("the lock succeeds");
            for start in starts {
                assert_eq!(
                    start.await.unwrap(),
                    (Ok(false), true),
                    "a start that got the slot after the lock found no keys to start with"
                );
            }
            // And a start from a caller that waited for nothing, after the lock has returned.
            let slot = fx.acct.engine.lock().await;
            let state = fx.state();
            assert_eq!(
                authorize_engine_start(
                    &state,
                    &fx.acct,
                    &slot,
                    &LocalDataPaths::for_test(&state_dir, &state_dir)
                )
                .map(|permit| matches!(permit, StartPermit::Go(_))),
                Ok(false)
            );
            drop(slot);
            assert!(
                !state_dir.join("state.db").exists(),
                "nothing was bound for a vault that is locked"
            );
            fx.finish().await;
        });
    }

    // ---- A poisoned session mutex (fix round 1, item C) ----

    /// Poison `acct.session`: a thread panics while holding it. The key is still inside.
    fn poison_session(acct: &Arc<AccountRuntime>) {
        let held = acct.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.session.lock().unwrap();
            panic!("poisoning the session mutex on purpose (a test)");
        })
        .join();
        assert!(acct.session.is_poisoned(), "the premise: the mutex is poisoned");
    }

    #[test]
    fn lock_clears_the_session_behind_a_poisoned_mutex() {
        let mut fx = Fixture::new("finder-lock-poisoned");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            fx.install_finished_engine().await;
            poison_session(&fx.acct);
            lock_vault(fx.state())
                .await
                .expect("the lock cleared the key, so it may say so");
            // (Reading through the poison, so a key that was left behind shows as a failed assertion here.)
            assert!(
                fx.acct.session.lock().unwrap_or_else(|e| e.into_inner()).is_none(),
                "the key is NOT left in memory behind the poison"
            );
            assert!(
                !fx.acct.session.is_poisoned(),
                "and the poison is cleared, so the next unlock works"
            );
            fx.finish().await;
        });
    }

    /// FB-24: a signed-in sign-out whose Finder removal fails still happens (keys, engine and credentials are cleared),
    /// so it is `Ok(Completed)` with the `finder_removal_unconfirmed` warning, not an error.
    #[test]
    fn a_signed_in_sign_out_whose_removal_fails_completes_with_a_warning() {
        let mut fx = Fixture::new("finder-signout-completed-warning");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::RemovalFails));
            fx.install_keys();
            fx.install_finished_engine().await;
            let done = clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None)
                .await
                .expect("the sign-out happened");
            assert_eq!(
                done,
                SignedOut {
                    outcome: SignOutOutcome::Completed,
                    warning: Some(ActionWarning::FinderRemovalUnconfirmed),
                    preserved_location: None,
                    kept_unsent: None,
                }
            );
            assert!(fx.acct.session.lock().unwrap().is_none(), "the keys are cleared");
            assert!(fx.acct.engine.lock().await.is_none(), "the engine is stopped");
            assert!(!*fx.state().auth_present.lock().unwrap(), "signed out");
            fx.finish().await;
        });
    }

    /// The full sign-out (it is safe now: its Keychain clear goes to the in-memory test store).
    #[test]
    fn sign_out_clears_the_session_behind_a_poisoned_mutex() {
        let mut fx = Fixture::new("finder-signout-poisoned");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            fx.install_finished_engine().await;
            poison_session(&fx.acct);
            let outcome = clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None)
                .await
                .expect("a sign-out that cleared the key");
            assert_eq!(outcome, SignedOut::confirmed(SignOutOutcome::Completed));
            assert!(
                fx.acct.session.lock().unwrap_or_else(|e| e.into_inner()).is_none(),
                "the key is NOT left in memory behind the poison"
            );
            assert!(!fx.acct.session.is_poisoned(), "and the poison is cleared");
            assert!(
                keychain::test_store_touched(Some("finder-signout-poisoned")),
                "its Keychain clear went to the in-memory store"
            );
            fx.finish().await;
        });
    }

    /// Ruling P (Task 12 fix round 2): a sign-out ends the session an engine-start refusal was found for, so
    /// `sync_status` no longer shows it.
    #[test]
    fn a_sign_out_clears_the_engine_refusal() {
        let mut fx = Fixture::new("t12f2p-sign-out-clears-the-refusal");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            fx.install_finished_engine().await;
            *fx.acct.engine_refusal.lock().unwrap() = Some(crate::account_binding::Refusal::IdentityUnknown);
            fx.acct.keychain_email_pending.store(true, Ordering::SeqCst);
            assert_ne!(
                engine_refusal_view(&fx.acct),
                serde_json::Value::Null,
                "the premise: a refusal is shown"
            );
            let outcome = clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None)
                .await
                .expect("the sign-out completes");
            assert_eq!(outcome, SignedOut::confirmed(SignOutOutcome::Completed));
            assert_eq!(
                engine_refusal_view(&fx.acct),
                serde_json::Value::Null,
                "the sign-out cleared it"
            );
            assert!(
                !fx.acct.keychain_email_pending.load(Ordering::SeqCst),
                "and the Keychain email write still owed (M5)"
            );
            fx.finish().await;
        });
    }

    /// F2 (fix round 1 of Task 10): a deliberate sign-out leaves no account row and no owner behind, through the
    /// real full sign-out. The state dir here is the throwaway one the sign-out tests share (`init_for_test`):
    /// a `files` row and a sync cursor are rows the old purge left, so only the new clear removes them, whatever
    /// order the tests that share the directory run in.
    #[test]
    fn a_sign_out_leaves_no_account_row_and_no_owner() {
        let mut fx = Fixture::new("finder-signout-clears-rows");
        // The seed, the sign-out and the checks all work in the scratch state directory that every sign-out test shares,
        // so they hold its gate for the whole test (the sign-out inside knows it, and does not wait for it).
        runtime().block_on(with_the_shared_state_dir(async {
            let dir = crate::state_paths::init_for_test();
            let db = crate::state_db::StateDb::open(dir.join("state.db")).unwrap();
            db.upsert_file(&crate::state_db::FileEntry {
                file_id: "file-left-behind".into(),
                path: "/Left.txt".into(),
                status: crate::state_db::FileStatus::CloudOnly,
                size_bytes: 1,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: crate::state_db::ItemKind::File,
            })
            .unwrap();
            rusqlite::Connection::open(dir.join("state.db"))
                .unwrap()
                .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '7')", [])
                .unwrap();
            db.set_owner(&crate::account_binding::Identity::new(
                Some("u-a"),
                Some("a@beebeeb.io"),
            ))
            .unwrap();
            assert!(
                db.has_account_data().unwrap(),
                "the premise: rows of an account are there"
            );
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            fx.install_finished_engine().await;
            let outcome = tokio::time::timeout(Duration::from_secs(30), clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None))
                .await
                .expect("the sign-out does not wait for the gate its own test holds")
                .expect("the sign-out completes");
            assert_eq!(outcome, SignedOut::confirmed(SignOutOutcome::Completed));
            fx.finish().await;
            assert!(!db.has_account_data().unwrap(), "no account row survives a sign-out");
            assert_eq!(db.owner().unwrap(), None, "and nobody owns what is left");
            assert!(
                !db.finder_accepts_closed().unwrap(),
                "and Finder accepts are open: the purge cleared the key the keep step wrote"
            );
        }));
    }

    // ---- Repair ----

    /// Fix round 1 (item A): Repair stops the engine, and the reconciler's follow-up check then starts one
    /// again with the keys still in memory. When Repair's stop cannot be confirmed, the gate must close, or
    /// that check starts a SECOND engine beside the one that may still run. A Repair whose only issue is that stop
    /// reports `engine_stop_unconfirmed: true` with no warnings: the Mac's surfaces render the field's fixed
    /// sentence (quit and reopen Beebeeb), so a warning would only add a second message about the same thing.
    #[test]
    fn a_repair_whose_engine_stop_cannot_be_confirmed_closes_the_gate_and_says_to_restart() {
        let mut fx = Fixture::new("finder-repair-unconfirmed");
        multi_thread_runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            // No await point: tokio cannot force-abort it. A wall-clock deadline ends the thread.
            let deadline = Instant::now() + Duration::from_secs(7);
            let task = tokio::task::spawn_blocking(move || {
                while Instant::now() < deadline {
                    std::hint::black_box(());
                }
            });
            *fx.acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));
            assert!(
                keys_for_engine_start(&fx.acct).is_ok(),
                "before Repair the keys are in memory and a start is allowed"
            );

            let mut warnings = Vec::new();
            let stop = stop_engine_for_repair(&fx.acct, &mut warnings, || Ok(false)).await;

            assert!(
                fx.acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
                "Repair's unconfirmed stop closes the gate"
            );
            assert!(stop.engine_stop_unconfirmed, "and Repair reports it as the field");
            assert!(
                warnings.is_empty(),
                "the field's sentence is the one message; the warnings do not repeat it: {warnings:?}"
            );
            assert!(
                fx.acct.engine.lock().await.is_none(),
                "the slot is empty: the flag is all that stands between the follow-up check and a second engine"
            );
            let refused = keys_for_engine_start(&fx.acct).expect_err("the follow-up check's start must refuse");
            assert!(refused.to_string().contains("Restart Beebeeb"), "{refused}");
            fx.finish().await;
        });
    }

    /// A Repair whose stop is confirmed says nothing and leaves the gate open.
    #[test]
    fn a_repair_whose_engine_stops_leaves_the_gate_open() {
        let mut fx = Fixture::new("finder-repair-confirmed");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            fx.install_finished_engine().await;
            let mut warnings = Vec::new();
            stop_engine_for_repair(&fx.acct, &mut warnings, || Ok(false)).await;
            assert!(warnings.is_empty(), "{warnings:?}");
            assert!(!fx.acct.engine_stop_unconfirmed.load(Ordering::SeqCst));
            assert!(fx.acct.engine.lock().await.is_none());
            assert!(
                keys_for_engine_start(&fx.acct).is_ok(),
                "the follow-up check may start its engine"
            );
            fx.finish().await;
        });
    }

    // ---- Sign-out ----

    /// Lead rulings T1-4 (12) and T7-1 (6): what a failed removal hands to Repair's warnings and to
    /// every other caller of `finder_remove_for` is the OS error's domain and code, never its message.
    #[test]
    fn a_failed_removal_reaches_the_caller_as_a_domain_and_a_code_only() {
        let mut fx = Fixture::new("finder-remove-redacted");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::RemovalFails));
            let error = finder_remove_for(&fx.state(), finder_setup::core::Trigger::Repair)
                .await
                .expect_err("the removal failed");
            assert_eq!(error.message, "NSCocoaErrorDomain 516");
            assert_eq!(
                fx.finish().await,
                vec!["remove:repair:no_engine"],
                "Repair goes through the reconciler as `repair`"
            );
        });
    }

    #[test]
    fn sign_out_waits_for_the_reconciler_removal() {
        let mut fx = Fixture::new("finder-signout");
        runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            let outcome = clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None)
                .await
                .expect("a removal the reconciler confirmed");
            assert_eq!(outcome, SignedOut::confirmed(SignOutOutcome::NotSignedIn));
            // Item D: this sign-out cleared its credentials (`clear_keychain_session`, the real store's
            // legacy flat items included) in the in-memory test store, never in the real Keychain.
            assert!(
                keychain::test_store_touched(Some("finder-signout")),
                "the sign-out went through the in-memory Keychain store"
            );
            assert!(
                keychain::test_store_touched(None),
                "and so did its clear of the legacy flat layout"
            );
            assert_eq!(
                fx.finish().await,
                vec!["remove:sign_out:no_engine"],
                "sign-out waited for the reconciler's removal"
            );
        });
    }

    /// Review I1 (task 1882 round 2), on spec A's sign-out: the reconciler's removal comes first, so a sign-out
    /// that stops after it (here: an earlier engine stop was never confirmed) must still name the folder macOS
    /// kept. The error carries it, and the alert text is produced from it, before the error is returned. A removal
    /// that failed but kept files (review M2) names it too; one that kept nothing names nothing.
    #[test]
    fn test_1882_r2_a_sign_out_that_fails_after_the_removal_still_names_the_kept_folder() {
        for (id, mode, expected) in [
            ("signout-i1-kept", Reconciler::KeepsFiles, Some(KEPT_FOLDER)),
            (
                "signout-i1-kept-failed",
                Reconciler::KeepsFilesAndFails,
                Some(KEPT_FOLDER),
            ),
            ("signout-i1-none", Reconciler::Answers, None),
        ] {
            let mut fx = Fixture::new(id);
            runtime().block_on(async {
                fx.reconciler(Some(mode));
                fx.install_keys();
                fx.install_finished_engine().await;
                fx.acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
                let result = clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None).await;
                let failure = result.as_ref().expect_err("sign-out refuses after the removal");
                assert_eq!(failure.message, UNCONFIRMED_ENGINE_STOP_ERROR, "{id}");
                assert_eq!(
                    sign_out_folders(&result).0,
                    expected,
                    "{id}: the error carries the kept folder"
                );
                // The alert text the command and the menu raise before they return the error.
                assert_eq!(
                    sign_out::copy::sign_out_alert(None, sign_out_folders(&result).0).map(|(_, body)| body),
                    expected.map(|folder| format!("{}\n\n{folder}", finder_removal::PRESERVED_FILES_SENTENCE)),
                    "{id}"
                );
                fx.finish().await;
            });
        }
    }

    /// Task 1882 on spec A's sign-out: a sign-out that completes carries the folder its removal kept to the
    /// result the command and the menu raise the alert from, and nothing when nothing was kept.
    #[test]
    fn test_1882_r2_the_post_removal_step_carries_the_folder_on_success_and_nothing_when_nothing_was_kept() {
        for (id, mode, folder, warning) in [
            ("signout-kept-ok", Reconciler::KeepsFiles, Some(KEPT_FOLDER), None),
            (
                "signout-kept-ok-failed",
                Reconciler::KeepsFilesAndFails,
                Some(KEPT_FOLDER),
                Some(ActionWarning::FinderRemovalUnconfirmed),
            ),
            ("signout-none-ok", Reconciler::Answers, None, None),
        ] {
            let mut fx = Fixture::new(id);
            runtime().block_on(async {
                fx.reconciler(Some(mode));
                let result = clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None).await;
                let done = result.as_ref().expect("the sign-out happened");
                assert_eq!(done.outcome, SignOutOutcome::NotSignedIn, "{id}");
                assert_eq!(done.warning, warning, "{id}");
                assert_eq!(sign_out_folders(&result).0, folder, "{id}");
                fx.finish().await;
            });
        }
    }

    /// Lead ruling 1 (timeout path) and ruling 5 (a reconciler that is not running): sign-out
    /// completes its own work and says Finder cleanup could not be confirmed. Never plain success: it happened, so it
    /// is `Ok`, with the warning (FB-24).
    #[test]
    fn sign_out_the_reconciler_cannot_confirm_happens_and_warns_it_is_unconfirmed() {
        for (id, mode) in [
            ("finder-signout-silent", Some(Reconciler::Silent)),
            ("finder-signout-dead", None),
            ("finder-signout-dropped", Some(Reconciler::DropsTheAck)),
            ("finder-signout-fails", Some(Reconciler::RemovalFails)),
        ] {
            let mut fx = Fixture::new(id);
            runtime().block_on(async {
                fx.reconciler(mode);
                let started = Instant::now();
                let done = FINDER_WAIT_OVERRIDE
                    .scope(SHORT_WAIT, clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None))
                    .await
                    .expect("the sign-out happened");
                assert_eq!(
                    done,
                    SignedOut {
                        outcome: SignOutOutcome::NotSignedIn,
                        warning: Some(ActionWarning::FinderRemovalUnconfirmed),
                        preserved_location: None,
                        kept_unsent: None,
                    },
                    "{id}: a removal nobody confirmed is not a plain success"
                );
                if matches!(mode, Some(Reconciler::Silent)) {
                    assert!(started.elapsed() >= SHORT_WAIT, "it waited for the removal");
                }
                // Ruling 12: nothing the OS said reaches the caller.
                let sentence = ActionWarning::FinderRemovalUnconfirmed.sentence();
                assert!(
                    !sentence.contains("/Users") && !sentence.contains("Secret") && !sentence.contains("516"),
                    "{id}: {sentence}"
                );
                fx.finish().await;
            });
        }
    }

    /// Lead ruling 7, the order: the removal reaches the reconciler while the engine is still tracked. A
    /// sign-out that refuses afterwards because an earlier engine stop was never confirmed (the only way
    /// this Mac's sign-out can stop short with its keys still in memory) does NOT ask the reconciler to put
    /// Finder back (fix round 1, item E): every start refuses until a restart, and the restart's Launch
    /// re-adds Finder, so a re-check would only show a Finder failure next to the restart message.
    #[test]
    fn a_sign_out_that_refuses_on_an_unconfirmed_stop_does_not_ask_for_a_finder_re_add() {
        for (id, keys_in_memory) in [
            ("finder-signout-short-keys", true),
            ("finder-signout-short-nokeys", false),
        ] {
            let mut fx = Fixture::new(id);
            runtime().block_on(async {
                fx.reconciler(Some(Reconciler::Answers));
                if keys_in_memory {
                    fx.install_keys();
                } else {
                    set_auth_present(&fx.state(), true); // signed in on this Mac, no keys in memory
                }
                fx.install_finished_engine().await;
                // An earlier stop that was never confirmed: sign-out refuses AFTER the removal.
                fx.acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
                let error = clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None)
                    .await
                    .expect_err("sign-out refuses");
                assert_eq!(error.message, UNCONFIRMED_ENGINE_STOP_ERROR, "{id}");
                assert_eq!(
                    fx.acct.session.lock().unwrap().is_some(),
                    keys_in_memory,
                    "{id}: the keys are untouched"
                );
                assert!(
                    fx.acct.engine.lock().await.is_some(),
                    "{id}: the refusal did not touch the engine"
                );
                // Let the stand-in handle what was sent, then read what it saw.
                tokio::task::yield_now().await;
                assert_eq!(
                    fx.finish().await,
                    vec!["remove:sign_out:engine_tracked"],
                    "{id}: the removal, and no re-add request"
                );
            });
        }
    }

    /// Lead ruling T3-⚠2 and item E, on the guard itself (no abort path of a Mac's sign-out leaves the keys
    /// in memory without the flag set yet, so Task 10's failed reset is the first real one): it asks for
    /// Finder back only when the keys are still in memory AND no engine stop is unconfirmed.
    #[test]
    fn the_restore_guard_asks_for_finder_back_only_with_keys_in_memory_and_no_unconfirmed_stop() {
        for (id, keys_in_memory, stop_unconfirmed, expected) in [
            ("finder-guard-keys", true, false, vec!["trigger:keys_arrived"]),
            ("finder-guard-keys-flag", true, true, vec![]),
            ("finder-guard-nokeys", false, false, vec![]),
            ("finder-guard-nokeys-flag", false, true, vec![]),
        ] {
            let mut fx = Fixture::new(id);
            runtime().block_on(async {
                fx.reconciler(Some(Reconciler::Answers));
                if keys_in_memory {
                    fx.install_keys();
                }
                fx.acct
                    .engine_stop_unconfirmed
                    .store(stop_unconfirmed, Ordering::SeqCst);
                {
                    let state = fx.state();
                    let _guard = FinderRestoreOnAbort {
                        state: &state,
                        acct: &fx.acct,
                    };
                }
                tokio::task::yield_now().await;
                assert_eq!(fx.finish().await, expected, "{id}");
            });
        }
    }

    // ---- Session transitions take turns (Task 12, lead ruling 1) ----

    /// A loopback stand-in for the API whose FIRST request is held: the test learns that it arrived, and it is
    /// answered only when the test releases it. Every later request is answered at once. Every answer is
    /// `status_line` with `body`.
    struct HeldServer {
        base_url: String,
        arrived: tokio::sync::mpsc::Receiver<String>,
        release: Option<tokio::sync::oneshot::Sender<()>>,
    }

    impl HeldServer {
        fn start(status_line: &'static str, body: &'static str) -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let (arrived_tx, arrived) = tokio::sync::mpsc::channel(8);
            let (release, release_rx) = tokio::sync::oneshot::channel::<()>();
            std::thread::spawn(move || {
                let mut release_rx = Some(release_rx);
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut buf = [0u8; 8192];
                    let read = stream.read(&mut buf).unwrap_or(0);
                    let request_line = String::from_utf8_lossy(&buf[..read])
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    let _ = arrived_tx.blocking_send(request_line);
                    if let Some(release_rx) = release_rx.take() {
                        let _ = release_rx.blocking_recv();
                    }
                    let response = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            Self {
                base_url,
                arrived,
                release: Some(release),
            }
        }

        /// The request line of the next request that reached the server.
        async fn next_request(&mut self) -> String {
            tokio::time::timeout(Duration::from_secs(20), self.arrived.recv())
                .await
                .expect("a request reaches the stand-in")
                .expect("the stand-in is running")
        }

        fn release(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
        }
    }

    const T12_PROFILE: &str =
        r#"{"user_id":"u-t12","email":"t12@beebeeb.io","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}"#;

    /// What the Keychain (the in-memory test store) holds for `id`: (token, vault key, email).
    fn keychain_holds(id: &str) -> (bool, bool, bool) {
        let store = keychain::platform_keychain_store_for(id);
        (
            keychain::holds_session_token(&store),
            keychain::holds_vault_key(&store),
            keychain::holds_account_email(&store),
        )
    }

    /// Lead ruling 1 (Task 12): a Sign-out that has completed is never followed by an older startup restore. The
    /// restore is held at its probe (the server has the request and has not answered); the Sign-out runs to `Ok`; then
    /// the server answers "this token works" (and names the account). Afterwards nothing is in memory, nothing is
    /// written to the Keychain, and no engine start follows.
    #[test]
    fn a_sign_out_while_the_startup_restore_waits_for_the_server_is_never_followed_by_the_restore() {
        // The sign-out below reads the sandboxed config; its first read in a test process can take seconds, longer than
        // the probe's own 5 s timeout. Done first, so the probe is answered, not timed out.
        let _ = DesktopConfig::load();
        let mut fx = Fixture::new("t12-restore-held-probe");
        let id = fx.acct.id.as_str().to_string();
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            fx.reconciler(Some(Reconciler::Answers));
            // A complete stored session with no email: after the probe the restore asks the server to name it.
            persist_session_to_keychain(&SessionWrite::for_test(), &id, "tok-stored", &[4u8; 32], None).unwrap();
            set_auth_present(&fx.state(), true); // `setup()` seeds it from the Keychain
            let mut server = HeldServer::start("200 OK", T12_PROFILE);
            let base_url = server.base_url.clone();
            let state = fx.state();
            let restore = restore_stored_session(&state, &fx.acct, &base_url);
            let sign_out_then_answer = async {
                let probe = server.next_request().await;
                let signed_out = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await;
                server.release();
                (probe, signed_out)
            };
            let (restored, (probe, signed_out)) = tokio::join!(restore, sign_out_then_answer);
            assert!(probe.starts_with("GET /api/v1/auth/me"), "the restore was held at its probe: {probe}");
            assert_eq!(signed_out, Ok(SignedOut::confirmed(SignOutOutcome::Completed)), "the premise: the sign-out completed before the probe was answered");
            let session_in_memory = fx.acct.session.lock().unwrap().is_some();
            let auth_present = *state.auth_present.lock().unwrap();
            let (token, key, email) = keychain_holds(&id);
            assert_eq!(
                (restored, session_in_memory, auth_present, token, key, email),
                (false, false, false, false, false, false),
                "(an engine start follows, a session in memory, auth_present, a Keychain token, a Keychain vault key, a Keychain email)"
            );
            assert_eq!(keys_for_engine_start(&fx.acct).err(), Some(EngineStartBlocked::NoSession), "no engine could start with anything");
            assert_eq!(fx.acct.auth_email.lock().unwrap().clone(), None);
            fx.finish().await;
        }));
    }

    fn t12_profile(user_id: &str, email: &str) -> account_dto::AccountProfile {
        serde_json::from_str(&format!(
            r#"{{"user_id":"{user_id}","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#
        ))
        .unwrap()
    }

    fn stored_token_of(id: &str) -> Option<String> {
        load_session_token_from_keychain(id).unwrap()
    }

    /// R8 on a Mac, through the real restore: the server rejects the stored token (401), so the restore drops ONLY that
    /// token, keeps the vault key and the email, and boots signed out. A later sign-out (the switch) then leaves neither.
    #[test]
    fn a_startup_401_on_a_mac_drops_only_the_token_and_a_later_sign_out_leaves_nothing() {
        let mut fx = Fixture::new("t12-startup-401");
        let id = fx.acct.id.as_str().to_string();
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            fx.reconciler(Some(Reconciler::Answers));
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-revoked",
                &[4u8; 32],
                Some("t12@beebeeb.io"),
            )
            .unwrap();
            set_auth_present(&fx.state(), true);
            set_auth_email(&fx.state(), Some("t12@beebeeb.io".into()));
            let mut server = HeldServer::start("401 Unauthorized", "{}");
            server.release();
            let state = fx.state();
            assert!(
                !restore_stored_session(&state, &fx.acct, &server.base_url).await,
                "no engine start follows a rejected token"
            );
            assert_eq!(
                keychain_holds(&id),
                (false, true, true),
                "(token, vault key, email): only the token is gone (R8, R9)"
            );
            assert!(fx.acct.session.lock().unwrap().is_none(), "nothing in memory");
            assert!(
                !*state.auth_present.lock().unwrap() && fx.acct.auth_email.lock().unwrap().is_none(),
                "booted signed out"
            );
            // "Sign out and switch" from here: the already-signed-out path, which must leave no key and no email.
            assert_eq!(
                clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await,
                Ok(SignedOut::confirmed(SignOutOutcome::NotSignedIn))
            );
            assert_eq!(
                keychain_holds(&id),
                (false, false, false),
                "nothing of the account is left for the next one"
            );
            fx.finish().await;
        }));
    }

    /// Lead ruling 1: a startup 401 never removes a token that a sign-in stored while the probe waited. The restore read
    /// the old token and is held at its probe; the same account signs in again (a new token); then the server rejects
    /// the OLD token. The new session is left exactly as the sign-in made it.
    #[test]
    fn a_startup_401_never_removes_a_token_a_sign_in_stored_while_the_probe_waited() {
        let mut fx = Fixture::new("t12-401-after-resign-in");
        let id = fx.acct.id.as_str().to_string();
        let dir = tempfile::tempdir().unwrap();
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            fx.reconciler(Some(Reconciler::Answers));
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-old",
                &[4u8; 32],
                Some("t12@beebeeb.io"),
            )
            .unwrap();
            set_auth_present(&fx.state(), true);
            let mut server = HeldServer::start("401 Unauthorized", "{}");
            let base_url = server.base_url.clone();
            let state = fx.state();
            let restore = restore_stored_session(&state, &fx.acct, &base_url);
            let sign_in_then_answer = async {
                let probe = server.next_request().await;
                let sources = LocalSources::for_test(dir.path());
                let signed_in = reauth_swap_token(
                    &state,
                    &fx.acct,
                    &sources,
                    "tok-new",
                    &t12_profile("u-t12", "t12@beebeeb.io"),
                    None,
                    &mut fx.acct.session_generation(),
                );
                server.release();
                (probe, signed_in.map_err(|e| e.message))
            };
            let (restored, (probe, signed_in)) = tokio::join!(restore, sign_in_then_answer);
            assert!(probe.starts_with("GET /api/v1/auth/me"), "{probe}");
            assert_eq!(
                signed_in,
                Ok(true),
                "the premise: the sign-in stored its token and unlocked with the stored key"
            );
            assert!(!restored);
            assert_eq!(
                stored_token_of(&id).as_deref(),
                Some("tok-new"),
                "the new token is still stored"
            );
            assert_eq!(keychain_holds(&id), (true, true, true));
            assert_eq!(
                fx.acct
                    .session
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|session| session.token.clone())
                    .as_deref(),
                Some("tok-new"),
                "and in memory"
            );
            assert!(*state.auth_present.lock().unwrap(), "still signed in");
            fx.finish().await;
        }));
    }

    /// Lead ruling 3 (Task 12), through the Keychain seam: a sign-out clears the session BEFORE it releases the engine
    /// slot. The test freezes the sign-out at the session clear by holding the session-write lock (the Keychain
    /// writers' lock, which the clear takes first) once the sign-out holds the slot: at that moment the slot must still
    /// be held and the keys still in memory; released, the sign-out clears them and only then frees the slot.
    #[test]
    fn sign_out_holds_the_engine_slot_until_the_session_is_cleared() {
        // Fix round 2, item 4 (re-review N2): explicit signals, no timing sample. (1) The sign-out says itself that it
        // holds the slot (task 1891: a `try_lock` poll would also be satisfied by the stand-in reconciler's look at the
        // slot, which comes before the sign-out takes it, and this thread would then freeze the session mutex too
        // early). The engine ignores the cancel, so the sign-out then holds the slot for at least `abort()`'s 3 s
        // graceful window. (2) This thread then holds the session mutex, and waits for the sign-out's next generation
        // move: the one `clear_session_holding_slot` makes right before it takes the session mutex to clear it. At
        // that point the sign-out is at its clear, blocked on this thread; the slot must still be held.
        let _ = DesktopConfig::load();
        let mut fx = Fixture::new("t12-signout-freeze");
        let rt = multi_thread_runtime();
        rt.block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                fx.acct.id.as_str(),
                "tok-abc",
                &[9u8; 32],
                Some("sam.keys@beebeeb.io"),
            )
            .unwrap();
            fx.install_ticking_engine().await;
        });
        let acct = fx.acct.clone();
        let state = fx.state();
        let (sign_out_holds_the_slot, slot_taken) = std::sync::mpsc::channel::<()>();
        let (slot_held_at_the_clear, signed_out) = std::thread::scope(|scope| {
            let signing_out = scope.spawn(move || {
                runtime().block_on(SIGN_OUT_HOLDS_THE_ENGINE_SLOT.scope(
                    sign_out_holds_the_slot,
                    with_the_shared_state_dir(clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None)),
                ))
            });
            let deadline = Instant::now() + Duration::from_secs(60);
            // Wait for the sign-out's own signal. A sign-out that ENDS first never took the slot (it failed before
            // it): fail at once with its answer, not later with a message about a wait.
            loop {
                match slot_taken.recv_timeout(Duration::from_millis(5)) {
                    Ok(()) => break,
                    Err(_) if signing_out.is_finished() => {
                        panic!(
                            "the sign-out returned before it took the engine slot: {:?}",
                            signing_out.join().unwrap()
                        )
                    }
                    Err(_) => assert!(Instant::now() < deadline, "the sign-out never took the engine slot"),
                }
            }
            // The sign-out's start already moved the generation (before it took the slot); its next move is the clear's.
            let before_the_clear = acct.session_generation();
            let freeze = acct.session.lock().unwrap();
            while acct.session_unchanged_since(before_the_clear) {
                assert!(Instant::now() < deadline, "the sign-out never reached its clear");
                std::thread::sleep(Duration::from_millis(1));
            }
            // The clear's move happened while the slot was held (an early release would have come before it), and the
            // clear itself now waits for the session mutex this thread holds.
            let held = acct.engine.try_lock().is_err();
            assert!(
                freeze.is_some(),
                "the keys are still there: the clear is waiting for this thread"
            );
            drop(freeze);
            (held, signing_out.join().unwrap())
        });
        assert_eq!(signed_out, Ok(SignedOut::confirmed(SignOutOutcome::Completed)));
        assert!(
            slot_held_at_the_clear,
            "the engine slot was released before the session was cleared"
        );
        assert!(
            acct.session.lock().unwrap().is_none(),
            "and then the session is cleared"
        );
        assert!(acct.engine.try_lock().is_ok(), "and the slot is free again");
        rt.block_on(fx.finish());
    }

    /// Lead rulings 1 and 6 (Task 12): a Lock that completes while the recovery phrase is being checked is never undone
    /// by the unlock. The unlock's own re-check (Task 11) compares the stored token, the identity and the empty memory,
    /// and a Lock changes none of them; the unlock's turn is what stops it. The unlock runs as the command runs it: its
    /// turn captured before it reads the Keychain, the server check, then the one checked install.
    #[test]
    fn a_lock_while_the_recovery_phrase_is_checked_is_never_undone_by_the_unlock() {
        let mut fx = Fixture::new("t12-unlock-held-lock");
        let id = fx.acct.id.as_str().to_string();
        multi_thread_runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            // Signed in on this Mac, no vault key yet: what a sign-in leaves before the recovery phrase.
            persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-signed-in", Some("t12@beebeeb.io"))
                .unwrap();
            set_auth_present(&fx.state(), true);
            set_auth_email(&fx.state(), Some("t12@beebeeb.io".into()));
            let (phrase, _) = beebeeb_core::recovery::generate_recovery_phrase().unwrap();
            let mut server = HeldServer::start("200 OK", r#"{"valid":true}"#);
            let base_url = server.base_url.clone();
            let state = fx.state();
            let mut turn = fx.acct.session_generation();
            let token = load_session_token_from_keychain(&id)
                .unwrap()
                .expect("the premise: a stored token");
            let email = fx.acct.auth_email.lock().unwrap().clone();
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap();
            let unlock = async {
                let key = verify_vault_key_from_phrase(&client, &base_url, &token, &phrase).await?;
                install_recovered_session(&state, &fx.acct, &mut turn, &token, email, &key)
            };
            let lock_then_answer = async {
                let check = server.next_request().await;
                let locked = lock_vault(fx.state()).await;
                server.release();
                (check, locked)
            };
            let (unlocked, (check, locked)) = tokio::join!(unlock, lock_then_answer);
            assert!(
                check.starts_with("POST /api/v1/auth/verify-recovery-check"),
                "the unlock was held at its server check: {check}"
            );
            assert_eq!(
                locked,
                Ok(SessionActionOutcome { warning: None }),
                "the premise: the Lock completed first"
            );
            let session_in_memory = fx.acct.session.lock().unwrap().is_some();
            let (_, key_written, _) = keychain_holds(&id);
            assert_eq!(
                (unlocked, session_in_memory, key_written),
                (Err(RECOVERY_UNLOCK_LOCKED_MEANWHILE.to_string()), false, false),
                "(the unlock's result, keys in memory after the Lock, a vault key written after the Lock)"
            );
            assert_eq!(
                stored_token_of(&id).as_deref(),
                Some("tok-signed-in"),
                "the Lock keeps the session's token"
            );
            fx.finish().await;
        });
    }

    /// A Mac whose owner record names sam by email only and that stores a vault key: a sign-in as sam needs the server's
    /// key proof (`NeedsKeyProof`). Returns the throwaway state dir that holds the owner record.
    fn legacy_owner_with_a_stored_key(fx: &Fixture, session_in_memory: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        crate::state_db::StateDb::open(dir.path().join("state.db"))
            .unwrap()
            .set_owner(&crate::account_binding::Identity::new(None, Some("sam@beebeeb.io")))
            .unwrap();
        persist_session_to_keychain(
            &SessionWrite::for_test(),
            fx.acct.id.as_str(),
            "tok-old",
            &[5u8; 32],
            Some("sam@beebeeb.io"),
        )
        .unwrap();
        set_auth_present(&fx.state(), true);
        set_auth_email(&fx.state(), Some("sam@beebeeb.io".into()));
        if session_in_memory {
            *fx.acct.session.lock().unwrap() = Some(Session {
                token: "tok-old".into(),
                master_key: [5u8; 32],
                email: Some("sam@beebeeb.io".into()),
            });
        }
        dir
    }

    /// Lead ruling 6 (re-check N1): the key proof waits for the server (up to 15 s). A Sign-out that completes
    /// during that wait is never followed by the re-sign-in: nothing is decided, and nothing is written.
    #[test]
    fn a_sign_out_during_a_held_key_proof_is_never_followed_by_the_re_sign_in() {
        // As in the restore test: the sign-out's first config read must not outlast the proof's 15 s timeout.
        let _ = DesktopConfig::load();
        let mut fx = Fixture::new("t12-keyproof-signout");
        let id = fx.acct.id.as_str().to_string();
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            fx.reconciler(Some(Reconciler::Answers));
            let dir = legacy_owner_with_a_stored_key(&fx, false);
            let sources = LocalSources::for_test(dir.path());
            let sam = t12_profile("u-1", "sam@beebeeb.io");
            let mut server = HeldServer::start("200 OK", r#"{"valid":true}"#);
            let base_url = server.base_url.clone();
            let state = fx.state();
            let mut turn = fx.acct.session_generation();
            let (kind, _) = classify_sign_in(&state, &fx.acct, &sources, &sam);
            assert_eq!(
                kind,
                reauth::SignInKind::NeedsKeyProof,
                "the premise: the server's proof decides"
            );
            let decide = decide_sign_in_in_turn(&state, &fx.acct, &sources, &sam, "tok-new", &base_url, &turn);
            let sign_out_then_answer = async {
                let proof = server.next_request().await;
                let signed_out = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await;
                server.release();
                (proof, signed_out)
            };
            let (decided, (proof, signed_out)) = tokio::join!(decide, sign_out_then_answer);
            assert!(
                proof.starts_with("POST /api/v1/auth/verify-recovery-check"),
                "held at the key proof: {proof}"
            );
            assert_eq!(
                signed_out,
                Ok(SignedOut::confirmed(SignOutOutcome::Completed)),
                "the premise: the sign-out completed during the proof"
            );
            let decided = decided.map_err(|e| (e.message, e.token_stored));
            assert_eq!(
                decided,
                Err((SESSION_CHANGED_WHILE_WAITING.to_string(), false)),
                "nothing decided, nothing stored (the caller revokes)"
            );
            // And a swap that would run on the old decision anyway is refused before it writes.
            let swapped = reauth_swap_token(&state, &fx.acct, &sources, "tok-new", &sam, None, &mut turn)
                .map_err(|e| (e.message, e.token_stored));
            assert_eq!(swapped, Err((SESSION_CHANGED_WHILE_WAITING.to_string(), false)));
            assert_eq!(
                keychain_holds(&id),
                (false, false, false),
                "nothing written after the sign-out"
            );
            assert!(fx.acct.session.lock().unwrap().is_none() && !*state.auth_present.lock().unwrap());
            let owner = crate::state_db::StateDb::open(dir.path().join("state.db"))
                .unwrap()
                .owner()
                .unwrap();
            assert_eq!(
                owner,
                Some(crate::account_binding::Identity::new(None, Some("sam@beebeeb.io"))),
                "the owner record is not rewritten"
            );
            fx.finish().await;
        }));
    }

    /// Lead ruling 6 (security P3): a re-sign-in in flight never undoes a Lock that completes in the middle of it. Held
    /// at its key proof, the Lock runs to `Ok`; afterwards the vault is still locked and the stored token unchanged.
    #[test]
    fn a_lock_during_a_held_re_sign_in_is_never_undone() {
        let mut fx = Fixture::new("t12-resign-in-lock");
        let id = fx.acct.id.as_str().to_string();
        multi_thread_runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            let dir = legacy_owner_with_a_stored_key(&fx, true);
            let sources = LocalSources::for_test(dir.path());
            let sam = t12_profile("u-1", "sam@beebeeb.io");
            let mut server = HeldServer::start("200 OK", r#"{"valid":true}"#);
            let base_url = server.base_url.clone();
            let state = fx.state();
            let mut turn = fx.acct.session_generation();
            let decide = decide_sign_in_in_turn(&state, &fx.acct, &sources, &sam, "tok-new", &base_url, &turn);
            let lock_then_answer = async {
                let proof = server.next_request().await;
                let locked = lock_vault(fx.state()).await;
                server.release();
                (proof, locked)
            };
            let (decided, (proof, locked)) = tokio::join!(decide, lock_then_answer);
            assert!(proof.starts_with("POST /api/v1/auth/verify-recovery-check"), "{proof}");
            assert_eq!(locked, Ok(SessionActionOutcome { warning: None }));
            let decided = decided.map_err(|e| (e.message, e.token_stored));
            let swapped = reauth_swap_token(&state, &fx.acct, &sources, "tok-new", &sam, None, &mut turn)
                .map_err(|e| (e.message, e.token_stored));
            let session_in_memory = fx.acct.session.lock().unwrap().is_some();
            assert_eq!(
                (decided, swapped, session_in_memory, stored_token_of(&id)),
                (
                    Err((SIGN_IN_LOCKED_MEANWHILE.to_string(), false)),
                    Err((SIGN_IN_LOCKED_MEANWHILE.to_string(), false)),
                    false,
                    Some("tok-old".to_string())
                ),
                "(the decision, a swap on it anyway, keys in memory after the Lock, the stored token)"
            );
            fx.finish().await;
        });
    }

    /// Lead ruling 6 (P3): two sign-ins of DIFFERENT accounts that began together (both captured the same
    /// generation before either wrote) never leave one account's token beside the other's key or email: the first to
    /// write wins, the other is refused before it writes anything. In both orders, and then with the two sign-ins on
    /// separate threads.
    #[test]
    fn two_sign_ins_that_began_together_never_leave_one_accounts_token_beside_the_others_key() {
        let sam = || t12_profile("u-sam", "sam@beebeeb.io");
        let kim = || t12_profile("u-kim", "kim@beebeeb.io");
        // sam signs in by password (a token), kim through the browser (a token and a vault key).
        let fx = Fixture::new("t12-two-sign-ins-a");
        let (mut by_password, mut by_browser) = (fx.acct.session_generation(), fx.acct.session_generation());
        assert_eq!(
            store_first_sign_in(
                &fx.state(),
                &fx.acct,
                &mut by_password,
                "tok-sam",
                "sam@beebeeb.io",
                sam()
            ),
            Ok(())
        );
        let browser = install_new_session(
            &fx.state(),
            &fx.acct,
            &mut by_browser,
            "tok-kim",
            &[2u8; 32],
            Some("kim@beebeeb.io".into()),
            Some(kim()),
        );
        assert_eq!(
            browser,
            Ok(NewSession::Refused(SESSION_CHANGED_WHILE_WAITING.to_string()))
        );
        assert_eq!(
            (
                stored_token_of(fx.acct.id.as_str()).as_deref(),
                keychain_holds(fx.acct.id.as_str())
            ),
            (Some("tok-sam"), (true, false, true))
        );
        assert_eq!(
            keychain_account_email(fx.acct.id.as_str()).as_deref(),
            Some("sam@beebeeb.io")
        );
        assert!(
            fx.acct.session.lock().unwrap().is_none(),
            "kim's keys never reach memory"
        );
        // The other order.
        let fx = Fixture::new("t12-two-sign-ins-b");
        let (mut by_password, mut by_browser) = (fx.acct.session_generation(), fx.acct.session_generation());
        assert_eq!(
            install_new_session(
                &fx.state(),
                &fx.acct,
                &mut by_browser,
                "tok-kim",
                &[2u8; 32],
                Some("kim@beebeeb.io".into()),
                Some(kim())
            ),
            Ok(NewSession::Installed)
        );
        assert_eq!(
            store_first_sign_in(
                &fx.state(),
                &fx.acct,
                &mut by_password,
                "tok-sam",
                "sam@beebeeb.io",
                sam()
            ),
            Err(SESSION_CHANGED_WHILE_WAITING.to_string())
        );
        assert_eq!(
            stored_token_of(fx.acct.id.as_str()).as_deref(),
            Some("tok-kim"),
            "kim's session is whole: sam's token did not replace kim's"
        );
        assert_eq!(
            keychain_account_email(fx.acct.id.as_str()).as_deref(),
            Some("kim@beebeeb.io")
        );
        // As a race between threads, many times: whoever wins, the Keychain holds ONE account's token, key and email.
        for round in 0..100 {
            let fx = Fixture::new(&format!("t12-two-sign-ins-race-{round}"));
            let id = fx.acct.id.as_str().to_string();
            let (mut by_password, mut by_browser) = (fx.acct.session_generation(), fx.acct.session_generation());
            let barrier = std::sync::Barrier::new(2);
            let (state_a, state_b, acct) = (fx.state(), fx.state(), fx.acct.clone());
            let (password, browser) = std::thread::scope(|scope| {
                let password = scope.spawn(|| {
                    barrier.wait();
                    store_first_sign_in(&state_a, &acct, &mut by_password, "tok-sam", "sam@beebeeb.io", sam())
                });
                let browser = scope.spawn(|| {
                    barrier.wait();
                    install_new_session(
                        &state_b,
                        &acct,
                        &mut by_browser,
                        "tok-kim",
                        &[2u8; 32],
                        Some("kim@beebeeb.io".into()),
                        Some(kim()),
                    )
                });
                (password.join().unwrap(), browser.join().unwrap())
            });
            let stored = (stored_token_of(&id), keychain_holds(&id).1, keychain_account_email(&id));
            match (password, browser) {
                (Ok(()), Ok(NewSession::Refused(_))) => {
                    assert_eq!(
                        stored,
                        (Some("tok-sam".into()), false, Some("sam@beebeeb.io".into())),
                        "round {round}: sam's alone"
                    );
                }
                (Err(_), Ok(NewSession::Installed)) => {
                    assert_eq!(
                        stored,
                        (Some("tok-kim".into()), true, Some("kim@beebeeb.io".into())),
                        "round {round}: kim's alone"
                    );
                }
                other => panic!("round {round}: exactly one sign-in may write, got {other:?}"),
            }
        }
    }

    /// Lead rulings 4 and 5 (Task 12), the startup probe: while the session still works, the probe's own answer names
    /// it (no second request), caches the profile (so its user id is known), and completes an owner record written
    /// before ids were known, here with another letter case, so the next engine start compares ids and keeps the data.
    #[test]
    fn the_startup_probe_names_the_session_and_completes_a_legacy_owner_record() {
        let mut fx = Fixture::new("t12-probe-names");
        let id = fx.acct.id.as_str().to_string();
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            fx.reconciler(Some(Reconciler::Answers));
            let dir = crate::state_paths::init_for_test();
            let db = crate::state_db::StateDb::open(dir.join("state.db")).unwrap();
            let legacy = crate::account_binding::Identity::new(None, Some("T12@Beebeeb.io"));
            db.set_owner(&legacy).unwrap();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-stored",
                &[4u8; 32],
                Some("T12@Beebeeb.io"),
            )
            .unwrap();
            set_auth_present(&fx.state(), true);
            let mut server = HeldServer::start("200 OK", T12_PROFILE);
            server.release();
            let state = fx.state();
            assert!(
                restore_stored_session(&state, &fx.acct, &server.base_url).await,
                "an engine start follows"
            );
            let mut requests = Vec::new();
            while let Ok(request) = server.arrived.try_recv() {
                requests.push(request);
            }
            assert_eq!(
                requests.len(),
                1,
                "the probe's own answer named the session: {requests:?}"
            );
            assert_eq!(
                fx.acct
                    .cached_profile
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|p| p.user_id.clone())
                    .as_deref(),
                Some("u-t12"),
                "the profile is cached"
            );
            let email = fx
                .acct
                .session
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|session| session.email.clone());
            assert_eq!(
                email.as_deref(),
                Some("t12@beebeeb.io"),
                "the session is known by the server's spelling"
            );
            assert_eq!(keychain_account_email(&id).as_deref(), Some("t12@beebeeb.io"));
            let owner = db.owner().unwrap();
            assert_eq!(
                owner,
                Some(crate::account_binding::Identity::new(
                    Some("u-t12"),
                    Some("t12@beebeeb.io")
                )),
                "the legacy owner gained its id"
            );
            let session = identity_of_session(email.as_deref(), fx.acct.cached_profile.lock().unwrap().as_ref());
            assert_eq!(
                crate::account_binding::same_account(owner.as_ref().unwrap(), &session),
                Some(true),
                "so the engine start keeps the data"
            );
            // Leave the shared directory as the other sign-out tests expect it.
            db.clear_account_data(false).unwrap();
            fx.finish().await;
        }));
    }

    /// Lead ruling 5 (Task 12), "Try again": a session that has no email (it cannot be identified at all) asks the
    /// server FIRST, and only then asks the reconciler to retry, so the check it starts can start the engine.
    #[test]
    fn try_again_names_an_unidentified_session_before_it_asks_for_the_retry() {
        let mut fx = Fixture::new("t12-try-again");
        // The naming completes an owner record in the shared scratch state dir, so it takes its turn there.
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            fx.reconciler(Some(Reconciler::Answers));
            *fx.acct.session.lock().unwrap() = Some(Session {
                token: "tok-t12".into(),
                master_key: [3u8; 32],
                email: None,
            });
            let mut server = HeldServer::start("200 OK", T12_PROFILE);
            let base_url = server.base_url.clone();
            let state = fx.state();
            let seen = fx.seen.clone();
            let retry = finder_setup_try_again(&state, &base_url);
            let watch = async {
                let request = server.next_request().await;
                tokio::time::sleep(Duration::from_millis(200)).await;
                let triggers_before_the_answer = seen.lock().unwrap().clone();
                server.release();
                (request, triggers_before_the_answer)
            };
            let (retried, (request, before)) = tokio::join!(retry, watch);
            assert!(request.starts_with("GET /api/v1/auth/me"), "{request}");
            assert_eq!(retried, Ok(()));
            assert!(
                before.is_empty(),
                "no retry was asked for before the server answered: {before:?}"
            );
            assert_eq!(
                fx.acct
                    .cached_profile
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|p| p.user_id.clone())
                    .as_deref(),
                Some("u-t12"),
                "named"
            );
            tokio::task::yield_now().await;
            assert_eq!(fx.finish().await, vec!["trigger:try_again"]);
        }));
    }

    /// Area A M8: a session that has an email can already start its engine, so "Try again" asks the reconciler at once
    /// and never waits on the network (here: a server that would hold the request forever, and is never asked).
    #[test]
    fn try_again_for_a_session_with_an_email_retries_at_once_and_asks_the_server_nothing() {
        let mut fx = Fixture::new("t12-try-again-email");
        multi_thread_runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            *fx.acct.session.lock().unwrap() = Some(Session {
                token: "tok-t12".into(),
                master_key: [3u8; 32],
                email: Some("t12@beebeeb.io".into()),
            });
            let mut server = HeldServer::start("200 OK", T12_PROFILE);
            let base_url = server.base_url.clone();
            let state = fx.state();
            let retried = tokio::time::timeout(Duration::from_secs(5), finder_setup_try_again(&state, &base_url))
                .await
                .expect("it does not wait on the network");
            assert_eq!(retried, Ok(()));
            assert!(
                tokio::time::timeout(Duration::from_millis(300), server.next_request())
                    .await
                    .is_err(),
                "the server was not asked"
            );
            server.release();
            tokio::task::yield_now().await;
            assert_eq!(fx.finish().await, vec!["trigger:try_again"]);
        });
    }

    /// Lead ruling 5 (Task 12), app activation: a session that does not know its user id asks the server again (and the
    /// caller then starts the engine); one that does, or no session at all, asks nothing.
    #[test]
    fn app_activation_names_an_unidentified_session_and_asks_nothing_otherwise() {
        let fx = Fixture::new("t12-activation");
        runtime().block_on(with_the_shared_state_dir(async {
            let state = fx.state();
            let mut server = HeldServer::start("200 OK", T12_PROFILE);
            server.release();
            assert!(
                !identify_on_app_activation(&state, &server.base_url, std::time::Instant::now()).await,
                "no session: nothing to name"
            );
            *fx.acct.session.lock().unwrap() = Some(Session {
                token: "tok-t12".into(),
                master_key: [3u8; 32],
                email: Some("t12@beebeeb.io".into()),
            });
            assert!(
                identify_on_app_activation(&state, &server.base_url, std::time::Instant::now()).await,
                "an email but no user id: asked and named"
            );
            assert_eq!(
                fx.acct
                    .cached_profile
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|p| p.user_id.clone())
                    .as_deref(),
                Some("u-t12")
            );
            assert!(
                !identify_on_app_activation(&state, &server.base_url, std::time::Instant::now()).await,
                "identified now: nothing more to ask"
            );
            let mut requests = 0;
            while server.arrived.try_recv().is_ok() {
                requests += 1;
            }
            assert_eq!(requests, 1, "exactly one request, for the unidentified session");
        }));
    }

    /// The check a sign-out ends with: any item of the account left in the Keychain (token, vault key or email) is a
    /// sign-out that has not finished.
    #[test]
    fn a_sign_out_verifies_that_no_item_of_the_account_is_left() {
        use crate::keychain::AuthSecretStore as _;
        let id = "t12-ensure-empty";
        assert_eq!(ensure_keychain_holds_no_account(id), Ok(()));
        let store = keychain::platform_keychain_store_for(id);
        let leave = |what: &str| match what {
            "token" => store
                .save_session_token(&SessionToken::new("tok-left").unwrap())
                .unwrap(),
            "key" => store
                .save_wrapped_master_key(SecretBytes::new_master_key(&[1u8; 32]))
                .unwrap(),
            _ => store.save_account_email("left@beebeeb.io").unwrap(),
        };
        for what in ["token", "key", "email"] {
            leave(what);
            assert_eq!(
                ensure_keychain_holds_no_account(id),
                Err(SIGN_OUT_KEYCHAIN_NOT_CLEARED.to_string()),
                "{what} left behind"
            );
            clear_keychain_session(id).unwrap();
            assert_eq!(ensure_keychain_holds_no_account(id), Ok(()), "{what} cleared");
        }
    }

    // ---- Task 12 fix round 1 ----

    /// Fix round 1, item 2 (P2-A): a Keychain Unlock that read the stored session while a sign-out was between its
    /// memory clear and its Keychain clear never installs that session after the sign-out completed. The test freezes
    /// the sign-out between the two (it clears the 2FA challenge there, and this test holds that mutex); the Unlock
    /// runs as `unlock_vault` runs it: its turn, the Keychain read, then the checked install.
    #[test]
    fn an_unlock_that_read_the_keychain_during_a_sign_out_never_installs_after_it() {
        // The sign-out reads the sandboxed config; its first read in a test process can take seconds.
        let _ = DesktopConfig::load();
        let mut fx = Fixture::new("t12f1-unlock-in-the-gap");
        let id = fx.acct.id.as_str().to_string();
        let rt = multi_thread_runtime();
        rt.block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-abc",
                &[9u8; 32],
                Some("sam.keys@beebeeb.io"),
            )
            .unwrap();
        });
        let acct = fx.acct.clone();
        let (signing_out_state, gap_state) = (fx.state(), fx.state());
        let (mut turn, read, signed_out) = std::thread::scope(|scope| {
            let frozen = gap_state.pending_2fa.lock().unwrap();
            let signing_out = scope.spawn(move || {
                runtime().block_on(with_the_shared_state_dir(clear_session_impl(
                    &signing_out_state,
                    false,
                    None,
                    crate::UnknownFilesArg::Keep,
                    None,
                )))
            });
            let deadline = Instant::now() + Duration::from_secs(60);
            while acct.session.lock().unwrap().is_some() {
                assert!(Instant::now() < deadline, "the sign-out never cleared memory");
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(200)); // it is now waiting on the 2FA challenge, before its Keychain clear
            assert!(
                !signing_out.is_finished(),
                "the premise: the sign-out is frozen in the gap"
            );
            let turn = acct.session_generation();
            let email = acct.auth_email.lock().unwrap().clone();
            let read = load_session_from_keychain(acct.id.as_str(), email).unwrap();
            drop(frozen);
            (turn, read, signing_out.join().unwrap())
        });
        assert_eq!(
            signed_out,
            Ok(SignedOut::confirmed(SignOutOutcome::Completed)),
            "the premise: the sign-out completed"
        );
        let session = read.expect("the premise: the Unlock read the stored session before the Keychain clear");
        let state = fx.state();
        let unlocked = install_keychain_session(&state, &fx.acct, &mut turn, session);
        let session_in_memory = fx.acct.session.lock().unwrap().is_some();
        let auth_present = *state.auth_present.lock().unwrap();
        assert_eq!(
            (unlocked.is_ok(), session_in_memory, auth_present),
            (false, false, false),
            "(the Unlock installed, keys in memory after the sign-out returned Ok, auth_present)"
        );
        assert_eq!(
            keys_for_engine_start(&fx.acct).err(),
            Some(EngineStartBlocked::NoSession),
            "no engine could start with anything"
        );
        rt.block_on(fx.finish());
    }

    /// Fix round 1, item 2, the other order: the Unlock runs WHOLLY in the gap (it reads, claims and installs while the
    /// sign-out is between its memory clear and its Keychain clear). The sign-out's last turn clears memory once more, so
    /// once it has returned `Ok` no keys are in memory and no engine start can find any.
    #[test]
    fn an_unlock_wholly_inside_a_sign_out_is_cleared_by_its_last_turn() {
        let _ = DesktopConfig::load();
        let mut fx = Fixture::new("t12f1-unlock-wholly-in-the-gap");
        let id = fx.acct.id.as_str().to_string();
        let rt = multi_thread_runtime();
        rt.block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            fx.install_keys();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-abc",
                &[9u8; 32],
                Some("sam.keys@beebeeb.io"),
            )
            .unwrap();
        });
        let acct = fx.acct.clone();
        let (signing_out_state, gap_state, unlock_state) = (fx.state(), fx.state(), fx.state());
        let (unlocked, signed_out) = std::thread::scope(|scope| {
            let frozen = gap_state.pending_2fa.lock().unwrap();
            let signing_out = scope.spawn(move || {
                runtime().block_on(with_the_shared_state_dir(clear_session_impl(
                    &signing_out_state,
                    false,
                    None,
                    crate::UnknownFilesArg::Keep,
                    None,
                )))
            });
            let deadline = Instant::now() + Duration::from_secs(60);
            while acct.session.lock().unwrap().is_some() {
                assert!(Instant::now() < deadline, "the sign-out never cleared memory");
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(200));
            assert!(
                !signing_out.is_finished(),
                "the premise: the sign-out is frozen in the gap"
            );
            let mut turn = acct.session_generation();
            let email = acct.auth_email.lock().unwrap().clone();
            let session = load_session_from_keychain(acct.id.as_str(), email)
                .unwrap()
                .expect("the premise: still stored");
            let unlocked = install_keychain_session(&unlock_state, &acct, &mut turn, session);
            drop(frozen);
            (unlocked, signing_out.join().unwrap())
        });
        assert_eq!(
            unlocked,
            Ok(()),
            "the premise: the Unlock installed while the sign-out was frozen"
        );
        assert_eq!(signed_out, Ok(SignedOut::confirmed(SignOutOutcome::Completed)));
        assert!(
            fx.acct.session.lock().unwrap().is_none(),
            "the sign-out's last turn cleared it"
        );
        assert_eq!(
            keys_for_engine_start(&fx.acct).err(),
            Some(EngineStartBlocked::NoSession)
        );
        assert_eq!(keychain_holds(&id), (false, false, false));
        rt.block_on(fx.finish());
    }

    // ---- Task 12 fix round 2 ----

    /// The state a startup 401 leaves (R9): the vault key and the email in the Keychain, no token, nothing in memory.
    fn after_a_rejected_token(fx: &Fixture) {
        let store = keychain::platform_keychain_store_for(fx.acct.id.as_str());
        use crate::keychain::AuthSecretStore as _;
        store
            .save_wrapped_master_key(SecretBytes::new_master_key(&[5u8; 32]))
            .unwrap();
        store.save_account_email("sam@beebeeb.io").unwrap();
        set_auth_present(&fx.state(), false);
    }

    /// An engine stand-in that runs until it is stopped.
    fn a_running_engine() -> EngineRunner {
        EngineRunner::for_test_with_task(tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }))
    }

    /// Fix round 2, item 1 (re-check N1): a sign-in that completes during an already-signed-out sign-out leaves no
    /// engine running. The sign-out takes the engine slot BEFORE its purge, so a same-account sign-in that completes
    /// while the purge runs (it began after the sign-out, so its turn is current) starts no engine before the sign-out
    /// is done; once the sign-out returned `Ok` no engine runs and no keys are in memory. The test holds the purge's
    /// gate to keep the sign-out inside its purge.
    #[test]
    fn a_sign_in_that_completes_during_an_already_signed_out_sign_out_leaves_no_engine() {
        let _ = DesktopConfig::load();
        let mut fx = Fixture::new("t12f2-sign-in-during-signed-out-sign-out");
        let dir = tempfile::tempdir().unwrap();
        crate::state_db::StateDb::open(dir.path().join("state.db"))
            .unwrap()
            .set_owner(&crate::account_binding::Identity::new(
                Some("u-1"),
                Some("sam@beebeeb.io"),
            ))
            .unwrap();
        multi_thread_runtime().block_on(async {
            fx.reconciler(Some(Reconciler::Answers));
            after_a_rejected_token(&fx);
            let gate = SIGN_OUT_TEST_PURGE_GATE.lock().await; // the sign-out waits here, inside its purge
            let state = fx.state();
            let sam = t12_profile("u-1", "sam@beebeeb.io");
            let sign_out = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None);
            let sign_in = async {
                let deadline = Instant::now() + Duration::from_secs(30);
                while !fx
                    .seen
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|event| event.starts_with("remove:sign_out"))
                {
                    assert!(Instant::now() < deadline, "the sign-out never began");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                // Fix round 3, M4: a signal, not a sleep. The sign-out holds the engine slot from here (it took it before
                // its purge, where it now waits for the gate this test holds).
                while fx.acct.engine.try_lock().is_ok() {
                    assert!(Instant::now() < deadline, "the sign-out never held the engine slot");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let mut turn = fx.acct.session_generation();
                let signed_in = reauth_swap_token(
                    &state,
                    &fx.acct,
                    &LocalSources::for_test(dir.path()),
                    "tok-new",
                    &sam,
                    None,
                    &mut turn,
                )
                .map_err(|e| e.message);
                drop(gate);
                let mut slot = fx.acct.engine.lock().await;
                let started = start_engine_bound(
                    &state,
                    &fx.acct,
                    &mut slot,
                    &LocalDataPaths::for_test(dir.path(), dir.path()),
                    |_root, _token, _key| a_running_engine(),
                );
                drop(slot);
                (signed_in, started)
            };
            let (signed_out, (signed_in, started)) = tokio::join!(sign_out, sign_in);
            assert_eq!(
                signed_in,
                Ok(true),
                "the premise: the same account signed in again and its keys were loaded"
            );
            assert_eq!(
                signed_out,
                Ok(SignedOut::confirmed(SignOutOutcome::NotSignedIn)),
                "the premise: the sign-out returned Ok"
            );
            let engine_running = fx.acct.engine.lock().await.is_some();
            let keys_in_memory = fx.acct.session.lock().unwrap().is_some();
            assert_eq!(
                (started, engine_running, keys_in_memory),
                (Ok(EngineStart::NoSession), false, false),
                "(the sign-in's engine start, an engine after the sign-out returned Ok, keys in memory)"
            );
            assert_eq!(keychain_holds(fx.acct.id.as_str()), (false, false, false));
            fx.finish().await;
        });
    }

    /// Fix round 2, item 1: the already-signed-out path stops an engine it finds in the slot, through the one stop
    /// helper, as the signed-in path does.
    #[test]
    fn an_already_signed_out_sign_out_stops_an_engine_it_finds() {
        let mut fx = Fixture::new("t12f2-signed-out-engine-found");
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            fx.reconciler(Some(Reconciler::Answers));
            after_a_rejected_token(&fx);
            *fx.acct.engine.lock().await = Some(a_running_engine());
            let signed_out = clear_session_impl(&fx.state(), false, None, crate::UnknownFilesArg::Keep, None).await;
            assert_eq!(signed_out, Ok(SignedOut::confirmed(SignOutOutcome::NotSignedIn)));
            assert!(fx.acct.engine.lock().await.is_none(), "the engine it found is stopped");
            assert!(
                !fx.acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
                "and its stop was confirmed"
            );
            fx.finish().await;
        }));
    }

    /// Fix round 2, item 2 (re-review N1): an inconclusive startup probe, the start by the session's email, then the
    /// background naming confirms the same account. The running engine is neither stopped nor restarted; only an empty
    /// slot would get a start.
    #[test]
    fn background_naming_never_restarts_the_running_engine_of_the_same_account() {
        let fx = Fixture::new("t12f2-naming-keeps-the-engine");
        let id = fx.acct.id.as_str().to_string();
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-stored",
                &[4u8; 32],
                Some("t12@beebeeb.io"),
            )
            .unwrap();
            let mut inconclusive = HeldServer::start("503 Service Unavailable", "{}");
            inconclusive.release();
            let state = fx.state();
            assert!(
                restore_stored_session(&state, &fx.acct, &inconclusive.base_url).await,
                "the premise: restored, by its email"
            );
            assert!(
                fx.acct.cached_profile.lock().unwrap().is_none(),
                "the premise: the probe named nothing"
            );
            // The start by email (what `start_engine_if_possible` put in the slot), ticking.
            let ticks = Arc::new(AtomicUsize::new(0));
            let counter = ticks.clone();
            *fx.acct.engine.lock().await = Some(EngineRunner::for_test_with_task(tokio::spawn(async move {
                loop {
                    counter.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })));
            // The background request now succeeds and names the same account.
            let mut naming = HeldServer::start("200 OK", T12_PROFILE);
            naming.release();
            assert!(
                identify_unidentified_session(&state, &naming.base_url, std::time::Instant::now(), IdentifyPace::Now)
                    .await
            );
            let after = {
                let mut slot = fx.acct.engine.lock().await;
                keep_or_start_after_naming(&mut slot, |_| {
                    panic!("a running engine of the same account is not restarted")
                })
            };
            assert_eq!(after, Ok(AfterNaming::KeptRunning));
            let before = ticks.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                ticks.load(Ordering::SeqCst) > before,
                "the engine is still the one that ran, and still running"
            );
            // An empty slot (a start that was refused before the naming) is where a start happens.
            let empty_slot = tokio::sync::Mutex::new(None::<EngineRunner>);
            let mut guard = empty_slot.lock().await;
            assert_eq!(
                keep_or_start_after_naming(&mut guard, |_| Ok(EngineStart::Started)),
                Ok(AfterNaming::Started(EngineStart::Started))
            );
        }));
    }

    /// Fix round 3, M1 (re-review): a naming keeps a running engine only when nothing says it runs for another account.
    /// When the owner record of the local data and the named session both know a user id and the ids differ, the
    /// engine is stopped through the one stop helper and the start goes through the binding (here a stand-in that
    /// counts). The same id, an owner with no id, or a named session with no id: the engine is kept, never restarted.
    #[test]
    fn a_naming_that_finds_another_accounts_owner_by_id_stops_the_engine_and_starts_through_the_binding() {
        use crate::account_binding::Identity;
        let fx = Fixture::new("t12f3-m1-another-owner");
        multi_thread_runtime().block_on(async {
            let named = Identity::new(Some("u-1"), Some("sam@beebeeb.io"));
            let mut slot = fx.acct.engine.lock().await;
            *slot = Some(a_running_engine());
            let starts = AtomicUsize::new(0);
            let another = Identity::new(Some("u-other"), Some("sam@beebeeb.io"));
            let after = keep_or_rebind_after_naming(&fx.acct, &mut slot, Some(&another), &named, |slot| {
                starts.fetch_add(1, Ordering::SeqCst);
                assert!(slot.is_none(), "the old engine left the slot before the start");
                **slot = Some(a_running_engine());
                Ok(EngineStart::Started)
            })
            .await;
            assert_eq!(
                after,
                Ok(AfterNaming::Restarted(EngineStart::Started)),
                "another owner by id: stopped and started through the binding"
            );
            assert_eq!(starts.load(Ordering::SeqCst), 1);
            assert!(
                !fx.acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
                "through the one stop helper, confirmed"
            );
            for (name, owner, session) in [
                (
                    "the same id",
                    Identity::new(Some("u-1"), Some("old@beebeeb.io")),
                    named.clone(),
                ),
                (
                    "an owner with no id",
                    Identity::new(None, Some("sam@beebeeb.io")),
                    named.clone(),
                ),
                (
                    "a named session with no id",
                    another.clone(),
                    Identity::new(None, Some("sam@beebeeb.io")),
                ),
            ] {
                let after = keep_or_rebind_after_naming(&fx.acct, &mut slot, Some(&owner), &session, |_| {
                    panic!("{name}: not restarted")
                })
                .await;
                assert_eq!(after, Ok(AfterNaming::KeptRunning), "{name}");
                assert!(slot.is_some(), "{name}: the engine still runs");
            }
            let after =
                keep_or_rebind_after_naming(&fx.acct, &mut slot, None, &named, |_| panic!("no owner: not restarted"))
                    .await;
            assert_eq!(after, Ok(AfterNaming::KeptRunning), "no owner record");
            if let Some(engine) = slot.take() {
                stop_engine_in_slot(&fx.acct, engine).await;
            }
        });
    }

    /// Fix round 3, M1, the branch the test above leaves out: the owner record names another account by user id and the
    /// engine is stopped, but the stop is NOT confirmed (its task cannot be aborted, so it may still run with the
    /// keys). Nothing may start in its place and the result says so: the start refuses with the restart sentence, the
    /// slot stays empty, and the gate stays closed for every later start.
    #[test]
    fn a_naming_that_cannot_confirm_the_stop_of_another_accounts_engine_starts_nothing_and_says_so() {
        use crate::account_binding::Identity;
        let fx = Fixture::new("t12f3-m1-unconfirmed-stop");
        multi_thread_runtime().block_on(async {
            let named = Identity::new(Some("u-1"), Some("sam@beebeeb.io"));
            let another = Identity::new(Some("u-other"), Some("sam@beebeeb.io"));
            let mut slot = fx.acct.engine.lock().await;
            // An engine with no await point: tokio cannot force-abort it, so its stop cannot be confirmed. It gives up
            // by itself at a deadline, so no thread keeps spinning into the rest of the run.
            let deadline = Instant::now() + Duration::from_secs(8);
            *slot = Some(EngineRunner::for_test_with_task(tokio::task::spawn_blocking(
                move || {
                    while Instant::now() < deadline {
                        std::hint::black_box(());
                    }
                },
            )));
            let starts = AtomicUsize::new(0);
            // The start the production site passes: the one start gate first, and only then a spawn.
            let after = keep_or_rebind_after_naming(&fx.acct, &mut slot, Some(&another), &named, |slot| {
                engine_start_refusal(&fx.acct)?;
                starts.fetch_add(1, Ordering::SeqCst);
                **slot = Some(a_running_engine());
                Ok(EngineStart::Started)
            })
            .await;
            assert_eq!(
                after,
                Err(ENGINE_START_REFUSED_ERROR.to_string()),
                "the stop was not confirmed: no restart, and the result says to restart Beebeeb"
            );
            assert_eq!(starts.load(Ordering::SeqCst), 0, "nothing was spawned");
            assert!(slot.is_none(), "the old engine left the slot, and no new one took it");
            assert!(
                fx.acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
                "the one stop helper recorded the unconfirmed stop"
            );
            assert_eq!(
                engine_start_refusal(&fx.acct),
                Err(ENGINE_START_REFUSED_ERROR.to_string()),
                "and every later start refuses too"
            );
        });
    }

    /// Fix round 2, item 3, first half (re-check N2): the naming's Keychain email write failed (a locked
    /// Keychain, here the test store refusing it) after the owner backfill succeeded. It is not ignored: it is marked,
    /// and the next naming trigger writes it again (no request: the session is named), and clears the mark.
    #[test]
    fn a_failed_keychain_email_write_is_written_again_at_the_next_naming() {
        let fx = Fixture::new("t12f2-email-write-fails");
        let id = fx.acct.id.as_str().to_string();
        runtime().block_on(with_the_shared_state_dir(async {
            let dir = tempfile::tempdir().unwrap();
            crate::state_db::StateDb::open(dir.path().join("state.db"))
                .unwrap()
                .set_owner(&crate::account_binding::Identity::new(None, Some("T12@Beebeeb.io")))
                .unwrap();
            AuthVault::new(keychain::platform_keychain_store_for(&id))
                .store_account_email("T12@Beebeeb.io")
                .unwrap();
            *fx.acct.session.lock().unwrap() = Some(Session {
                token: "tok-t12".into(),
                master_key: [3u8; 32],
                email: Some("T12@Beebeeb.io".into()),
            });
            let state = fx.state();
            keychain::test_fail_account_email_writes(Some(&id), true);
            let named = name_session_after_profile(
                &SessionWrite::for_test(),
                &state,
                &fx.acct,
                "tok-t12",
                &t12_profile("u-t12", "t12@beebeeb.io"),
                Some(dir.path()),
            );
            keychain::test_fail_account_email_writes(Some(&id), false);
            assert!(named, "the backfill succeeded, so the session is named");
            assert_eq!(
                keychain_account_email(&id).as_deref(),
                Some("T12@Beebeeb.io"),
                "the premise: the Keychain write failed"
            );
            assert!(
                fx.acct.keychain_email_pending.load(Ordering::SeqCst),
                "and it is marked, not ignored"
            );
            // The next naming trigger: the session is named already, so no request is made (nothing listens here).
            let nobody = {
                let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let url = format!("http://{}", l.local_addr().unwrap());
                drop(l);
                url
            };
            assert!(
                !identify_unidentified_session(&state, &nobody, std::time::Instant::now(), IdentifyPace::Now).await
            );
            assert_eq!(
                keychain_account_email(&id).as_deref(),
                Some("t12@beebeeb.io"),
                "written again"
            );
            assert!(
                !fx.acct.keychain_email_pending.load(Ordering::SeqCst),
                "and the mark is cleared"
            );
        }));
    }

    /// An unidentified session in memory: an email, no profile naming it.
    fn unidentified(fx: &Fixture) {
        *fx.acct.session.lock().unwrap() = Some(Session {
            token: "tok-t12".into(),
            master_key: [3u8; 32],
            email: Some("t12@beebeeb.io".into()),
        });
    }

    fn requests_seen(server: &mut HeldServer) -> usize {
        let mut seen = 0;
        while server.arrived.try_recv().is_ok() {
            seen += 1;
        }
        seen
    }

    /// Fix round 1, item 4 (review M2), with an injected clock: app activation asks at most once a minute per account;
    /// "Try again" is not debounced. The server answers 503, so the session stays unidentified and every allowed call
    /// asks.
    #[test]
    fn app_activation_asks_at_most_once_a_minute_and_try_again_is_not_debounced() {
        let fx = Fixture::new("t12f1-identify-debounce");
        runtime().block_on(with_the_shared_state_dir(async {
            unidentified(&fx);
            let mut server = HeldServer::start("503 Service Unavailable", "{}");
            server.release();
            let state = fx.state();
            let t0 = std::time::Instant::now();
            let at = |seconds| t0 + Duration::from_secs(seconds);
            assert!(!identify_on_app_activation(&state, &server.base_url, at(0)).await);
            assert_eq!(requests_seen(&mut server), 1, "the first activation asks");
            assert!(!identify_on_app_activation(&state, &server.base_url, at(30)).await);
            assert_eq!(requests_seen(&mut server), 0, "30 s later: debounced");
            assert!(!identify_on_app_activation(&state, &server.base_url, at(61)).await);
            assert_eq!(requests_seen(&mut server), 1, "61 s later: asks again");
            assert!(!identify_unidentified_session(&state, &server.base_url, at(62), IdentifyPace::Now).await);
            assert_eq!(requests_seen(&mut server), 1, "Try again 1 s later: not debounced");
        }));
    }

    /// Fix round 1, item 4: at most one identify request in flight per account. While one is held at the server, another
    /// activation (past the debounce) and a "Try again" ask nothing and return at once.
    #[test]
    fn only_one_identify_request_is_in_flight_per_account() {
        let fx = Fixture::new("t12f1-identify-single-flight");
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            unidentified(&fx);
            let mut server = HeldServer::start("503 Service Unavailable", "{}");
            let base_url = server.base_url.clone();
            let state = fx.state();
            let t0 = std::time::Instant::now();
            let first = identify_on_app_activation(&state, &base_url, t0);
            let others = async {
                let held = server.next_request().await;
                let later = identify_on_app_activation(&state, &base_url, t0 + Duration::from_secs(120));
                let again =
                    identify_unidentified_session(&state, &base_url, t0 + Duration::from_secs(121), IdentifyPace::Now);
                let (later, again) = tokio::time::timeout(Duration::from_secs(5), async { (later.await, again.await) })
                    .await
                    .expect("neither waits for the request in flight");
                tokio::time::sleep(Duration::from_millis(300)).await;
                let asked_meanwhile = requests_seen(&mut server);
                server.release();
                (held, later, again, asked_meanwhile)
            };
            let (first, (held, later, again, asked_meanwhile)) = tokio::join!(first, others);
            assert!(held.starts_with("GET /api/v1/auth/me"), "{held}");
            assert_eq!(
                (first, later, again, asked_meanwhile),
                (false, false, false, 0),
                "(first, later activation, Try again, requests while one was in flight)"
            );
            assert!(
                !fx.acct.identify_flight.lock().unwrap().in_flight,
                "the flight ended with its request"
            );
        }));
    }

    /// Fix round 1, item 5 (review M3): a fetch that names nothing (the server knows the session's token under another
    /// address) moves no generation, so a sign-in in flight is not refused by it; one that names the session moves it.
    #[test]
    fn an_identify_that_names_nothing_moves_no_generation() {
        let fx = Fixture::new("t12f1-identify-no-op");
        runtime().block_on(with_the_shared_state_dir(async {
            unidentified(&fx);
            let state = fx.state();
            let in_flight = fx.acct.session_generation();
            let mut elsewhere = HeldServer::start("200 OK", r#"{"user_id":"u-k","email":"kim@beebeeb.io","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}"#);
            elsewhere.release();
            assert!(!establish_session_identity(&state, &fx.acct, &elsewhere.base_url, None).await, "another address names nothing");
            assert!(fx.acct.session_unchanged_since(in_flight), "and moves no generation");
            let mut naming = HeldServer::start("200 OK", T12_PROFILE);
            naming.release();
            assert!(establish_session_identity(&state, &fx.acct, &naming.base_url, None).await, "the same address names it");
            assert!(!fx.acct.session_unchanged_since(in_flight), "which moves the generation");
        }));
    }

    /// Fix round 1, item 9 (review M9): the identify's own generation check. A transition that leaves the session in
    /// memory as it is (so the token check alone would pass) but happens while the fetch is in flight: nothing is named.
    #[test]
    fn an_identify_whose_turn_moved_while_it_waited_names_nothing() {
        let fx = Fixture::new("t12f1-identify-turn-moved");
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            unidentified(&fx);
            let mut server = HeldServer::start("200 OK", T12_PROFILE);
            let base_url = server.base_url.clone();
            let state = fx.state();
            let identify = establish_session_identity(&state, &fx.acct, &base_url, None);
            let transition_then_answer = async {
                server.next_request().await;
                drop(end_sessions_in_flight(&fx.acct, account::SessionTransition::Write));
                server.release();
            };
            let (named, ()) = tokio::join!(identify, transition_then_answer);
            assert!(!named, "nothing is named after the turn moved");
            assert!(fx.acct.cached_profile.lock().unwrap().is_none());
            assert_eq!(
                fx.acct
                    .session
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|s| s.email.clone())
                    .as_deref(),
                Some("t12@beebeeb.io")
            );
        }));
    }

    /// Fix round 1, item 7 (review M6): the startup restore asks the server once (its probe) and never waits for a
    /// second request: the probe here is inconclusive (503) and any later request would hang, yet the restore returns at
    /// once with the session installed (its second request runs in the background, see `restore_session_inner`).
    #[test]
    fn the_startup_restore_never_waits_for_a_second_request() {
        let fx = Fixture::new("t12f1-restore-no-second-wait");
        let id = fx.acct.id.as_str().to_string();
        multi_thread_runtime().block_on(with_the_shared_state_dir(async {
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-stored",
                &[4u8; 32],
                Some("t12@beebeeb.io"),
            )
            .unwrap();
            // The first request is answered 503 at once; every later one is held until the test ends.
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(AtomicUsize::new(0));
            let counter = seen.clone();
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                let mut held = Vec::new();
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut buf = [0u8; 8192];
                    let _ = stream.read(&mut buf);
                    if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                        let _ = stream.write_all(
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        );
                    } else {
                        held.push(stream);
                    }
                }
            });
            let state = fx.state();
            let restored = tokio::time::timeout(
                Duration::from_secs(4),
                restore_stored_session(&state, &fx.acct, &base_url),
            )
            .await
            .expect("the restore does not wait for a second request");
            assert!(restored, "the session is installed and an engine start follows");
            assert_eq!(seen.load(Ordering::SeqCst), 1, "one request: the probe");
            assert!(fx.acct.session.lock().unwrap().is_some());
        }));
    }

    /// Fix round 1, item 9 (review M9): the browser sign-in's install, refused because its turn moved, revokes the
    /// session it would have stored; an install that goes through revokes nothing.
    #[test]
    fn a_refused_browser_install_revokes_the_session_it_would_have_stored() {
        let fx = Fixture::new("t12f1-browser-revoke");
        multi_thread_runtime().block_on(async {
            let mut server = HeldServer::start("200 OK", "{}");
            server.release();
            let state = fx.state();
            let mut stale = fx.acct.session_generation();
            drop(end_sessions_in_flight(&fx.acct, account::SessionTransition::SignOut));
            let refused = install_new_session_or_revoke(
                &state,
                &fx.acct,
                &mut stale,
                "tok-refused",
                &[2u8; 32],
                Some("t12@beebeeb.io".into()),
                None,
                &server.base_url,
            )
            .await;
            assert_eq!(refused, Err(SESSION_CHANGED_WHILE_WAITING.to_string()));
            let revoked = server.next_request().await;
            assert!(
                revoked.starts_with("POST /api/v1/auth/logout"),
                "the refused session is revoked: {revoked}"
            );
            assert!(
                fx.acct.session.lock().unwrap().is_none()
                    && keychain_holds(fx.acct.id.as_str()) == (false, false, false),
                "nothing stored"
            );
            let mut current = fx.acct.session_generation();
            let installed = install_new_session_or_revoke(
                &state,
                &fx.acct,
                &mut current,
                "tok-ok",
                &[2u8; 32],
                Some("t12@beebeeb.io".into()),
                None,
                &server.base_url,
            )
            .await;
            assert_eq!(installed, Ok(()));
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(
                requests_seen(&mut server),
                0,
                "an install that went through revokes nothing"
            );
        });
    }
