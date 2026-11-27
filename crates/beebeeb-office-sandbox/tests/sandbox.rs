//! The sandbox's claims, each with its instrument shown able to fail.
//!
//! Every claim runs the SAME probe twice: plainly (started by `std::process::Command`, here in the test, the crate
//! has no such path) and confined (started by the crate). The plain run must SUCCEED at the attempt, the confined
//! run must fail with the pinned system code. A probe that died, timed out or answered anything else is a failed
//! test, never "blocked": every attempt sits between two answered pings.
//!
//! Assertion messages carry codes and counts only: no path, no address, no name.

#[cfg(target_os = "macos")]
#[path = "macos/confinement.rs"]
mod macos;

#[cfg(not(any(windows, target_os = "macos")))]
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
    use std::ffi::c_void;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard, mpsc};
    use std::time::{Duration, Instant};

    use beebeeb_office_sandbox::{
        Access, Child, Granted, Limits, Profile, Sandbox, SandboxError, Section, helper_environment_names,
    };
    use windows_sys::Win32::Foundation::{
        HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};
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
    /// None of the three folders exists: "path not found", three times.
    const NO_OWN_FOLDER: &str = "own ok=0 codes=3,3,3";
    /// The system's socket library refuses to start inside the container, so no verb that goes through it gets a
    /// socket (what is left below the library: `no_network_verb_succeeds_in_the_confined_helper`).
    const NETWORK_REFUSED: &str = "err network-start 10107";
    /// The system resolver's answer to the confined helper: no DNS server for this caller (`DNS_ERROR_NO_DNS_SERVERS`,
    /// measured on the Windows build VM). A server's answer instead (the name exists, or does not, or has no record)
    /// means a lookup was made for the helper, even one the DNS client service's cache does not show: with the
    /// package opt-out removed, the lookup is answered and no cache line appears.
    const LOOKUP_REFUSED: &str = "err 9852";
    /// The system ends a container process that uses a handle value it does not hold (invalid handle).
    const ENDED_FOR_INVALID_HANDLE: u32 = 0xC000_0008;
    /// Kill on close, one active process, die on an unhandled exception, memory cap; and all eight UI classes.
    /// The job's limit of one process: "not enough quota".
    const PROCESS_QUOTA: &str = "err 1816";
    const JOB_FLAGS: u32 = 0x2000 | 0x8 | 0x400 | 0x200;

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
            Self::started(Sandbox::new(limits).unwrap())
        }

        fn confined_as(profile: Profile) -> Self {
            Self::started(Sandbox::with_profile(Limits::default(), profile).unwrap())
        }

        fn started(sandbox: Sandbox) -> Self {
            let profile = sandbox.profile();
            let mut child = sandbox
                .spawn(Path::new(PROBE))
                .unwrap_or_else(|error| panic!("the sandbox did not start the probe ({profile:?}): {error}"));
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

    /// Folders of container profiles under our name prefix, in the place Windows keeps them for this user.
    fn our_package_folders() -> usize {
        let packages = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("set for the test")).join("Packages");
        let entries = std::fs::read_dir(&packages).expect("the profile folder list is readable");
        let mut seen = 0;
        let mut ours = 0;
        for entry in entries.flatten() {
            seen += 1;
            if entry.file_name().to_string_lossy().starts_with("beebeeb.office.") {
                ours += 1;
            }
        }
        assert!(seen > 0, "the instrument sees profile folders at all");
        ours
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
        // The answer and the listener's count as one pair, so a failure shows both.
        let answer = confined.attempt(&format!("connect {loopback}"));
        assert_eq!((answer.as_str(), accepted(&listener)), (NETWORK_REFUSED, 0));
        assert_eq!(
            confined.attempt(&format!("connect {}", SocketAddr::new(own, port))),
            NETWORK_REFUSED
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
        // Windows gives a container process a folder of its own and points LOCALAPPDATA, TEMP and TMP into it.
        // They are built from the placeholder the crate passes, so they name folders that do not exist; the probe
        // tries to create and write all three. The profile's real folder (under the user's profile) is deleted
        // before the helper runs: the last assertion counts it, and is the one a build that keeps the profile
        // turns red.
        let name = format!("beebeeb-sandbox-test-{}-own.tmp", std::process::id());
        let folders = ["LOCALAPPDATA", "TEMP", "TMP"]
            .map(|variable| PathBuf::from(std::env::var_os(variable).expect("set for the test")));
        let controls: Vec<Scratch> = folders
            .iter()
            .enumerate()
            .map(|(index, folder)| Scratch(folder.join(format!("{name}.{index}"))))
            .collect();

        let mut plain = Probe::plain();
        assert_eq!(
            plain.attempt(&format!("writeown {name}")),
            "own ok=3 codes=",
            "the instrument can fail: a plain probe writes to all three"
        );
        assert_eq!(controls.iter().filter(|control| control.0.exists()).count(), 3);

        let mut confined = Probe::confined();
        assert_eq!(confined.attempt(&format!("writeown {name}")), NO_OWN_FOLDER);
        let where_ = confined.attempt("ownfolders");
        println!("own-folders: {where_}");
        assert_eq!(
            where_.matches("exists=0").count(),
            3,
            "none of the three exists: {where_}"
        );
        // While the helper runs and answers, nothing under our profile-name prefix is left on this PC.
        assert_eq!(confined.ask("ping"), "pong");
        assert_eq!(
            our_package_folders(),
            0,
            "a container profile folder of ours exists while its helper runs"
        );
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
        // Two refusals, both measured ten times over: a program the helper can read is refused by the job's limit
        // of one process (quota); the helper's own program it cannot even read (access denied).
        for _ in 0..3 {
            assert_eq!(confined.attempt(&format!("spawn {shell}")), PROCESS_QUOTA);
            assert_eq!(confined.attempt(&format!("spawn {PROBE}")), ACCESS_DENIED);
        }
    }

    /// The whole job as the helper itself reads it: every limit, not only the one a refused start shows.
    #[test]
    fn a_confined_helper_runs_in_a_job_of_one_process_with_its_memory_cap_and_every_ui_restriction() {
        let _serial = serial();
        let cap = 96 * 1024 * 1024;
        let expected = format!("job flags={JOB_FLAGS:#x} processes=1 memory={cap} ui=0xff");
        let mut plain = Probe::plain();
        assert_ne!(
            plain.attempt("job"),
            expected,
            "the instrument can fail: a plain probe is in no such job"
        );
        let mut confined = Probe::confined_with(Limits { memory_bytes: cap });
        assert_eq!(confined.attempt("job"), expected);
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
        // The handle value means nothing in the confined helper. The system does not merely refuse the call: it
        // ends a container process that uses a handle it does not hold. That exact ending is the pinned result.
        assert_eq!(confined.ask("ping"), "pong");
        confined.send(&request);
        let mut line = String::new();
        let read = confined.from.as_mut().unwrap().read_line(&mut line).unwrap();
        assert_eq!((read, line.trim()), (0, ""), "no answer: the helper was ended");
        let status = confined
            .child()
            .wait_timeout(BOUND)
            .unwrap()
            .expect("ended within the bound");
        assert_eq!(
            status.code(),
            ENDED_FOR_INVALID_HANDLE,
            "ended for an invalid handle, code {:#x}",
            status.code()
        );
        assert!(!signalled_within(&event, Duration::ZERO), "the event was not touched");
    }

    /// A diagnostic switch (`BB_DIAG`, `BB_ENV`) existed on this branch while the spawn was bisected; it could
    /// leave out the container, the job and the restricted token. It is gone. This test fails if it, or anything
    /// like it, comes back: with those variables set to their "everything off" values in the parent, the helper
    /// is confined exactly as without them.
    #[test]
    fn no_environment_variable_of_the_parent_weakens_the_confinement() {
        let _serial = serial();
        /// Takes the variables away again, also when the test fails: a later test must not inherit them.
        struct Unset(&'static [(&'static str, &'static str)]);
        impl Drop for Unset {
            fn drop(&mut self) {
                for (name, _) in self.0 {
                    // SAFETY: as below; the test still holds `SERIAL` when its locals are dropped.
                    unsafe { std::env::remove_var(name) };
                }
            }
        }
        const SWITCHES: &[(&str, &str)] = &[
            ("BB_DIAG", "127"),
            ("BB_ENV", "USERPROFILE,APPDATA,PATH"),
            ("BEEBEEB_SANDBOX", "off"),
            ("BEEBEEB_NO_SANDBOX", "1"),
        ];
        let _unset = Unset(SWITCHES);
        for (name, value) in SWITCHES {
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
        let names = format!("env {}", helper_environment_names().join(","));
        assert_eq!(answers, [NETWORK_REFUSED, ACCESS_DENIED, PROCESS_QUOTA, names.as_str()]);
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
    // ── 8. memory the app shares with the helper ────────────────────────────────────────────────────────────

    /// Not a multiple of the page size, so a lost tail shows.
    const SECTION_BYTES: usize = 200_003;
    /// What the app puts in a section before the helper reads it.
    const DOCUMENT_BYTE: u8 = 0x42;
    /// The byte the probe's `map-handle` writes into the first byte of a section it could map.
    const MAPPED_MARK: u8 = 0x5A;
    /// The privileges the helper's token keeps: the restricted token removes every other one.
    const KEPT_PRIVILEGES: &[&str] = &["SeChangeNotifyPrivilege"];

    /// The probe's `write-granted` pattern: byte `i` is `i % 251`.
    fn pattern(index: usize) -> u8 {
        (index % 251) as u8
    }

    fn count_equal(section: &Section, wanted: u8) -> usize {
        section.as_slice().iter().filter(|&&byte| byte == wanted).count()
    }

    fn count_pattern(section: &Section) -> usize {
        let bytes = section.as_slice().iter().enumerate();
        bytes.filter(|&(index, &byte)| byte == pattern(index)).count()
    }

    fn document_section() -> Section {
        let mut section = Section::new(SECTION_BYTES).expect("a section");
        section.as_mut_slice().fill(DOCUMENT_BYTE);
        section
    }

    fn with_grant(request: &str, granted: Granted) -> String {
        format!("{request} {} {}", granted.wire, granted.len)
    }

    fn make_inheritable(handle: RawHandle) {
        // SAFETY: `handle` is an open handle this test holds.
        assert_ne!(
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) },
            0
        );
    }

    #[test]
    fn a_read_grant_reads_and_refuses_a_write() {
        let _serial = serial();
        let section = document_section();
        let mut probe = Probe::confined();

        let read = probe.child().grant(&section, Access::Read).expect("a grant");
        assert_eq!(read.len, SECTION_BYTES as u64);
        assert_eq!(
            probe.attempt(&format!("{} {DOCUMENT_BYTE}", with_grant("read-granted", read))),
            format!("ok {SECTION_BYTES}"),
            "every byte arrived"
        );

        // A grant maps once (the helper closes its handle with the mapping), so the write gets a grant of its own.
        let refused = probe.child().grant(&section, Access::Read).expect("a grant");
        assert_eq!(probe.attempt(&with_grant("write-granted", refused)), ACCESS_DENIED);
        assert_eq!(
            count_equal(&section, DOCUMENT_BYTE),
            SECTION_BYTES,
            "not one byte changed"
        );

        // The instrument can fail: the same request through a read-write grant writes.
        let writable = probe.child().grant(&section, Access::ReadWrite).expect("a grant");
        assert_eq!(
            probe.attempt(&with_grant("write-granted", writable)),
            format!("ok {SECTION_BYTES}")
        );
        assert_eq!(count_pattern(&section), SECTION_BYTES);
    }

    #[test]
    fn a_read_write_grant_carries_the_rendered_bytes() {
        let _serial = serial();
        let section = Section::new(SECTION_BYTES).expect("a section");
        let before = count_pattern(&section);
        assert!(
            before < SECTION_BYTES / 100,
            "the instrument can fail: a new section is not the pattern ({before} bytes match)"
        );

        let mut probe = Probe::confined();
        let granted = probe.child().grant(&section, Access::ReadWrite).expect("a grant");
        assert_eq!(
            probe.attempt(&with_grant("write-granted", granted)),
            format!("ok {SECTION_BYTES}")
        );
        assert_eq!(
            count_pattern(&section),
            SECTION_BYTES,
            "the app reads back every byte the helper wrote"
        );
    }

    #[test]
    fn an_ungranted_section_is_out_of_reach() {
        let _serial = serial();
        // The instrument can fail: a plain probe that inherits a section the app holds maps it and writes to it.
        let control = document_section();
        make_inheritable(control.as_raw_handle());
        let mut plain = Probe::plain();
        assert_eq!(
            plain.attempt(&format!("map-handle {:x}", control.as_raw_handle() as usize)),
            format!("ok {DOCUMENT_BYTE}")
        );
        assert_eq!(
            control.as_slice()[0],
            MAPPED_MARK,
            "a plain child reached the section and wrote to it"
        );
        drop(plain);
        drop(control);

        // Made before the helper starts (so a handle the crate let it inherit would be there), held, never granted.
        let kept = document_section();
        let mut confined = Probe::confined();
        // The value means nothing in the confined helper; the system ends a container process that uses a handle it
        // does not hold. That exact ending is the pinned result, as for the event handle above.
        assert_eq!(confined.ask("ping"), "pong");
        confined.send(&format!("map-handle {:x}", kept.as_raw_handle() as usize));
        let mut line = String::new();
        let read = confined.from.as_mut().unwrap().read_line(&mut line).unwrap();
        assert_eq!((read, line.trim()), (0, ""), "no answer: the helper was ended");
        let status = confined
            .child()
            .wait_timeout(BOUND)
            .unwrap()
            .expect("ended within the bound");
        assert_eq!(
            status.code(),
            ENDED_FOR_INVALID_HANDLE,
            "ended for an invalid handle, code {:#x}",
            status.code()
        );
        assert_eq!(
            count_equal(&kept, DOCUMENT_BYTE),
            SECTION_BYTES,
            "the section's bytes are unchanged"
        );
    }

    /// The length in bytes of an object's name, as the system's object query reads it; 0 is "no name".
    fn object_name_length(handle: RawHandle) -> u16 {
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtQueryObject(handle: RawHandle, class: u32, out: *mut c_void, length: u32, returned: *mut u32) -> i32;
        }
        const OBJECT_NAME_INFORMATION: u32 = 1;
        // A counted text (its length in bytes first, as a u16) and then the text itself. Aligned for the pointer in it.
        let mut buffer = [0u64; 256];
        let mut returned = 0u32;
        // SAFETY: `handle` is an open handle this test holds; the buffer is writable for the length passed.
        let status = unsafe {
            NtQueryObject(
                handle,
                OBJECT_NAME_INFORMATION,
                buffer.as_mut_ptr().cast(),
                size_of_val(&buffer) as u32,
                &mut returned,
            )
        };
        assert_eq!(status, 0, "the name query answered, status {status:#x}");
        (buffer[0] & 0xFFFF) as u16
    }

    #[test]
    fn a_section_has_no_name() {
        let _serial = serial();
        // The instrument can fail: a section made with a name reads back a name through the same query.
        let name: Vec<u16> = format!("Local\\beebeeb-sandbox-test-{}", std::process::id())
            .encode_utf16()
            .chain([0])
            .collect();
        // SAFETY: no file, default security, read-write pages, one page, a terminated name.
        let raw = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                4096,
                name.as_ptr(),
            )
        };
        assert!(!raw.is_null(), "the named control section was made");
        // SAFETY: `raw` is a new open handle that nothing else owns.
        let named = unsafe { OwnedHandle::from_raw_handle(raw) };
        assert!(
            object_name_length(named.as_raw_handle()) > 0,
            "the query reads a name where there is one"
        );

        let section = Section::new(4096).expect("a section");
        assert_eq!(
            object_name_length(section.as_raw_handle()),
            0,
            "the section has no name"
        );
    }

    /// The names in a `privileges` answer; their count is checked against the one the probe gives.
    fn privilege_names(answer: &str) -> Vec<String> {
        let mut words = answer.split(' ');
        assert_eq!(words.next(), Some("privileges"), "a privileges answer: {answer}");
        let count: usize = words.next().and_then(|count| count.parse().ok()).expect("a count");
        let names: Vec<String> = words
            .next()
            .unwrap_or("")
            .split(',')
            .filter(|name| !name.is_empty())
            .map(String::from)
            .collect();
        assert_eq!(names.len(), count, "one name per privilege");
        names
    }

    #[test]
    fn the_helper_holds_no_privileges() {
        let _serial = serial();
        let mut plain = Probe::plain();
        let parents = privilege_names(&plain.attempt("privileges"));
        let mut confined = Probe::confined();
        let helpers = privilege_names(&confined.attempt("privileges"));
        println!(
            "privileges: plain {} confined {} ({})",
            parents.len(),
            helpers.len(),
            helpers.join(",")
        );
        assert!(
            helpers.iter().all(|name| parents.contains(name)),
            "the helper holds nothing a plain child does not"
        );
        assert!(
            helpers.len() < parents.len(),
            "the instrument can fail: a plain child holds more ({} against {})",
            parents.len(),
            helpers.len()
        );
        assert_eq!(helpers, KEPT_PRIVILEGES, "only what Windows does not remove");
    }

    /// UDP datagrams that arrived on `socket` (non-blocking), after a short wait for any still on their way.
    fn datagrams(socket: &UdpSocket) -> usize {
        std::thread::sleep(Duration::from_millis(200));
        let mut buffer = [0u8; 64];
        let mut count = 0;
        while socket.recv_from(&mut buffer).is_ok() {
            count += 1;
        }
        count
    }

    /// The port of a `listen` answer (`ok <port>`).
    fn listening_port(answer: &str) -> u16 {
        answer
            .strip_prefix("ok ")
            .and_then(|port| port.parse().ok())
            .unwrap_or_else(|| panic!("a listen answer: {answer}"))
    }

    /// No network verb succeeds in the confined helper: TCP connect (loopback, and this machine's own address on
    /// its network, a routable one), a UDP datagram to loopback, bind and listen on loopback, a name lookup. Each
    /// verb is first shown to work for a plain probe, with its effect counted where the test holds the other end.
    ///
    /// **Where the refusal happens.** The probe starts the system's socket library before every verb, and in the
    /// container that start fails (`NETWORK_REFUSED`, measured on the Windows build VM), so every verb is refused
    /// at that same point. The kernel's network filter for containers without a network capability is therefore
    /// not reached by these verbs. It was seen refusing only in a mutant: with the package opt-out removed (and
    /// still no capability) the library starts and a loopback connect times out.
    ///
    /// **The socket driver is a measurement, not an assertion.** The device itself refuses the confined helper,
    /// but a name under it (`\Device\Afd\Endpoint`, the one socket libraries open) opens. An open handle there
    /// is not a network verb: no address is bound or reached by it, and every verb above is refused. Whether a
    /// helper that drives the driver directly, without the library, meets the kernel's filter is not measured
    /// here. Both answers are printed (`--nocapture` or `--show-output` shows them) so a change is seen.
    #[test]
    fn no_network_verb_succeeds_in_the_confined_helper() {
        let _serial = serial();
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        tcp.set_nonblocking(true).unwrap();
        let tcp_loopback = tcp.local_addr().unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.set_nonblocking(true).unwrap();
        let udp_loopback = udp.local_addr().unwrap();
        let own = own_address();
        let lookup = "resolve localhost";
        let listen = "listen 127.0.0.1:0";
        let driver = ["open-afd", "open-afd Endpoint"];

        // The instrument can fail: each verb works for a plain probe, and its effect is counted.
        let mut plain = Probe::plain();
        assert_eq!(plain.attempt(&format!("connect {tcp_loopback}")), "ok");
        assert_eq!(accepted(&tcp), 1, "the listener counts a connection");
        // A service of the system itself listens on this machine's own address; the first port the plain probe
        // reaches is the one the confined probe is then refused on.
        let port = [135u16, 445, 22, 5985, 3389]
            .into_iter()
            .find(|&port| plain.attempt(&format!("connect {}", SocketAddr::new(own, port))) == "ok")
            .expect("the plain probe reaches a listening port on this machine's own address");
        let routable = SocketAddr::new(own, port);
        assert_eq!(plain.attempt(&format!("udp-send {udp_loopback}")), "ok 5");
        assert_eq!(datagrams(&udp), 1, "the socket counts a datagram");
        let held = listening_port(&plain.attempt(listen));
        assert!(
            TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], held)), BOUND).is_ok(),
            "the plain probe listens: the test connects to it"
        );
        let resolved = plain.attempt(lookup);
        assert!(
            resolved.starts_with("ok ") && resolved != "ok 0",
            "the plain probe looks the name up: {resolved}"
        );
        let driver_plain = driver.map(|request| plain.attempt(request));
        drop(plain);

        let mut confined = Probe::confined();
        let connect_loopback = confined.attempt(&format!("connect {tcp_loopback}"));
        let connect_routable = confined.attempt(&format!("connect {routable}"));
        let send_loopback = confined.attempt(&format!("udp-send {udp_loopback}"));
        let listen_loopback = confined.attempt(listen);
        let looked_up = confined.attempt(lookup);
        let driver_confined = driver.map(|request| confined.attempt(request));
        // What reached the test's own sockets, counted after every confined attempt.
        let reached = (accepted(&tcp), datagrams(&udp));
        println!(
            "network: confined connect loopback {connect_loopback:?}, connect routable {connect_routable:?}, \
             udp loopback {send_loopback:?}, listen loopback {listen_loopback:?}, lookup {looked_up:?}"
        );
        // A measurement (see the doc above): printed, never asserted.
        println!("measurement socket-driver: plain {driver_plain:?} confined {driver_confined:?}");

        assert_eq!(connect_loopback, NETWORK_REFUSED, "TCP connect to loopback");
        assert_eq!(
            connect_routable, NETWORK_REFUSED,
            "TCP connect to this machine's own address"
        );
        assert_eq!(send_loopback, NETWORK_REFUSED, "a UDP datagram to loopback");
        assert_eq!(listen_loopback, NETWORK_REFUSED, "bind and listen on loopback");
        assert_eq!(looked_up, NETWORK_REFUSED, "a name lookup");
        assert_eq!(
            reached,
            (0, 0),
            "connections and datagrams that reached the test's sockets"
        );
    }

    /// A TCP connect through the socket driver alone (no socket library) is refused in the confined helper. The
    /// verbs above stop where the library starts, so they never reach the kernel's network filter; this one goes
    /// past the library, as a helper that drives the driver itself would.
    ///
    /// **The target is a listening TCP port on another machine** (`BB_OFFICE_N1_TARGET=<ip:port>`), never
    /// loopback or this machine's own address: the filter refuses those even to a container that holds a network
    /// capability, so only another machine lets the capability mutant (`network-capability` in
    /// `mutants-on-windows.ps1`) turn this test red. The plain probe connecting there through the driver alone is
    /// the proof that the instrument can see a connection. Ignored unless asked for; asked for without a target is a
    /// red with the reason.
    #[test]
    #[ignore = "needs BB_OFFICE_N1_TARGET: a listening TCP port on another machine of the network"]
    fn a_raw_driver_connect_is_refused_in_the_confined_helper() {
        let _serial = serial();
        let target = std::env::var("BB_OFFICE_N1_TARGET")
            .expect("BB_OFFICE_N1_TARGET is not set: this measurement was asked to run and could not");
        let target: SocketAddr = target.parse().expect("BB_OFFICE_N1_TARGET is <ip:port>");
        assert!(
            !target.ip().is_loopback() && target.ip() != own_address(),
            "the target is another machine, not loopback or this machine's own address"
        );

        let mut plain = Probe::plain();
        let plain_raw = plain.attempt(&format!("raw-connect {target}"));
        let plain_library = plain.attempt(&format!("connect {target}"));
        drop(plain);
        let mut confined = Probe::confined();
        let confined_raw = confined.attempt(&format!("raw-connect {target}"));
        let confined_library = confined.attempt(&format!("connect {target}"));
        println!(
            "raw-connect: plain {plain_raw:?} (library {plain_library:?}); confined {confined_raw:?} (library \
             {confined_library:?})"
        );

        assert_eq!(
            plain_raw, "ok",
            "the plain probe connects through the socket driver alone"
        );
        assert!(
            confined_raw.starts_with("err "),
            "the confined helper's connect through the socket driver: {confined_raw}"
        );
    }

    /// A fresh host name under `domain`, never asked before: `ocp-<time><pid><n>.<domain>`.
    fn fresh_name(domain: &str) -> String {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        format!(
            "ocp-{nanos:x}{:x}{}.{domain}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        )
    }

    /// How many lines of the DNS client service's cache name `name` (`ipconfig /displaydns`; a name that does not
    /// exist is cached too). The cache is the machine's, so it counts a lookup whoever asked the service for it.
    fn cached(name: &str) -> usize {
        let shown = Command::new("ipconfig")
            .arg("/displaydns")
            .output()
            .expect("ipconfig runs");
        String::from_utf8_lossy(&shown.stdout)
            .to_ascii_lowercase()
            .lines()
            .filter(|line| line.contains(name))
            .count()
    }

    /// `cached(name)` once it is non-zero, or after `within`.
    fn cached_within(name: &str, within: Duration) -> usize {
        let until = Instant::now() + within;
        loop {
            let seen = cached(name);
            if seen > 0 || Instant::now() >= until {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// The most lines `cached(name)` showed, sampled from now until `within` passed, on the same schedule as
    /// [`cached_within`]: an entry that was in the cache for a moment and left again still counts.
    fn cached_most_within(name: &str, within: Duration) -> usize {
        let until = Instant::now() + within;
        let mut most = 0;
        loop {
            most = most.max(cached(name));
            if Instant::now() >= until {
                return most;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// No name lookup is made on the confined helper's behalf: neither through the system resolver's interface (the
    /// DNS client service makes that lookup, in its own process) nor by opening a share path whose host is a NAME
    /// (the system resolves it before it can refuse the open). A document can carry such a path in a linked
    /// picture, and a lookup of a crafted name would carry text off the machine even if no connection follows.
    ///
    /// **Counted machine-wide**, in the DNS client service's cache, never per process: the lookups happen in system
    /// services, not in the helper. Each case uses a fresh name and is first shown to land in the cache for a plain
    /// probe. A web-folder share path (`\\<name>@SSL\DavWWWRoot\x`) is printed as a measurement only: whether the
    /// plain probe's lookup shows depends on the web-folder service running. `BB_OFFICE_LOOKUP_DOMAIN` names a
    /// domain whose fresh names do not exist (for example `example.com`); asked for without it is a red.
    #[test]
    #[ignore = "needs BB_OFFICE_LOOKUP_DOMAIN: a domain whose fresh names do not exist; sends DNS queries"]
    fn no_name_lookup_is_made_on_the_confined_helpers_behalf() {
        let _serial = serial();
        let domain = std::env::var("BB_OFFICE_LOOKUP_DOMAIN")
            .expect("BB_OFFICE_LOOKUP_DOMAIN is not set: this measurement was asked to run and could not")
            .to_ascii_lowercase();
        let within = Duration::from_secs(5);
        // (the case, the request it makes for a host name)
        type Case = (&'static str, fn(&str) -> String);
        let cases: [Case; 3] = [
            ("resolver", |name| format!("dns {name}")),
            ("share-name", |name| format!(r"read \\{name}\s\x")),
            ("web-folder", |name| format!(r"read \\{name}@SSL\DavWWWRoot\x")),
        ];

        let mut plain = Probe::plain();
        let plain_seen = cases.map(|(case, request)| {
            let name = fresh_name(&domain);
            let answer = plain.attempt(&request(&name));
            (case, answer, cached_within(&name, within))
        });
        drop(plain);
        let mut confined = Probe::confined();
        let confined_seen = cases.map(|(case, request)| {
            let name = fresh_name(&domain);
            let answer = confined.attempt(&request(&name));
            (case, answer, cached_most_within(&name, within))
        });
        println!("lookups (case, answer, lines in the cache): plain {plain_seen:?}; confined {confined_seen:?}");

        for index in 0..2 {
            let (case, _, seen) = &plain_seen[index];
            assert!(
                *seen > 0,
                "{case}: the plain probe's lookup is seen in the cache (the instrument can count)"
            );
            let (case, answer, seen) = &confined_seen[index];
            assert_eq!(
                *seen, 0,
                "{case}: lookups made for the confined helper (it answered {answer})"
            );
        }
        // The cache shows only the lookups the service makes and keeps; the answer shows whether any server answered.
        assert_eq!(
            confined_seen[0].1, LOOKUP_REFUSED,
            "resolver: the confined helper's lookup is refused, not answered by a server"
        );
    }

    /// A file on a network share, opened by its share path, is out of reach of the confined helper: on Windows the
    /// sandbox is what stands between the helper and the file-sharing client (the Office engine's own refusal of
    /// such paths is tested in the Office engine).
    ///
    /// The literal path `\\127.0.0.1\c$\x` fails for any process (there is no such file), so its answer is
    /// recorded, and a refusal is asserted, but it cannot show the instrument. The instrument is the system's
    /// shell, which the confined probe reads by its local path: a plain probe reads the same bytes through the
    /// administrative share on loopback, and the confined probe is refused there.
    #[test]
    fn a_confined_helper_cannot_open_a_path_on_a_network_share() {
        let _serial = serial();
        const MISSING: &str = r"\\127.0.0.1\c$\x";
        let system_root = std::env::var("SystemRoot").expect("SystemRoot");
        let local = format!(r"{system_root}\System32\cmd.exe");
        let (drive, rest) = system_root
            .split_once(':')
            .expect("the system folder has a drive letter");
        let shared = format!(
            r"\\127.0.0.1\{drive}$\{}\System32\cmd.exe",
            rest.trim_start_matches('\\')
        );

        let mut plain = Probe::plain();
        let plain_local = plain.attempt(&format!("read {local}"));
        let plain_shared = plain.attempt(&format!("read {shared}"));
        let plain_missing = plain.attempt(&format!("read {MISSING}"));
        drop(plain);

        let mut confined = Probe::confined();
        let confined_local = confined.attempt(&format!("read {local}"));
        let confined_shared = confined.attempt(&format!("read {shared}"));
        let confined_missing = confined.attempt(&format!("read {MISSING}"));
        // The recorded answers (codes and byte counts only).
        println!(
            "share: plain local {plain_local:?} shared {plain_shared:?} missing {plain_missing:?}; \
             confined local {confined_local:?} shared {confined_shared:?} missing {confined_missing:?}"
        );

        assert!(plain_local.starts_with("ok "), "the shell is readable: {plain_local}");
        assert_eq!(
            plain_shared, plain_local,
            "the instrument can fail: a plain probe reads the same bytes through the share"
        );
        assert_eq!(
            confined_local, plain_local,
            "the confined probe reads the shell by its local path, so a refusal below is about the share"
        );
        assert!(
            confined_shared.starts_with("err "),
            "the confined helper read a file through a network share: {confined_shared}"
        );
        assert!(
            confined_missing.starts_with("err "),
            "the confined helper's open of the share path did not fail: {confined_missing}"
        );
    }

    /// A measurement the viewer's design needs: the helper draws text with the system's fonts.
    #[test]
    fn the_helper_reads_the_font_folder() {
        let _serial = serial();
        let mut plain = Probe::plain();
        assert_eq!(
            plain.attempt("read-font"),
            "ok",
            "the instrument: the font file is there"
        );
        let mut confined = Probe::confined();
        assert_eq!(
            confined.attempt("read-font"),
            "ok",
            "the confined helper reads the system font folder"
        );
    }

    // The profiles (decision 2541): in `tests/windows/profiles.rs`.
    mod hardening;
    mod profiles;
}
