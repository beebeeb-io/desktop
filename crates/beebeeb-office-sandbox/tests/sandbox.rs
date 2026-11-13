//! The sandbox's claims, each with its instrument shown able to fail.
//!
//! Every claim runs the SAME probe twice: plainly (started by `std::process::Command`, here in the test, the crate
//! has no such path) and confined (started by the crate). The plain run must SUCCEED at the attempt, the confined
//! run must fail with the pinned system code. A probe that died, timed out or answered anything else is a failed
//! test, never "blocked": every attempt sits between two answered pings.
//!
//! Assertion messages carry codes and counts only: no path, no address, no name.

#[cfg(not(windows))]
#[test]
fn without_a_sandbox_module_the_probe_is_not_started() {
    use beebeeb_office_sandbox::{Limits, Sandbox, SandboxError};
    let probe = std::path::Path::new(env!("CARGO_BIN_EXE_beebeeb-sandbox-probe"));
    assert!(
        probe.is_file(),
        "the probe was built, so a fallback could have started it"
    );
    assert_eq!(
        Sandbox::new(Limits::default()).unwrap().spawn(probe).unwrap_err(),
        SandboxError::Unsupported
    );
}

#[cfg(windows)]
mod windows {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{IpAddr, SocketAddr, TcpListener, UdpSocket};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard, mpsc};
    use std::time::{Duration, Instant};

    use beebeeb_office_sandbox::{Child, Limits, Sandbox, SandboxError, helper_environment_names};
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        ResetEvent, TerminateProcess, WaitForSingleObject,
    };

    const PROBE: &str = env!("CARGO_BIN_EXE_beebeeb-sandbox-probe");
    /// The bound of every "gone within" claim.
    const BOUND: Duration = Duration::from_millis(2000);
    /// How long a helper is watched to show that it really is still running.
    const STILL_RUNNING: Duration = Duration::from_millis(400);

    /// The codes the confined probe gets, as measured on the Windows build VM (the PR body has the run).
    const ACCESS_DENIED: &str = "err 5";
    const PATH_NOT_FOUND: &str = "err 3";
    const SOCKET_ACCESS_DENIED: &str = "err 10013";
    const PROCESS_QUOTA: &str = "err 1816";
    const INVALID_HANDLE: &str = "err 6";

    /// One probe at a time: the timing bounds and the handle-inheritance control are not disturbed by a sibling.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    enum Process {
        Confined(Child),
        Plain(std::process::Child),
    }

    struct Probe {
        process: Process,
        to: Box<dyn Write + Send>,
        from: Option<BufReader<Box<dyn Read + Send>>>,
    }

    impl Probe {
        fn confined() -> Self {
            Self::confined_with(Limits::default())
        }

        fn confined_with(limits: Limits) -> Self {
            let mut child = Sandbox::new(limits)
                .unwrap()
                .spawn(Path::new(PROBE))
                .unwrap_or_else(|error| panic!("the sandbox did not start the probe: {error}"));
            let pipes = child.take_pipes().expect("pipes, once");
            assert!(child.take_pipes().is_none(), "pipes are handed out once");
            Self {
                process: Process::Confined(child),
                to: Box::new(pipes.to_helper),
                from: Some(BufReader::new(Box::new(pipes.from_helper))),
            }
        }

        fn plain() -> Self {
            let mut child = Command::new(PROBE)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("the plain probe starts");
            let to = Box::new(child.stdin.take().unwrap());
            let from = BufReader::new(Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>);
            Self {
                process: Process::Plain(child),
                to,
                from: Some(from),
            }
        }

        fn id(&self) -> u32 {
            match &self.process {
                Process::Confined(child) => child.id(),
                Process::Plain(child) => child.id(),
            }
        }

        fn child(&self) -> &Child {
            match &self.process {
                Process::Confined(child) => child,
                Process::Plain(_) => panic!("not a confined probe"),
            }
        }

        fn send(&mut self, request: &str) {
            writeln!(self.to, "{request}")
                .and_then(|()| self.to.flush())
                .expect("the probe takes a request");
        }

        fn ask(&mut self, request: &str) -> String {
            self.send(request);
            let mut line = String::new();
            let read = self
                .from
                .as_mut()
                .expect("reader")
                .read_line(&mut line)
                .expect("the probe's answer is readable");
            if read == 0 {
                let code = match &self.process {
                    Process::Confined(child) => child.wait_timeout(BOUND).ok().flatten().map(|status| status.code()),
                    Process::Plain(_) => None,
                };
                panic!(
                    "the probe ended instead of answering (exit code {code:x?}): that is a broken instrument, not a block"
                );
            }
            line.trim().to_string()
        }

        /// One attempt between two answered pings: a probe that is not alive before and after proves nothing.
        fn attempt(&mut self, request: &str) -> String {
            assert_eq!(self.ask("ping"), "pong");
            let answer = self.ask(request);
            assert_eq!(self.ask("ping"), "pong");
            answer
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            // A confined probe is ended by `Child`'s own drop; a plain one has to be ended here.
            if let Process::Plain(child) = &mut self.process {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    // ── small system helpers of the tests ───────────────────────────────────────────────────────────────────

    fn open_process(id: u32) -> OwnedHandle {
        // SAFETY: plain call; the result is checked before use.
        let raw = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                id,
            )
        };
        assert!(
            !raw.is_null(),
            "the process can be opened: system code {}",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        );
        // SAFETY: `raw` is a new open handle that nothing else owns.
        unsafe { OwnedHandle::from_raw_handle(raw) }
    }

    fn signalled_within(handle: &OwnedHandle, bound: Duration) -> bool {
        // SAFETY: `handle` is an open process or event handle.
        unsafe { WaitForSingleObject(handle.as_raw_handle(), bound.as_millis() as u32) == WAIT_OBJECT_0 }
    }

    fn terminate(process: &OwnedHandle) {
        // SAFETY: `process` is an open process handle with terminate access, opened by this test from an id it recorded.
        unsafe { TerminateProcess(process.as_raw_handle(), 9) };
    }

    fn inheritable_event() -> OwnedHandle {
        // SAFETY: null attributes and name: an unnamed, manual-reset, unsignalled event.
        let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        assert!(!raw.is_null());
        // SAFETY: `raw` is the open event handle just created.
        assert_ne!(
            unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) },
            0
        );
        // SAFETY: `raw` is a new open handle that nothing else owns.
        unsafe { OwnedHandle::from_raw_handle(raw) }
    }

    static NAMES: AtomicU32 = AtomicU32::new(0);

    /// A file path that does not exist yet; whatever is there afterwards is removed when this is dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn in_dir(dir: &Path) -> Self {
            let name = format!(
                "beebeeb-sandbox-test-{}-{}.tmp",
                std::process::id(),
                NAMES.fetch_add(1, Ordering::SeqCst)
            );
            let path = dir.join(name);
            assert!(!path.exists());
            Self(path)
        }

        fn text(&self) -> &str {
            self.0.to_str().expect("a test path is text")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// The places a helper must not be able to write to (or read from). Temp and the profile are always there;
    /// the others are counted in only where the folder exists.
    fn user_places() -> Vec<PathBuf> {
        let profile = PathBuf::from(std::env::var_os("USERPROFILE").expect("a user profile"));
        let mut places = vec![std::env::temp_dir(), profile.clone()];
        let maybe = [
            // Writable by low-integrity programs of this user.
            Some(profile.join("AppData").join("LocalLow")),
            // The helper's own folder.
            Path::new(PROBE).parent().map(Path::to_path_buf),
            std::env::var_os("PUBLIC").map(PathBuf::from),
        ];
        places.extend(maybe.into_iter().flatten().filter(|place| place.is_dir()));
        places
    }

    // ── 1. no network ───────────────────────────────────────────────────────────────────────────────────────

    fn accepted(listener: &TcpListener) -> usize {
        std::thread::sleep(Duration::from_millis(200));
        let mut count = 0;
        while listener.accept().is_ok() {
            count += 1;
        }
        count
    }

    /// This machine's own address on its network (never another host: a UDP "connect" sends nothing, it only
    /// makes the system pick the local address it would use).
    fn own_address() -> IpAddr {
        let socket = UdpSocket::bind("0.0.0.0:0").expect("a UDP socket");
        socket
            .connect("192.0.2.1:9")
            .expect("this machine has a network address");
        let address = socket.local_addr().unwrap().ip();
        assert!(
            !address.is_loopback() && !address.is_unspecified(),
            "a non-loopback address of this machine"
        );
        address
    }

    #[test]
    fn a_confined_helper_cannot_connect_to_loopback_or_to_this_machines_own_address() {
        let _serial = serial();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let loopback = listener.local_addr().unwrap();

        // The instrument can fail: the plain probe connects, and the listener counts it.
        let mut plain = Probe::plain();
        assert_eq!(plain.attempt(&format!("connect {loopback}")), "ok");
        assert_eq!(accepted(&listener), 1, "the listener counts a connection");
        // A service of the system itself listens on this machine's own address; the first port the plain probe
        // reaches is the one the confined probe is then refused on.
        let own = own_address();
        let reachable = [135u16, 445, 22, 5985, 3389]
            .into_iter()
            .find(|&port| plain.attempt(&format!("connect {}", SocketAddr::new(own, port))) == "ok");
        let port = reachable.expect("the plain probe reaches a listening port on this machine's own address");
        drop(plain);

        let mut confined = Probe::confined();
        assert_eq!(confined.attempt(&format!("connect {loopback}")), SOCKET_ACCESS_DENIED);
        assert_eq!(
            accepted(&listener),
            0,
            "the listener saw nothing from the confined probe"
        );
        assert_eq!(
            confined.attempt(&format!("connect {}", SocketAddr::new(own, port))),
            SOCKET_ACCESS_DENIED
        );
    }

    // ── 2. no file write, no read of a user file ────────────────────────────────────────────────────────────

    #[test]
    fn a_confined_helper_cannot_create_a_file_anywhere_the_user_can() {
        let _serial = serial();
        let places = user_places();
        assert!(
            places.len() >= 4,
            "temp, the profile and at least two more: {}",
            places.len()
        );

        let mut plain = Probe::plain();
        let mut confined = Probe::confined();
        let mut written_plain = 0;
        let mut refused_confined = 0;
        for place in &places {
            let control = Scratch::in_dir(place);
            assert_eq!(plain.attempt(&format!("write {}", control.text())), "ok");
            assert_eq!(std::fs::read(&control.0).unwrap(), b"probe");
            written_plain += 1;

            let target = Scratch::in_dir(place);
            assert_eq!(confined.attempt(&format!("write {}", target.text())), ACCESS_DENIED);
            assert!(!target.0.exists(), "nothing was created");
            refused_confined += 1;
        }
        assert_eq!((written_plain, refused_confined), (places.len(), places.len()));
    }

    #[test]
    fn a_confined_helper_cannot_read_a_file_of_the_user() {
        let _serial = serial();
        let places = user_places();
        let mut plain = Probe::plain();
        let mut confined = Probe::confined();
        let mut count = 0;
        for place in &places {
            let canary = Scratch::in_dir(place);
            std::fs::write(&canary.0, b"canary").unwrap();
            assert_eq!(plain.attempt(&format!("read {}", canary.text())), "ok 6");
            assert_eq!(confined.attempt(&format!("read {}", canary.text())), ACCESS_DENIED);
            count += 1;
        }
        assert!(count >= 4);
    }

    #[test]
    fn a_confined_helper_has_no_folder_of_its_own_to_write_to() {
        let _serial = serial();
        // The folder a container process is given by the system (its TEMP) is gone before the helper runs.
        let control = Scratch::in_dir(&std::env::temp_dir());
        let name = control.0.file_name().unwrap().to_str().unwrap().to_string();
        let mut plain = Probe::plain();
        assert_eq!(
            plain.attempt(&format!("writetemp {name}")),
            "ok",
            "the instrument can fail: a plain probe writes to its TEMP"
        );
        assert!(control.0.exists());

        let mut confined = Probe::confined();
        assert_eq!(confined.attempt(&format!("writetemp {name}-confined")), PATH_NOT_FOUND);
    }

    // ── 3. no child process ─────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_confined_helper_cannot_start_a_process() {
        let _serial = serial();
        // A program the confined probe CAN read (the read test of the system folder below shows it), so the
        // refusal is about starting, not about finding or reading.
        let system_root = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
        let shell = system_root.join("System32").join("cmd.exe");
        let shell = shell.to_str().unwrap();

        let mut plain = Probe::plain();
        assert_eq!(plain.attempt(&format!("spawn {shell}")), "ok");
        assert_eq!(plain.attempt(&format!("spawn {PROBE}")), "ok");

        let mut confined = Probe::confined();
        let readable = confined.attempt(&format!("read {shell}"));
        assert!(
            readable.starts_with("ok "),
            "the confined probe can read the program it then fails to start: {readable}"
        );
        assert_eq!(confined.attempt(&format!("spawn {shell}")), PROCESS_QUOTA);
        assert_eq!(confined.attempt(&format!("spawn {PROBE}")), PROCESS_QUOTA);
    }

    // ── 4. gone with the parent; a dead or hung helper is seen and killed within the bound ──────────────────

    /// Starts `probe parent <mode>`, which starts a second probe and makes it hang. Kills the first by the handle
    /// `std` holds for it and reports whether the second ended within the bound.
    fn second_probe_ends_after_its_parent_is_killed(mode: &str) -> (bool, Duration) {
        let mut parent = Command::new(PROBE)
            .args(["parent", mode])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(parent.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let id: u32 = line
            .trim()
            .strip_prefix("child ")
            .unwrap_or_else(|| panic!("the parent probe reported: {}", line.trim()))
            .parse()
            .unwrap();
        let second = open_process(id);
        assert!(
            !signalled_within(&second, STILL_RUNNING),
            "the second probe runs while its parent lives"
        );

        parent.kill().unwrap();
        parent.wait().unwrap();
        let start = Instant::now();
        let ended = signalled_within(&second, BOUND);
        let elapsed = start.elapsed();
        if !ended {
            terminate(&second);
            assert!(signalled_within(&second, BOUND), "the test ends the orphan it made");
        }
        (ended, elapsed)
    }

    #[test]
    fn a_confined_helper_is_gone_when_its_parent_is_killed() {
        let _serial = serial();
        let (plain_ended, _) = second_probe_ends_after_its_parent_is_killed("plain");
        assert!(
            !plain_ended,
            "the instrument can fail: a plain child outlives its killed parent"
        );
        let (confined_ended, elapsed) = second_probe_ends_after_its_parent_is_killed("sandboxed");
        assert!(
            confined_ended,
            "the confined helper was still there {} ms after its parent",
            BOUND.as_millis()
        );
        println!(
            "gone-with-parent: {} ms (bound {} ms)",
            elapsed.as_millis(),
            BOUND.as_millis()
        );
    }

    #[test]
    fn a_hung_helper_is_seen_and_killed_within_the_bound_and_a_waiting_read_ends() {
        let _serial = serial();
        for (request, answer) in [("hang", "hanging"), ("spin", "spinning")] {
            let mut probe = Probe::confined();
            assert_eq!(probe.ask(request), answer);

            // A read that waits for the helper's next answer, as the app's reader thread would.
            let mut reader = probe.from.take().unwrap();
            let (done, read_ended) = mpsc::channel();
            std::thread::spawn(move || {
                let mut line = String::new();
                let _ = done.send(reader.read_line(&mut line).unwrap_or(usize::MAX));
            });

            // The parent sees the hang: no exit and no answer within its own bound. Without a kill it stays so.
            assert_eq!(probe.child().wait_timeout(STILL_RUNNING).unwrap(), None);
            assert!(read_ended.try_recv().is_err(), "the read is still waiting");

            let start = Instant::now();
            probe.child().kill().unwrap();
            let status = probe
                .child()
                .wait_timeout(BOUND)
                .unwrap()
                .expect("the killed helper ended within the bound");
            assert!(status.was_killed());
            assert_eq!(
                read_ended.recv_timeout(BOUND),
                Ok(0),
                "the waiting read ended with end-of-file"
            );
            println!(
                "kill-{request}: {} ms (bound {} ms)",
                start.elapsed().as_millis(),
                BOUND.as_millis()
            );
            probe
                .child()
                .kill()
                .expect("killing a helper that is gone is not an error");
        }
    }

    #[test]
    fn starting_a_helper_leaves_it_answering_and_reports_the_time() {
        let _serial = serial();
        let start = Instant::now();
        let mut probe = Probe::confined();
        assert_eq!(probe.ask("ping"), "pong");
        println!("start-to-first-answer: {} ms", start.elapsed().as_millis());
    }

    #[test]
    fn a_helper_that_ends_by_itself_is_noticed_with_its_exit_code() {
        let _serial = serial();
        let mut probe = Probe::confined();
        assert_eq!(probe.ask("ping"), "pong");
        assert_eq!(probe.child().try_wait().unwrap(), None);
        probe.send("exit 7");
        let status = probe
            .child()
            .wait_timeout(BOUND)
            .unwrap()
            .expect("noticed within the bound");
        assert_eq!((status.code(), status.was_killed()), (7, false));
        let mut line = String::new();
        assert_eq!(
            probe.from.as_mut().unwrap().read_line(&mut line).unwrap(),
            0,
            "its pipe reads end-of-file"
        );
    }

    #[test]
    fn dropping_the_child_ends_the_helper() {
        let _serial = serial();
        let mut probe = Probe::confined();
        assert_eq!(probe.ask("hang"), "hanging");
        let process = open_process(probe.id());
        assert!(
            !signalled_within(&process, STILL_RUNNING),
            "it runs until it is dropped"
        );
        drop(probe);
        assert!(signalled_within(&process, BOUND));
    }

    // ── 5. the memory cap ───────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_confined_helper_cannot_hold_more_memory_than_its_cap() {
        let _serial = serial();
        // The instrument can fail: with the default cap (1 GiB) the same 256 MiB are granted.
        let mut roomy = Probe::confined();
        assert_eq!(roomy.attempt("alloc 256"), "ok");
        drop(roomy);

        let mut capped = Probe::confined_with(Limits {
            memory_bytes: 64 * 1024 * 1024,
        });
        assert_eq!(capped.attempt("alloc 16"), "ok", "under the cap is granted");
        assert_eq!(capped.attempt("alloc 256"), "err alloc");
    }

    // ── 6. nothing else is handed over ──────────────────────────────────────────────────────────────────────

    #[test]
    fn a_confined_helper_gets_four_environment_variables_and_none_of_the_parents() {
        let _serial = serial();
        let mut plain = Probe::plain();
        let inherited = plain.attempt("env");
        assert!(
            inherited.split(',').count() > 1,
            "the instrument can fail: a plain child inherits {} names",
            inherited.split(',').count()
        );

        let mut confined = Probe::confined();
        assert_eq!(
            confined.attempt("env"),
            format!("env {}", helper_environment_names().join(","))
        );
        assert_eq!(helper_environment_names().len(), 4);
    }

    #[test]
    fn an_inheritable_handle_of_the_parent_does_not_arrive_in_a_confined_helper() {
        let _serial = serial();
        let event = inheritable_event();
        let request = format!("setevent {:x}", event.as_raw_handle() as usize);

        let mut plain = Probe::plain();
        assert_eq!(plain.attempt(&request), "ok");
        assert!(
            signalled_within(&event, Duration::ZERO),
            "the instrument can fail: a plain child got the handle and used it"
        );
        // SAFETY: `event` is the open event handle this test created.
        assert_ne!(unsafe { ResetEvent(event.as_raw_handle()) }, 0);
        drop(plain);

        let mut confined = Probe::confined();
        assert_eq!(confined.attempt(&request), INVALID_HANDLE);
        assert!(!signalled_within(&event, Duration::ZERO), "the event was not touched");
    }

    /// A diagnostic switch (`BB_DIAG`, `BB_ENV`) existed on this branch while the spawn was bisected; it could
    /// leave out the container, the job and the restricted token. It is gone. This test fails if it, or anything
    /// like it, comes back: with those variables set to their "everything off" values in the parent, the helper
    /// is confined exactly as without them.
    #[test]
    fn no_environment_variable_of_the_parent_weakens_the_confinement() {
        let _serial = serial();
        let switches = [
            ("BB_DIAG", "127"),
            ("BB_ENV", "USERPROFILE,APPDATA,PATH"),
            ("BEEBEEB_SANDBOX", "off"),
            ("BEEBEEB_NO_SANDBOX", "1"),
        ];
        for (name, value) in switches {
            // SAFETY: every test of this file holds `SERIAL` while it runs, so no other thread of this process
            // reads or writes the environment now.
            unsafe { std::env::set_var(name, value) };
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let target = Scratch::in_dir(&std::env::temp_dir());
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("cmd.exe");

        let mut confined = Probe::confined();
        let answers = [
            confined.attempt(&format!("connect {}", listener.local_addr().unwrap())),
            confined.attempt(&format!("write {}", target.text())),
            confined.attempt(&format!("spawn {}", shell.to_str().unwrap())),
            confined.attempt("env"),
        ];
        for (name, _) in switches {
            // SAFETY: as above.
            unsafe { std::env::remove_var(name) };
        }
        let names = format!("env {}", helper_environment_names().join(","));
        assert_eq!(
            answers,
            [SOCKET_ACCESS_DENIED, ACCESS_DENIED, PROCESS_QUOTA, names.as_str()]
        );
        assert_eq!(accepted(&listener), 0);
        assert!(!target.0.exists());
    }

    // ── 7. refusals before anything starts ──────────────────────────────────────────────────────────────────

    #[test]
    fn a_helper_path_that_is_relative_or_not_a_file_is_refused() {
        let _serial = serial();
        let sandbox = Sandbox::new(Limits::default()).unwrap();
        assert!(matches!(
            sandbox.spawn(Path::new("beebeeb-sandbox-probe.exe")),
            Err(SandboxError::HelperPath(_))
        ));
        let missing = Scratch::in_dir(&std::env::temp_dir());
        assert_eq!(
            sandbox.spawn(&missing.0).unwrap_err(),
            SandboxError::HelperPath("not a file")
        );
        assert_eq!(
            sandbox.spawn(&std::env::temp_dir()).unwrap_err(),
            SandboxError::HelperPath("not a file")
        );
    }
}
