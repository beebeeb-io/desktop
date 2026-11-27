//! The probe on macOS: how it starts (confined like every helper, or as one of the tests' controls and liars), and
//! the requests only macOS has.
//!
//! Started with no argument the probe confines itself first, as the Office helper does. The tests' other starts:
//!   plain-control   the control of every claim: NOT confined (started by the test itself, never through the crate)
//!   --image         the liar: sends the "confined" handshake on its control socket without confining itself
//!   nested          puts a profile that allows everything on itself, then confines: the system refuses the second
//!                   profile and the probe ends with the crate's exit code for that (prints `confined-twice` if not)
//!   in-sandbox      puts the same profile on itself, then starts a probe through the crate, as an app inside the
//!                   App Sandbox would: prints `refused <why>` or `started`
//!
//! Requests:
//!   force-write <wire> <len>  -> reprotect ok|err <code>, then a write into the read-only mapping (which ends the
//!                                probe with a bus error; `err wrote` if the write went through)
//!   fd <n>                    -> open | closed          (whether descriptor n is open in the probe)
//!   fds                       -> fds <open descriptors below 256, comma-separated>
//!   alloc-chunks <mebibytes>  -> ok                     (allocates 1 MiB at a time, touches every page, keeps all)
//!   reserve <mebibytes>       -> ok | err alloc         (asks for one allocation and touches none of it)
//!   shm-create                -> ok | err <code>        (makes a named shared-memory object, removes it again)
//!   signal-parent             -> ok | err <code>        (signal 0 to the process that started the probe)
//!   second-profile            -> ok | err refused       (applies a profile that allows everything)

use std::ffi::{c_char, c_int, c_void};
use std::io::Write;

use beebeeb_office_sandbox::{Access, Granted, helper};

unsafe extern "C" {
    fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
    fn mprotect(address: *mut c_void, len: usize, protection: c_int) -> c_int;
    fn shm_open(name: *const c_char, flags: c_int, ...) -> c_int;
    fn shm_unlink(name: *const c_char) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
    fn getppid() -> c_int;
    fn send(socket: c_int, bytes: *const c_void, len: usize, flags: c_int) -> isize;
    fn sandbox_init_with_parameters(
        profile: *const c_char,
        flags: u64,
        parameters: *const *const c_char,
        error: *mut *mut c_char,
    ) -> c_int;
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Puts a profile that allows everything on this process. True when the system took it.
fn allow_everything() -> bool {
    let parameters: [*const c_char; 1] = [std::ptr::null()];
    let mut error: *mut c_char = std::ptr::null_mut();
    // SAFETY: a terminated profile text, a parameter list of one null entry, a valid place for the error text (which
    // is left to the system: the probe ends soon).
    unsafe {
        sandbox_init_with_parameters(
            c"(version 1)(allow default)".as_ptr(),
            0,
            parameters.as_ptr(),
            &raw mut error,
        ) == 0
    }
}

/// The probe's start. Returns to serve requests, except in the modes that end by themselves.
pub fn start(mode: Option<&str>) {
    match mode {
        // The stand-in for the app, and the tests' control: neither is a helper.
        Some("parent" | "plain-control") => {}
        Some("--image") => {
            // The crate's handshake, word for word, from a process that did not confine itself.
            let mut claim = Vec::from(*b"BBSBCNF1");
            for word in [1u32, 0, 0, 1] {
                claim.extend_from_slice(&word.to_le_bytes());
            }
            // SAFETY: `claim` is readable for its length; descriptor 3 is the control socket, or the call fails.
            unsafe { send(3, claim.as_ptr().cast(), claim.len(), 0) };
        }
        Some("nested") => {
            if !allow_everything() {
                std::process::exit(65);
            }
            helper::confine();
            println!("confined-twice");
            std::process::exit(0);
        }
        Some("in-sandbox") => {
            if !allow_everything() {
                std::process::exit(65);
            }
            let own = std::env::current_exe().expect("own path");
            let sandbox =
                beebeeb_office_sandbox::Sandbox::new(beebeeb_office_sandbox::Limits::default()).expect("limits");
            match sandbox.spawn(&own) {
                Ok(_) => println!("started"),
                Err(error) => println!("refused {error}"),
            }
            std::process::exit(0);
        }
        _ => helper::confine(),
    }
}

fn force_write(rest: &str) -> String {
    let Some([wire, len]) = super::numbers::<2>(rest) else {
        return "err parse".into();
    };
    let mapped = match helper::map(Granted { wire, len }, Access::Read) {
        Ok(mapped) => mapped,
        Err(error) => return super::refusal(error),
    };
    let address = mapped.as_slice().as_ptr().cast_mut();
    // SAFETY: the mapping's own address and length; the call changes no memory, only (if the system allowed it) its
    // protection.
    let reprotected = unsafe { mprotect(address.cast(), mapped.as_slice().len(), 1 | 2) };
    let first = if reprotected == 0 {
        "reprotect ok".to_string()
    } else {
        format!("reprotect err {}", errno())
    };
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{first}");
    let _ = stdout.flush();
    // SAFETY: none: this is the attempt the sandbox must stop. The address is inside a live read-only mapping, so
    // the write either faults (the claim) or lands in the app's section (the failure the test would then see).
    unsafe { std::ptr::write_volatile(address, 0x66) };
    "err wrote".into()
}

fn is_open(fd: c_int) -> bool {
    // SAFETY: asks for a descriptor's flags; a closed one is an error, which is the answer.
    unsafe { fcntl(fd, 1) != -1 }
}

fn alloc_chunks(rest: &str) -> String {
    let Ok(mebibytes) = rest.parse::<usize>() else {
        return "err parse".into();
    };
    let mut kept: Vec<Vec<u8>> = Vec::new();
    for _ in 0..mebibytes {
        let mut chunk = vec![0u8; 1024 * 1024];
        for page in chunk.chunks_mut(4096) {
            page[0] = 1;
        }
        kept.push(chunk);
    }
    let touched: usize = kept.iter().map(|chunk| chunk[0] as usize).sum();
    std::mem::forget(kept);
    if touched == mebibytes {
        "ok".into()
    } else {
        "err touch".into()
    }
}

fn shm_create() -> String {
    let name = std::ffi::CString::new(format!("/bb.probe.{}", std::process::id())).expect("no NUL");
    // SAFETY: a terminated name; create-or-open, read-write; the mode is one promoted integer.
    let fd = unsafe { shm_open(name.as_ptr(), 2 | 0x200, 0o600 as c_int) };
    if fd == -1 {
        return format!("err {}", errno());
    }
    // SAFETY: the descriptor and the name just made.
    unsafe {
        close(fd);
        shm_unlink(name.as_ptr());
    }
    "ok".into()
}

/// The answer to a request only macOS has.
pub fn request(request: &str, rest: &str) -> String {
    match request {
        "force-write" => force_write(rest),
        "fd" => match rest.parse::<c_int>() {
            Ok(fd) if is_open(fd) => "open".into(),
            Ok(_) => "closed".into(),
            Err(_) => "err parse".into(),
        },
        "fds" => {
            let open: Vec<String> = (0..256).filter(|fd| is_open(*fd)).map(|fd| fd.to_string()).collect();
            format!("fds {}", open.join(","))
        }
        "alloc-chunks" => alloc_chunks(rest),
        "reserve" => match rest.parse::<usize>() {
            Ok(mebibytes) if Vec::<u8>::new().try_reserve_exact(mebibytes * 1024 * 1024).is_ok() => "ok".into(),
            Ok(_) => "err alloc".into(),
            Err(_) => "err parse".into(),
        },
        "shm-create" => shm_create(),
        // SAFETY: signal 0 only asks whether a signal could be sent.
        "signal-parent" => match unsafe { kill(getppid(), 0) } {
            0 => "ok".into(),
            _ => format!("err {}", errno()),
        },
        "second-profile" => if allow_everything() { "ok" } else { "err refused" }.into(),
        _ => "err unknown".to_string(),
    }
}
