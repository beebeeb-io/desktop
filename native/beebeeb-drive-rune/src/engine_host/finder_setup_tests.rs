//! The reconciler wiring, with the driver running on the macOS ports over fakes (no bridge call, no engine, no
//! Keychain). The call-site pins read the sources.

use super::*;

#[test]
fn the_session_edges_are_the_triggers() {
    let (none, keys, signed_in, out) = (
        SessionFacts {
            keys: false,
            auth: false,
        },
        SessionFacts { keys: true, auth: true },
        SessionFacts {
            keys: false,
            auth: true,
        },
        SessionFacts {
            keys: false,
            auth: false,
        },
    );
    assert_eq!(edge(signed_in, keys), Some(Edge::KeysArrived));
    assert_eq!(edge(none, keys), Some(Edge::KeysArrived));
    assert_eq!(edge(keys, signed_in), Some(Edge::Lock));
    assert_eq!(edge(keys, out), Some(Edge::SignOut));
    assert_eq!(edge(signed_in, out), Some(Edge::SignOut));
    assert_eq!(edge(keys, keys), None);
    assert_eq!(edge(out, out), None);
}

/// Both OS input sets: macOS (a screen lock leaves the keys in memory) and Windows Explorer (a lock takes them out).
#[test]
fn sign_out_then_first_try_sign_in_sends_keys_arrived() {
    let (signed_in, out) = (SessionFacts { keys: true, auth: true }, SessionFacts::default());
    for os_before in [
        signed_in, /* macOS */
        SessionFacts {
            keys: false,
            auth: true,
        }, /* Windows, locked */
    ] {
        let remembered = after_sign_out(os_before);
        assert_eq!(remembered, out);
        // The sign-out thread reports (no edge), then the first sign-in (no 2FA) brings keys and session at once.
        assert_eq!(edge(remembered, out), None);
        assert_eq!(edge(remembered, signed_in), Some(Edge::KeysArrived));
    }
}

#[test]
fn lock_then_unlock_lifts_the_hold_on_both_os_input_sets() {
    let open = SessionFacts { keys: true, auth: true };
    let remembered = after_lock(open);
    assert_eq!(
        remembered,
        SessionFacts {
            keys: false,
            auth: true
        }
    );
    // macOS: the screen lock leaves the facts as they are; Windows: the keys leave. The unlock sees keys either way.
    assert_eq!(
        edge(remembered, open),
        Some(Edge::KeysArrived),
        "Touch ID or password unlock"
    );
}

#[test]
fn the_session_edge_sites_report_and_remember() {
    let own = include_str!("finder_setup.rs");
    assert!(own.contains("remember(after_lock);") && own.contains("remember(after_sign_out);"));
    assert!(include_str!("signin.rs").contains("unlock_screen_for(asked_for).is_ok() {\n"));
    assert!(include_str!("account_actions.rs").contains("finder_setup::session_changed();"));
}

#[test]
fn every_trigger_call_is_wired_at_its_site() {
    let signin = include_str!("signin.rs");
    assert!(
        signin.contains("job(&for_worker);\n                super::finder_setup::session_changed();"),
        "the sign-in worker reports the session after every job"
    );
    let host = include_str!("mod.rs");
    assert!(host.contains("mod finder_setup;"), "the module is declared");
    assert!(
        host.contains(
            "pub fn start_finder_setup(&self) {\n        finder_setup::start(self.engine.clone(), self.identity);"
        ),
        "the host has one start of the reconciler, apart from the Windows first run"
    );
    assert!(
        !host.contains("self.first_run.read(&*self.engine);\n        finder_setup::start"),
        "the first run no longer carries the reconciler's start (macOS never ran it)"
    );
    let shell = include_str!("../shell/mod.rs");
    assert_eq!(
        shell.matches("live.start_finder_setup();").count(),
        1,
        "the macOS start calls finder_setup::start once"
    );
    assert!(
        shell.contains("cfg!(target_os = \"macos\")\n        && let Some((_, live)) = &real_engine"),
        "the call is for macOS and for the real engine only"
    );
    let own = include_str!("finder_setup.rs");
    for pin in [
        "Some(Edge::KeysArrived) => trigger(Trigger::KeysArrived)",
        "Some(Edge::Lock) => lock()",
        "Some(Edge::SignOut) => {",
        "remove(Trigger::SignOut)",
        "remove(Trigger::Repair)",
        "trigger(Trigger::TryAgain)",
        "let _ = handle.trigger(Trigger::Launch);",
        "running.handle.app_activated()",
        "copy_details_from(view, app_version, macos_version, lifecycle_tail)",
    ] {
        assert!(own.contains(pin), "missing: {pin}");
    }
}

#[test]
fn copy_details_reads_the_lifecycle_tail_and_names_no_path() {
    let text = copy_details(&view(), "1.2.3", "26.0");
    assert!(text.contains("Beebeeb 1.2.3 on macOS 26.0"), "{text}");
    assert!(text.contains("Lifecycle log (last "), "{text}");
    assert!(!text.contains("/Users/"), "{text}");
}

/// The tests that touch the process-wide debt (`REMOVAL_OWED`) take turns.
static DEBT: Mutex<()> = Mutex::new(());

fn debt_turn() -> std::sync::MutexGuard<'static, ()> {
    DEBT.lock().unwrap_or_else(PoisonError::into_inner)
}

fn owed_on_disk(path: &Path) -> bool {
    beebeeb_engine::config::DesktopConfig::load_from(path)
        .expect("the settings file loads")
        .finder_removal_owed
}

/// A quit: this run's memory is gone, the settings file stays; the next start restores from the file.
fn restart(path: &Path) {
    REMOVAL_OWED.store(false, Ordering::SeqCst);
    restore_removal_owed(beebeeb_engine::config::DesktopConfig::load_from(path).ok().as_ref());
}

#[test]
fn a_sign_out_removal_that_failed_is_owed_until_a_removal_is_confirmed() {
    let _turn = debt_turn();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop.toml");
    set_removal_owed(Some(&path), false);
    assert!(!removal_owed_now() && !owed_on_disk(&path));
    set_removal_owed(Some(&path), true);
    assert!(removal_owed_now(), "the live session reads the run's debt");
    assert!(owed_on_disk(&path), "and the settings file keeps it");
    set_removal_owed(Some(&path), false);
    assert!(!removal_owed_now() && !owed_on_disk(&path));
    // No settings file: this run still owes it.
    set_removal_owed(None, true);
    assert!(removal_owed_now());
    set_removal_owed(None, false);
    let src = include_str!("finder_setup.rs");
    assert!(
        src.contains("fn removal_owed(&self) -> bool {\n        removal_owed_now()"),
        "wired to the debt"
    );
    assert!(
        src.contains("fn clear_removal_owed(&self) {\n        set_removal_owed(self.config_path().as_deref(), false);"),
        "a confirmed removal releases the saved debt"
    );
    assert!(
        src.contains("restore_removal_owed(config.as_ref());\n        let persisted"),
        "the launch restores the debt before the driver is spawned"
    );
}

/// dm-90: a quit between a failed sign-out removal and the next sign-in keeps the debt, and the sign-in that clears
/// `finder_signed_out_by_choice` does not clear it.
#[test]
fn a_failed_removal_survives_a_restart_and_a_sign_in() {
    let _turn = debt_turn();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop.toml");
    beebeeb_engine::config::DesktopConfig::try_update_at(&path, |cfg| {
        cfg.finder_signed_out_by_choice = true;
        (true, ())
    })
    .unwrap();
    set_removal_owed(Some(&path), true);
    restart(&path);
    assert!(removal_owed_now(), "the next start owes it again");
    // The sign-in's reset (`on_observed` → `PersistSignedOutByChoice(false)`) leaves the debt alone.
    beebeeb_engine::config::DesktopConfig::try_update_at(&path, |cfg| {
        cfg.finder_signed_out_by_choice = false;
        (true, ())
    })
    .unwrap();
    restart(&path);
    assert!(removal_owed_now() && owed_on_disk(&path));
    // A launch never releases one: not on a file without the key, not on an unreadable file.
    restore_removal_owed(Some(&beebeeb_engine::config::DesktopConfig::default()));
    restore_removal_owed(None);
    assert!(removal_owed_now());
    set_removal_owed(Some(&path), false);
    restart(&path);
    assert!(!removal_owed_now(), "released for good once confirmed");
}

#[cfg(target_os = "macos")]
mod on_ports {
    use super::*;
    use crate::shell::platform::macos::macos_fp::ports::{FpBridge, FpHost};
    use crate::shell::platform::macos::macos_fp::removal::{DomainRemoval, RemovalFailure};
    use beebeeb_engine::finder_setup::core::DomainState;
    use beebeeb_engine::finder_setup::driver::FinderSetupHandle;
    use beebeeb_engine::surfaces::phase::FinderSetup;
    use std::sync::atomic::AtomicUsize;

    type Log = Arc<Mutex<Vec<&'static str>>>;

    struct FakeSession {
        facts: Mutex<SessionFacts>,
        owed: AtomicBool,
        log: Log,
        /// `start_engine` waits for this before answering (a check in flight).
        hold_start: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        changes: AtomicUsize,
        /// Set: the debt is the live session's (`REMOVAL_OWED`, saved in this settings file), not `owed`.
        config: Option<PathBuf>,
    }

    impl Session for FakeSession {
        fn facts(&self) -> SessionFacts {
            *self.facts.lock().unwrap()
        }
        fn removal_owed(&self) -> bool {
            match self.config {
                Some(_) => removal_owed_now(),
                None => self.owed.load(Ordering::SeqCst),
            }
        }
        fn config_path(&self) -> Option<PathBuf> {
            self.config.clone()
        }
        fn start_engine(&self) -> Result<bool, FpError> {
            self.log.lock().unwrap().push("start_engine");
            if let Some(release) = self.hold_start.lock().unwrap().take() {
                let _ = release.recv();
            }
            Ok(true)
        }
        fn finish_ready(&self) -> Result<(), FpError> {
            Ok(())
        }
        fn stop_engine(&self) -> Result<(), FpError> {
            self.log.lock().unwrap().push("stop_engine");
            Ok(())
        }
        fn clear_removal_owed(&self) {
            match &self.config {
                Some(path) => set_removal_owed(Some(path), false),
                None => self.owed.store(false, Ordering::SeqCst),
            }
            self.log.lock().unwrap().push("owed_cleared");
        }
        fn clear_domain_gone(&self) {}
        fn changed(&self) {
            self.changes.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FakeBridge {
        state: Mutex<DomainState>,
        log: Log,
        fail_remove: AtomicBool,
    }

    impl FpBridge for FakeBridge {
        fn add_domain(&self) -> Result<(), FpError> {
            self.log.lock().unwrap().push("add");
            *self.state.lock().unwrap() = DomainState::Enabled;
            Ok(())
        }
        fn domain_state(&self) -> Result<DomainState, FpError> {
            Ok(*self.state.lock().unwrap())
        }
        fn wait_for_domain_ready(&self, _timeout: Duration) -> Result<(), FpError> {
            Ok(())
        }
        fn remove(&self) -> Result<DomainRemoval, RemovalFailure> {
            self.log.lock().unwrap().push("remove");
            if self.fail_remove.load(Ordering::SeqCst) {
                return Err("the removal failed".to_string().into());
            }
            *self.state.lock().unwrap() = DomainState::NotRegistered;
            Ok(DomainRemoval::default())
        }
    }

    struct World {
        session: Arc<FakeSession>,
        bridge: Arc<FakeBridge>,
        log: Log,
    }

    fn world(facts: SessionFacts, domain: DomainState, owed: bool) -> World {
        let log: Log = Arc::default();
        World {
            session: Arc::new(FakeSession {
                facts: Mutex::new(facts),
                owed: AtomicBool::new(owed),
                log: log.clone(),
                hold_start: Mutex::new(None),
                changes: AtomicUsize::new(0),
                config: None,
            }),
            bridge: Arc::new(FakeBridge {
                state: Mutex::new(domain),
                log: log.clone(),
                fail_remove: AtomicBool::new(false),
            }),
            log,
        }
    }

    const SIGNED_IN: SessionFacts = SessionFacts { keys: true, auth: true };

    fn run<T>(test: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(test)
    }

    fn start(world: &World) -> FinderSetupHandle {
        spawn_with(
            world.session.clone(),
            world.bridge.clone(),
            LaunchLocation::Applications,
            None,
        )
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..5_000 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        panic!("never reached: {what}");
    }

    async fn ready(handle: &FinderSetupHandle) {
        until("Ready", || handle.view().setup == FinderSetup::Ready).await;
    }

    #[test]
    fn launching_signed_in_lands_ready() {
        let w = world(SIGNED_IN, DomainState::NotRegistered, false);
        run(async {
            let handle = start(&w);
            ready(&handle).await;
            assert!(!handle.is_held());
        });
        let log = w.log.lock().unwrap().clone();
        let at = |name| {
            log.iter()
                .position(|l| *l == name)
                .unwrap_or_else(|| panic!("no {name} in {log:?}"))
        };
        assert!(at("start_engine") < at("add"), "{log:?}");
    }

    #[test]
    fn lock_cancels_a_check_and_holds() {
        let w = world(SIGNED_IN, DomainState::NotRegistered, false);
        let (release, held) = std::sync::mpsc::channel();
        *w.session.hold_start.lock().unwrap() = Some(held);
        run(async {
            let handle = start(&w);
            until("the check is in flight", || {
                w.log.lock().unwrap().contains(&"start_engine")
            })
            .await;
            let locking = tokio::spawn({
                let handle = handle.clone();
                async move { handle.lock(Duration::from_secs(5)).await }
            });
            tokio::time::sleep(Duration::from_millis(20)).await;
            release.send(()).unwrap();
            locking.await.unwrap().expect("the lock is acknowledged");
            assert!(handle.is_held());
            assert_ne!(handle.view().setup, FinderSetup::Ready);
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(
                !w.log.lock().unwrap().contains(&"add"),
                "the cancelled check added nothing"
            );
            // Held: Try again does nothing.
            handle.trigger(Trigger::TryAgain).unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(!w.log.lock().unwrap().contains(&"add"));
        });
    }

    #[test]
    fn keys_arriving_lift_the_hold() {
        let w = world(SIGNED_IN, DomainState::NotRegistered, false);
        run(async {
            let handle = start(&w);
            ready(&handle).await;
            handle.lock(Duration::from_secs(5)).await.unwrap();
            assert!(handle.is_held());
            assert_eq!(
                handle.view().setup,
                FinderSetup::Ready,
                "a lock never changes a landing state"
            );
            handle.trigger(Trigger::KeysArrived).unwrap();
            until("the hold is lifted", || !handle.is_held()).await;
        });
    }

    #[test]
    fn sign_out_removes_and_holds_until_keys_arrive() {
        let w = world(SIGNED_IN, DomainState::NotRegistered, false);
        run(async {
            let handle = start(&w);
            ready(&handle).await;
            *w.session.facts.lock().unwrap() = SessionFacts::default();
            handle
                .remove(Trigger::SignOut, Duration::from_secs(5))
                .await
                .expect("the removal is confirmed");
            assert!(w.log.lock().unwrap().contains(&"remove"));
            assert!(handle.is_held());
            assert_eq!(handle.view().setup, FinderSetup::Missing);
            *w.session.facts.lock().unwrap() = SIGNED_IN;
            handle.trigger(Trigger::KeysArrived).unwrap();
            ready(&handle).await;
            assert!(!handle.is_held());
        });
    }

    #[test]
    fn an_owed_removal_blocks_the_engine_start_until_it_is_confirmed() {
        let w = world(SIGNED_IN, DomainState::Enabled, true);
        run(async {
            // The host itself refuses while the debt stands.
            let host = FinderHost::new(w.session.clone());
            assert!(host.start_engine(Duration::from_secs(1)).await.is_err());
            assert!(
                !w.log.lock().unwrap().contains(&"start_engine"),
                "no engine started under a debt"
            );
            // The driver removes first, the debt is cleared, then the engine starts.
            let handle = start(&w);
            ready(&handle).await;
        });
        let log = w.log.lock().unwrap().clone();
        let at = |name| {
            log.iter()
                .position(|l| *l == name)
                .unwrap_or_else(|| panic!("no {name} in {log:?}"))
        };
        assert!(at("remove") < at("owed_cleared"), "{log:?}");
        assert!(at("owed_cleared") < at("start_engine"), "{log:?}");
    }

    /// A run of the app over the settings file at `path`, with the old account's domain `domain`.
    fn world_on_disk(facts: SessionFacts, domain: DomainState, path: &Path, fail_remove: bool) -> World {
        let mut w = world(facts, domain, false);
        Arc::get_mut(&mut w.session).unwrap().config = Some(path.to_path_buf());
        w.bridge.fail_remove.store(fail_remove, Ordering::SeqCst);
        w
    }

    /// dm-90, the review blocker: a sign-out removal fails and the person quits. The next start (and its sign-in,
    /// possibly another account) removes the old domain before any engine starts, and the debt is released only once
    /// a removal worked.
    #[test]
    fn a_failed_sign_out_removal_is_paid_after_a_restart_before_the_domain_is_reused() {
        let _turn = debt_turn();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.toml");
        REMOVAL_OWED.store(false, Ordering::SeqCst);

        // Run 1: signed in and Ready, then a sign-out whose removal fails.
        let first = world_on_disk(SIGNED_IN, DomainState::NotRegistered, &path, true);
        run(async {
            let handle = start(&first);
            ready(&handle).await;
            *first.session.facts.lock().unwrap() = SessionFacts::default();
            let config = path.clone();
            let removing = handle.clone();
            let removed = tokio::task::spawn_blocking(move || remove_on(&removing, Some(&config), Trigger::SignOut))
                .await
                .unwrap();
            assert!(removed.is_err(), "the removal failed");
        });
        assert!(
            removal_owed_now() && owed_on_disk(&path),
            "a failed sign-out removal is owed"
        );

        // Run 2, after a quit: signed in again, the old domain still there, the removal fails again.
        restart(&path);
        let second = world_on_disk(SIGNED_IN, DomainState::Enabled, &path, true);
        run(async {
            let _handle = start(&second);
            until("the owed removal is tried", || {
                second.log.lock().unwrap().contains(&"remove")
            })
            .await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let log = second.log.lock().unwrap().clone();
        assert!(!log.contains(&"start_engine"), "no engine on the old domain: {log:?}");
        assert!(!log.contains(&"owed_cleared"), "{log:?}");
        assert!(owed_on_disk(&path), "still owed: the removal did not work");

        // Run 3, after another quit: the removal works, first; the debt clears; then the engine starts.
        restart(&path);
        assert!(removal_owed_now());
        let third = world_on_disk(SIGNED_IN, DomainState::Enabled, &path, false);
        run(async {
            let handle = start(&third);
            ready(&handle).await;
        });
        let log = third.log.lock().unwrap().clone();
        let at = |name| {
            log.iter()
                .position(|l| *l == name)
                .unwrap_or_else(|| panic!("no {name} in {log:?}"))
        };
        assert!(at("remove") < at("owed_cleared"), "{log:?}");
        assert!(at("owed_cleared") < at("start_engine"), "{log:?}");
        assert!(
            !removal_owed_now() && !owed_on_disk(&path),
            "released once the removal worked"
        );
    }
}

fn view() -> FinderSetupView {
    FinderSetupView::initial(LaunchLocation::Applications)
}

/// dm-43: the lifecycle log and the reimport marker carry the release version, not the crate's 0.0.1.
#[test]
fn the_version_comes_from_the_release_version_when_set() {
    assert_eq!(
        RELEASE_VERSION,
        option_env!("BEEBEEB_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
    );
}
