    use super::desktop_capabilities::HostOs;
    use super::finder_setup_command_tests::{body_between, production_source};

    #[test]
    fn a_mac_refuses_an_in_place_install_with_a_typed_error_and_nothing_else_does() {
        let refused = super::in_app_install_refusal(HostOs::Macos).expect_err("a Mac cannot install in place");
        assert!(refused.starts_with("update_install_unsupported:"), "the typed code leads: {refused}");
        assert!(refused.contains("downloads updates"), "{refused}");
        for host in [HostOs::Windows, HostOs::Linux, HostOs::Unknown] {
            assert_eq!(super::in_app_install_refusal(host), Ok(()), "{host:?} keeps its installer");
        }
    }

    #[test]
    fn both_install_commands_refuse_before_they_touch_the_updater() {
        let production = production_source();
        for signature in ["async fn install_update(", "async fn install_channel_downgrade("] {
            let body = body_between(&production, signature, "\n}\n");
            let refusal = body
                .find("in_app_install_refusal(desktop_capabilities::host_os())?")
                .unwrap_or_else(|| panic!("{signature} refuses on a Mac"));
            let updater = body.find("updater_for_release_channel(").unwrap_or_else(|| panic!("{signature} builds the updater"));
            assert!(refusal < updater, "{signature}: the refusal comes first");
        }
    }

    #[test]
    fn only_those_two_commands_install_an_update() {
        let production = production_source();
        assert_eq!(
            production.matches(".download_and_install(").count(),
            2,
            "a third call site is an install that the refusal does not cover: decide it here, on purpose"
        );
        // The background loop only tells the window an update exists; it never downloads one.
        let loop_body = body_between(&production, "async fn check_for_update(", "\n}\n");
        assert!(loop_body.contains("\"update-available\""));
        assert!(!loop_body.contains("download"), "the update loop announces, it does not install");
    }
