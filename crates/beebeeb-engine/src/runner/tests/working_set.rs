//! working_set tests split out of runner.rs.

use super::*;

// Task 1697 review fix (T3): the per-tick signal merge decision. RED-first:
// written against the pass-through stub and seen failing (it returned an
// empty batch for non-empty inputs) before the merge landed.
#[test]
fn tick1697_working_set_signal_merges_local_and_remote_into_one_signal() {
    // Local only: unchanged reason, ids pass through.
    let (reason, ids) = tick_working_set_signal(&["f-1".to_string()], &[]);
    assert_eq!(reason, "operations_applied");
    assert_eq!(ids, vec!["f-1".to_string()]);
    // Remote only: the ingestion side is named.
    let (reason, ids) = tick_working_set_signal(&[], &["r-1".to_string()]);
    assert_eq!(reason, "remote_changes_applied");
    assert_eq!(ids, vec!["r-1".to_string()]);
    // Mixed tick: ONE signal with the UNION, never two signals.
    let (reason, ids) = tick_working_set_signal(
        &["f-1".to_string(), "f-2".to_string()],
        &["r-1".to_string(), "f-2".to_string()],
    );
    assert_eq!(reason, "operations_applied+remote_changes_applied");
    assert_eq!(
        ids,
        vec!["f-1".to_string(), "f-2".to_string(), "r-1".to_string()],
        "the union dedupes while preserving first-seen order"
    );
    // Nothing changed: same shape as before the fix (empty batch; the macOS
    // signal path no-ops on it but still runs its log sweep).
    let (reason, ids) = tick_working_set_signal(&[], &[]);
    assert_eq!(reason, "operations_applied");
    assert!(ids.is_empty());
}

// Task 9 fix round 1 (item B): a working-set signal that meets a busy bridge gate is owed to the next
// tick. The signal is the daemon's ONLY replica-refresh channel, so a dropped one would leave Finder
// stale until some later tick happened to change a materialized parent.
/// Task 2190 (the lead's condition): a decision's working-set signal that was not delivered is
/// carried by the next tick's signal, through the same debt: taken once, kept while the gate is
/// busy, paid when delivered. Both tick-side signal sites take it first.
#[test]
fn task_2190_a_commands_undelivered_signal_is_carried_by_the_next_tick() {
    owe_working_set_signal(&["parked".to_string()]);
    owe_working_set_signal(&["parked".to_string(), "copy".to_string()]);
    let mut owed = OwedWorkingSetSignal::default();
    owed.take_command_debt();
    assert_eq!(
        owed.merged_with(&[]),
        vec!["parked".to_string(), "copy".to_string()],
        "the next tick signals them"
    );
    owed.take_command_debt();
    let merged = owed.merged_with(&["tick".to_string()]);
    assert_eq!(
        merged,
        vec!["parked".to_string(), "copy".to_string(), "tick".to_string()],
        "taken once"
    );
    owed.record(true, merged);
    assert!(owed.is_owing(), "a busy gate keeps them owed");
    let merged = owed.merged_with(&[]);
    owed.record(false, merged);
    assert!(!owed.is_owing(), "delivered: paid");

    // A Windows checkout is CRLF (core.autocrlf), as the other source-text pins here allow for.
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let body = |name: &str| {
        let from = source.find(&format!("fn {name}(")).expect(name);
        &source[from..from + source[from..].find("\n}\n").expect("the end of the fn")]
    };
    assert!(body("signal_working_set_owing").contains("owed.take_command_debt();"));
    assert!(body("signal_owed_debt_after_a_failed_tick").contains("owed_working_set_signal.take_command_debt();"));
    // The decision owes what its own signal did not deliver.
    let lib = crate::root_source::lib_rs().replace("\r\n", "\n");
    let decide = &lib[lib
        .find("async fn resolve_parked_conflict(")
        .expect("the decision command")..];
    assert!(decide.contains(
            "if crate::finder_setup::macos_ports::signal_working_set().is_err() {\n                crate::runner::owe_working_set_signal(&ids);"
        ));
}

#[test]
fn a_dropped_working_set_signal_is_owed_to_the_next_tick_and_a_delivered_one_clears_the_debt() {
    let mut owed = OwedWorkingSetSignal::default();
    assert_eq!(
        owed.merged_with(&[]),
        Vec::<String>::new(),
        "nothing owed, nothing to signal"
    );
    // The first tick's signal was dropped on a busy gate.
    owed.record(true, vec!["a".to_string(), "b".to_string()]);
    // A quiet next tick still signals: the owed ids make the batch non-empty, which is what keeps
    // `should_signal_working_set` from skipping it.
    assert_eq!(owed.merged_with(&[]), vec!["a".to_string(), "b".to_string()]);
    // A busy next tick adds its own ids to the debt, first-seen order, no duplicates.
    let merged = owed.merged_with(&["b".to_string(), "c".to_string()]);
    assert_eq!(merged, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    owed.record(true, merged);
    // Delivered: the debt is paid.
    let merged = owed.merged_with(&["d".to_string()]);
    assert_eq!(
        merged,
        vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()]
    );
    owed.record(false, merged);
    assert_eq!(owed.merged_with(&[]), Vec::<String>::new());
}

/// A gate that stays busy for hours must not make the debt grow without bound: past the cap the debt
/// is one id no row has, which `working_set_signal_needed` treats as an unknown parent and signals.
#[test]
fn the_owed_working_set_signal_is_bounded() {
    let mut owed = OwedWorkingSetSignal::default();
    let many: Vec<String> = (0..OWED_SIGNAL_MAX_IDS + 1).map(|n| format!("f-{n}")).collect();
    owed.record(true, many);
    assert_eq!(
        owed.merged_with(&["x".to_string()]),
        vec![OWED_SIGNAL_UNKNOWN_ID.to_string(), "x".to_string()]
    );
    assert!(
        working_set_signal_needed(&[None], &["folder-1".to_string()]),
        "an unknown parent always signals"
    );
}

/// The rows the startup reconcile reset are owed to the first tick: its signal carries them before
/// that tick's own ids, and once a signal went out the debt is paid. Nothing reset owes nothing.
#[test]
fn the_startup_resets_are_owed_to_the_first_ticks_signal() {
    let mut owed = OwedWorkingSetSignal::owing(vec!["reset-1".to_string(), "reset-2".to_string()]);
    let first = owed.merged_with(&["tick-1".to_string()]);
    assert_eq!(
        first,
        vec!["reset-1".to_string(), "reset-2".to_string(), "tick-1".to_string()]
    );
    owed.record(false, first);
    assert_eq!(owed.merged_with(&[]), Vec::<String>::new(), "paid by the first signal");
    assert_eq!(
        OwedWorkingSetSignal::owing(Vec::new()).merged_with(&[]),
        Vec::<String>::new()
    );
}

/// `run` opens the tick loop's debt with the ids the startup reconcile returned, so a reset reaches
/// the system at the first tick, not at the next unrelated change. `run` needs a real window
/// system, so the wiring is asserted on the source, on every platform.
#[test]
fn the_tick_loop_opens_its_debt_with_the_startup_resets() {
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("`run` holds {needle}"));
    assert_eq!(body.matches("OwedWorkingSetSignal::").count(), 1, "one debt");
    assert!(
        at("let reset_item_ids = match db.reconcile_stale_in_flight_on_startup()")
            < at("let mut owed_working_set_signal = OwedWorkingSetSignal::owing(reset_item_ids);"),
        "the debt opens with the reconcile's ids"
    );
}

/// A failed tick pays a pending debt with exactly one signal, and sends none when nothing is owed. The
/// shared gate is held, so the signal meets a busy gate and no real bridge call is made; the count is of
/// the times the bridge was asked.
#[cfg(target_os = "macos")]
#[test]
fn a_failed_tick_pays_a_pending_debt_with_one_signal() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let mut owed = OwedWorkingSetSignal::owing(vec!["reset-1".to_string()]);
    let before = SIGNALS_ASKED.with(|asked| asked.get());
    crate::file_provider_port::with_busy_gate(|| {
        signal_owed_debt_after_a_failed_tick(&mut owed, &db);
    });
    assert_eq!(SIGNALS_ASKED.with(|asked| asked.get()) - before, 1, "one signal");
    assert_eq!(
        owed.merged_with(&[]),
        vec!["reset-1".to_string()],
        "the busy gate dropped it, so it is still owed"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_failed_tick_with_nothing_owed_sends_no_signal() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let mut owed = OwedWorkingSetSignal::default();
    let before = SIGNALS_ASKED.with(|asked| asked.get());
    crate::file_provider_port::with_busy_gate(|| {
        signal_owed_debt_after_a_failed_tick(&mut owed, &db);
    });
    assert_eq!(SIGNALS_ASKED.with(|asked| asked.get()) - before, 0, "no signal");
    assert_eq!(owed.merged_with(&[]), Vec::<String>::new());
}

/// The reconciler's gate is the one `signal_file_provider_working_set` takes: with it held, the signal
/// is DROPPED, and says so (`true`), and the next tick's call carries the ids. No real bridge call is
/// made: a busy gate answers at once, and the test only ever runs calls that hit it busy.
#[cfg(target_os = "macos")]
#[test]
fn a_working_set_signal_that_meets_a_busy_gate_reports_the_drop_and_the_debt_carries_over() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let ids = vec!["f-1".to_string()];
    let mut owed = OwedWorkingSetSignal::default();
    crate::file_provider_port::with_busy_gate(|| {
        assert!(
            signal_file_provider_working_set(&db, "operations_applied", &ids),
            "a busy gate drops the signal, and says so"
        );
        // Nothing to signal is not a drop.
        assert!(!signal_file_provider_working_set(&db, "operations_applied", &[]));
        // The wrapper the tick loop uses: the drop becomes a debt, and the next tick adds to it.
        signal_working_set_owing(&mut owed, &db, "operations_applied", &ids);
        assert_eq!(owed.merged_with(&[]), ids);
        signal_working_set_owing(&mut owed, &db, "remote_changes_applied", &["r-1".to_string()]);
        assert_eq!(
            owed.merged_with(&[]),
            vec!["f-1".to_string(), "r-1".to_string()],
            "still owed, nothing lost"
        );
    });
}

/// The tick loop signals through the owing wrapper only, at four named sites: revoked shared content,
/// the queue step's one signal, a queue step that failed (the tick's remote applies), and a failed tick that
/// owes a debt. A fifth call fails here, so no signal bypasses the debt. The queue step's two sites are in
/// `queue_step`, which has two callers: the tick, and the send phase's wake while a sign-out's phase is armed
/// (spec 2026-10-10 §6.2), which passes no remote applies. A third caller fails here too. A failed tick reaches
/// the first site (it runs before the tick, in every iteration) and the fourth; a delivered signal pays the debt,
/// so at most one signal is delivered per tick. Between ticks, an armed phase's queue step signals as well: at
/// most once a second, and only while the phase is armed, at most a minute.
#[test]
fn every_working_set_signal_in_the_tick_loop_goes_through_the_owed_wrapper() {
    use crate::source_pin::squeeze;
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let production = &source[..source.find("#[cfg(test)]\nmod tests {").expect("the tests follow")];
    assert_eq!(
        production.matches("signal_file_provider_working_set(&db").count(),
        0,
        "the tick loop calls the signal directly"
    );
    assert_eq!(
        production
            .matches("signal_file_provider_working_set(db, reason, &merged)")
            .count(),
        1,
        "only the owing wrapper calls the signal"
    );
    let squeezed = squeeze(production);
    assert_eq!(
        squeezed.matches("signal_working_set_owing(").count() - squeezed.matches("fnsignal_working_set_owing(").count(),
        4,
        "four call sites"
    );
    for site in [
        "signal_working_set_owing(&mut owed_working_set_signal, &db, \"shared_roots_changed\", &outcome.removed_shared_file_ids)",
        "signal_working_set_owing(owed_working_set_signal, db, reason, &ids)",
        "signal_working_set_owing(owed_working_set_signal, db, \"remote_changes_applied\", applied)",
        "signal_working_set_owing(owed_working_set_signal, db, \"owed_debt\", &[])",
    ] {
        assert_eq!(squeezed.matches(&squeeze(site)).count(), 1, "the site {site}");
    }
    // The queue step's two sites are `queue_step`'s; the tick, the phase's wake and (task 2409) a Try again's nudge
    // call it, once each. The two wakes have no remote applies to merge.
    assert_eq!(
        squeezed.matches("queue_step(").count() - squeezed.matches("fnqueue_step(").count(),
        3,
        "the queue step's three callers"
    );
    for (call, count) in [
        (
            "queue_step(&bridge, &sync_root, &db, &mut owed_working_set_signal, &tick.applied_item_ids, &send_phase)",
            1,
        ),
        (
            "queue_step(&bridge, &sync_root, &db, &mut owed_working_set_signal, &[], &send_phase)",
            2,
        ),
    ] {
        assert_eq!(squeezed.matches(&squeeze(call)).count(), count, "the call {call}");
    }
    // The fourth site's helper is called once, from the failed tick's arm of `run`.
    assert_eq!(
        squeezed.matches("signal_owed_debt_after_a_failed_tick(").count(),
        2,
        "its definition and one call"
    );
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("`run` holds {needle}"));
    let call = at("signal_owed_debt_after_a_failed_tick(&mut owed_working_set_signal, &db);");
    assert!(
        at("\"sync tick failed\"") < call && call < at("refresh_status_ui_snapshot(&db, RunMode::Running, true)"),
        "in the failed tick's arm"
    );
}

/// U35: the phase's wake runs the tick's own queue step (`queue_step`) with no remote applies, at most once a
/// second, never while paused or disarmed, and never the remote tick, a sweep or the shared-root refresh. The queue
/// step notes what each attempt met while a phase is armed, so the tick's attempts count too, and it pays the
/// working-set signal, so the phase adds no signal site.
#[test]
fn the_send_phase_runs_the_queue_step_alone() {
    use crate::source_pin::squeeze;
    let start = std::time::Instant::now();
    let mut last = None;
    let mut runs = 0;
    for tenth in 0..=600u64 {
        let now = start + std::time::Duration::from_millis(tenth * 100);
        if phase_step_due(true, RunMode::Running, last, now) {
            runs += 1;
            last = Some(now);
        }
    }
    assert!((60..=61).contains(&runs), "{runs} runs in 60 s");
    assert!(!phase_step_due(true, RunMode::Paused, None, start), "not while paused");
    assert!(
        !phase_step_due(false, RunMode::Running, None, start),
        "never once disarmed"
    );
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let arm = &source[source.find("Wake::Phase => {").expect("the phase's arm")..];
    let arm = &arm[..arm.find("\n            }\n").unwrap()];
    assert!(
        squeeze(arm).contains(&squeeze(
            "queue_step(&bridge, &sync_root, &db, &mut owed_working_set_signal, &[], &send_phase)"
        )),
        "the arm runs the tick's queue step, with no remote applies:\n{arm}"
    );
    for denied in [
        "process_due_operations(",
        "signal_working_set_owing(",
        "sync_tick_outcome(",
        "refresh_shared_roots(",
        "sweep_",
        "backup_scheduler.tick(",
    ] {
        assert!(!arm.contains(denied), "{denied} in the arm");
    }
    let step = &source[source.find("async fn queue_step(").expect("the queue step")..];
    let step = &step[..step.find("\n}\n").unwrap()];
    assert!(
        squeeze(step).contains("ifsend_phase.is_armed(){send_phase.note(&outcome);}"),
        "every attempt is noted while a phase is armed:\n{step}"
    );
    let tick =
        &source[source.find("Wake::Tick => {").expect("the tick's arm")..source.find("Wake::Phase => {").unwrap()];
    assert!(
        squeeze(tick).contains(&squeeze(
            "queue_step(&bridge, &sync_root, &db, &mut owed_working_set_signal, &tick.applied_item_ids, &send_phase)"
        )),
        "the tick's queue step is the same one, so the tick's attempts count too"
    );
    // No precondition on the wake. `select!` reads one when it starts waiting, so an arm disabled then stays
    // disabled until another arm fires: up to a 30 s tick.
    let wait = &source[source.find("async fn next_wake(").expect("next_wake")..];
    let wait = &wait[..wait.find("\n}\n").unwrap()];
    assert!(
        wait.contains("_ = send_phase.wake.notified() => Wake::Phase,"),
        "{wait}"
    );
    assert!(
        !wait.contains(", if "),
        "no arm of the wait has a precondition:\n{wait}"
    );
    // §6.2: the arm skips its step while sync is paused, as the tick does: it reads the runner's own pause flag.
    assert!(
            squeeze(arm).contains(&squeeze(
                "phase_step_due(send_phase.is_armed(), RunMode::of_flag(sync_paused.load(Ordering::Relaxed)), last_phase_run, now)"
            )),
            "the arm reads the pause flag the tick reads:\n{arm}"
        );
    // §6.1 condition 3: the phase reads the link monitor that this runner's requests write.
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    assert_eq!(
        body.matches("send_phase.attach(link.clone());").count(),
        1,
        "run attaches its link monitor to the phase's link"
    );
}

/// U35: a phase armed while the runner already waits between ticks wakes it at once, not at its next tick 30 s
/// on. On tokio's paused clock: time moves only when every task waits, and then straight to the next timer, so no
/// real time passes and a loaded machine cannot flake the bound. With a precondition on the wake, the only timers
/// left are the 1 s timeout and the 30 s tick, so the clock moves to the timeout and the test is red at once.
#[tokio::test(start_paused = true)]
async fn a_phase_armed_while_the_runner_waits_wakes_it_at_once() {
    let send_phase = Arc::new(SendPhaseLink::default());
    let (_keep_open, mut cancel) = oneshot::channel::<()>();
    let mut tick = tokio::time::interval(TICK_INTERVAL);
    assert_eq!(
        next_wake(&mut cancel, &mut tick, &send_phase).await,
        Wake::Tick,
        "an interval's first tick is at once"
    );
    let waiter = {
        let send_phase = send_phase.clone();
        tokio::spawn(async move { next_wake(&mut cancel, &mut tick, &send_phase).await })
    };
    // Paused clock: the waiter runs first and starts waiting, then the clock moves 50 ms. It now waits, 29.95 s
    // from its next tick.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let armed_at = tokio::time::Instant::now();
    send_phase.arm();
    let woke = tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("woken within a second of the arm, not at the next tick");
    assert_eq!(woke.unwrap(), Wake::Phase);
    assert!(armed_at.elapsed() < Duration::from_secs(1));
}

/// U46's chain, link by link: what an attempt met reaches the phase's signals, signals add up within a phase, and
/// arming a new phase forgets the last one's.
#[test]
fn an_attempts_refusal_reaches_the_phase_signals() {
    use crate::engine_bridge::TransferLoopOutcome;
    let link = SendPhaseLink::default();
    link.arm();
    assert_eq!(link.signals(), AttemptSignals::default());
    link.note(&TransferLoopOutcome {
        account_refused: true,
        ..Default::default()
    });
    assert_eq!(
        link.signals(),
        AttemptSignals {
            account_refused: true,
            ..Default::default()
        }
    );
    link.note(&TransferLoopOutcome {
        link_failed: true,
        ..Default::default()
    });
    assert_eq!(
        link.signals(),
        AttemptSignals {
            account_refused: true,
            link_failed: true,
            session_refused: false
        },
        "they add up"
    );
    link.note(&TransferLoopOutcome {
        session_refused: true,
        ..Default::default()
    });
    assert!(link.signals().session_refused);
    link.arm();
    assert_eq!(
        link.signals(),
        AttemptSignals::default(),
        "a new phase starts with none"
    );
}

// Task 1697: the retired `file-provider-invalidate` Tauri event and its
// payload builder are gone — the only replica-refresh channel is the
// working-set signal (see `signal_file_provider_working_set`). The pure
// decision core for that signal is tested in `file_provider_port`.

// macOS-only like the signal path it mirrors (the runtime call sites above are gated with
// `signal_file_provider_working_set`).
#[test]
#[cfg(target_os = "macos")]
fn signal_file_provider_working_set_decision_gates_on_changed_items() {
    // Mirrors file_provider_port::should_signal_working_set so a runner
    // change that bypasses the gate fails here too.
    assert!(crate::file_provider_port::should_signal_working_set(&[
        "f-1".to_string()
    ]));
    assert!(!crate::file_provider_port::should_signal_working_set(&[]));
}

// Task 1697 audit P0 item 4: working-set signals filtered to changes
// whose old-or-new parent is materialized. RED-first: written against the
// unmodified working-set signal path and seen failing (the pure fn did
// not exist) before it landed.

#[test]
fn working_set_signal_filter_skips_batches_with_no_materialized_parent() {
    assert!(
        !working_set_signal_needed(
            &[Some("folder-9".into()), Some("folder-8".into())],
            &["folder-1".into()]
        ),
        "no changed parent is materialized: no signal"
    );
    assert!(
        working_set_signal_needed(&[Some("folder-1".into())], &["folder-1".into()]),
        "a materialized parent changed: signal"
    );
    assert!(
        working_set_signal_needed(
            &[Some("folder-9".into()), Some("folder-1".into())],
            &["folder-1".into()]
        ),
        "ANY changed parent materialized: signal"
    );
}

#[test]
fn working_set_signal_filter_fails_open_when_the_set_is_unknown() {
    assert!(
        working_set_signal_needed(&[Some("folder-9".into())], &[]),
        "an untracked materialized set = the whole dataset (fail open)"
    );
    assert!(!working_set_signal_needed(&[], &[]), "an empty batch never signals");
}

#[test]
fn working_set_signal_filter_always_signals_root_and_unknown_parents() {
    let materialized = vec!["folder-1".to_string()];
    assert!(
        working_set_signal_needed(
            &[Some("NSFileProviderRootContainerItemIdentifier".into())],
            &materialized
        ),
        "the single root container is always materialized"
    );
    assert!(working_set_signal_needed(&[Some("__fp_root__".into())], &materialized));
    assert!(
        working_set_signal_needed(&[None], &materialized),
        "unknown parent (a deletion whose row is gone) fails open"
    );
    assert!(
        !working_set_signal_needed(&[Some("namespace:my_files".into())], &materialized),
        "1701: synthetic namespace ids are no longer a Finder surface — they are not materialized parents"
    );
}

/// Task 2409: Try again's nudge wakes a runner that waits between ticks at once, as an arm does (on tokio's paused
/// clock, so a loaded machine cannot flake the bound).
#[tokio::test(start_paused = true)]
async fn a_nudge_while_the_runner_waits_wakes_it_at_once() {
    let send_phase = Arc::new(SendPhaseLink::default());
    let (_keep_open, mut cancel) = oneshot::channel::<()>();
    let mut tick = tokio::time::interval(TICK_INTERVAL);
    assert_eq!(next_wake(&mut cancel, &mut tick, &send_phase).await, Wake::Tick);
    let waiter = {
        let send_phase = send_phase.clone();
        tokio::spawn(async move { next_wake(&mut cancel, &mut tick, &send_phase).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    send_phase.nudge();
    let woke = tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("woken within a second of the nudge, not at the next tick");
    assert_eq!(woke.unwrap(), Wake::Nudge);
}

/// Task 2409 (self-review): a nudge within the second after the last nudge step waits out the rest of that second;
/// it is never dropped (a second Try again would otherwise sit until the next tick). Outside it, the step runs now.
#[test]
fn a_nudge_inside_the_rate_limit_waits_out_the_second() {
    let t0 = std::time::Instant::now();
    assert_eq!(nudge_wait(None, t0), None, "the first nudge runs at once");
    assert_eq!(
        nudge_wait(Some(t0), t0 + Duration::from_millis(300)),
        Some(Duration::from_millis(700))
    );
    assert_eq!(nudge_wait(Some(t0), t0 + Duration::from_secs(1)), None);
    assert_eq!(nudge_wait(Some(t0), t0 + Duration::from_secs(5)), None);
}

/// Task 2409: the nudge's arm runs the queue step alone (no pull, no sweeps), at most once a second, never while
/// paused, and never before this run's first pass (D11b), as the phase's arm does. A nudge inside the second is
/// deferred (asked again once the second is out), never dropped.
#[test]
fn the_nudge_runs_the_queue_step_alone_and_waits_for_the_first_pass() {
    use crate::source_pin::squeeze;
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let arm = &source[source.find("Wake::Nudge => {").expect("the nudge's arm")..];
    let arm = squeeze(&arm[..arm.find("\n            }\n").unwrap()]);
    let deferred = arm
            .find(&squeeze(
                "if !mode.is_paused() && let Some(wait) = nudge_wait(last_nudge_run, now) { let link = Arc::clone(&send_phase); tokio::spawn(async move { tokio::time::sleep(wait).await; link.nudge(); }); continue; }",
            ))
            .unwrap_or_else(|| panic!("a nudge inside the second is asked again, not dropped:\n{arm}"));
    let due = arm
        .find(&squeeze("phase_step_due(true, mode, last_nudge_run, now)"))
        .unwrap_or_else(|| panic!("the arm reads the pause flag and its own last run:\n{arm}"));
    assert!(deferred < due, "{arm}");
    let first_pass = arm
        .find(&squeeze(
            "#[cfg(not(target_os = \"macos\"))] if !reconciled_this_run { continue; }",
        ))
        .unwrap_or_else(|| panic!("the arm waits for the first pass:\n{arm}"));
    let step = arm
        .find(&squeeze(
            "queue_step(&bridge, &sync_root, &db, &mut owed_working_set_signal, &[], &send_phase)",
        ))
        .unwrap_or_else(|| panic!("the arm runs the queue step:\n{arm}"));
    assert!(due < step && first_pass < step, "{arm}");
    for denied in [
        "sync_tick_outcome(",
        "refresh_shared_roots(",
        "sweep_",
        "backup_scheduler.tick(",
    ] {
        assert!(!arm.contains(denied), "{denied} in the arm");
    }
    let wait = &source[source.find("async fn next_wake(").expect("next_wake")..];
    let wait = &wait[..wait.find("\n}\n").unwrap()];
    assert!(
        wait.contains("_ = send_phase.nudge.notified() => Wake::Nudge,"),
        "{wait}"
    );
}
