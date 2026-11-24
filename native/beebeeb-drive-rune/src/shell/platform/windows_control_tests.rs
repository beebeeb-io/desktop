//! Real pipes, real ACL, on this machine. Each test uses a name of its own, so a running app
//! (or another test) is never touched.

use std::sync::Mutex;

use super::*;

const REAL: AppIdentity = AppIdentity::Real;

fn test_name(what: &str) -> String {
    let sid = current_user_sid().unwrap();
    name_for(
        &format!("io.beebeeb.app.test.harness.{what}.{}", std::process::id()),
        &sid,
        current_session_id().unwrap(),
    )
}

fn recording(reply: Reply) -> (ControlHandler, Arc<Mutex<Vec<Request>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let handler: ControlHandler = Arc::new(move |request| {
        sink.lock().unwrap().push(request);
        reply
    });
    (handler, seen)
}

fn raw_exchange(name: &str, bytes: &[u8], wait: Duration) -> Option<Vec<u8>> {
    let pipe = open_client(name).expect("the pipe opens for its own user");
    assert!(write_all(&pipe, bytes));
    read_until(&pipe, MAX_REPLY, Instant::now() + wait, |b| b.contains(&b'\n'))
}

#[test]
fn the_name_and_the_access_list_are_per_identity_and_per_user() {
    let sid = current_user_sid().unwrap();
    assert!(sid.starts_with("S-1-"), "a SID");
    assert_eq!(
        sddl_for("S-1-5-21-1-2-3-1001"),
        "O:S-1-5-21-1-2-3-1001D:P(A;;GA;;;S-1-5-21-1-2-3-1001)"
    );
    let session = current_session_id().unwrap();
    let (real, test) = (
        pipe_name(AppIdentity::Real).unwrap(),
        pipe_name(AppIdentity::Test).unwrap(),
    );
    assert_eq!(real, format!("\\\\.\\pipe\\io.beebeeb.app.control.{sid}.{session}"));
    assert_eq!(
        test,
        format!("\\\\.\\pipe\\io.beebeeb.app.test.control.{sid}.{session}")
    );
    // The same account in another Windows session has a channel of its own.
    assert_ne!(name_for("c", &sid, session), name_for("c", &sid, session + 1));
    assert_ne!(real, test);
}

/// P2-4: a real round trip of every verb, with the acknowledgement.
#[test]
fn a_request_crosses_a_real_pipe_and_is_acknowledged() {
    let name = test_name("roundtrip");
    let (handler, seen) = recording(Reply::Ok);
    let _server = serve(&name, REAL, handler).unwrap();
    let wait = Duration::from_secs(5);
    assert_eq!(send(&name, &Request::Show, wait), Delivery::Acknowledged);
    assert_eq!(send(&name, &Request::QuitForUpdate, wait), Delivery::Acknowledged);
    let link = Request::Open("beebeeb://settings/security".into());
    assert_eq!(send(&name, &link, wait), Delivery::Acknowledged);
    assert_eq!(*seen.lock().unwrap(), vec![Request::Show, Request::QuitForUpdate, link]);
}

/// Serving the name is being the instance: a second server is refused, and the name is
/// free again once the first is dropped.
#[test]
fn only_one_process_serves_a_name() {
    let name = test_name("single");
    let (handler, _) = recording(Reply::Ok);
    let first = serve(&name, REAL, handler.clone()).unwrap();
    assert!(matches!(
        serve(&name, REAL, handler.clone()),
        Err(ServeError::AlreadyRunning)
    ));
    drop(first);
    assert_eq!(
        send(&name, &Request::Show, Duration::from_millis(300)),
        Delivery::Unreachable
    );
    let again = serve(&name, REAL, handler);
    assert!(again.is_ok(), "the name is free after the server is gone");
}

/// P2-9: an instance that is there and does not take the request is "no answer", within
/// the caller's timeout; no instance at all is "unreachable".
#[test]
fn a_busy_or_silent_instance_is_no_answer_and_none_is_unreachable() {
    let name = test_name("busy");
    assert_eq!(
        send(&name, &Request::Show, Duration::from_millis(300)),
        Delivery::Unreachable
    );
    let (handler, seen) = recording(Reply::Busy);
    let server = serve(&name, REAL, handler).unwrap();
    assert_eq!(send(&name, &Request::Show, Duration::from_secs(5)), Delivery::NoAnswer);
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(server);

    // A handler that takes longer than the caller waits.
    let slow_name = test_name("slow");
    let slow: ControlHandler = Arc::new(|_| {
        std::thread::sleep(Duration::from_millis(1500));
        Reply::Ok
    });
    let _server = serve(&slow_name, REAL, slow).unwrap();
    let started = Instant::now();
    assert_eq!(
        send(&slow_name, &Request::Show, Duration::from_millis(300)),
        Delivery::NoAnswer
    );
    assert!(
        started.elapsed() < Duration::from_millis(1400),
        "the caller's timeout holds"
    );
}

/// Untrusted input: garbage, an unknown verb, an oversized line and a silent client get no
/// answer and reach no handler, and the channel still works afterwards.
#[test]
fn anything_that_is_not_a_request_is_ignored_and_the_channel_survives() {
    let name = test_name("garbage");
    let (handler, seen) = recording(Reply::Ok);
    let _server = serve(&name, REAL, handler).unwrap();
    let short = Duration::from_millis(400);
    for bytes in [
        &b"hello\n"[..],
        b"BBCTL1 sign-out\n",
        b"BBCTL1 open https://example.com/\n",
        b"BBCTL1 quit-for-update now\n",
        b"\0\0\0\0\n",
    ] {
        assert_eq!(
            raw_exchange(&name, bytes, short),
            None,
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }
    let long = vec![b'a'; MAX_REQUEST + 200];
    assert_eq!(raw_exchange(&name, &long, short), None, "too long");
    // Connects and says nothing: the server gives up on it after its own wait.
    let silent = open_client(&name).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    drop(silent);
    assert!(seen.lock().unwrap().is_empty(), "no handler ran");
    assert_eq!(
        send(&name, &Request::Show, Duration::from_secs(5)),
        Delivery::Acknowledged
    );
    assert_eq!(*seen.lock().unwrap(), vec![Request::Show]);
}

/// A pipe somebody made before the app: `sddl` is its security descriptor. It takes one
/// client, keeps what that client wrote, and answers a forged `ok`.
struct Squatter {
    name: String,
    received: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Squatter {
    fn received(&self) -> Vec<u8> {
        self.received.lock().unwrap().clone()
    }
}

impl Drop for Squatter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let Some(worker) = self.worker.take() else { return };
        let deadline = Instant::now() + Duration::from_secs(3);
        while !worker.is_finished() && Instant::now() < deadline {
            drop(open_client(&self.name));
            std::thread::sleep(Duration::from_millis(20));
        }
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
}

/// `Err`: the Win32 error of creating the pipe (a descriptor this user may not set).
fn squat(name: &str, sddl: &str) -> Result<Squatter, u32> {
    let sddl_w = wide(sddl);
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `sddl_w` is NUL-terminated and outlives the call; `descriptor` is a live local
    // that receives a LocalAlloc'd descriptor on success, freed below.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_w.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    assert!(ok != 0 && !descriptor.is_null(), "{sddl} is SDDL");
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let name_w = wide(name);
    // SAFETY: `name_w` is NUL-terminated; `attributes` and its descriptor are alive for the
    // call, which copies the descriptor onto the new pipe object.
    let pipe = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            1024,
            1024,
            0,
            &attributes,
        )
    };
    let error = last_error();
    // SAFETY: allocated by the conversion above, freed exactly once.
    unsafe { LocalFree(descriptor) };
    if pipe == INVALID_HANDLE_VALUE || pipe.is_null() {
        return Err(error);
    }
    let pipe = Handle(pipe as isize);
    let received = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (sink, thread_stop) = (received.clone(), stop.clone());
    let worker = std::thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            // SAFETY: `pipe` is this thread's live pipe handle; no OVERLAPPED.
            let connected =
                unsafe { ConnectNamedPipe(pipe.raw(), ptr::null_mut()) } != 0 || last_error() == ERROR_PIPE_CONNECTED;
            if connected && !thread_stop.load(Ordering::SeqCst) {
                let line = read_until(&pipe, MAX_REQUEST, Instant::now() + Duration::from_millis(600), |b| {
                    b.contains(&b'\n')
                });
                if let Some(line) = line {
                    sink.lock().unwrap().extend_from_slice(&line);
                    write_all(&pipe, b"ok\n");
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            // SAFETY: `pipe` is this thread's live pipe handle.
            unsafe { DisconnectNamedPipe(pipe.raw()) };
        }
    });
    Ok(Squatter {
        name: name.to_owned(),
        received,
        stop,
        worker: Some(worker),
    })
}

/// Task 2455, the review's scenario as far as one account can stage it: somebody created
/// the app's pipe name FIRST, with an access list that lets everyone in, and answers `ok`
/// to anything. A launch must not take that for the running Beebeeb: it writes nothing (no
/// verb, no link) and reports the channel unreachable, and the app cannot serve the name.
#[test]
fn a_pipe_that_admits_everyone_is_not_the_running_app_and_gets_no_request() {
    let name = test_name("squat-open");
    let sid = current_user_sid().unwrap();
    // Everyone (WD) and this user: the squatter lets its victim connect.
    let squatter = squat(&name, &format!("D:(A;;GA;;;WD)(A;;GA;;;{sid})")).unwrap();
    let (handler, _) = recording(Reply::Ok);
    assert!(
        matches!(serve(&name, REAL, handler), Err(ServeError::AlreadyRunning)),
        "the name is taken: the app is not the instance"
    );
    let link = Request::Open("beebeeb://open/files?folder=Secret_1".into());
    for request in [link, Request::QuitForUpdate, Request::Show] {
        assert_eq!(
            send(&name, &request, Duration::from_secs(2)),
            Delivery::Unreachable,
            "{request:?}"
        );
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        String::from_utf8_lossy(&squatter.received()),
        "",
        "the squatter was handed a request"
    );
}

/// What a launch reads about the app's OWN pipe is exactly what `control::check_server`
/// asks for: this user's process, this user as the owner (also in an elevated process, whose
/// default owner is the Administrators group), and one entry that admits this user.
#[test]
fn the_apps_own_pipe_reads_back_as_this_users_only() {
    let name = test_name("facts");
    let (handler, _) = recording(Reply::Ok);
    let _server = serve(&name, REAL, handler).unwrap();
    let sid = current_user_sid().unwrap();
    let client = open_client(&name).unwrap();
    let (facts, server) = server_facts(&client).unwrap();
    assert_eq!(server, std::process::id());
    assert_eq!(
        facts,
        ServerFacts {
            process_user: sid.clone(),
            owner: sid.clone(),
            access: Some(vec![AccessEntry {
                allows: true,
                sid: sid.clone()
            }]),
        }
    );
    assert_eq!(control::check_server(&sid, &facts), Ok(()));
    assert_eq!(verify_server(&client), Ok(std::process::id()));
    assert_eq!(process_user_sid(std::process::id()), Ok(sid));
    assert!(
        process_user_sid(0).is_err(),
        "a process that cannot be opened is an error"
    );
}

/// The other descriptors one account can stage, each read back from the real pipe and each
/// refused for its own reason, with nothing written: no access list at all; a denial beside
/// the user's entry; and an owner that is not this user (the test needs an elevated administrator,
/// as on the build VM, and FAILS anywhere else: the case is never skipped). The last is the nearest a single account gets to
/// "another user created the pipe".
#[test]
fn every_other_descriptor_is_refused_for_its_reason_and_gets_no_request() {
    use crate::shell::control::NotOurs;
    let sid = current_user_sid().unwrap();
    let cases = [
        (
            "null-dacl",
            format!("O:{sid}D:NO_ACCESS_CONTROL"),
            NotOurs::NoAccessList,
        ),
        (
            "extra-entry",
            format!("O:{sid}D:P(A;;GA;;;{sid})(A;;GR;;;AU)"),
            NotOurs::AccessListAdmitsOthers,
        ),
        (
            "denial",
            format!("O:{sid}D:P(D;;GA;;;AN)(A;;GA;;;{sid})"),
            NotOurs::AccessListAdmitsOthers,
        ),
        ("owner", format!("O:BAD:P(A;;GA;;;{sid})"), NotOurs::OwnedByAnotherUser),
    ];
    let mut checked = 0;
    for (what, sddl, reason) in cases {
        let name = test_name(&format!("squat-{what}"));
        let squatter = match squat(&name, &sddl) {
            Ok(squatter) => squatter,
            // No case is skipped. The `owner` case needs an account that may make the
            // Administrators group an owner (an elevated administrator, as on the build VM);
            // anywhere else it fails here with ERROR_INVALID_OWNER (1307), and so does the test.
            Err(error) => panic!("{what}: the pipe could not be made: Win32 error {error}"),
        };
        {
            let client = open_client(&name).unwrap();
            let (facts, _) = server_facts(&client).unwrap();
            assert_eq!(control::check_server(&sid, &facts), Err(reason), "{what}: {facts:?}");
            assert_eq!(verify_server(&client), Err(reason.words().to_owned()), "{what}");
        }
        assert_eq!(
            send(&name, &Request::QuitForUpdate, Duration::from_secs(2)),
            Delivery::Unreachable,
            "{what}"
        );
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(String::from_utf8_lossy(&squatter.received()), "", "{what}");
        checked += 1;
        eprintln!("2455-CASE-RAN: {what}");
    }
    assert_eq!(checked, 4, "every descriptor was checked, the owner case too");
}

/// A panic in the handler is a "busy" answer, not the end of the channel.
#[test]
fn a_panicking_handler_does_not_end_the_channel() {
    let name = test_name("panic");
    let calls = Arc::new(Mutex::new(0));
    let counter = calls.clone();
    let handler: ControlHandler = Arc::new(move |_| {
        let mut calls = counter.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *calls += 1;
        if *calls == 1 {
            drop(calls);
            panic!("the handler");
        }
        Reply::Ok
    });
    let _server = serve(&name, REAL, handler).unwrap();
    assert_eq!(send(&name, &Request::Show, Duration::from_secs(5)), Delivery::NoAnswer);
    assert_eq!(
        send(&name, &Request::Show, Duration::from_secs(5)),
        Delivery::Acknowledged
    );
}
