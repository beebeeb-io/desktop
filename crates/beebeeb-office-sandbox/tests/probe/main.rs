//! The sandbox tests' helper. It TRIES what a confined helper must not be able to do and answers with the result,
//! one line per request on its standard output. It prints codes and counts only.
//!
//! Requests (one per line on standard input):
//!   ping                 -> pong
//!   connect <ip:port>    -> ok | err <system code> | err network-start <system code>
//!   job                  -> job flags=<hex> processes=<n> memory=<bytes> ui=<hex> | err <system code>
//!   write <path>         -> ok | err <system code>      (creates a new file and writes 5 bytes)
//!   writeown <name>      -> own ok=<n> codes=<...>       (the same, as <name>.0/.1/.2 in the folders its own
//!                                                         LOCALAPPDATA, TEMP and TMP variables name)
//!   read <path>          -> ok <bytes read> | err <system code>
//!   spawn <program>      -> ok | err <system code>      (starts it with `/c exit`, waits for it)
//!   alloc <mebibytes>    -> ok | err alloc              (reserves and touches every page)
//!   env                  -> env <names, comma-separated, sorted>
//!   setevent <handle>    -> ok | err <system code>      (Windows: signals the event behind a handle value)
//!   hang                 -> hanging, then sleeps for ever
//!   spin                 -> spinning, then burns one core for ever
//!   exit <code>          -> exits with that code
//!
//! `beebeeb-sandbox-probe parent sandboxed|plain` is the "parent that dies" of the tests: it starts a second
//! probe (confined through the crate, or plainly), makes it hang, prints `child <process id>` and sleeps.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

fn code(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => format!("err {code}"),
        None => format!("err kind-{:?}", error.kind()),
    }
}

/// Starts the system's network library the way every program has to before its first socket, through the system
/// call itself: the standard library does the same but panics when it fails, and the answer wanted is the code.
#[cfg(windows)]
fn network_start() -> Result<(), String> {
    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn WSAStartup(version: u16, data: *mut u64) -> i32;
    }
    // Larger than the structure the call fills in (about 400 bytes), and aligned for it.
    let mut data = [0u64; 128];
    // SAFETY: version 2.2 and a writable, aligned buffer larger than the structure the call fills in.
    match unsafe { WSAStartup(0x0202, data.as_mut_ptr()) } {
        0 => Ok(()),
        code => Err(format!("err network-start {code}")),
    }
}

#[cfg(not(windows))]
fn network_start() -> Result<(), String> {
    Ok(())
}

fn connect(target: &str) -> String {
    if let Err(answer) = network_start() {
        return answer;
    }
    let Ok(address) = target.parse::<SocketAddr>() else {
        return "err parse".into();
    };
    match TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
        Ok(_) => "ok".into(),
        Err(error) => code(&error),
    }
}

fn write(path: &str) -> String {
    let attempt = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(b"probe"));
    match attempt {
        Ok(()) => "ok".into(),
        Err(error) => code(&error),
    }
}

fn write_own(name: &str) -> String {
    let mut written = 0;
    let mut codes = Vec::new();
    for (index, variable) in ["LOCALAPPDATA", "TEMP", "TMP"].into_iter().enumerate() {
        let answer = match std::env::var_os(variable) {
            Some(folder) => write(
                &std::path::Path::new(&folder)
                    .join(format!("{name}.{index}"))
                    .to_string_lossy(),
            ),
            None => "err unset".to_string(),
        };
        if answer == "ok" {
            written += 1;
        } else {
            codes.push(answer.trim_start_matches("err ").to_string());
        }
    }
    format!("own ok={written} codes={}", codes.join(","))
}

fn read(path: &str) -> String {
    match std::fs::read(path) {
        Ok(bytes) => format!("ok {}", bytes.len()),
        Err(error) => code(&error),
    }
}

/// Starts `program` with `/c exit` and waits for it. On Windows through the system call itself, so the answer
/// is the system's own code and nothing the standard library puts in front of it.
#[cfg(windows)]
fn spawn(program: &str) -> String {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
    };
    let application: Vec<u16> = program.encode_utf16().chain([0]).collect();
    let mut command: Vec<u16> = format!("\"{program}\" /c exit").encode_utf16().chain([0]).collect();
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut created = PROCESS_INFORMATION::default();
    // SAFETY: `application` and `command` are terminated UTF-16 buffers (`command` writable, as the call
    // requires); `startup` is initialised with its size; `created` receives the handles; no handle is inherited.
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            DETACHED_PROCESS,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut created,
        )
    };
    if ok == 0 {
        return code(&std::io::Error::last_os_error());
    }
    // SAFETY: both handles were just returned open by the call above and are closed once.
    unsafe {
        WaitForSingleObject(created.hProcess, 10_000);
        CloseHandle(created.hThread);
        CloseHandle(created.hProcess);
    }
    "ok".into()
}

#[cfg(not(windows))]
fn spawn(program: &str) -> String {
    match Command::new(program)
        .args(["/c", "exit"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let _ = child.wait();
            "ok".into()
        }
        Err(error) => code(&error),
    }
}

fn alloc(mebibytes: &str) -> String {
    let Ok(mebibytes) = mebibytes.parse::<usize>() else {
        return "err parse".into();
    };
    let bytes = mebibytes * 1024 * 1024;
    let mut block: Vec<u8> = Vec::new();
    if block.try_reserve_exact(bytes).is_err() {
        return "err alloc".into();
    }
    block.resize(bytes, 0);
    for page in block.chunks_mut(4096) {
        page[0] = 1;
    }
    let touched: usize = block.iter().step_by(4096).map(|&b| b as usize).sum();
    if touched == bytes.div_ceil(4096) {
        "ok".into()
    } else {
        "err touch".into()
    }
}

fn env() -> String {
    let mut names: Vec<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('='))
        .collect();
    names.sort();
    format!("env {}", names.join(","))
}

#[cfg(windows)]
fn set_event(value: &str) -> String {
    let Ok(value) = usize::from_str_radix(value, 16) else {
        return "err parse".into();
    };
    // SAFETY: `SetEvent` takes any handle value; one that is not an open event handle of this process makes the
    // call fail with an error code, which is the answer the test wants.
    let ok =
        unsafe { windows_sys::Win32::System::Threading::SetEvent(value as windows_sys::Win32::Foundation::HANDLE) };
    if ok != 0 {
        "ok".into()
    } else {
        code(&std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn set_event(_value: &str) -> String {
    "err unsupported".into()
}

/// The limits of the job this process runs in, read by the process itself.
#[cfg(windows)]
fn job() -> String {
    use windows_sys::Win32::System::JobObjects::{
        JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicUIRestrictions,
        JobObjectExtendedLimitInformation, QueryInformationJobObject,
    };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    let mut ui = JOBOBJECT_BASIC_UI_RESTRICTIONS::default();
    // SAFETY: a null job handle means "the job of this process"; each buffer is writable for the size passed,
    // which is the size of the class asked for.
    let ok = unsafe {
        QueryInformationJobObject(
            std::ptr::null_mut(),
            JobObjectExtendedLimitInformation,
            (&raw mut limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            std::ptr::null_mut(),
        ) != 0
            && QueryInformationJobObject(
                std::ptr::null_mut(),
                JobObjectBasicUIRestrictions,
                (&raw mut ui).cast(),
                size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
                std::ptr::null_mut(),
            ) != 0
    };
    if !ok {
        return code(&std::io::Error::last_os_error());
    }
    format!(
        "job flags={:#x} processes={} memory={} ui={:#x}",
        limits.BasicLimitInformation.LimitFlags,
        limits.BasicLimitInformation.ActiveProcessLimit,
        limits.JobMemoryLimit,
        ui.UIRestrictionsClass
    )
}

#[cfg(not(windows))]
fn job() -> String {
    "err unsupported".into()
}

fn sleep_for_ever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn hang_child(mut to_child: impl Write, from_child: impl Read) -> bool {
    if to_child.write_all(b"hang\n").and_then(|()| to_child.flush()).is_err() {
        return false;
    }
    let mut line = String::new();
    let read = BufReader::new(from_child).read_line(&mut line);
    // The pipe to the child stays open (it is leaked on purpose): the child must not end because a pipe closed.
    std::mem::forget(to_child);
    read.is_ok() && line.trim() == "hanging"
}

fn parent(mode: &str) -> ! {
    let own = std::env::current_exe().expect("own path");
    match mode {
        "sandboxed" => {
            let sandbox =
                beebeeb_office_sandbox::Sandbox::new(beebeeb_office_sandbox::Limits::default()).expect("limits");
            match sandbox.spawn(&own) {
                Ok(mut child) => {
                    let pipes = child.take_pipes().expect("pipes");
                    let hanging = hang_child(pipes.to_helper, pipes.from_helper);
                    println!("{} {}", if hanging { "child" } else { "error not-hanging" }, child.id());
                    sleep_for_ever();
                }
                Err(error) => {
                    println!("error {error}");
                    std::process::exit(2);
                }
            }
        }
        "plain" => {
            // Never waited for on purpose: this process is the parent that gets killed, the child is the orphan.
            #[allow(clippy::zombie_processes)]
            let mut child = Command::new(&own)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("plain child");
            let hanging = hang_child(child.stdin.take().expect("stdin"), child.stdout.take().expect("stdout"));
            println!("{} {}", if hanging { "child" } else { "error not-hanging" }, child.id());
            sleep_for_ever();
        }
        _ => std::process::exit(64),
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) == Some("parent") {
        parent(arguments.get(2).map(String::as_str).unwrap_or(""));
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let (request, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let answer = match request {
            "ping" => "pong".to_string(),
            "connect" => connect(rest),
            "write" => write(rest),
            "writeown" => write_own(rest),
            "read" => read(rest),
            "spawn" => spawn(rest),
            "alloc" => alloc(rest),
            "env" => env(),
            "setevent" => set_event(rest),
            "job" => job(),
            "hang" | "spin" => {
                let _ = writeln!(stdout, "{}", if request == "hang" { "hanging" } else { "spinning" });
                let _ = stdout.flush();
                if request == "hang" {
                    sleep_for_ever();
                }
                loop {
                    std::hint::spin_loop();
                }
            }
            "exit" => std::process::exit(rest.parse().unwrap_or(64)),
            _ => "err unknown".to_string(),
        };
        if writeln!(stdout, "{answer}").and_then(|()| stdout.flush()).is_err() {
            break;
        }
    }
}
