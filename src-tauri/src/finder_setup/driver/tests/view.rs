//! The view a person and TypeScript read, and the `Copy details` text: what each state
//! reports, the exact JSON shape, and the launch-location reasons.
use super::*;

    #[test]
    fn copy_details_names_what_support_needs_and_never_a_path() {
        let view = FinderSetupView {
            setup: FinderSetup::Failed,
            reason: Some(FinderFailureReason::FolderTaken),
            launch_location: LaunchLocation::Applications,
            attempt: 1,
            max_attempts: 1,
            last_failure: Some(FailureRecord {
                reason: FinderFailureReason::FolderTaken,
                domain: "NSCocoaErrorDomain".into(),
                code: 516,
                at: 1_791_291_909,
            }),
        };
        let tail = vec!["2026-10-06T13:05:09Z trigger keys_arrived".to_string()];
        let text = copy_details_text(&view, "0.8.12-alpha.1", "26.0", &tail);
        let header = &text[..text.find("Lifecycle log").expect("the log section")];
        assert_eq!(
            header,
            "Beebeeb 0.8.12-alpha.1 on macOS 26.0\nFinder setup: failed\nReason: folder_taken\nError: NSCocoaErrorDomain 516\nLast attempt: 2026-10-06T13:05:09Z\nAttempts: 1 of 1\nLaunched from: applications\n\n"
        );
        assert!(
            text.ends_with("Lifecycle log (last 1 lines):\n2026-10-06T13:05:09Z trigger keys_arrived\n"),
            "{text}"
        );
        assert!(!header.contains('/') && !header.contains('@'), "{header}");
    }

    // ---- Task 7 additions: lead rulings T3-1, T3-4, T14-c3, T1-4, T5-b, plus the driver's own seams ----

    #[test]
    fn the_view_is_exactly_the_json_the_typescript_parses() {
        // The brief's sample, as text: the contract with `src/finderSetup.ts` (`parseFinderSetupView`).
        let sample = r#"{"setup":"failed","reason":"folder_taken","launch_location":"applications","attempt":1,"max_attempts":1,"last_failure":{"reason":"folder_taken","domain":"NSCocoaErrorDomain","code":516,"at":1791291909}}"#;
        let json = serde_json::to_value(failed_view()).unwrap();
        assert_eq!(json, serde_json::from_str::<serde_json::Value>(sample).unwrap());

        let object = json.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "attempt",
                "last_failure",
                "launch_location",
                "max_attempts",
                "reason",
                "setup"
            ],
            "no extra and no missing key"
        );
        assert_eq!(object["setup"], "failed");
        assert_eq!(object["reason"], "folder_taken");
        assert_eq!(object["launch_location"], "applications");
        let failure = object["last_failure"].as_object().unwrap();
        let mut failure_keys: Vec<&str> = failure.keys().map(String::as_str).collect();
        failure_keys.sort_unstable();
        assert_eq!(failure_keys, ["at", "code", "domain", "reason"]);
        // Numbers are JSON numbers, never strings.
        for (name, value) in [
            ("attempt", &object["attempt"]),
            ("max_attempts", &object["max_attempts"]),
            ("code", &failure["code"]),
            ("at", &failure["at"]),
        ] {
            assert!(value.is_i64(), "{name} must be a JSON number: {value}");
        }

        // Absent values are present-and-null, the same JSON `finder_setup_state` returns at start.
        assert_eq!(
            serde_json::to_value(FinderSetupView::initial(LaunchLocation::Elsewhere)).unwrap(),
            serde_json::json!({
                "setup": "missing", "reason": null, "launch_location": "elsewhere",
                "attempt": 0, "max_attempts": 0, "last_failure": null
            })
        );

        // `launch_location` is exactly one of four strings, and every setup state keeps its wire name.
        for (location, wire) in [
            (LaunchLocation::Applications, "applications"),
            (LaunchLocation::Translocated, "translocated"),
            (LaunchLocation::DiskImage, "disk_image"),
            (LaunchLocation::Elsewhere, "elsewhere"),
        ] {
            assert_eq!(
                serde_json::to_value(FinderSetupView::initial(location)).unwrap()["launch_location"],
                wire
            );
        }
        for (setup, wire) in [
            (FinderSetup::Ready, "ready"),
            (FinderSetup::Missing, "missing"),
            (FinderSetup::Adding, "adding"),
            (FinderSetup::Failed, "failed"),
            (FinderSetup::UserDisabled, "user_disabled"),
        ] {
            let mut core = CoreState::new(LaunchLocation::Applications);
            core.setup = setup;
            assert_eq!(
                serde_json::to_value(FinderSetupView::of(&core, None)).unwrap()["setup"],
                wire
            );
        }
    }

    #[test]
    fn a_view_carries_a_reason_when_failed_or_user_disabled_or_missing_for_the_launch_location() {
        // Every state against every reason the core could be holding (lead ruling T3-4, corrected).
        for setup in [
            FinderSetup::Ready,
            FinderSetup::Missing,
            FinderSetup::Adding,
            FinderSetup::Failed,
            FinderSetup::UserDisabled,
        ] {
            for reason in FinderFailureReason::ALL {
                let mut core = CoreState::new(LaunchLocation::Applications);
                core.setup = setup;
                core.reason = Some(reason);
                let expected = match setup {
                    FinderSetup::Failed | FinderSetup::UserDisabled => Some(reason),
                    FinderSetup::Missing => (reason == FinderFailureReason::NotInApplications).then_some(reason),
                    FinderSetup::Ready | FinderSetup::Adding => None,
                };
                assert_eq!(
                    FinderSetupView::of(&core, None).reason,
                    expected,
                    "{setup:?} holding {reason:?}"
                );
            }
            let mut core = CoreState::new(LaunchLocation::Applications);
            core.setup = setup;
            core.reason = None;
            assert_eq!(
                FinderSetupView::of(&core, None).reason,
                None,
                "{setup:?} holding no reason"
            );
        }
    }

    #[test]
    fn a_disk_image_or_translocated_launch_is_missing_for_not_in_applications_before_any_check() {
        // Spec D7: "opened from the mounted dmg: `not_in_applications` reported by
        // `finder_setup_state` before sign-in". The core knows it at construction.
        for location in [LaunchLocation::DiskImage, LaunchLocation::Translocated] {
            let view = FinderSetupView::initial(location);
            assert_eq!(
                (view.setup, view.reason),
                (FinderSetup::Missing, Some(FinderFailureReason::NotInApplications)),
                "{location:?}"
            );
        }
        for location in [LaunchLocation::Applications, LaunchLocation::Elsewhere] {
            let view = FinderSetupView::initial(location);
            assert_eq!((view.setup, view.reason), (FinderSetup::Missing, None), "{location:?}");
        }
    }

    #[tokio::test]
    async fn a_dmg_launch_publishes_not_in_applications_at_start_with_no_trigger() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::DiskImage,
            None,
            RetryPolicy::default(),
        );
        assert_eq!(
            handle.view().reason,
            Some(FinderFailureReason::NotInApplications),
            "the first read, before the task runs"
        );
        tokio::spawn(task);
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        let record = record.lock().unwrap();
        let first = record.published.first().expect("published at start");
        assert_eq!(
            (first.setup, first.reason, first.launch_location),
            (
                FinderSetup::Missing,
                Some(FinderFailureReason::NotInApplications),
                LaunchLocation::DiskImage
            )
        );
        assert!(
            record.ops.is_empty(),
            "nothing ran: the notice needs no sign-in and no check"
        );
    }

    #[tokio::test]
    async fn a_silent_retry_never_publishes_its_reason() {
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        ports.add = Err(FpError::new(FILE_PROVIDER_DOMAIN, -2001, "not loaded"));
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Failed).await;
        let record = record.lock().unwrap();
        assert!(
            record
                .published
                .iter()
                .any(|v| v.setup == FinderSetup::Adding && v.attempt == 3),
            "the third attempt's retry was published as Adding: {:?}",
            record.published
        );
        for view in &record.published {
            if !matches!(view.setup, FinderSetup::Failed | FinderSetup::UserDisabled) {
                assert_eq!(
                    view.reason, None,
                    "a {:?} view must not carry a reason: {view:?}",
                    view.setup
                );
            }
        }
        assert_eq!(
            record.published.last().and_then(|v| v.reason),
            Some(FinderFailureReason::ExtensionLoading)
        );
    }

    #[test]
    fn copy_details_reads_the_last_200_lifecycle_lines() {
        let asked = std::cell::Cell::new(0);
        let text = copy_details_from(&failed_view(), "0.8.12", "26.0", |n| {
            asked.set(n);
            vec![
                "2026-10-06T13:05:09Z trigger launch".to_string(),
                "2026-10-06T13:05:10Z trigger try_again".to_string(),
            ]
        });
        assert_eq!(asked.get(), 200);
        assert!(
            text.ends_with("Lifecycle log (last 2 lines):\n2026-10-06T13:05:09Z trigger launch\n2026-10-06T13:05:10Z trigger try_again\n"),
            "{text}"
        );
    }
