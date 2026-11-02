//! The persisted flags (removal owed, domain gone), the config's one-update rule, and the
//! kept folder that a late removal saves and alerts.
use super::*;

    /// F3: the macOS port releases the persisted "removal owed" flag when the core says a removal is confirmed, and
    /// the facts the core observes carry the same flag the engine start reads.
    #[test]
    fn the_macos_port_releases_the_removal_debt_and_the_facts_carry_it() {
        let source = include_str!("../../macos_ports.rs").replace("\r\n", "\n");
        let source = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let release = &source[source
            .find("fn domain_removal_confirmed(")
            .expect("the port implements it")..];
        let release = &release[..release.find("\n    }\n").unwrap()];
        assert!(release.contains("crate::clear_finder_removal_owed();"), "{release}");
        let facts = &source[source.find("fn session_facts(").unwrap()..];
        let facts = &facts[..facts.find("\n}\n").unwrap()];
        assert!(facts.contains("crate::finder_removal_owed()"), "{facts}");
    }

    /// Fix round 3: the macOS port clears the persisted "domain gone" mark when the core says the domain was
    /// registered.
    #[test]
    fn the_macos_port_clears_the_domain_gone_mark_when_the_domain_is_added() {
        let source = include_str!("../../macos_ports.rs").replace("\r\n", "\n");
        let source = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let added = &source[source.find("fn domain_added(").expect("the port implements it")..];
        let added = &added[..added.find("\n    }\n").unwrap()];
        assert!(added.contains("crate::clear_finder_domain_gone();"), "{added}");
    }

    #[test]
    fn an_unreadable_config_never_counts_as_signed_out_by_choice() {
        // Fail safe: "signed out by choice" removes the domain, so only a config we could read and
        // that says so may produce it.
        let unreadable = facts_from(false, false, false, Err("could not read desktop.toml".to_string()));
        assert_eq!(
            unreadable,
            SessionFacts {
                vault_unlocked: false,
                auth_present: false,
                signed_out_by_choice: false,
                removal_owed: false
            }
        );
        let cfg = DesktopConfig {
            finder_signed_out_by_choice: true,
            ..DesktopConfig::default()
        };
        assert!(facts_from(false, false, false, Ok(cfg)).signed_out_by_choice);
        let readable = facts_from(true, true, false, Ok(DesktopConfig::default()));
        assert_eq!(
            readable,
            SessionFacts {
                vault_unlocked: true,
                auth_present: true,
                signed_out_by_choice: false,
                removal_owed: false
            }
        );
    }

    #[test]
    fn the_config_is_written_only_when_a_value_changes() {
        let mut cfg = DesktopConfig::default();
        let record = FailureRecord {
            reason: FinderFailureReason::FolderTaken,
            domain: "NSCocoaErrorDomain".into(),
            code: 516,
            at: 1_791_291_909,
        };
        assert!(
            !set_failure(&mut cfg, None),
            "no failure saved, none to clear: nothing to write"
        );
        assert!(set_failure(&mut cfg, Some(record.clone())));
        assert_eq!(cfg.finder_last_failure, Some(record.clone()));
        assert!(
            !set_failure(&mut cfg, Some(record.clone())),
            "the same record again writes nothing"
        );
        assert!(set_failure(&mut cfg, None), "Ready clears it");
        assert_eq!(cfg.finder_last_failure, None);

        assert!(!set_signed_out_by_choice(&mut cfg, false));
        assert!(set_signed_out_by_choice(&mut cfg, true));
        assert!(cfg.finder_signed_out_by_choice);
        assert!(!set_signed_out_by_choice(&mut cfg, true));
        assert!(set_signed_out_by_choice(&mut cfg, false));
    }

    // ── Rebase re-review I1: a removal that outlives the op limit still shows its folder ──────────

    const KEPT_AT: &str = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";

    fn kept_at(path: &str) -> KeptFolder {
        KeptFolder::Kept {
            path: path.to_string(),
            contents_checked: true,
            empty: false,
        }
    }

    type Alerts = Arc<std::sync::Mutex<Vec<String>>>;

    /// The app's late sink, `surface_reconciler_kept`, with its two steps on a test config file and the alert
    /// recorded: the same functions and order (`surface_kept_folder_with`, then `save_kept_folder_for_row_with`),
    /// without a window.
    fn recording_sink(config: std::path::PathBuf, alerts: Alerts) -> impl FnOnce(KeptFolder) + Send + 'static {
        move |kept| {
            let location = DomainRemoval { kept }.kept_location("test");
            crate::surface_kept_folder_with(
                location.as_deref(),
                |location| {
                    crate::save_kept_folder_for_row_with(
                        location,
                        |location| {
                            crate::finder_removal::remember_kept_folder_at(&config, location)
                                .expect("the folder is saved for the row");
                        },
                        || {},
                    )
                },
                |location| alerts.lock().unwrap().push(location.to_string()),
            );
        }
    }

    /// The removal arm's two halves: the gate's call (`answering`) and the channel `removal_answer` reads.
    fn removal_with_late_sink(
        remove: impl FnOnce() -> Removal + Send + 'static,
        late: impl FnOnce(KeptFolder) + Send + 'static,
    ) -> (
        impl FnOnce() -> Result<(), FpError> + Send + 'static,
        oneshot::Receiver<Removal>,
    ) {
        let (answer, answered) = oneshot::channel();
        (answering(answer, remove, late), answered)
    }

    fn saved_folder(config: &std::path::Path) -> Option<String> {
        if !config.exists() {
            return None;
        }
        DesktopConfig::load_from(config)
            .expect("the test config reads back")
            .kept_unsynced_folder
    }

    /// The reviewer's case: `fileproviderd` is slow, the reconciler gives up at its limit and answers `OP_TIMEOUT`
    /// with no folder, and the OS call then returns having kept files. The folder still reaches the person, once:
    /// saved for the Settings › Sync row, then the alert. The limit is injected (30 ms, not 15 s).
    #[tokio::test]
    async fn a_removal_that_finishes_after_the_op_limit_still_saves_the_row_and_raises_the_alert() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = dir.path().join("desktop.toml");
        let alerts = Alerts::default();
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let (call, answered) = removal_with_late_sink(
            move || {
                let _ = hold.recv();
                Ok(DomainRemoval { kept: kept_at(KEPT_AT) })
            },
            recording_sink(config.clone(), alerts.clone()),
        );
        let cut = within(
            Op::RemoveDomain,
            Duration::from_millis(30),
            gate.run_waiting(BRIDGE_GATE_WAIT, call),
        )
        .await;
        match removal_answer(cut, answered) {
            OpResult::Removed(Err(error), kept) => {
                assert_eq!(error.message, "RemoveDomain did not answer within 30ms");
                assert_eq!(kept, KeptFolder::default(), "the reconciler has no folder yet");
            }
            other => panic!("the reconciler gives up at its limit, as before: {other:?}"),
        }
        assert!(alerts.lock().unwrap().is_empty(), "nothing kept yet, nothing shown");
        assert_eq!(saved_folder(&config), None);

        release.send(()).expect("the OS call is still running");
        until("the gate to be released by the returning OS call", || gate.is_idle()).await;
        assert_eq!(
            *alerts.lock().unwrap(),
            vec![KEPT_AT.to_string()],
            "the late folder raises one alert, naming it"
        );
        assert_eq!(
            saved_folder(&config).as_deref(),
            Some(KEPT_AT),
            "and is saved for the Settings › Sync row"
        );
    }

    /// The other side of "exactly once": a removal that answers inside the limit gives the reconciler its folder
    /// (the driver hands it to the waiter or the port), and nothing goes to the late sink. Also for a removal that
    /// failed and still kept a folder (review M2).
    #[tokio::test]
    async fn a_removal_that_answers_in_time_gives_the_reconciler_its_folder_and_nothing_goes_to_the_late_sink() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = dir.path().join("desktop.toml");
        let alerts = Alerts::default();
        let gate = BridgeGate::new();
        let (call, answered) = removal_with_late_sink(
            || Ok(DomainRemoval { kept: kept_at(KEPT_AT) }),
            recording_sink(config.clone(), alerts.clone()),
        );
        let cut = within(
            Op::RemoveDomain,
            Duration::from_secs(5),
            gate.run_waiting(BRIDGE_GATE_WAIT, call),
        )
        .await;
        assert_eq!(
            removal_answer(cut, answered),
            OpResult::Removed(Ok(()), kept_at(KEPT_AT))
        );

        let os_error = FpError::new("NSFileProviderErrorDomain", -2001, "the provider is not running");
        let (call, answered) = removal_with_late_sink(
            {
                let os_error = os_error.clone();
                move || {
                    Err(BridgeRemovalFailure {
                        error: os_error,
                        kept: kept_at(KEPT_AT),
                    })
                }
            },
            recording_sink(config.clone(), alerts.clone()),
        );
        let cut = within(
            Op::RemoveDomain,
            Duration::from_secs(5),
            gate.run_waiting(BRIDGE_GATE_WAIT, call),
        )
        .await;
        assert_eq!(
            removal_answer(cut, answered),
            OpResult::Removed(Err(os_error), kept_at(KEPT_AT))
        );
        until("the gate to be idle", || gate.is_idle()).await;
        assert!(alerts.lock().unwrap().is_empty(), "the late sink saw nothing");
        assert_eq!(saved_folder(&config), None);
    }

    /// The moment of the cut-off, both orders, without a clock: an answer sent before the reconciler reads it is
    /// the reconciler's (and the late sink sees nothing); a call that returns after the reconciler gave up hands its
    /// folder to the late sink (and the reconciler has none). A failed removal's folder counts too (review M2).
    #[test]
    fn at_the_cut_off_the_folder_goes_to_the_reconciler_or_to_the_late_sink_never_both_never_neither() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = dir.path().join("desktop.toml");
        let alerts = Alerts::default();
        let timeout = || FpError::app(app_code::OP_TIMEOUT, "RemoveDomain did not answer within 30ms");

        let (call, answered) = removal_with_late_sink(
            || Ok(DomainRemoval { kept: kept_at(KEPT_AT) }),
            recording_sink(config.clone(), alerts.clone()),
        );
        assert_eq!(call(), Ok(()), "the OS call returned and sent");
        assert_eq!(
            removal_answer(Err(timeout()), answered),
            OpResult::Removed(Ok(()), kept_at(KEPT_AT)),
            "an answer already sent is read, though `within` fired"
        );
        assert!(alerts.lock().unwrap().is_empty());

        let later = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 11:05)";
        let (call, answered) = removal_with_late_sink(
            move || {
                Err(BridgeRemovalFailure {
                    error: FpError::new("NSFileProviderErrorDomain", -2001, "the provider is not running"),
                    kept: kept_at(later),
                })
            },
            recording_sink(config.clone(), alerts.clone()),
        );
        assert_eq!(
            removal_answer(Err(timeout()), answered),
            OpResult::Removed(Err(timeout()), KeptFolder::default()),
            "the reconciler gave up first: it has no folder"
        );
        assert_eq!(call(), Ok(()), "the OS call returns after that");
        assert_eq!(*alerts.lock().unwrap(), vec![later.to_string()], "the late sink has it");
        assert_eq!(saved_folder(&config).as_deref(), Some(later));
    }

    // ── Rebase re-review Minor 3: the reconciler's config writes are one load-change-save under the lock ──

    /// Runs `slow` on its own thread and returns once `slow` has signalled that it is between its load and its save;
    /// the caller's next step runs while it is in flight. `slow` gets the signal to send.
    fn in_flight<R: Send + 'static>(
        slow: impl FnOnce(std::sync::mpsc::Sender<()>) -> R + Send + 'static,
    ) -> std::thread::JoinHandle<R> {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || slow(entered_tx));
        entered_rx.recv().expect("the slow update started");
        handle
    }

    /// The reviewer's case, both ways round (this test and the next): a Repair saves the kept folder while the
    /// reconciler saves its failure record (§5.3 (6)'s fresh check), or the other way round. Each is one
    /// load-change-save under the config-write lock (`DesktopConfig::update_at`, 1882 r5), so neither overwrites the
    /// other with the copy it loaded first. Here the reconciler's update is between its load and its save while the
    /// folder is saved.
    #[test]
    fn a_kept_folder_saved_during_a_reconciler_config_update_survives_it() {
        let record = FailureRecord {
            reason: FinderFailureReason::Timeout,
            domain: "io.beebeeb.app".into(),
            code: app_code::OP_TIMEOUT,
            at: 1_791_291_909,
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let reconciler = {
            let (path, record) = (path.clone(), record.clone());
            in_flight(move |entered| {
                update_config_at(&path, move |cfg| {
                    let changed = set_failure(cfg, Some(record));
                    entered.send(()).expect("the test is waiting");
                    std::thread::sleep(Duration::from_millis(200));
                    changed
                })
            })
        };
        crate::finder_removal::remember_kept_folder_at(&path, KEPT_AT).expect("the folder is saved");
        reconciler.join().expect("the reconciler's update did not panic");
        let on_disk = DesktopConfig::load_from(&path).expect("reads back");
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT_AT),
            "the kept folder survived the reconciler's update"
        );
        assert_eq!(on_disk.finder_last_failure, Some(record), "and the update landed");
    }

    /// Minor 3: the two tests around this one drive `update_config_at`; this pins that the port's writes reach it, and
    /// that nothing in the ports saves a copy of the config it loaded earlier.
    #[test]
    fn the_ports_write_the_config_only_through_one_update_at() {
        let source = production();
        assert!(!source.contains(".save()"), "no save of a config loaded earlier");
        let update = squeeze(&source[source.find("fn update_config(").expect("update_config")..]);
        let update = &update[..update.find(&squeeze("fn update_config_at(")).expect("its seam follows")];
        assert!(
            update.contains(&squeeze("Ok(path) => update_config_at(&path, change),")),
            "{update}"
        );
        let at = squeeze(&source[source.find("fn update_config_at(").expect("the seam")..]);
        let at = &at[..at
            .find(&squeeze("impl Ports for MacosPorts"))
            .expect("the ports follow")];
        assert!(
            at.contains(&squeeze("DesktopConfig::update_at(path, |cfg| (change(cfg), ()))")),
            "one load-change-save under the lock:\n{at}"
        );
        for persist in ["fn persist_failure(", "fn persist_signed_out_by_choice("] {
            let body = &source[source.find(persist).expect(persist)..];
            let body = &body[..body.find("\n    }\n").expect("it ends")];
            assert!(body.contains("update_config(move |cfg|"), "{persist}:\n{body}");
        }
    }

    /// The other way round: the folder's save is between its load and its save while the reconciler updates the
    /// config.
    #[test]
    fn a_reconciler_config_update_during_a_kept_folder_save_keeps_the_folder() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let saving = {
            let path = path.clone();
            in_flight(move |entered| {
                DesktopConfig::update_at(&path, move |cfg| {
                    let changed = crate::finder_removal::record_kept_folder(cfg, KEPT_AT);
                    entered.send(()).expect("the test is waiting");
                    std::thread::sleep(Duration::from_millis(200));
                    (changed, ())
                })
            })
        };
        update_config_at(&path, |cfg| set_signed_out_by_choice(cfg, true));
        saving
            .join()
            .expect("the folder's save did not panic")
            .expect("the folder was saved");
        let on_disk = DesktopConfig::load_from(&path).expect("reads back");
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT_AT),
            "the kept folder survived the reconciler's update"
        );
        assert!(on_disk.finder_signed_out_by_choice, "and the update landed");
    }
