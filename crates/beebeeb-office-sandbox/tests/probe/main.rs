//! The sandbox tests' helper. It TRIES what a confined helper must not be able to do and answers with the result,
//! one line per request on its standard output. It prints codes and counts only.
//!
//! Requests (one per line on standard input):
//!   ping                 -> pong
//!   connect <ip:port>    -> ok | err <system code> | err network-start <system code>
//!   udp-send <ip:port>   -> ok <bytes sent> | err <system code> | err network-start <system code>
//!                                                        (from a UDP socket bound to loopback, sends 5 bytes)
//!   listen <ip:port>     -> ok <port> | err <system code> | err network-start <system code>
//!                                                        (binds and listens; holds the listener until the next
//!                                                         `listen` or the end, so the test can connect to it)
//!   resolve <name>       -> ok <addresses> | err <system code> | err network-start <system code>
//!                                                        (a name lookup through the system's resolver)
//!   job                  -> job flags=<hex> processes=<n> memory=<bytes> ui=<hex> | err <system code>
//!   write <path>         -> ok | err <system code>      (creates a new file and writes 5 bytes)
//!   writeown <name>      -> own ok=<n> codes=<...>       (the same, as <name>.0/.1/.2 in the folders its own
//!                                                         LOCALAPPDATA, TEMP and TMP variables name, after
//!                                                         trying to create those folders)
//!   ownfolders           -> folders <per variable: system=0|1,ours=0|1,exists=0|1>
//!   read <path>          -> ok <bytes read> | err <system code>
//!   spawn <program>      -> ok | err <system code>      (starts it with `/c exit`, waits for it)
//!   alloc <mebibytes>    -> ok | err alloc              (reserves and touches every page)
//!   env                  -> env <names, comma-separated, sorted>
//!   setevent <handle>    -> ok | err <system code>      (Windows: signals the event behind a handle value)
//!   read-granted <wire> <len> <byte>
//!                        -> ok <bytes equal to byte> | err <system code>   (maps a grant read-only, counts)
//!   write-granted <wire> <len>
//!                        -> ok <bytes written> | err <system code>          (maps a grant read-write, writes the
//!                                                                             pattern: byte i is i % 251)
//!   map-handle <handle>  -> ok <first byte> | err <system code>  (Windows: maps the section behind a handle value
//!                                                               read-write, then writes 0x5A into its first byte)
//!   privileges           -> privileges <n> <names, comma-separated, sorted>  (Windows: those in its own token)
//!   open-afd [<name>]    -> ok | err <status>           (Windows: opens the socket driver, \Device\Afd or
//!                                                         \Device\Afd\<name>)
//!   raw-connect <ip:port> -> ok | err <step> <status>   (Windows: a TCP connect through the socket driver alone,
//!                                                         no socket library: open, bind, connect; a connect that
//!                                                         has not finished after 5 s answers `err connect timeout`)
//!   dns <name>           -> ok | err <status>           (Windows: a name lookup through the system resolver's
//!                                                         interface, which the DNS client service answers; no
//!                                                         socket library in this process)
//!   read-font            -> ok | err <system code>      (opens the system's Segoe UI font file, reads 4 bytes)
//!   read-registry <hive> <key>
//!                        -> ok | err <system code>      (Windows: opens a registry key to read; hive is hklm, hkcu
//!                                                         or hkcr)
//!   capabilities         -> capabilities <n> <identifiers, comma-separated, sorted>  (Windows: the capabilities in
//!                                                         its own token, as S-1-15-3-... identifiers)
//!   section-commit <mebibytes>
//!                        -> ok <shared commit bytes> | err <step> <system code>   (Windows: makes an unnamed section
//!                                                         backed by the paging file, maps it, touches every page,
//!                                                         unmaps it and keeps its handle; then reads its own shared
//!                                                         commit)
//!   hold-grants <wire> <len> [<wire> <len>]...
//!                        -> ok <n> | err <system code>   (maps each grant read-only and never lets one go)
//!   window               -> ok | err <step> <system code> (Windows: loads the window libraries at run time, never
//!                                                         by import, makes a top-level window and reads one screen
//!                                                         pixel)
//!   jit                  -> ok | err <system code>      (Windows: makes a page executable and writable, writes a
//!                                                         `ret` into it)
//!   bad-handle           -> ok refused <system code> | err live | err waited <hex>   (Windows: first closes the
//!                                                         value 0x7FFFFFF8, which must fail (no such handle is
//!                                                         held), then waits on it; under strict handle checks the
//!                                                         wait ends the process with 0xC0000008)
//!   strict-handles       -> ok raise=<0|1> permanent=<0|1> | err <system code>   (Windows: its own strict-handle-
//!                                                         check policy)
//!   token                -> token lpac=<0|1> rid=<hex> | err <step> <system code>   (Windows: its own token's
//!                                                         less-privileged container flag and integrity level)
//!   memory               -> memory private=<bytes> shared=<bytes> job=<bytes|none> peak-job=<bytes|none>
//!                                                        (Windows: its own commit, and its job's, if it has one)
//!   decoders             -> ok | err <step> <code>      (Windows: starts COM, Media Foundation, the AAC decoder and the
//!                                                         WIC imaging factory, as the media helper's system route does)
//!   hang                 -> hanging, then sleeps for ever
//!   spin                 -> spinning, then burns one core for ever
//!   exit <code>          -> exits with that code
//!
//! Numbers are decimal, handle values hexadecimal (as `setevent`); a grant's `wire` is decimal (it is a `u64`).
//!
//! `beebeeb-sandbox-probe parent sandboxed|plain` is the "parent that dies" of the tests: it starts a second
//! probe (confined through the crate, or plainly), makes it hang, prints `child <process id>` and sleeps.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use beebeeb_office_sandbox::{Access, Granted, SandboxError, helper};

mod hardening;

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

/// Sends 5 bytes to `target` from a UDP socket bound to loopback (never the wildcard address).
fn udp_send(target: &str) -> String {
    if let Err(answer) = network_start() {
        return answer;
    }
    let Ok(address) = target.parse::<SocketAddr>() else {
        return "err parse".into();
    };
    match UdpSocket::bind("127.0.0.1:0").and_then(|socket| socket.send_to(b"probe", address)) {
        Ok(sent) => format!("ok {sent}"),
        Err(error) => code(&error),
    }
}

/// The listener of the last `listen`, held so the test can connect to it after the answer.
static LISTENING: Mutex<Option<TcpListener>> = Mutex::new(None);

/// Binds `target` and listens (the standard library does both); answers the port it listens on.
fn listen(target: &str) -> String {
    if let Err(answer) = network_start() {
        return answer;
    }
    let Ok(address) = target.parse::<SocketAddr>() else {
        return "err parse".into();
    };
    let listener = match TcpListener::bind(address) {
        Ok(listener) => listener,
        Err(error) => return code(&error),
    };
    let port = match listener.local_addr() {
        Ok(local) => local.port(),
        Err(error) => return code(&error),
    };
    *LISTENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(listener);
    format!("ok {port}")
}

/// Looks `name` up through the system's resolver; answers the number of addresses, never the addresses.
fn resolve(name: &str) -> String {
    if let Err(answer) = network_start() {
        return answer;
    }
    match (name, 0u16).to_socket_addrs() {
        Ok(addresses) => format!("ok {}", addresses.count()),
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
            Some(folder) => {
                // A helper that wants to write would first make the folder if it is missing.
                let _ = std::fs::create_dir_all(&folder);
                write(
                    &std::path::Path::new(&folder)
                        .join(format!("{name}.{index}"))
                        .to_string_lossy(),
                )
            }
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

/// Where the three folder variables point, as yes/no answers only (never the value): under the system folder
/// (the placeholder the parent passed), holding our profile-name prefix (the container's own folder), existing.
fn own_folders() -> String {
    let system = std::env::var("SystemRoot").unwrap_or_default().to_lowercase();
    let mut parts = Vec::new();
    for variable in ["LOCALAPPDATA", "TEMP", "TMP"] {
        let value = std::env::var(variable).unwrap_or_default();
        let lower = value.to_lowercase();
        parts.push(format!(
            "{variable}:system={},ours={},exists={}",
            u8::from(!system.is_empty() && lower.starts_with(&system)),
            u8::from(lower.contains("beebeeb.office.")),
            u8::from(std::path::Path::new(&value).exists())
        ));
    }
    format!("folders {}", parts.join(" "))
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

/// Exactly `N` decimal numbers, separated by single spaces.
fn numbers<const N: usize>(rest: &str) -> Option<[u64; N]> {
    let mut out = [0u64; N];
    let mut words = rest.split(' ');
    for slot in &mut out {
        *slot = words.next()?.parse().ok()?;
    }
    words.next().is_none().then_some(out)
}

/// A refusal of the crate as an answer: the system code where there is one.
fn refusal(error: SandboxError) -> String {
    match error {
        SandboxError::Os { code, .. } => format!("err {code}"),
        other => format!("err {other:?}"),
    }
}

/// The bytes `write-granted` writes: byte `i` is `i % 251`, so a lost page or tail shows.
fn pattern(index: usize) -> u8 {
    (index % 251) as u8
}

fn read_granted(rest: &str) -> String {
    let Some([wire, len, expected]) = numbers::<3>(rest) else {
        return "err parse".into();
    };
    let Ok(expected) = u8::try_from(expected) else {
        return "err parse".into();
    };
    match helper::map(Granted { wire, len }, Access::Read) {
        Ok(mapped) => format!("ok {}", count_of(mapped.as_slice(), expected)),
        Err(error) => refusal(error),
    }
}

fn count_of(bytes: &[u8], wanted: u8) -> usize {
    bytes.iter().filter(|&&byte| byte == wanted).count()
}

fn write_granted(rest: &str) -> String {
    let Some([wire, len]) = numbers::<2>(rest) else {
        return "err parse".into();
    };
    let mut mapped = match helper::map(Granted { wire, len }, Access::ReadWrite) {
        Ok(mapped) => mapped,
        Err(error) => return refusal(error),
    };
    let Some(bytes) = mapped.as_mut_slice() else {
        return "err read-only".into();
    };
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = pattern(index);
    }
    format!("ok {}", bytes.len())
}

/// Maps the section behind a handle value read-write, reads its first byte and writes a mark there. Through the
/// system call itself: the handle is one the app holds, never one of a grant.
#[cfg(windows)]
fn map_handle(value: &str) -> String {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Memory::{FILE_MAP_READ, FILE_MAP_WRITE, MapViewOfFile, UnmapViewOfFile};
    /// What the test finds in the first byte of a section this probe could map.
    const MAPPED_MARK: u8 = 0x5A;
    let Ok(value) = usize::from_str_radix(value, 16) else {
        return "err parse".into();
    };
    // SAFETY: `MapViewOfFile` takes any handle value; one that is not an open section handle of this process makes
    // the call fail with an error code (or, under strict handle checks, ends this process), which is the answer the
    // test wants. One byte is asked for; a whole page is mapped.
    let view = unsafe { MapViewOfFile(value as HANDLE, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, 1) };
    if view.Value.is_null() {
        return code(&std::io::Error::last_os_error());
    }
    let first = view.Value.cast::<u8>();
    // SAFETY: the view is mapped read-write and at least one byte long.
    let before = unsafe { first.read_volatile() };
    // SAFETY: as above.
    unsafe { first.write_volatile(MAPPED_MARK) };
    // SAFETY: the address the call above returned, unmapped once.
    unsafe { UnmapViewOfFile(view) };
    format!("ok {before}")
}

#[cfg(not(windows))]
fn map_handle(_value: &str) -> String {
    "err unsupported".into()
}

/// The names of the privileges Windows defines, by identifier: 2 is the first, 36 the last. Built in, so that the
/// answer needs no lookup service, which a confined process might not reach.
#[cfg(windows)]
const PRIVILEGE_NAMES: [&str; 35] = [
    "SeCreateTokenPrivilege",
    "SeAssignPrimaryTokenPrivilege",
    "SeLockMemoryPrivilege",
    "SeIncreaseQuotaPrivilege",
    "SeMachineAccountPrivilege",
    "SeTcbPrivilege",
    "SeSecurityPrivilege",
    "SeTakeOwnershipPrivilege",
    "SeLoadDriverPrivilege",
    "SeSystemProfilePrivilege",
    "SeSystemtimePrivilege",
    "SeProfileSingleProcessPrivilege",
    "SeIncreaseBasePriorityPrivilege",
    "SeCreatePagefilePrivilege",
    "SeCreatePermanentPrivilege",
    "SeBackupPrivilege",
    "SeRestorePrivilege",
    "SeShutdownPrivilege",
    "SeDebugPrivilege",
    "SeAuditPrivilege",
    "SeSystemEnvironmentPrivilege",
    "SeChangeNotifyPrivilege",
    "SeRemoteShutdownPrivilege",
    "SeUndockPrivilege",
    "SeSyncAgentPrivilege",
    "SeEnableDelegationPrivilege",
    "SeManageVolumePrivilege",
    "SeImpersonatePrivilege",
    "SeCreateGlobalPrivilege",
    "SeTrustedCredManAccessPrivilege",
    "SeRelabelPrivilege",
    "SeIncreaseWorkingSetPrivilege",
    "SeTimeZonePrivilege",
    "SeCreateSymbolicLinkPrivilege",
    "SeDelegateSessionUserImpersonatePrivilege",
];

/// A privilege's name from its identifier (two 32-bit halves); an identifier Windows does not define is kept as
/// its number, never dropped.
#[cfg(windows)]
fn privilege_name(low: u32, high: u32) -> String {
    let known = if high == 0 {
        low.checked_sub(2).and_then(|slot| PRIVILEGE_NAMES.get(slot as usize))
    } else {
        None
    };
    match known {
        Some(name) => (*name).to_string(),
        None => format!("luid-{high}-{low}"),
    }
}

/// The privileges in this process's own token, removed or not, by name.
#[cfg(windows)]
fn privileges() -> String {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenPrivileges};
    /// `GetCurrentProcessToken()`: the pseudo-handle of this process's own token; nothing is opened.
    const OWN_TOKEN: HANDLE = -4isize as HANDLE;
    // A count, then 12 bytes per privilege: its identifier as two 32-bit halves, then its attributes. In u32 units.
    let mut buffer = [0u32; 1024];
    let mut returned = 0u32;
    // SAFETY: the pseudo-handle is valid for a query; the buffer is writable for the length passed and aligned for
    // the structure.
    let ok = unsafe {
        GetTokenInformation(
            OWN_TOKEN,
            TokenPrivileges,
            buffer.as_mut_ptr().cast(),
            size_of_val(&buffer) as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return code(&std::io::Error::last_os_error());
    }
    let count = (buffer[0] as usize).min((buffer.len() - 1) / 3);
    let mut names: Vec<String> = (0..count)
        .map(|index| privilege_name(buffer[1 + 3 * index], buffer[2 + 3 * index]))
        .collect();
    names.sort();
    format!("privileges {} {}", names.len(), names.join(","))
}

#[cfg(not(windows))]
fn privileges() -> String {
    "err unsupported".into()
}

/// Opens the socket driver the way socket libraries do, through the system call itself: `\Device\Afd`, or
/// `\Device\Afd\<name>` when a name is given. The answer is the call's own status.
#[cfg(windows)]
fn open_afd(name: &str) -> String {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};

    #[repr(C)]
    #[allow(dead_code)] // read by the system, not by this code
    struct CountedText {
        length: u16,
        maximum_length: u16,
        buffer: *const u16,
    }
    #[repr(C)]
    #[allow(dead_code)] // read by the system, not by this code
    struct ObjectAttributes {
        length: u32,
        root_directory: HANDLE,
        object_name: *const CountedText,
        attributes: u32,
        security_descriptor: *const c_void,
        security_quality_of_service: *const c_void,
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        #[allow(clippy::too_many_arguments)]
        fn NtCreateFile(
            handle: *mut HANDLE,
            access: u32,
            attributes: *const ObjectAttributes,
            io_status: *mut [usize; 2],
            allocation_size: *const i64,
            file_attributes: u32,
            share: u32,
            disposition: u32,
            options: u32,
            extended_attributes: *const c_void,
            extended_attributes_length: u32,
        ) -> i32;
    }
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const SHARE_READ_AND_WRITE: u32 = 0x1 | 0x2;
    const OPEN_EXISTING_ONLY: u32 = 1;
    const CASE_INSENSITIVE: u32 = 0x40;

    let path = if name.is_empty() {
        r"\Device\Afd".to_string()
    } else {
        format!(r"\Device\Afd\{name}")
    };
    let units: Vec<u16> = path.encode_utf16().collect();
    let Ok(bytes) = u16::try_from(units.len() * 2) else {
        return "err parse".into();
    };
    let object_name = CountedText {
        length: bytes,
        maximum_length: bytes,
        buffer: units.as_ptr(),
    };
    let attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(),
        object_name: &object_name,
        attributes: CASE_INSENSITIVE,
        security_descriptor: std::ptr::null(),
        security_quality_of_service: std::ptr::null(),
    };
    let mut handle: HANDLE = std::ptr::null_mut();
    let mut io_status = [0usize; 2];
    // SAFETY: `handle` and `io_status` are writable places of the sizes the call writes; `attributes` and the
    // name it points to live past the call; no allocation size and no extended attributes; open only, never create.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            SYNCHRONIZE,
            &attributes,
            &mut io_status,
            std::ptr::null(),
            0,
            SHARE_READ_AND_WRITE,
            OPEN_EXISTING_ONLY,
            0,
            std::ptr::null(),
            0,
        )
    };
    if status != 0 {
        return format!("err {status:#010x}");
    }
    // SAFETY: the handle the call above opened, closed once.
    unsafe { CloseHandle(handle) };
    "ok".into()
}

#[cfg(not(windows))]
fn open_afd(_name: &str) -> String {
    "err unsupported".into()
}

/// A TCP connect through the socket driver alone, the way a helper that skips the system's socket library would
/// make one: an endpoint opened with the driver's create packet (IPv4, stream, TCP), bound to the wildcard address,
/// then connected. No socket library is loaded or started on this path, so the refusal, if any, comes from the
/// driver or the kernel's network filter. The answer names the step and the call's own status. Layouts: the
/// driver's transport-layer mode as published in the native API headers (`AFD_OPEN_PACKET`, `AFD_BIND_INFO_TL`,
/// `AFD_CONNECT_JOIN_INFO_TL`; codes 0x12003 bind, 0x12007 connect). A plain probe connecting is what proves them.
#[cfg(windows)]
fn raw_connect(target: &str) -> String {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};

    #[repr(C)]
    #[allow(dead_code)] // read by the system, not by this code
    struct CountedText {
        length: u16,
        maximum_length: u16,
        buffer: *const u16,
    }
    #[repr(C)]
    #[allow(dead_code)] // read by the system, not by this code
    struct ObjectAttributes {
        length: u32,
        root_directory: HANDLE,
        object_name: *const CountedText,
        attributes: u32,
        security_descriptor: *const c_void,
        security_quality_of_service: *const c_void,
    }
    /// One extended attribute named `AfdOpenPacketXX` whose value is the driver's create packet.
    #[repr(C)]
    #[allow(dead_code)] // read by the system, not by this code
    struct OpenPacket {
        next_entry_offset: u32,
        flags: u8,
        name_length: u8,
        value_length: u16,
        name: [u8; 16],
        endpoint_flags: u32,
        group: u32,
        address_family: i32,
        socket_type: i32,
        protocol: i32,
        transport_name_length: u32,
        transport_name: [u16; 1],
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    #[allow(dead_code)] // read by the system, not by this code
    struct SocketAddressV4 {
        family: u16,
        port: [u8; 2],
        address: [u8; 4],
        zero: [u8; 8],
    }
    #[repr(C)]
    #[allow(dead_code)] // read by the system, not by this code
    struct BindInfo {
        share_access: u32,
        address: SocketAddressV4,
    }
    #[repr(C)]
    #[allow(dead_code)] // read by the system, not by this code
    struct ConnectInfo {
        san_active: u8,
        root_endpoint: HANDLE,
        connect_endpoint: HANDLE,
        remote: SocketAddressV4,
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        #[allow(clippy::too_many_arguments)]
        fn NtCreateFile(
            handle: *mut HANDLE,
            access: u32,
            attributes: *const ObjectAttributes,
            io_status: *mut [usize; 2],
            allocation_size: *const i64,
            file_attributes: u32,
            share: u32,
            disposition: u32,
            options: u32,
            extended_attributes: *const c_void,
            extended_attributes_length: u32,
        ) -> i32;
        #[allow(clippy::too_many_arguments)]
        fn NtDeviceIoControlFile(
            handle: HANDLE,
            event: HANDLE,
            apc_routine: *const c_void,
            apc_context: *const c_void,
            io_status: *mut [usize; 2],
            code: u32,
            input: *const c_void,
            input_length: u32,
            output: *mut c_void,
            output_length: u32,
        ) -> i32;
        fn NtCancelIoFile(handle: HANDLE, io_status: *mut [usize; 2]) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateEventW(attributes: *const c_void, manual_reset: i32, initial_state: i32, name: *const u16) -> HANDLE;
        fn WaitForSingleObject(handle: HANDLE, milliseconds: u32) -> u32;
    }
    const GENERIC_READ_WRITE: u32 = 0x8000_0000 | 0x4000_0000;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const SHARE_READ_AND_WRITE: u32 = 0x1 | 0x2;
    const OPEN_OR_CREATE: u32 = 3;
    const CASE_INSENSITIVE: u32 = 0x40;
    const PENDING: i32 = 0x103;
    const IOCTL_BIND: u32 = 0x12003;
    const IOCTL_CONNECT: u32 = 0x12007;
    const WAIT_MS: u32 = 5_000;

    let Ok(SocketAddr::V4(remote)) = target.parse::<SocketAddr>() else {
        return "err parse".into();
    };
    let v4 = |address: [u8; 4], port: u16| SocketAddressV4 {
        family: 2, // AF_INET
        port: port.to_be_bytes(),
        address,
        zero: [0; 8],
    };

    let units: Vec<u16> = r"\Device\Afd\Endpoint".encode_utf16().collect();
    let bytes = (units.len() * 2) as u16;
    let object_name = CountedText {
        length: bytes,
        maximum_length: bytes,
        buffer: units.as_ptr(),
    };
    let attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: std::ptr::null_mut(),
        object_name: &object_name,
        attributes: CASE_INSENSITIVE,
        security_descriptor: std::ptr::null(),
        security_quality_of_service: std::ptr::null(),
    };
    let mut name = [0u8; 16];
    name[..15].copy_from_slice(b"AfdOpenPacketXX");
    let packet = OpenPacket {
        next_entry_offset: 0,
        flags: 0,
        name_length: 15,
        value_length: (size_of::<OpenPacket>() - 24) as u16,
        name,
        endpoint_flags: 0,
        group: 0,
        address_family: 2, // AF_INET
        socket_type: 1,    // SOCK_STREAM
        protocol: 6,       // IPPROTO_TCP
        transport_name_length: 0,
        transport_name: [0],
    };
    let mut endpoint: HANDLE = std::ptr::null_mut();
    let mut io_status = [0usize; 2];
    // SAFETY: `endpoint` and `io_status` are writable places of the sizes the call writes; `attributes`, the name
    // it points to and `packet` live past the call; the handle is opened for overlapped use (no synchronous flag).
    let status = unsafe {
        NtCreateFile(
            &mut endpoint,
            GENERIC_READ_WRITE | SYNCHRONIZE,
            &attributes,
            &mut io_status,
            std::ptr::null(),
            0,
            SHARE_READ_AND_WRITE,
            OPEN_OR_CREATE,
            0,
            (&packet as *const OpenPacket).cast(),
            size_of::<OpenPacket>() as u32,
        )
    };
    if status != 0 {
        return format!("err open {status:#010x}");
    }

    // One request to the driver, waited for at most `WAIT_MS`; a request still running then is cancelled, and its
    // status block and event are left alive (leaked) because the driver may still write to them.
    let request = |code: u32, input: *const c_void, input_length: usize, output: *mut c_void, output_length: usize| {
        // SAFETY: an unnamed, unsecured, manual-reset event, closed below or deliberately leaked.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err("event".to_string());
        }
        let io_status = Box::into_raw(Box::new([0usize; 2]));
        // SAFETY: the endpoint handle opened above; input and output point to live buffers of the lengths given;
        // the status block lives on the heap until the request is known to be finished.
        let mut status = unsafe {
            NtDeviceIoControlFile(
                endpoint,
                event,
                std::ptr::null(),
                std::ptr::null(),
                io_status,
                code,
                input,
                input_length as u32,
                output,
                output_length as u32,
            )
        };
        if status == PENDING {
            // SAFETY: the event created above.
            if unsafe { WaitForSingleObject(event, WAIT_MS) } != 0 {
                let mut cancel_status = [0usize; 2];
                // SAFETY: the endpoint handle; a local status block for the cancel call itself.
                unsafe { NtCancelIoFile(endpoint, &mut cancel_status) };
                return Err("timeout".to_string());
            }
            // SAFETY: the request has finished (its event is set), so the driver no longer writes the block.
            status = unsafe { (*io_status)[0] } as u32 as i32;
        }
        // SAFETY: the request is finished; the block came from `Box::into_raw` above and the event from the call.
        unsafe {
            drop(Box::from_raw(io_status));
            CloseHandle(event);
        }
        if status == 0 {
            Ok(())
        } else {
            Err(format!("{status:#010x}"))
        }
    };

    let bind = BindInfo {
        share_access: 0, // normal address use
        address: v4([0, 0, 0, 0], 0),
    };
    let mut bound = v4([0; 4], 0);
    let answer = match request(
        IOCTL_BIND,
        (&bind as *const BindInfo).cast(),
        size_of::<BindInfo>(),
        (&mut bound as *mut SocketAddressV4).cast(),
        size_of::<SocketAddressV4>(),
    ) {
        Err(why) => format!("err bind {why}"),
        Ok(()) => {
            let connect = ConnectInfo {
                san_active: 0,
                root_endpoint: std::ptr::null_mut(),
                connect_endpoint: std::ptr::null_mut(),
                remote: v4(remote.ip().octets(), remote.port()),
            };
            match request(
                IOCTL_CONNECT,
                (&connect as *const ConnectInfo).cast(),
                size_of::<ConnectInfo>(),
                std::ptr::null_mut(),
                0,
            ) {
                Err(why) => format!("err connect {why}"),
                Ok(()) => "ok".to_string(),
            }
        }
    };
    // SAFETY: the endpoint opened above, closed once (a cancelled request ends with it).
    unsafe { CloseHandle(endpoint) };
    answer
}

#[cfg(not(windows))]
fn raw_connect(_target: &str) -> String {
    "err unsupported".into()
}

/// A name lookup through the system resolver's interface (`DnsQuery_W`, an A record, standard options). The lookup
/// itself is made by the DNS client service, not by this process, so a lookup made on the helper's behalf shows in
/// that service's cache, which the test reads. The answer is the call's own status (9003: the name does not exist).
#[cfg(windows)]
fn dns(name: &str) -> String {
    use std::ffi::c_void;

    #[link(name = "dnsapi")]
    unsafe extern "system" {
        fn DnsQuery_W(
            name: *const u16,
            record_type: u16,
            options: u32,
            extra: *const c_void,
            results: *mut *mut c_void,
            reserved: *mut c_void,
        ) -> i32;
        fn DnsRecordListFree(records: *mut c_void, free_type: i32);
    }
    const TYPE_A: u16 = 1;
    const STANDARD: u32 = 0;
    const FREE_RECORD_LIST: i32 = 1;

    if name.is_empty() {
        return "err parse".into();
    }
    let units: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut records: *mut c_void = std::ptr::null_mut();
    // SAFETY: a NUL-terminated name that lives past the call; `records` is a writable pointer the call fills in or
    // leaves null; no extra parameters; the reserved pointer is null as documented.
    let status = unsafe {
        DnsQuery_W(
            units.as_ptr(),
            TYPE_A,
            STANDARD,
            std::ptr::null(),
            &mut records,
            std::ptr::null_mut(),
        )
    };
    if !records.is_null() {
        // SAFETY: the list the call returned, freed once with the documented free type.
        unsafe { DnsRecordListFree(records, FREE_RECORD_LIST) };
    }
    if status == 0 {
        "ok".into()
    } else {
        format!("err {status}")
    }
}

#[cfg(not(windows))]
fn dns(_name: &str) -> String {
    "err unsupported".into()
}

/// Opens the system's Segoe UI font file and reads its first 4 bytes: the helper draws text with the system's fonts.
fn read_font() -> String {
    let Some(system_root) = std::env::var_os("SystemRoot") else {
        return "err unset".into();
    };
    let path = std::path::Path::new(&system_root).join("Fonts").join("segoeui.ttf");
    let mut head = [0u8; 4];
    match std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut head)) {
        Ok(()) => "ok".into(),
        Err(error) => code(&error),
    }
}

/// Opens a registry key for reading and closes it again: `<hive> <key>`, hive `hklm`, `hkcu` or `hkcr`. The answer is
/// the call's own code. Windows' decoders need to read the registry; only the system-decoder profile may.
#[cfg(windows)]
fn read_registry(rest: &str) -> String {
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW,
    };
    let Some((hive, key)) = rest.split_once(' ') else {
        return "err parse".into();
    };
    let root = match hive {
        "hklm" => HKEY_LOCAL_MACHINE,
        "hkcu" => HKEY_CURRENT_USER,
        "hkcr" => HKEY_CLASSES_ROOT,
        _ => return "err parse".into(),
    };
    let wide: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let mut opened: HKEY = std::ptr::null_mut();
    // SAFETY: a predefined root, a terminated UTF-16 key path, read access only; `opened` receives a new handle.
    let status = unsafe { RegOpenKeyExW(root, wide.as_ptr(), 0, KEY_READ, &mut opened) };
    if status != 0 {
        return format!("err {status}");
    }
    // SAFETY: the key the call above opened, closed once.
    unsafe { RegCloseKey(opened) };
    "ok".into()
}

#[cfg(not(windows))]
fn read_registry(_rest: &str) -> String {
    "err unsupported".into()
}

/// A security identifier as text (`S-1-15-3-...`), from its bytes: revision, count, a 48-bit big-endian authority,
/// then `count` 32-bit little-endian parts.
#[cfg(windows)]
fn identifier_text(bytes: &[u8]) -> String {
    let authority = bytes[2..8]
        .iter()
        .fold(0u64, |value, &byte| (value << 8) | u64::from(byte));
    let mut text = format!("S-{}-{authority}", bytes[0]);
    for part in bytes[8..].as_chunks::<4>().0 {
        text.push_str(&format!(
            "-{}",
            u32::from_le_bytes([part[0], part[1], part[2], part[3]])
        ));
    }
    text
}

/// The capabilities in this process's own token, as identifiers.
#[cfg(windows)]
fn capabilities() -> String {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{GetTokenInformation, SID_AND_ATTRIBUTES, TOKEN_GROUPS, TokenCapabilities};
    /// `GetCurrentProcessToken()`: the pseudo-handle of this process's own token; nothing is opened.
    const OWN_TOKEN: HANDLE = -4isize as HANDLE;
    // A count, then that many identifier-and-attributes entries whose identifiers point into the same buffer.
    let mut buffer = [0u64; 512];
    let mut returned = 0u32;
    // SAFETY: the pseudo-handle is valid for a query; the buffer is writable for the length passed and aligned for
    // the structure.
    let ok = unsafe {
        GetTokenInformation(
            OWN_TOKEN,
            TokenCapabilities,
            buffer.as_mut_ptr().cast(),
            size_of_val(&buffer) as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return code(&std::io::Error::last_os_error());
    }
    let groups = buffer.as_ptr().cast::<TOKEN_GROUPS>();
    // SAFETY: the call succeeded, so the buffer starts with the structure; the count is checked against the room.
    let count = unsafe { (*groups).GroupCount } as usize;
    let room = (size_of_val(&buffer) - std::mem::offset_of!(TOKEN_GROUPS, Groups)) / size_of::<SID_AND_ATTRIBUTES>();
    if count > room {
        return "err room".into();
    }
    let start = buffer.as_ptr().cast::<u8>() as usize;
    let end = start + size_of_val(&buffer);
    let mut names = Vec::with_capacity(count);
    for index in 0..count {
        // SAFETY: entry `index` lies inside the buffer (checked against its room above).
        let sid = unsafe { (*(&raw const (*groups).Groups).cast::<SID_AND_ATTRIBUTES>().add(index)).Sid };
        let at = sid as usize;
        // An identifier is 8 bytes plus 4 per part; it has to lie inside the buffer before it is read.
        if at < start || at + 8 > end {
            return "err identifier".into();
        }
        // SAFETY: the 8-byte head lies inside the buffer (checked above).
        let parts = usize::from(unsafe { *sid.cast::<u8>().add(1) });
        if at + 8 + 4 * parts > end {
            return "err identifier".into();
        }
        // SAFETY: the whole identifier lies inside the buffer (checked above), which lives until this returns.
        let bytes = unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), 8 + 4 * parts) };
        names.push(identifier_text(bytes));
    }
    names.sort();
    format!("capabilities {} {}", names.len(), names.join(","))
}

#[cfg(not(windows))]
fn capabilities() -> String {
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
            "udp-send" => udp_send(rest),
            "listen" => listen(rest),
            "resolve" => resolve(rest),
            "write" => write(rest),
            "writeown" => write_own(rest),
            "ownfolders" => own_folders(),
            "read" => read(rest),
            "spawn" => spawn(rest),
            "alloc" => alloc(rest),
            "env" => env(),
            "setevent" => set_event(rest),
            "job" => job(),
            "read-granted" => read_granted(rest),
            "write-granted" => write_granted(rest),
            "map-handle" => map_handle(rest),
            "privileges" => privileges(),
            "open-afd" => open_afd(rest),
            "raw-connect" => raw_connect(rest),
            "dns" => dns(rest),
            "read-font" => read_font(),
            "read-registry" => read_registry(rest),
            "capabilities" => capabilities(),
            "section-commit" => hardening::section_commit(rest),
            "hold-grants" => hardening::hold_grants(rest),
            "window" => hardening::window(),
            "jit" => hardening::jit(),
            "bad-handle" => hardening::bad_handle(),
            "strict-handles" => hardening::strict_handles(),
            "token" => hardening::token(),
            "memory" => hardening::memory(),
            "decoders" => hardening::decoders(),
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
