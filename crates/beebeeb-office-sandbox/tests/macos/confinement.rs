//! macOS: the sandbox's claims, each with its instrument shown able to fail.
//!
//! As on Windows, a claim runs the SAME probe twice where the attempt has a plain outcome: as the control (started by
//! the test with `std::process::Command` and the probe's `plain-control` word, which is the only way it skips its
//! own confinement) and confined (started by the crate). The control must SUCCEED at the attempt, the confined run
//! must fail with the pinned system code. A probe that died, timed out or answered anything else is a failed test,
//! never "blocked".
//!
//! Assertion messages carry codes and counts only: no path, no address, no name.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use beebeeb_office_sandbox::{
    Access, Child, Limits, Mode, Profile, Sandbox, SandboxError, Section, helper, helper_environment_names,
};

const PROBE: &str = env!("CARGO_BIN_EXE_beebeeb-sandbox-probe");
/// The bound of every "gone within" claim.
const BOUND: Duration = Duration::from_millis(2000);
/// How long a process is watched to show that it really is still running.
const STILL_RUNNING: Duration = Duration::from_millis(400);
/// "Operation not permitted": the profile's answer to everything it does not allow.
const DENIED: &str = "err 1";
/// The exit code of a helper that a bus error ended (the crate adds the signal's number, 10, to 0x10000).
const BUS_ERROR: u32 = 0x0001_0000 + 10;
const MIB: u64 = 1024 * 1024;

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn fcntl(fd: i32, command: i32, ...) -> i32;
}

/// One probe at a time: the timing bounds and the descriptor control are not disturbed by a sibling.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether a process with this identifier exists (signal 0 only asks).
fn exists(pid: u32) -> bool {
    // SAFETY: signal 0 sends nothing; the call takes no pointer.
    unsafe { kill(pid as i32, 0) == 0 }
}

fn gone_within(pid: u32, bound: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < bound {
        if !exists(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    !exists(pid)
}

enum Process {
    Confined(Child),
    Plain(std::process::Child),
}

struct Probe {
    process: Process,
    to: Box<dyn Write + Send>,
    from: BufReader<Box<dyn Read + Send>>,
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
            from: BufReader::new(Box::new(pipes.from_helper)),
        }
    }

    /// The control: the same program, not confined. Never through the crate, which has no such path.
    fn plain() -> Self {
        let mut child = Command::new(PROBE)
            .arg("plain-control")
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
            from,
        }
    }

    fn child(&self) -> &Child {
        match &self.process {
            Process::Confined(child) => child,
            Process::Plain(_) => panic!("the control has no sandbox child"),
        }
    }

    /// One request, one answer line. `None` when the probe ended instead of answering.
    fn try_ask(&mut self, request: &str) -> Option<String> {
        if writeln!(self.to, "{request}").and_then(|()| self.to.flush()).is_err() {
            return None;
        }
        let mut line = String::new();
        match self.from.read_line(&mut line) {
            Ok(n) if n > 0 => Some(line.trim_end().to_string()),
            _ => None,
        }
    }

    fn ask(&mut self, request: &str) -> String {
        let word = request.split(' ').next().unwrap_or("");
        self.try_ask(request)
            .unwrap_or_else(|| panic!("the probe ended instead of answering '{word}'"))
    }

    /// An attempt between two answered pings: the probe was alive before and after.
    fn attempt(&mut self, request: &str) -> String {
        assert_eq!(self.ask("ping"), "pong");
        let answer = self.ask(request);
        assert_eq!(self.ask("ping"), "pong");
        answer
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        if let Process::Plain(child) = &mut self.process {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A file of this user's home that exists and can be read (by its place in a fixed list, never by name in a
/// message).
fn home_file() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").expect("a home folder"));
    [".CFUserTextEncoding", ".zshrc", ".zprofile", ".profile", ".gitconfig"]
        .iter()
        .map(|name| home.join(name))
        .find(|path| std::fs::read(path).is_ok())
        .expect("one of five usual files of a home folder can be read by the test")
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bb-dm79-{}-{name}", std::process::id()))
}

#[test]
fn a_confined_helper_cannot_read_a_file_of_the_user() {
    let _serial = serial();
    let in_home = home_file();
    let in_temp = scratch("read");
    std::fs::write(&in_temp, b"twelve bytes").unwrap();
    let mut read = 0;
    for path in [&in_home, &in_temp] {
        let request = format!("read {}", path.display());
        let plain = Probe::plain().attempt(&request);
        assert!(
            plain.starts_with("ok "),
            "the instrument can fail: the control reads it"
        );
        assert_eq!(Probe::confined().attempt(&request), DENIED);
        read += 1;
    }
    std::fs::remove_file(&in_temp).unwrap();
    assert_eq!(read, 2);
}

#[test]
fn a_confined_helper_cannot_create_a_file_anywhere_the_user_can() {
    let _serial = serial();
    let control = scratch("write-control");
    let confined = scratch("write-confined");
    let in_home = PathBuf::from(std::env::var_os("HOME").unwrap()).join(format!(".bb-dm79-{}", std::process::id()));
    let _ = (std::fs::remove_file(&control), std::fs::remove_file(&confined));

    assert_eq!(Probe::plain().attempt(&format!("write {}", control.display())), "ok");
    assert!(control.is_file(), "the instrument can fail: the control made its file");
    std::fs::remove_file(&control).unwrap();

    let mut probe = Probe::confined();
    let mut refused = 0;
    for path in [&confined, &in_home] {
        assert_eq!(probe.attempt(&format!("write {}", path.display())), DENIED);
        assert!(!path.exists(), "nothing was made");
        refused += 1;
    }
    // Where it may read, it may not write either.
    assert_eq!(probe.attempt("write /Library/Fonts/bb-dm79-probe"), DENIED);
    assert_eq!(refused, 2);
}

#[test]
fn the_helper_reads_the_font_folder_and_nothing_next_to_it() {
    let _serial = serial();
    let mut probe = Probe::confined();
    assert_eq!(probe.attempt("read-font"), "ok");
    assert_eq!(
        probe.attempt("read /System/Library/CoreServices/SystemVersion.plist"),
        DENIED
    );
    assert_eq!(probe.attempt("read /private/etc/passwd"), DENIED);
    assert!(
        Probe::plain().attempt("read /private/etc/passwd").starts_with("ok "),
        "the instrument can fail"
    );
}

#[test]
fn a_confined_helper_cannot_use_the_network() {
    let _serial = serial();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let target = listener.local_addr().unwrap();

    assert_eq!(Probe::plain().attempt(&format!("connect {target}")), "ok");
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        listener.accept().is_ok(),
        "the instrument can fail: the control connected"
    );

    let mut probe = Probe::confined();
    assert_eq!(probe.attempt(&format!("connect {target}")), DENIED);
    std::thread::sleep(Duration::from_millis(50));
    assert!(listener.accept().is_err(), "no connection arrived");
    // Listening and sending a datagram are refused too.
    assert_eq!(probe.attempt("listen 127.0.0.1:0"), DENIED);
    assert_eq!(probe.attempt(&format!("udp-send {target}")), DENIED);
    assert!(Probe::plain().attempt("listen 127.0.0.1:0").starts_with("ok "));
    assert_eq!(Probe::plain().attempt(&format!("udp-send {target}")), "ok 5");
}

#[test]
fn a_confined_helper_cannot_start_a_process() {
    let _serial = serial();
    assert_eq!(Probe::plain().attempt("spawn /bin/sh"), "ok");
    let mut probe = Probe::confined();
    assert_eq!(probe.attempt("spawn /bin/sh"), DENIED);
    assert_eq!(probe.attempt("spawn /usr/bin/true"), DENIED);
}

#[test]
fn a_confined_helper_cannot_replace_its_profile_signal_the_app_or_name_shared_memory() {
    let _serial = serial();
    let mut control = Probe::plain();
    assert_eq!(control.attempt("shm-create"), "ok");
    assert_eq!(control.attempt("signal-parent"), "ok");
    // Last: from here on the control is in a sandbox of its own (one that allows everything).
    assert_eq!(control.attempt("second-profile"), "ok");

    let mut probe = Probe::confined();
    assert_eq!(probe.attempt("second-profile"), "err refused");
    assert_eq!(probe.attempt("shm-create"), DENIED);
    assert_eq!(probe.attempt("signal-parent"), DENIED);
    // Still under the first profile after the refused second one.
    assert_eq!(probe.attempt("spawn /bin/sh"), DENIED);
}

fn pattern(index: usize) -> u8 {
    (index % 251) as u8
}

fn count_equal(section: &Section, wanted: u8) -> usize {
    let mut bytes = vec![0u8; section.len()];
    section.copy_to(&mut bytes).unwrap();
    bytes.iter().filter(|&&byte| byte == wanted).count()
}

/// Neither a multiple of eight nor of the page size.
const DOCUMENT_LEN: usize = 3 * 16384 + 5;

fn document_section() -> Section {
    let mut section = Section::new(DOCUMENT_LEN).unwrap();
    section.as_mut_slice().fill(7);
    section
}

#[test]
fn a_read_grant_reads_and_refuses_a_write() {
    let _serial = serial();
    let section = document_section();
    let mut probe = Probe::confined();

    let granted = probe.child().grant(&section, Access::Read).unwrap();
    assert_eq!(granted.len, DOCUMENT_LEN as u64);
    assert_eq!(
        probe.attempt(&format!("read-granted {} {} 7", granted.wire, granted.len)),
        format!("ok {DOCUMENT_LEN}")
    );

    // A second read grant, mapped read-write by the helper: the system refuses the mapping.
    let again = probe.child().grant(&section, Access::Read).unwrap();
    assert_ne!(again.wire, granted.wire, "each grant has a number of its own");
    assert_eq!(
        probe.attempt(&format!("write-granted {} {}", again.wire, again.len)),
        DENIED
    );
    assert_eq!(
        count_equal(&section, 7),
        DOCUMENT_LEN,
        "no byte of the document changed"
    );

    // The instrument can fail: a read-write grant of the same section takes the same request.
    let writable = probe.child().grant(&section, Access::ReadWrite).unwrap();
    assert_eq!(
        probe.attempt(&format!("write-granted {} {}", writable.wire, writable.len)),
        format!("ok {DOCUMENT_LEN}")
    );
    assert_eq!(
        count_equal(&section, 7),
        DOCUMENT_LEN / 251 + 1,
        "only where the pattern is 7"
    );
}

/// The seal of a read grant, tried the hard way: the helper maps it read-only, asks the system to make that mapping
/// writable (refused, system code 13) and then writes into it anyway (a bus error ends it). The app's bytes stay.
#[test]
fn a_write_into_a_read_only_grant_ends_the_helper_and_changes_nothing() {
    let _serial = serial();
    let section = document_section();
    let mut probe = Probe::confined();
    let granted = probe.child().grant(&section, Access::Read).unwrap();
    assert_eq!(probe.ask("ping"), "pong");
    assert_eq!(
        probe.ask(&format!("force-write {} {}", granted.wire, granted.len)),
        "reprotect err 13"
    );
    assert_eq!(
        probe.try_ask("ping"),
        None,
        "the helper is gone, it did not answer 'err wrote'"
    );
    let status = probe
        .child()
        .wait_timeout(BOUND)
        .unwrap()
        .expect("ended within the bound");
    assert_eq!(status.code(), BUS_ERROR);
    assert!(!status.was_killed());
    assert_eq!(
        count_equal(&section, 7),
        DOCUMENT_LEN,
        "no byte of the document changed"
    );
}

#[test]
fn a_read_write_grant_carries_the_rendered_bytes() {
    let _serial = serial();
    let section = Section::new(DOCUMENT_LEN).unwrap();
    let mut probe = Probe::confined();
    let granted = probe.child().grant(&section, Access::ReadWrite).unwrap();
    assert_eq!(
        probe.attempt(&format!("write-granted {} {}", granted.wire, granted.len)),
        format!("ok {DOCUMENT_LEN}")
    );
    let mut bytes = vec![0u8; DOCUMENT_LEN];
    section.copy_to(&mut bytes).unwrap();
    let right = bytes.iter().enumerate().filter(|(i, b)| **b == pattern(*i)).count();
    assert_eq!(right, DOCUMENT_LEN, "every byte the helper wrote arrived");
}

#[test]
fn a_grant_maps_once_and_a_number_that_was_never_granted_maps_nothing() {
    let _serial = serial();
    let section = document_section();
    let mut probe = Probe::confined();
    let granted = probe.child().grant(&section, Access::Read).unwrap();
    let request = format!("read-granted {} {} 7", granted.wire, granted.len);
    assert_eq!(probe.attempt(&request), format!("ok {DOCUMENT_LEN}"));
    assert_eq!(probe.attempt(&request), "err 2", "the grant is used up");
    assert_eq!(probe.attempt(&format!("read-granted 99999 {DOCUMENT_LEN} 7")), "err 2");
    // A length past the section's is refused by the system, and uses the grant up.
    let long = probe.child().grant(&section, Access::Read).unwrap();
    assert_eq!(
        probe.attempt(&format!("read-granted {} {} 7", long.wire, 64 * 16384)),
        "err 22"
    );
    // The control was handed no control socket: it can map nothing.
    assert_eq!(Probe::plain().attempt(&request), "err 2");
}

#[test]
fn a_helper_that_holds_more_memory_than_its_cap_is_ended() {
    let _serial = serial();
    // The instrument can fail: under the default cap the same allocation is answered.
    let mut roomy = Probe::confined();
    assert_eq!(roomy.attempt("alloc-chunks 192"), "ok");
    assert!(!roomy.child().killed_for_memory());
    drop(roomy);

    let mut probe = Probe::confined_with(Limits { memory_bytes: 64 * MIB });
    assert_eq!(probe.ask("ping"), "pong");
    // It may answer first: the watch reads every 100 ms and the allocation can be quicker than one read.
    let _ = probe.try_ask("alloc-chunks 192");
    let status = probe
        .child()
        .wait_timeout(BOUND)
        .unwrap()
        .expect("ended within the bound");
    assert!(status.was_killed());
    assert!(probe.child().killed_for_memory(), "and the memory watch says why");
}

/// The second line: one allocation larger than the address space the helper may still take (its limit and room for
/// two full sections, 8 GiB) fails inside the helper, which goes on answering. Nothing of it is touched.
#[test]
fn one_allocation_past_the_address_space_limit_fails_inside_the_helper() {
    let _serial = serial();
    assert_eq!(Probe::plain().attempt("reserve 9000"), "ok", "the instrument can fail");
    let mut probe = Probe::confined_with(Limits { memory_bytes: 32 * MIB });
    assert_eq!(probe.attempt("reserve 9000"), "err alloc");
    assert_eq!(probe.attempt("reserve 16"), "ok");
    assert!(!probe.child().killed_for_memory());
}

/// A section of the app that the helper fills is not held against the helper: 96 MiB written through a grant by a
/// helper whose own cap is 48 MiB.
#[test]
fn a_section_the_helper_fills_is_not_counted_against_its_cap() {
    let _serial = serial();
    let mut probe = Probe::confined_with(Limits { memory_bytes: 48 * MIB });
    let section = Section::new(96 * MIB as usize).unwrap();
    let granted = probe.child().grant(&section, Access::ReadWrite).unwrap();
    assert_eq!(
        probe.attempt(&format!("write-granted {} {}", granted.wire, granted.len)),
        format!("ok {}", 96 * MIB)
    );
    std::thread::sleep(STILL_RUNNING);
    assert_eq!(probe.ask("ping"), "pong");
    assert!(!probe.child().killed_for_memory());
}

#[test]
fn a_helper_that_says_it_is_confined_and_is_not_is_ended() {
    let _serial = serial();
    // The probe's `--image` start on macOS: it sends the crate's handshake and does not confine itself.
    let refused = Sandbox::new(Limits::default())
        .unwrap()
        .spawn_in_mode(Path::new(PROBE), Mode::Image)
        .unwrap_err();
    assert_eq!(refused, SandboxError::NotConfined("the kernel reports no sandbox"));
}

#[test]
fn a_program_that_never_confines_itself_is_handed_nothing() {
    let _serial = serial();
    let sandbox = Sandbox::new(Limits::default()).unwrap();
    let started = Instant::now();
    assert_eq!(
        sandbox.spawn(Path::new("/usr/bin/true")).unwrap_err(),
        SandboxError::NotConfined("no handshake")
    );
    assert!(started.elapsed() < BOUND, "a program that ends is not waited for");
    // Every profile is the same confinement, and none starts an unconfined program.
    for profile in [
        Profile::SystemDecoder { hevc: false },
        Profile::SystemDecoder { hevc: true },
    ] {
        let sandbox = Sandbox::with_profile(Limits::default(), profile).unwrap();
        assert_eq!(
            sandbox.spawn(Path::new("/usr/bin/true")).unwrap_err(),
            SandboxError::NotConfined("no handshake")
        );
    }
}

/// What the App Sandbox does to this design, in small: a process that already has a sandbox cannot start a helper
/// through the crate (nothing is started), and a helper that already has one cannot confine itself (it ends).
#[test]
fn inside_a_sandbox_no_helper_is_started_and_none_confines_itself() {
    let _serial = serial();
    let run = |mode: &str| {
        Command::new(PROBE)
            .arg(mode)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .expect("the probe starts")
    };
    let app = run("in-sandbox");
    assert_eq!(
        String::from_utf8_lossy(&app.stdout).trim(),
        "refused the helper was not confined (the app has a sandbox its helper would inherit and cannot tighten); \
         it was ended before it ran"
    );
    let nested = run("nested");
    assert_eq!(nested.status.code(), Some(i32::from(helper::UNCONFINED_EXIT_CODE)));
    assert!(nested.stdout.is_empty(), "it did not go on");
}

#[test]
fn nothing_of_the_app_arrives_in_a_confined_helper() {
    let _serial = serial();
    // A descriptor of the app that every process it starts would inherit: a duplicate (command 0) numbered 200 or
    // higher, above every descriptor a helper has of its own, and without the close-on-exec flag.
    let file = std::fs::File::open("/dev/null").unwrap();
    // SAFETY: `file` is open; the command takes the lowest number wanted.
    let fd = unsafe { fcntl(std::os::fd::AsRawFd::as_raw_fd(&file), 0, 200) };
    assert!(fd >= 200);
    // SAFETY: the duplicate just made, owned from here on and closed once.
    let _inheritable = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    // SAFETY: test-only, under the serial lock: no other thread of this test reads the environment now.
    unsafe { std::env::set_var("BB_DM79_MARK", "1") };

    let mut control = Probe::plain();
    assert_eq!(control.attempt(&format!("fd {fd}")), "open", "the instrument can fail");
    assert!(control.attempt("env").contains("BB_DM79_MARK"));

    let mut probe = Probe::confined();
    assert_eq!(probe.attempt(&format!("fd {fd}")), "closed");
    // Its two pipes, /dev/null as standard error, the control socket, and the queue of its own parent watch.
    assert_eq!(probe.attempt("fds"), "fds 0,1,2,3,4");
    assert_eq!(probe.attempt("env"), "env", "no variable at all");
    assert!(helper_environment_names().is_empty());
    assert_eq!(probe.child().handle_count().unwrap(), 5);
    // SAFETY: as above.
    unsafe { std::env::remove_var("BB_DM79_MARK") };
}

#[test]
fn a_hung_helper_is_killed_and_one_that_ends_by_itself_is_noticed() {
    let _serial = serial();
    let mut probe = Probe::confined();
    assert_eq!(probe.ask("hang"), "hanging");
    assert_eq!(
        probe.child().wait_timeout(STILL_RUNNING).unwrap(),
        None,
        "still running"
    );
    probe.child().kill().unwrap();
    let status = probe
        .child()
        .wait_timeout(BOUND)
        .unwrap()
        .expect("ended within the bound");
    assert!(status.was_killed());
    assert_eq!(probe.try_ask("ping"), None, "a read from it ends");
    probe.child().kill().unwrap();

    let mut probe = Probe::confined();
    assert_eq!(probe.ask("ping"), "pong");
    assert_eq!(probe.try_ask("exit 7"), None);
    let status = probe
        .child()
        .wait_timeout(BOUND)
        .unwrap()
        .expect("ended within the bound");
    assert_eq!(status.code(), 7);
    assert!(!status.was_killed() && !probe.child().killed_for_memory());
}

#[test]
fn dropping_the_child_ends_the_helper() {
    let _serial = serial();
    let mut probe = Probe::confined();
    assert_eq!(probe.ask("hang"), "hanging");
    let pid = probe.child().id();
    assert!(exists(pid), "the instrument can fail: it runs");
    drop(probe);
    assert!(!exists(pid), "gone, and collected, when the drop returns");
}

/// Starts the probe as "the app" that starts a second probe and makes it hang; kills the first. Returns whether
/// the second was gone within the bound, and its process identifier.
fn second_probe_ends_after_its_parent_is_killed(mode: &str) -> (bool, u32) {
    let mut parent = Command::new(PROBE)
        .args(["parent", mode])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the parent probe starts");
    let mut line = String::new();
    BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let pid: u32 = line
        .trim()
        .strip_prefix("child ")
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| {
            panic!(
                "the parent probe did not start its child ({} bytes answered)",
                line.len()
            )
        });
    assert!(exists(pid), "the second probe runs");
    parent.kill().unwrap();
    parent.wait().unwrap();
    (gone_within(pid, BOUND), pid)
}

#[test]
fn a_confined_helper_is_gone_when_its_parent_is_killed() {
    let _serial = serial();
    let (gone, pid) = second_probe_ends_after_its_parent_is_killed("plain");
    assert!(!gone, "the instrument can fail: a plain child outlives its parent");
    // The orphan this test made, by the identifier its own parent printed.
    // SAFETY: signal 9 to that one process; the call takes no pointer.
    unsafe { kill(pid as i32, 9) };
    assert!(gone_within(pid, BOUND));

    let (gone, _) = second_probe_ends_after_its_parent_is_killed("sandboxed");
    assert!(gone, "the confined helper ended with its parent");
}

#[test]
fn starting_a_helper_leaves_it_answering_and_reports_the_time() {
    let _serial = serial();
    let started = Instant::now();
    let mut probe = Probe::confined();
    assert_eq!(probe.ask("ping"), "pong");
    println!("confined start to first answer: {} ms", started.elapsed().as_millis());
    assert!(probe.child().id() > 1);
}
