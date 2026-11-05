    use super::finder_setup_command_tests::{body_between, production_source};
    use super::{FINDER_IPC_NOT_READY, finder_install_error_is_timeout};

    #[test]
    fn the_sentence_a_person_reads_is_a_timeout_and_so_is_every_wording_that_was_saved_before() {
        assert!(finder_install_error_is_timeout(FINDER_IPC_NOT_READY));
        assert!(finder_install_error_is_timeout("Timed out waiting for the local Beebeeb sync daemon before installing Finder location."));
        assert!(finder_install_error_is_timeout("Timed out waiting for the Beebeeb File Provider domain"));
        assert!(finder_install_error_is_timeout("request timeout"));
    }

    #[test]
    fn nothing_else_is_a_timeout() {
        for error in [
            "Entitlement com.apple.security.application-groups is ignored",
            "NSFileProviderErrorDomain Code=-2011",
            "File Provider is only available on macOS.",
            "Beebeeb wasn’t ready to sync.",
            "",
        ] {
            assert!(!finder_install_error_is_timeout(error), "{error:?}");
        }
    }

    #[test]
    fn the_wait_fails_with_the_constant_and_the_classifier_asks_the_predicate() {
        let production = production_source();
        let wait = body_between(&production, "async fn wait_for_file_provider_ipc_ready(", "\n}\n");
        assert!(wait.contains("return Err(FINDER_IPC_NOT_READY.to_string());"), "the wait fails with the shared sentence");
        assert!(!wait.contains("wasn’t ready in time"), "no second copy of the sentence to drift from the classifier");
        let classifier = body_between(&production, "fn classify_finder_install_error(error: &str) -> String {", "\n}\n");
        assert!(classifier.contains("finder_install_error_is_timeout(error)"), "the classifier asks the predicate");
    }
