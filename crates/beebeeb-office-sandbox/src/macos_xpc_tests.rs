//! The app's side of a helper that is an XPC service, held with a fake service: one document at a time, checked
//! before any grant, ended by the app. And, with the system's own objects, what a grant carries.

use std::sync::atomic::AtomicUsize;

use super::super::{ChildInner, SectionInner, service};
use super::*;
use crate::Access;

/// What a fake service does and what was done to it.
#[derive(Debug)]
struct Script {
    answer: Option<(Status, c_int)>,
    facts: ServiceFacts,
    /// How long after the kill its process is gone.
    dies_after: Duration,
    memory_bytes: AtomicU64,
    kills: AtomicUsize,
    killed_at: Mutex<Option<Instant>>,
    /// Gone without the app's kill (its own memory watch, a crash).
    ended_itself: AtomicBool,
    grants: Mutex<Vec<(u64, u64, bool, usize, usize)>>,
}

#[derive(Debug)]
struct Fake(Arc<Script>);

impl Link for Fake {
    fn start(&self, stdin: &OwnedFd, stdout: &OwnedFd, memory_bytes: u64, _: Duration) -> Option<(Status, c_int)> {
        assert!(stdin.as_raw_fd() > 2 && stdout.as_raw_fd() > 2);
        self.0.memory_bytes.store(memory_bytes, Ordering::SeqCst);
        self.0.answer
    }

    fn facts(&self, _pid: c_int, status: Option<Status>) -> ServiceFacts {
        ServiceFacts {
            facts: Facts {
                status,
                ..self.0.facts.facts
            },
            ..self.0.facts
        }
    }

    fn grant(&self, wire: u64, len: u64, writable: bool, region: *mut u8, span: usize) -> Result<(), i32> {
        lock(&self.0.grants).push((wire, len, writable, region as usize, span));
        Ok(())
    }

    fn kill(&self) {
        self.0.kills.fetch_add(1, Ordering::SeqCst);
        lock(&self.0.killed_at).get_or_insert_with(Instant::now);
    }

    fn ended_within(&self, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        loop {
            let gone = self.0.ended_itself.load(Ordering::SeqCst)
                || lock(&self.0.killed_at).is_some_and(|at| at.elapsed() >= self.0.dies_after);
            if gone || Instant::now() >= deadline {
                return gone;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn app_sandbox() -> Status {
    Status {
        state: STATE_APP_SANDBOX,
        profile_version: PROFILE_VERSION,
    }
}

/// The facts of a service in an App Sandbox with the one entitlement, as measured.
fn vouched() -> ServiceFacts {
    let asked = APP_SANDBOX_DENIES.len() + HOME_CHECKS.len();
    ServiceFacts {
        facts: Facts {
            status: Some(app_sandbox()),
            sandboxed: true,
            denied: APP_SANDBOX_DENIES.len(),
            allowed: MUST_BE_ALLOWED.len(),
        },
        app_denied: asked,
        app_asked: asked,
        signature_valid: true,
        entitlements: Some((1, true)),
    }
}

fn script(facts: ServiceFacts) -> Arc<Script> {
    Arc::new(Script {
        answer: Some((app_sandbox(), 4242)),
        facts,
        dies_after: Duration::ZERO,
        memory_bytes: AtomicU64::new(0),
        kills: AtomicUsize::new(0),
        killed_at: Mutex::new(None),
        ended_itself: AtomicBool::new(false),
        grants: Mutex::new(Vec::new()),
    })
}

/// A service slot of the test's own, so no test waits for another's.
fn slot() -> &'static Slot {
    Box::leak(Box::new(Slot::new()))
}

fn started(script: &Arc<Script>, slot: &'static Slot) -> Result<(ServiceChild, Pipes), SandboxError> {
    let guard = slot.take(Duration::ZERO).expect("the service is free");
    start_service(
        Box::new(Fake(Arc::clone(script))),
        guard,
        Limits { memory_bytes: 3 << 20 },
    )
}

/// (c) One document per process: while one helper runs a second start waits and is then turned away; once the
/// first is closed, and its process is gone, the second gets the service.
#[test]
fn a_second_document_waits_until_the_first_is_closed_and_its_process_is_gone() {
    let slot = slot();
    let first = Arc::new(Script {
        dies_after: Duration::from_millis(60),
        ..Arc::into_inner(script(vouched())).unwrap()
    });
    let (child, _pipes) = started(&first, slot).unwrap();
    assert_eq!(child.id(), 4242);
    assert_eq!(
        first.memory_bytes.load(Ordering::SeqCst),
        3 << 20,
        "its memory limit went with the pipes"
    );

    let waited = Instant::now();
    assert!(
        slot.take(Duration::from_millis(40)).is_none(),
        "turned away while the first runs"
    );
    assert!(waited.elapsed() >= Duration::from_millis(40), "after waiting for it");
    assert_eq!(
        first.kills.load(Ordering::SeqCst),
        0,
        "the first document was left alone"
    );

    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        drop(child);
    });
    let second = slot
        .take(Duration::from_secs(5))
        .expect("free once the first is closed");
    let killed_at = lock(&first.killed_at).expect("closing the document ended its process");
    assert!(
        killed_at.elapsed() >= first.dies_after,
        "the service was free only after the first process was gone"
    );
    assert_eq!(first.kills.load(Ordering::SeqCst), 1);
    closer.join().unwrap();
    drop(second);
    assert!(
        slot.take(Duration::ZERO).is_some(),
        "the instrument can fail: a free service is taken at once"
    );
}

/// (d) One row per fact: a service the kernel or its signature does not vouch for is ended, the caller gets no
/// pipes, and the service is free for the next start. The full set is accepted.
#[test]
fn a_service_that_is_not_vouched_for_is_ended_before_any_grant() {
    let full = vouched();
    assert_eq!(judge_service(&full), Ok(()));
    let with = |state, profile_version| ServiceFacts {
        facts: Facts {
            status: Some(Status { state, profile_version }),
            ..full.facts
        },
        ..full
    };
    let rows: [(ServiceFacts, &str); 13] = [
        (
            with(STATE_REFUSED, PROFILE_VERSION),
            "the service reports no sandbox of its own",
        ),
        (with(9, PROFILE_VERSION), "an unknown handshake"),
        (
            with(STATE_APP_SANDBOX, PROFILE_VERSION + 1),
            "another profile than this app's",
        ),
        (
            ServiceFacts {
                facts: Facts {
                    sandboxed: false,
                    ..full.facts
                },
                ..full
            },
            "the kernel reports no sandbox",
        ),
        (
            ServiceFacts {
                signature_valid: false,
                ..full
            },
            "the service's code is not what its signature covers",
        ),
        (
            ServiceFacts {
                entitlements: None,
                ..full
            },
            "the service's signature carries other entitlements than the App Sandbox alone",
        ),
        (
            // The sandbox and one more (a network entitlement, say).
            ServiceFacts {
                entitlements: Some((2, true)),
                ..full
            },
            "the service's signature carries other entitlements than the App Sandbox alone",
        ),
        (
            // One entitlement that is not the sandbox.
            ServiceFacts {
                entitlements: Some((1, false)),
                ..full
            },
            "the service's signature carries other entitlements than the App Sandbox alone",
        ),
        (
            ServiceFacts {
                facts: Facts {
                    allowed: 0,
                    ..full.facts
                },
                ..full
            },
            "the kernel's answers are not a confined helper's",
        ),
        (
            ServiceFacts {
                app_denied: full.app_asked - 1,
                ..full
            },
            "the kernel allows what the App Sandbox denies",
        ),
        (
            // The user's home could not be named, so nothing was asked about it.
            ServiceFacts {
                app_denied: 0,
                app_asked: 0,
                ..full
            },
            "the kernel allows what the App Sandbox denies",
        ),
        (
            // It says the deny-default profile took, and the kernel denies only what an App Sandbox denies.
            with(STATE_CONFINED, PROFILE_VERSION),
            "the kernel allows what the profile denies",
        ),
        (
            ServiceFacts {
                facts: Facts {
                    status: None,
                    ..full.facts
                },
                ..full
            },
            "no handshake",
        ),
    ];
    let mut refused = 0;
    for (facts, why) in rows {
        assert_eq!(judge_service(&facts), Err(SandboxError::NotConfined(why)));
        let slot = slot();
        let script = Arc::new(Script {
            // The handshake of the row; the last row is a service that never answered.
            answer: facts.facts.status.map(|status| (status, 4242)),
            ..Arc::into_inner(script(facts)).unwrap()
        });
        assert_eq!(started(&script, slot).err(), Some(SandboxError::NotConfined(why)));
        assert_eq!(script.kills.load(Ordering::SeqCst), 1, "{why}: ended");
        assert!(lock(&script.grants).is_empty());
        assert!(slot.take(Duration::ZERO).is_some(), "{why}: the service is free again");
        refused += 1;
    }
    assert_eq!(refused, 13, "the instrument ran every row");

    // A service under the deny-default profile is held to that profile's whole list, and accepted with it.
    let profiled = ServiceFacts {
        facts: Facts {
            status: Some(Status {
                state: STATE_CONFINED,
                profile_version: PROFILE_VERSION,
            }),
            denied: MUST_BE_DENIED.len(),
            ..full.facts
        },
        ..full
    };
    assert_eq!(judge_service(&profiled), Ok(()));
}

/// (e) The app cannot read a service's memory, so what it has is its kill: the caller's own timer ends a service
/// that does not answer, and that end is told apart from one the service came to by itself (its own memory watch).
#[test]
fn the_callers_deadline_ends_a_service_and_its_own_end_is_told_apart() {
    let hung = script(vouched());
    let (child, _pipes) = started(&hung, slot()).unwrap();
    let child = ChildInner::Service(child);
    assert_eq!(
        child.wait(Duration::from_millis(20)),
        Ok(None),
        "it runs until it is ended"
    );
    child.kill().unwrap();
    let status = child
        .wait(Duration::from_secs(5))
        .unwrap()
        .expect("gone after the kill");
    assert!(status.was_killed());
    assert_eq!(hung.kills.load(Ordering::SeqCst), 1);
    assert!(
        !child.killed_for_memory(),
        "the app's memory watch does not exist for a service"
    );
    assert_eq!(
        child.handle_count(),
        Err(SandboxError::Os {
            step: "handle count",
            code: 1
        })
    );
    drop(child);
    assert_eq!(
        hung.kills.load(Ordering::SeqCst),
        1,
        "a helper that is gone is not ended twice"
    );

    let over = script(vouched());
    let (child, _pipes) = started(&over, slot()).unwrap();
    over.ended_itself.store(true, Ordering::SeqCst);
    let status = child.wait(Duration::from_secs(5)).unwrap().expect("gone by itself");
    assert!(!status.was_killed());
    assert_eq!(
        status.code(),
        UNKNOWN_EXIT_CODE,
        "the system does not report how a service ended"
    );
    assert_eq!(over.kills.load(Ordering::SeqCst), 0);
}

/// A read grant to a service carries the view that can never be written; a read-write one the section's own.
#[test]
fn a_read_grant_to_a_service_carries_the_sealed_view() {
    let script = script(vouched());
    let (child, _pipes) = started(&script, slot()).unwrap();
    let len = 3 * 16384 + 5;
    let section = SectionInner::new(len).unwrap();
    let read = child.grant(&section, Access::Read).unwrap();
    let write = child.grant(&section, Access::ReadWrite).unwrap();
    assert_eq!((read.wire, read.len, write.wire), (1, len as u64, 2));
    let grants = lock(&script.grants).clone();
    let sealed = section.sealed_view().unwrap() as usize;
    assert_eq!(
        grants,
        [
            (1, len as u64, false, sealed, 4 * 16384),
            (2, len as u64, true, section.view as usize, 4 * 16384)
        ]
    );
    assert_ne!(sealed, section.view as usize);
}

fn reprotect(view: *mut u8, len: usize) -> i32 {
    // SAFETY: a mapping of the calling test, for its length; the call changes no memory.
    if unsafe { mprotect(view.cast(), len, sys::PROT_READ | sys::PROT_WRITE) } == 0 {
        0
    } else {
        errno()
    }
}

/// The seal, with the system's own objects: the sealed view shows what the app writes, the kernel refuses to make
/// it writable (system code 13), and so does a mapping of the shared-memory object made from it, which is what a
/// service maps. The control: the object made from the section's own view maps writable.
#[test]
fn the_sealed_view_and_what_a_service_maps_of_it_cannot_be_made_writable() {
    let len = 2 * 16384 + 9;
    let mut section = SectionInner::new(len).unwrap();
    section.as_mut_slice().fill(0xA5);
    let sealed = section.sealed_view().unwrap();
    // SAFETY: the sealed view is mapped readable for the section's whole pages.
    assert_eq!(unsafe { *sealed.add(len - 1) }, 0xA5, "the same memory, not a copy");
    assert_eq!(reprotect(sealed, span(len)), 13);

    let map = |region: *mut u8| {
        let memory = shared_object(region, span(len));
        let mut view: *mut c_void = null_mut();
        // SAFETY: a shared-memory object; `view` is a valid place for the address. Released once.
        let mapped = unsafe { xpc_shmem_map(memory, &raw mut view) };
        // SAFETY: the reference `shared_object` returned.
        unsafe { xpc_release(memory) };
        assert_eq!(mapped, span(len));
        view.cast::<u8>()
    };
    let through_sealed = map(sealed);
    // SAFETY: mapped readable for `span(len)` bytes just above.
    assert_eq!(unsafe { *through_sealed.add(len - 1) }, 0xA5);
    assert_eq!(
        reprotect(through_sealed, span(len)),
        13,
        "what a service maps of a read grant"
    );
    let through_own = map(section.view);
    assert_eq!(
        reprotect(through_own, span(len)),
        0,
        "the instrument can fail: a read-write grant maps writable"
    );
    // SAFETY: mapped read-write just above; the section's own view shows the write.
    unsafe { *through_own = 0x5A };
    assert_eq!(section.as_slice()[0], 0x5A);
    // SAFETY: the two views mapped above, unmapped once.
    unsafe {
        sys::munmap(through_sealed.cast(), span(len));
        sys::munmap(through_own.cast(), span(len));
    }
}

/// The service's side of a grant, with the system's own objects: it maps once, a read-write mapping of a read grant
/// is refused with code 1, a length past the grant's with code 22, and what is mapped counts for its memory watch
/// until it is unmapped.
#[test]
fn a_service_maps_a_grant_once_and_never_writable_when_it_was_sent_read_only() {
    let len = 16384 + 100;
    let mut section = SectionInner::new(len).unwrap();
    section.as_mut_slice().fill(7);
    let sealed = section.sealed_view().unwrap();
    let wait = Duration::from_millis(40);
    let wire = 0xD82_0000u64;
    let refused = |code| Some(SandboxError::Os { step: "map", code });

    service::arrive(wire, false, shared_object(sealed, span(len)));
    let before = service::mapped_bytes();
    let mapped = service::map_for_test(wire, len, Access::Read, wait).unwrap();
    assert_eq!(mapped.as_slice().iter().filter(|&&byte| byte == 7).count(), len);
    assert_eq!(service::mapped_bytes() - before, span(len) as u64);
    assert_eq!(
        service::map_for_test(wire, len, Access::Read, wait).err(),
        refused(2),
        "a grant maps once"
    );
    drop(mapped);
    assert_eq!(service::mapped_bytes(), before);

    service::arrive(wire + 1, false, shared_object(sealed, span(len)));
    assert_eq!(
        service::map_for_test(wire + 1, len, Access::ReadWrite, wait).err(),
        refused(1)
    );
    assert_eq!(
        service::map_for_test(wire + 1, len, Access::Read, wait).err(),
        refused(2),
        "used up by the refusal"
    );

    service::arrive(wire + 2, true, shared_object(section.view, span(len)));
    assert_eq!(
        service::map_for_test(wire + 2, span(len) + 1, Access::Read, wait).err(),
        refused(22)
    );

    service::arrive(wire + 3, true, shared_object(section.view, span(len)));
    let mut writable = service::map_for_test(wire + 3, len, Access::ReadWrite, wait).unwrap();
    writable.as_mut_slice().expect("a read-write grant")[len - 1] = 9;
    drop(writable);
    assert_eq!(
        section.as_slice()[len - 1],
        9,
        "the helper's write is in the app's section"
    );

    // A read-write grant mapped for reading only gives no writable bytes and cannot be made writable by asking.
    service::arrive(wire + 4, true, shared_object(section.view, span(len)));
    let mut read_only = service::map_for_test(wire + 4, len, Access::Read, wait).unwrap();
    assert!(read_only.as_mut_slice().is_none());
    assert_eq!(service::mapped_bytes(), before + span(len) as u64);
    drop(read_only);
}

/// The reader of a signature reads real values: this test binary runs, its code is what its signature covers, and
/// it does not carry the App Sandbox entitlement. With a handshake that claims an App Sandbox it is refused.
#[test]
fn the_signature_and_the_kernel_are_read_for_a_process_that_is_no_confined_service() {
    // SAFETY: takes no pointer.
    let me = unsafe { sys::getpid() };
    let own = signature(me).expect("the system names the code of a running process");
    assert!(own.valid);
    assert_ne!(own.entitlements, Some((1, true)));
    assert!(
        signature(1_000_000).is_none(),
        "a process that does not exist has no code"
    );

    let facts = read_service_facts(me, Some(app_sandbox()));
    assert_eq!(facts.app_asked, APP_SANDBOX_DENIES.len() + HOME_CHECKS.len());
    assert_eq!(facts.app_denied, 0, "nothing is denied to a process without a sandbox");
    assert_eq!(
        judge_service(&facts),
        Err(SandboxError::NotConfined("the kernel reports no sandbox"))
    );
    let home = home().expect("the user database names a home");
    assert!(home.starts_with(b"/") && std::path::Path::new(std::ffi::OsStr::from_bytes(&home)).is_dir());
}

/// The two lists are the deny-default profile's list, split by what an App Sandbox denies: nothing of it is left
/// out, so the difference between the two confinements is exactly [`APP_SANDBOX_ALLOWS`].
#[test]
fn the_app_sandbox_lists_split_the_profiles_list() {
    let text = |checks: &[Check]| checks.iter().map(|check| format!("{check:?}")).collect::<Vec<_>>();
    let (denies, allows, all) = (text(APP_SANDBOX_DENIES), text(APP_SANDBOX_ALLOWS), text(MUST_BE_DENIED));
    assert_eq!((denies.len(), allows.len(), all.len()), (13, 9, 22));
    assert!(all.iter().all(|check| denies.contains(check) != allows.contains(check)));
    for kept in ["network-outbound", "network-inbound", "network-bind", "\"/Users\""] {
        assert!(denies.iter().any(|check| check.contains(kept)), "{kept}");
    }
}

/// Only the executable of an XPC service's bundle names a service; every other path is a helper the app starts.
#[test]
fn only_a_path_inside_a_service_bundle_names_a_service() {
    let name = |path: &str| service_name(Path::new(path)).map(|name| name.into_string().unwrap());
    assert_eq!(
        name(
            "/Applications/Beebeeb.app/Contents/XPCServices/io.beebeeb.app.office-render.xpc/Contents/MacOS/beebeeb-office-render"
        ),
        Some("io.beebeeb.app.office-render".into())
    );
    assert_eq!(
        name("/Applications/Beebeeb.app/Contents/MacOS/beebeeb-office-render"),
        None
    );
    assert_eq!(name("/x/a.xpc/Contents/Resources/helper"), None);
    assert_eq!(name("/x/a.xpc/MacOS/helper"), None);
    assert_eq!(name("/x/.xpc/Contents/MacOS/helper"), None);
    assert_eq!(name("/x/a.app/Contents/MacOS/helper"), None);
    assert_eq!(name("helper"), None);
    assert!(!own_service(), "a test binary is no service");
    assert!(!app_is_sandboxed(), "the tests run outside a sandbox");
}

/// A helper in a mode has an argument, and a service takes none: refused before anything is connected.
#[test]
fn a_mode_with_an_argument_has_no_service_start() {
    let refused = spawn_service(
        c"io.beebeeb.test.none",
        Limits::default(),
        Mode::Image,
        crate::launch_lock(),
    );
    assert!(matches!(refused, Err(SandboxError::NotConfined(_))));
}

/// The app's log line names the confinement and what was checked, and a refusal says why; never a path.
#[test]
fn the_confinement_line_names_the_confinement_and_the_checks() {
    let accepted = confinement_line(4242, &vouched(), Ok(()));
    assert!(accepted.starts_with("office helper: XPC service pid 4242 accepted, confined by the App Sandbox alone"));
    assert!(accepted.contains("entitlements: 1 (app-sandbox: true)"), "{accepted}");
    let why = SandboxError::NotConfined("the kernel reports no sandbox");
    let refused = confinement_line(7, &vouched(), Err(why));
    assert!(
        refused.contains("pid 7 refused") && refused.contains("no sandbox"),
        "{refused}"
    );
    assert!(
        !refused.contains("/Users") && !refused.contains(".xpc"),
        "no path in the line"
    );
}
