//! The one bridge gate: one OS call at a time, cut off by `within`, with the slot freed only
//! when the OS call returns; the op limits that run over it, and the open that takes the gate.
use super::*;

    /// Lead ruling T7-1, shape corrected in fix round 1: a bridge call is cut off by `within`; an
    /// engine start or stop is NOT, because a future dropped after it spawned or took the engine
    /// loses it (the engine would keep running untracked, holding the session's keys). Those two
    /// limit only their wait for the engine slot, inside `start_check_engine` / `stop_check_engine`.
    #[test]
    fn every_operation_is_bounded_in_the_shape_that_cannot_orphan_an_engine() {
        let source = production();
        let run = &source[source.find("fn run(&mut self, op: Op)").expect("run")..];
        let run = &run[..run.find("fn publish(").expect("publish follows run")];
        assert!(
            run.contains("let limit = op_limit(op);"),
            "one limit per operation, from the driver's table"
        );
        for pattern in [
            "Op::Observe =>",
            "Op::AddDomain =>",
            "Op::ReadDomain =>",
            "Op::WaitStable(timeout) =>",
            "Op::RemoveDomain =>",
        ] {
            let body = arm(run, pattern);
            // Read through `squeeze`: rustfmt wraps `within(op, limit, ..)` over lines.
            assert!(
                squeeze(body).contains(&squeeze("within(op, limit,")),
                "{pattern} is not bounded:\n{body}"
            );
            assert!(
                body.contains("gate.run(") || body.contains("gate.run_waiting("),
                "{pattern} does not go through the bridge gate:\n{body}"
            );
        }
        // The engine slot is the only await of `ensure_sync_root_and_engine` (pinned in lib.rs), so
        // cutting the whole of FinishReady off can only cut off that wait.
        assert!(squeeze(arm(run, "Op::FinishReady =>")).contains(&squeeze("within(op, limit,")));
        for (pattern, call) in [
            ("Op::StartEngine =>", "start_check_engine("),
            ("Op::StopEngine =>", "stop_check_engine("),
        ] {
            let body = arm(run, pattern);
            assert!(
                body.contains(call) && body.contains("limit"),
                "{pattern} must pass its limit to {call}:\n{body}"
            );
            assert!(
                !body.contains("within("),
                "{pattern} must not be wrapped in a cut-off:\n{body}"
            );
        }
        // And no bridge call is made outside the gate and `within`.
        assert!(
            run.matches("gate.run(").count() + run.matches("gate.run_waiting(").count() >= 5,
            "the bridge calls are in `run`"
        );
        for call in run
            .match_indices("gate.run(")
            .chain(run.match_indices("gate.run_waiting("))
        {
            // The call is the third argument of the nearest `within(` before it, however rustfmt wraps that call.
            let before = &run[..call.0];
            let opened = before.rfind("within(").unwrap_or(0);
            assert_eq!(
                squeeze(&before[opened..]),
                squeeze("within(op, limit,"),
                "an unbounded bridge call, after: {}",
                &before[opened..]
            );
        }
        assert_eq!(
            source.matches("spawn_blocking").count(),
            1,
            "the one blocking spawn is the gate's"
        );
    }

    /// Fix round 1 (finding 2): the gate's permit is taken BEFORE the blocking spawn and moved
    /// into the blocking closure, so it is released when the OS call returns and at no earlier
    /// point (a cut-off, a drop of the waiting future).
    #[test]
    fn the_gate_takes_its_permit_before_the_spawn_and_the_blocking_closure_owns_it() {
        let source = production();
        // Both ways in take the permit first and hand it to the ONE blocking spawn, whose closure owns it.
        let blocking = &source[source
            .find("async fn blocking_holding<T: Send + 'static>")
            .expect("the one blocking spawn")..];
        let blocking = &blocking[..blocking.find("\n    }\n").expect("it ends")];
        assert!(blocking.contains("spawn_blocking("), "{blocking}");
        assert!(
            blocking.contains("move ||") && blocking.contains("drop(permit)"),
            "the closure owns and drops the permit:\n{blocking}"
        );
        let fail_fast = &source[source.find("async fn run<T: Send + 'static>").expect("the gate's run")..];
        let fail_fast = &fail_fast[..fail_fast.find("\n    }\n").expect("run ends")];
        let acquire = fail_fast
            .find("try_acquire_owned()")
            .expect("the permit is taken without queueing");
        assert!(
            acquire < fail_fast.find("blocking_holding(").expect("then handed to the spawn"),
            "{fail_fast}"
        );
        // Read through `squeeze`: rustfmt puts `.await` on a line of its own in a longer chain.
        assert!(
            !squeeze(fail_fast).contains(&squeeze("acquire_owned().await")),
            "a busy gate fails at once, it never queues:\n{fail_fast}"
        );
        let waiting = &source[source
            .find("async fn run_waiting<T: Send + 'static>")
            .expect("the waiting run")..];
        let waiting = &waiting[..waiting.find("\n    }\n").expect("run_waiting ends")];
        assert!(
            waiting.find("acquire_owned()").expect("it waits for the permit")
                < waiting.find("blocking_holding(").expect("then spawns"),
            "{waiting}"
        );
        assert!(
            waiting.contains("tokio::time::timeout("),
            "the wait is bounded:\n{waiting}"
        );
        assert!(
            !waiting.contains("spawn_blocking"),
            "waiting costs no thread:\n{waiting}"
        );
    }

    pub(super) async fn until(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for: {what}");
    }

    /// Fix round 1 (finding 2): ONE bridge call at a time. A call stuck in the OS holds the gate;
    /// the next call fails at once with `OP_TIMEOUT`, queues behind nothing, and spawns no thread.
    #[tokio::test]
    async fn a_stuck_bridge_call_holds_the_gate_so_the_next_call_fails_fast_and_spawns_nothing() {
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let stuck = {
            let (gate, started) = (gate.clone(), started.clone());
            tokio::spawn(async move {
                gate.run(move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    let _ = hold.recv();
                    Ok(1)
                })
                .await
            })
        };
        until("the stuck call to be inside the OS", || {
            started.load(Ordering::SeqCst) == 1
        })
        .await;

        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_second = ran.clone();
        let second = tokio::time::timeout(
            Duration::from_secs(5),
            gate.run(move || {
                ran_by_second.fetch_add(1, Ordering::SeqCst);
                Ok(2)
            }),
        )
        .await
        .expect("a busy gate fails at once; it never waits behind the stuck call");
        let error = second.expect_err("the gate is held");
        assert_eq!(
            (error.domain.as_str(), error.code),
            (crate::finder_setup::error::APP_DOMAIN, app_code::OP_TIMEOUT)
        );
        assert_eq!(error.message, "an earlier bridge call has not returned");
        assert_eq!(
            crate::finder_setup::policy::classify(&error),
            crate::surfaces::phase::FinderFailureReason::Timeout
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0, "no second blocking call was started");

        release.send(()).unwrap();
        assert_eq!(stuck.await.unwrap(), Ok(1));
    }

    /// Fix round 1 (finding 2): the permit is released when the stuck call RETURNS, not when the
    /// driver's limit cut it off. Between the two the OS call is still running, and a second one
    /// must not start beside it.
    #[tokio::test]
    async fn the_gate_is_released_when_the_stuck_call_returns_not_when_it_was_cut_off() {
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let inside = started.clone();
        // Cut off at 30 ms, as the driver cuts off an `addDomain` that never answers.
        let cut = crate::finder_setup::driver::within(
            Op::AddDomain,
            Duration::from_millis(30),
            gate.run(move || {
                inside.fetch_add(1, Ordering::SeqCst);
                let _ = hold.recv();
                Ok(())
            }),
        )
        .await;
        assert_eq!(
            cut.expect_err("cut off").message,
            "AddDomain did not answer within 30ms"
        );
        until("the cut-off call to be inside the OS", || {
            started.load(Ordering::SeqCst) == 1
        })
        .await;

        // The driver gave up; the OS call did not. The gate is still held.
        assert!(
            !gate.is_idle(),
            "the cut-off must not free the gate while the OS call is still running"
        );
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_next = ran.clone();
        let next = gate
            .run(move || {
                ran_by_next.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert_eq!(next.expect_err("still held").code, app_code::OP_TIMEOUT);
        assert_eq!(
            ran.load(Ordering::SeqCst),
            0,
            "no second bridge call ran beside the stuck one"
        );

        // The OS call finally returns: now, and only now, the gate is free.
        release.send(()).unwrap();
        until("the gate to be released", || gate.is_idle()).await;
        assert_eq!(gate.run(|| Ok(5)).await, Ok(5));
    }

    /// Lead ruling T8-gate-all: one gate for the process. A synchronous caller (the engine's working-set
    /// signal, "Open in Finder") takes the same permit as the reconciler, holds it for the whole call and
    /// frees it after, and a busy gate fails at once.
    #[tokio::test]
    async fn a_synchronous_bridge_call_shares_the_one_gate() {
        assert!(
            Arc::ptr_eq(&BridgeGate::shared().0, &BridgeGate::shared().0),
            "one gate for the process"
        );
        let gate = BridgeGate::new();
        // Inside the call the gate is held, so a second synchronous call is refused at once.
        let inner = gate.clone();
        let result = gate.run_sync(|| {
            assert!(!gate.is_idle(), "the permit is held while the OS call runs");
            Ok(inner.run_sync(|| Ok(1)))
        });
        let nested = result.expect("the outer call ran").expect_err("the gate was held");
        assert_eq!(
            (nested.domain.as_str(), nested.code),
            (crate::finder_setup::error::APP_DOMAIN, app_code::OP_TIMEOUT)
        );
        assert!(gate.is_idle(), "the permit is back after the call");
        assert_eq!(gate.run_sync(|| Ok(7)), Ok(7));

        // A call stuck in the async path holds it against a synchronous one, too.
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let inside = started.clone();
        let stuck = {
            let gate = gate.clone();
            tokio::spawn(async move {
                gate.run(move || {
                    inside.fetch_add(1, Ordering::SeqCst);
                    let _ = hold.recv();
                    Ok(())
                })
                .await
            })
        };
        until("the stuck call to be inside the OS", || {
            started.load(Ordering::SeqCst) == 1
        })
        .await;
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_sync = ran.clone();
        let refused = gate
            .run_sync(move || {
                ran_by_sync.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .expect_err("a synchronous call does not run beside a stuck one");
        assert_eq!(refused.code, app_code::OP_TIMEOUT);
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
        stuck.await.unwrap().unwrap();
    }

    /// The reconciler and everyone else share ONE gate: the ports take it from `shared()`.
    #[test]
    fn the_ports_use_the_shared_gate_and_the_wrappers_are_the_only_other_callers() {
        let source = production();
        let new = &source[source.find("pub fn new(app: tauri::AppHandle)").expect("new")..];
        let new = &new[..new.find("\n    }\n").unwrap()];
        assert!(
            new.contains("BridgeGate::shared()") && !new.contains("BridgeGate::new()"),
            "{new}"
        );
        for wrapper in [
            "fn open_location(",
            "fn reveal_item(",
            "fn domain_state(",
            "fn signal_working_set(",
            "fn cleanup_stale_domains(",
        ] {
            let body = &source[source.find(wrapper).unwrap_or_else(|| panic!("{wrapper}"))..];
            let body = &body[..body.find("\n}\n").unwrap()];
            assert!(
                body.contains("BridgeGate::shared()"),
                "{wrapper} takes the shared gate:\n{body}"
            );
        }
    }

    // ---- Task 9 fix round 2: the removal and the launch read wait a little for the gate ----

    /// A removal that finds the gate busy for a second still happens: it waits for the permit (no thread
    /// of its own while it waits) instead of failing at once. Before this, sign-out left the domain in
    /// Finder whenever a bridge call happened to be in flight, and the core never retries a removal.
    #[tokio::test]
    async fn a_waiting_call_gets_the_gate_when_the_holder_returns_inside_the_wait() {
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let inside = started.clone();
        let holder = {
            let gate = gate.clone();
            tokio::spawn(async move {
                gate.run(move || {
                    inside.fetch_add(1, Ordering::SeqCst);
                    let _ = hold.recv();
                    Ok(())
                })
                .await
            })
        };
        until("the holder to be inside the OS", || started.load(Ordering::SeqCst) == 1).await;
        // The holder returns after one second.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let _ = release.send(());
        });
        let began = std::time::Instant::now();
        let waited = gate.run_waiting(BRIDGE_GATE_WAIT, || Ok(5)).await;
        assert_eq!(
            waited,
            Ok(5),
            "a gate busy for 1 s inside a {BRIDGE_GATE_WAIT:?} wait does not fail the call"
        );
        assert!(
            began.elapsed() >= Duration::from_millis(900),
            "it really waited: {:?}",
            began.elapsed()
        );
        assert!(
            began.elapsed() < BRIDGE_GATE_WAIT,
            "and did not wait out the whole bound: {:?}",
            began.elapsed()
        );
        holder.await.unwrap().unwrap();
        assert!(gate.is_idle(), "the permit is back");
    }

    /// Busy for longer than the wait: the call fails with the busy error, runs nothing and spawns nothing.
    #[tokio::test]
    async fn a_waiting_call_that_outlasts_the_wait_fails_with_the_busy_error_and_runs_nothing() {
        let gate = BridgeGate::new();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let inside = started.clone();
        let holder = {
            let gate = gate.clone();
            tokio::spawn(async move {
                gate.run(move || {
                    inside.fetch_add(1, Ordering::SeqCst);
                    let _ = hold.recv();
                    Ok(())
                })
                .await
            })
        };
        until("the holder to be inside the OS", || started.load(Ordering::SeqCst) == 1).await;
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_waiter = ran.clone();
        let began = std::time::Instant::now();
        let error = gate
            .run_waiting(Duration::from_millis(200), move || {
                ran_by_waiter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .expect_err("the holder never returned inside the wait");
        assert_eq!(
            (error.domain.as_str(), error.code),
            (crate::finder_setup::error::APP_DOMAIN, app_code::OP_TIMEOUT)
        );
        assert_eq!(error.message, "an earlier bridge call has not returned");
        assert!(is_gate_busy(&error));
        assert!(
            began.elapsed() >= Duration::from_millis(200),
            "it waited its bound: {:?}",
            began.elapsed()
        );
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "and no longer: {:?}",
            began.elapsed()
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0, "nothing ran beside the stuck call");
        assert!(!gate.is_idle(), "the stuck call still holds the permit");
        // The waiter left the queue: when the holder returns the gate is free for the next call.
        release.send(()).unwrap();
        holder.await.unwrap().unwrap();
        until("the gate to be released", || gate.is_idle()).await;
        assert_eq!(gate.run(|| Ok(7)).await, Ok(7));
    }

    /// Lead ruling (fix round 2): the removal and the launch read wait. The working-set signal, the
    /// reconciler's other reads, the poll and the startup sweep keep failing fast, so a busy gate never
    /// stalls them and the engine's tick never waits. Since the F7 follow-up the two reads a person waits on
    /// wait as well (`the_calls_a_person_waits_on_wait_a_little_for_the_gate`).
    #[tokio::test]
    async fn the_signal_the_poll_and_the_sweep_still_fail_fast() {
        // The signal: `run_sync` on the shared gate, held. It answers at once.
        super::with_shared_gate_held(|| {
            let began = std::time::Instant::now();
            let busy = signal_working_set().map(|_| ());
            assert!(is_gate_busy(&busy.expect_err("the gate is held")));
            assert!(
                began.elapsed() < Duration::from_millis(500),
                "it failed fast: {:?}",
                began.elapsed()
            );
        });
        // And by shape: exactly two bridge calls in the reconciler wait, and they are the ones named.
        let source = production();
        let run = &source[source.find("fn run(&mut self, op: Op)").expect("run")..];
        let run = &run[..run.find("fn publish(").expect("publish follows run")];
        assert_eq!(
            run.matches("gate.run_waiting(BRIDGE_GATE_WAIT,").count(),
            2,
            "Observe and RemoveDomain, nothing else"
        );
        for waits in ["Op::Observe =>", "Op::RemoveDomain =>"] {
            assert!(
                arm(run, waits).contains("gate.run_waiting(BRIDGE_GATE_WAIT,"),
                "{waits} waits for the gate"
            );
        }
        for fails_fast in ["Op::AddDomain =>", "Op::ReadDomain =>", "Op::WaitStable(timeout) =>"] {
            let body = arm(run, fails_fast);
            assert!(
                body.contains("gate.run(") && !body.contains("run_waiting"),
                "{fails_fast} fails fast:\n{body}"
            );
        }
        for wrapper in ["fn signal_working_set(", "fn cleanup_stale_domains("] {
            let body = &source[source.find(wrapper).unwrap()..];
            let body = &body[..body.find("\n}\n").unwrap()];
            assert!(
                !body.contains("run_waiting") && !body.contains("run_sync_waiting"),
                "{wrapper} fails fast:\n{body}"
            );
        }
    }

    /// F7 follow-up (review minor 1): "Open in Finder" (`open_location`, `reveal_item`) and the onboarding card's
    /// `finder_domain_user_enabled` (`domain_state`) are what a person clicked and waits on. Since the `Ready` poll
    /// reads the domain every 3 s, a click can meet the gate held by that read: they wait for it, on the same bound
    /// as the removal, instead of failing at once. The engine's working-set signal does not (it owes the signal to
    /// its next tick instead).
    #[test]
    fn the_calls_a_person_waits_on_wait_a_little_for_the_gate() {
        let source = production();
        let state = &source[source.find("fn domain_state(").expect("domain_state")..];
        let state = &state[..state.find("\n}\n").unwrap()];
        assert!(
            squeeze(state).contains(&squeeze(
                "BridgeGate::shared().run_sync_waiting(BRIDGE_GATE_WAIT, macos_file_provider::domain_state)"
            )),
            "fn domain_state( waits a little for the shared gate:\n{state}"
        );
        // The open and the reveal wait for the gate inside `resolve_then`, which each of them calls on the shared gate
        // with its own resolve and its own open (fix round 1, I1).
        let then = &source[source.find("fn resolve_then<").expect("resolve_then")..];
        let then = &then[..then.find("\n}\n").unwrap()];
        assert!(
            squeeze(then).contains(&squeeze("gate.run_sync_waiting(BRIDGE_GATE_WAIT, resolve)")),
            "the resolve waits a little for the gate:\n{then}"
        );
        for (wrapper, resolve, open) in [
            (
                "fn open_location(",
                "macos_file_provider::resolve_location,",
                "macos_workspace::open_scoped",
            ),
            (
                "fn reveal_item(",
                "|| macos_file_provider::resolve_item(item_id),",
                "macos_workspace::reveal_scoped",
            ),
        ] {
            let body = &source[source.find(wrapper).unwrap_or_else(|| panic!("{wrapper}"))..];
            let body = &body[..body.find("\n}\n").unwrap()];
            assert!(
                squeeze(body).contains(&squeeze(&format!(
                    "resolve_then(&BridgeGate::shared(),{resolve}{open},)"
                ))),
                "{wrapper} resolves under the shared gate and opens outside it:\n{body}"
            );
        }
        let signal = &source[source.find("fn signal_working_set(").unwrap()..];
        let signal = &signal[..signal.find("\n}\n").unwrap()];
        assert!(
            squeeze(signal).contains(&squeeze(
                "BridgeGate::shared().run_sync(macos_file_provider::signal_working_set)"
            )),
            "the signal still fails fast:\n{signal}"
        );
    }

    /// Task 1885 fix round 1 (I1), in the source: `resolve_then` takes the gate for the resolve ALONE. The statement
    /// that waits for the gate ends before the open is called, and the open is not an argument of it. (The behaviour
    /// test below proves it with a parked open; this keeps the shape from being rewritten to hold the gate longer.)
    #[test]
    fn test_1885_the_open_is_called_after_the_statement_that_holds_the_gate() {
        let source = production();
        let then = &source[source.find("fn resolve_then<").expect("resolve_then")..];
        let then = squeeze(&then[..then.find("\n}\n").unwrap()]);
        let gate = then
            .find("gate.run_sync_waiting(BRIDGE_GATE_WAIT,resolve)?;")
            .expect("the gate call is a statement of its own that ends in `?;`");
        let open = then.find("then(handle)").expect("the open is called");
        assert!(gate < open, "the gate statement ends before the open starts:\n{then}");
        assert_eq!(then.matches("then(").count(), 1, "the open is called once:\n{then}");
        assert_eq!(
            then.matches("run_sync_waiting").count(),
            1,
            "one gate call, for the resolve:\n{then}"
        );
    }

    /// Task 1885 fix round 1 (I1). An open that is blocked inside LaunchServices must not hold the File Provider
    /// gate: `NSWorkspace` is not a File Provider call, and a sign-out's removal waits only `BRIDGE_GATE_WAIT` (3 s)
    /// for the gate and is never retried. So the gate covers the resolve and is released before the open. Here the
    /// open is parked until the test lets it go, and a removal's own gate call, in its strictest form (fail fast),
    /// runs meanwhile.
    #[test]
    fn test_1885_a_removal_gets_the_gate_while_an_open_is_blocked_inside_launch_services() {
        use std::sync::mpsc;
        let gate = BridgeGate::new();
        let (in_open, open_started) = mpsc::channel::<()>();
        let (release, released) = mpsc::channel::<()>();
        let opener = {
            let gate = gate.clone();
            std::thread::spawn(move || {
                resolve_then(
                    &gate,
                    || Ok(Some(7u32)),
                    move |handle| {
                        assert_eq!(handle, 7, "the open gets the handle the resolve returned");
                        in_open.send(()).unwrap();
                        released
                            .recv_timeout(Duration::from_secs(20))
                            .expect("the test releases the open");
                        Ok(())
                    },
                )
            })
        };
        open_started
            .recv_timeout(Duration::from_secs(20))
            .expect("the open started");
        let removal = gate.run_sync(|| Ok("removed"));
        release.send(()).unwrap();
        assert_eq!(opener.join().unwrap(), Ok(true), "the open finished");
        assert_eq!(
            removal,
            Ok("removed"),
            "the removal got the gate while the open was blocked inside LaunchServices"
        );
    }

    /// Task 1885 fix round 1 (I1): the three other ways through the open. No location: nothing is opened. A resolve
    /// that fails (including the gate answering busy): nothing is opened and the error is the caller's. An open that
    /// fails: its error is the caller's, and the gate is free again.
    #[test]
    fn test_1885_the_open_runs_only_after_a_resolve_that_found_a_location_and_its_errors_are_the_callers() {
        let gate = BridgeGate::new();
        let opened = AtomicUsize::new(0);
        let open = |_: u32| {
            opened.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };

        assert_eq!(resolve_then(&gate, || Ok(None::<u32>), open), Ok(false));
        let refused = FpError::new("NSFileProviderErrorDomain", -2001, "no");
        assert_eq!(
            resolve_then(&gate, || Err(refused.clone()), open),
            Err(refused),
            "a resolve error is the caller's error"
        );
        assert_eq!(
            opened.load(Ordering::SeqCst),
            0,
            "no location and a failed resolve open nothing"
        );

        let launch_services = FpError::new("NSCocoaErrorDomain", 256, "no");
        assert_eq!(
            resolve_then(&gate, || Ok(Some(1u32)), |_| Err(launch_services.clone())),
            Err(launch_services),
            "an open error is the caller's error"
        );
        assert!(gate.is_idle(), "the gate is free after a failed open");

        assert_eq!(resolve_then(&gate, || Ok(Some(2u32)), open), Ok(true));
        assert_eq!(opened.load(Ordering::SeqCst), 1);
    }

    /// F7 follow-up (review minor 1): the synchronous wait. A call that meets the gate held by a call in flight
    /// (the `Ready` poll's read) waits for it and runs; held for longer than the wait, it fails with the same
    /// busy error as before, runs nothing, and leaves the queue. Its callers are synchronous: a command's work on the
    /// blocking pool and a menu action inside a task, so the last part waits from inside a runtime.
    #[test]
    fn a_synchronous_call_waits_for_a_gate_held_less_than_the_bound_and_fails_busy_after_it() {
        use std::sync::mpsc;
        fn hold(gate: &BridgeGate) -> (mpsc::Sender<()>, std::thread::JoinHandle<Result<i32, FpError>>) {
            let (release, held) = mpsc::channel::<()>();
            let (inside, entered) = mpsc::channel::<()>();
            let gate = gate.clone();
            let holder = std::thread::spawn(move || {
                gate.run_sync(move || {
                    inside.send(()).unwrap();
                    let _ = held.recv();
                    Ok(1)
                })
            });
            entered.recv().expect("the holder is inside its call");
            (release, holder)
        }

        // Held for 1 s, inside the bound: the call waits, then runs.
        let gate = BridgeGate::new();
        let (release, holder) = hold(&gate);
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(1));
            let _ = release.send(());
        });
        let began = std::time::Instant::now();
        assert_eq!(
            gate.run_sync_waiting(BRIDGE_GATE_WAIT, || Ok(5)),
            Ok(5),
            "a gate busy for 1 s inside a {BRIDGE_GATE_WAIT:?} wait does not fail the call"
        );
        let waited = began.elapsed();
        assert!(waited >= Duration::from_millis(900), "it really waited: {waited:?}");
        assert!(waited < BRIDGE_GATE_WAIT, "and not the whole bound: {waited:?}");
        releaser.join().unwrap();
        assert_eq!(holder.join().unwrap(), Ok(1));
        assert!(gate.is_idle(), "the permit is back");

        // Held for longer than the bound: the existing busy error, and nothing ran.
        let (release, holder) = hold(&gate);
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_by_waiter = ran.clone();
        let began = std::time::Instant::now();
        let error = gate
            .run_sync_waiting(Duration::from_millis(200), move || {
                ran_by_waiter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .expect_err("the holder never returned inside the wait");
        let waited = began.elapsed();
        assert_eq!(
            (error.domain.as_str(), error.code, error.message.as_str()),
            (
                crate::finder_setup::error::APP_DOMAIN,
                app_code::OP_TIMEOUT,
                "an earlier bridge call has not returned"
            )
        );
        assert!(is_gate_busy(&error));
        assert!(waited >= Duration::from_millis(200), "it waited its bound: {waited:?}");
        assert!(waited < Duration::from_secs(2), "and no longer: {waited:?}");
        assert_eq!(ran.load(Ordering::SeqCst), 0, "nothing ran beside the held call");
        assert!(!gate.is_idle(), "the holder still has the permit");
        release.send(()).unwrap();
        assert_eq!(holder.join().unwrap(), Ok(1));
        assert!(gate.is_idle(), "the waiter that gave up left the queue");
        assert_eq!(gate.run_sync(|| Ok(7)), Ok(7));

        // From inside a task (the upload menu action calls "Open in Finder" there): no panic, and the same wait.
        let (release, holder) = hold(&gate);
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let _ = release.send(());
        });
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let inside_a_task = runtime.block_on(async { gate.run_sync_waiting(BRIDGE_GATE_WAIT, || Ok(9)) });
        assert_eq!(inside_a_task, Ok(9));
        releaser.join().unwrap();
        assert_eq!(holder.join().unwrap(), Ok(1));
        assert!(gate.is_idle());

        // And from a task that has spent its cooperative budget: a free gate is taken at once, not waited out.
        let (taken, took) = runtime.block_on(async {
            let spend = tokio::sync::Semaphore::new(1);
            while tokio::task::coop::has_budget_remaining() {
                drop(spend.acquire().await.unwrap());
            }
            let began = std::time::Instant::now();
            (gate.run_sync_waiting(BRIDGE_GATE_WAIT, || Ok(11)), began.elapsed())
        });
        assert_eq!(taken, Ok(11));
        assert!(
            took < Duration::from_millis(500),
            "a free gate is not waited for: {took:?}"
        );
    }

    /// The wait sits INSIDE `within`'s limit for both operations, so it adds nothing to the bound the
    /// Lock and Sign-out acknowledgement waits are derived from (`FINDER_LOCK_TIMEOUT`: 38 s for the longest
    /// operation, `FINDER_REMOVE_TIMEOUT`: that plus `RemoveDomain`'s 15 s): the operation's limit is
    /// unchanged and still covers the wait and the call.
    #[test]
    fn the_gate_wait_fits_inside_the_limits_it_runs_under() {
        use crate::finder_setup::core::Op;
        use crate::finder_setup::driver::op_limit;
        assert!(BRIDGE_GATE_WAIT <= Duration::from_secs(3));
        assert!(
            BRIDGE_GATE_WAIT < op_limit(Op::Observe),
            "inside Observe's {:?}",
            op_limit(Op::Observe)
        );
        assert!(
            BRIDGE_GATE_WAIT < op_limit(Op::RemoveDomain),
            "inside RemoveDomain's {:?}",
            op_limit(Op::RemoveDomain)
        );
        assert_eq!(
            op_limit(Op::RemoveDomain),
            Duration::from_secs(15),
            "the limit the 53 s derivation uses is unchanged"
        );
        assert_eq!(op_limit(Op::Observe), Duration::from_secs(10));
        // `within` wraps the waiting call, so the wait is part of the limit, not added to it.
        let source = production();
        for (pattern, call) in [
            ("Op::Observe =>", "macos_file_provider::domain_state"),
            // Rebase re-review I1: the removal's answer travels on a channel (`answering`).
            (
                "Op::RemoveDomain =>",
                "answering(answer, macos_file_provider::remove, late)",
            ),
        ] {
            let run = &source[source.find("fn run(&mut self, op: Op)").unwrap()..];
            let body = arm(&run[..run.find("fn publish(").unwrap()], pattern);
            // Read through `squeeze`: rustfmt wraps `within(op, limit, ..)` over lines.
            let squeezed = squeeze(body);
            let within = squeezed.find(&squeeze("within(op, limit,")).expect("bounded");
            assert!(
                squeezed[within..].contains(&squeeze(&format!("gate.run_waiting(BRIDGE_GATE_WAIT, {call}"))),
                "{pattern}: the wait is inside `within`:\n{body}"
            );
        }
    }

    /// Spec §6.1, lead ruling 11: nothing the OS said leaves the ports except through the typed
    /// view, the redacted lifecycle log and stdout tracing.
    #[test]
    fn no_os_message_leaves_the_ports_except_to_the_log_and_stdout() {
        let source = production();
        assert!(!source.contains(".message"), "the ports never read an OS message");
        assert!(
            !source.contains("error.to_string()"),
            "no Display text of an error is built"
        );
        let publish = &source[source.find("fn publish(").unwrap()..];
        let publish = &publish[..publish.find("fn log(").unwrap()];
        assert!(
            publish.contains("emit(FINDER_SETUP_CHANGED_EVENT, view)"),
            "the event is the whole view, nothing else:\n{publish}"
        );
        // `FailureRecord` is what reaches `desktop.toml`: a reason, a domain, a code, a time.
        let persist = &source[source.find("fn persist_failure(").unwrap()..];
        assert!(persist.starts_with("fn persist_failure(&mut self, record: Option<FailureRecord>)"));
    }
