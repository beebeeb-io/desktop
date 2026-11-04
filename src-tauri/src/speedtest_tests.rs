    use super::{speedtest_outcome, SpeedtestSamples, SpeedtestSlot, SPEEDTEST_ALREADY_RUNNING, SPEEDTEST_NO_DOWNLOAD, SPEEDTEST_NO_PING, SPEEDTEST_NO_UPLOAD};
    use std::sync::atomic::AtomicBool;

    fn finished() -> SpeedtestSamples {
        SpeedtestSamples { ping_ms: vec![20, 30, 40], up_bps: vec![4_000_000.0, 5_000_000.0], down_bps: vec![10_000_000.0, 12_000_000.0, 11_000_000.0] }
    }

    #[test]
    fn a_run_that_finished_reports_the_means_and_the_latency_range() {
        let result = speedtest_outcome(&finished()).expect("a run with figures in all three parts is a result");
        assert_eq!((result.latency.min_ms, result.latency.avg_ms, result.latency.max_ms), (20, 30, 40));
        assert_eq!(result.upload_bps, 4_500_000);
        assert_eq!(result.download_bps, 11_000_000);
    }

    #[test]
    fn a_run_in_which_every_ping_failed_is_a_failure_not_a_latency_of_zero() {
        let samples = SpeedtestSamples { ping_ms: vec![], ..finished() };
        assert_eq!(speedtest_outcome(&samples).unwrap_err(), SPEEDTEST_NO_PING);
    }

    #[test]
    fn a_direction_that_never_finished_is_a_failure_not_zero_megabits() {
        let no_up = SpeedtestSamples { up_bps: vec![], ..finished() };
        assert_eq!(speedtest_outcome(&no_up).unwrap_err(), SPEEDTEST_NO_UPLOAD);
        let no_down = SpeedtestSamples { down_bps: vec![], ..finished() };
        assert_eq!(speedtest_outcome(&no_down).unwrap_err(), SPEEDTEST_NO_DOWNLOAD);
        assert_eq!(speedtest_outcome(&SpeedtestSamples::default()).unwrap_err(), SPEEDTEST_NO_PING, "nothing at all names the first part that is missing");
    }

    #[test]
    fn some_failed_transfers_do_not_fail_the_run_and_do_not_count_as_zero() {
        // 1 of 3 uploads finished: its speed is the speed. A failed one is not a 0 in the mean.
        let samples = SpeedtestSamples { up_bps: vec![6_000_000.0], ..finished() };
        assert_eq!(speedtest_outcome(&samples).unwrap().upload_bps, 6_000_000);
    }

    #[test]
    fn the_messages_are_plain_sentences_a_person_can_act_on() {
        for message in [SPEEDTEST_NO_PING, SPEEDTEST_NO_UPLOAD, SPEEDTEST_NO_DOWNLOAD] {
            assert!(message.ends_with("Check your internet connection and try again."), "{message}");
        }
    }

    #[test]
    fn a_second_test_is_refused_while_one_runs_and_the_slot_frees_when_it_ends() {
        let flag = AtomicBool::new(false);
        let first = SpeedtestSlot::take(&flag).expect("the first test starts");
        assert_eq!(SpeedtestSlot::take(&flag).err().as_deref(), Some(SPEEDTEST_ALREADY_RUNNING));
        drop(first);
        assert!(SpeedtestSlot::take(&flag).is_ok(), "a finished (or cancelled) test frees the slot");
    }

    #[test]
    fn the_command_takes_the_slot_before_it_sends_anything() {
        let production = super::finder_setup_command_tests::production_source();
        let body = super::finder_setup_command_tests::body_between(&production, "async fn run_speedtest(", "\n}\n");
        let slot = body.find("SpeedtestSlot::take(&SPEEDTEST_RUNNING)").expect("run_speedtest takes the slot");
        let first_request = body.find("api.ping_health()").expect("run_speedtest pings");
        assert!(slot < first_request, "the slot comes before the first request");
        assert!(body.contains("if api.ping_health().await.is_ok() {"), "only a ping the server answered is a round trip");
        assert!(body.contains("speedtest_outcome(&samples)"), "the result goes through speedtest_outcome");
    }
