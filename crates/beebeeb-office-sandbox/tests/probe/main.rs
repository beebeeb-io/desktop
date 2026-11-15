//! The sandbox tests' helper. It TRIES what a confined helper must not be able to do and answers with the result,
//! one line per request on its standard output. It prints codes and counts only.
//!
//! Requests (one per line on standard input):
//!   ping                 -> pong
//!   connect <ip:port>    -> ok | err <system code> | err network-start <system code>
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
//!   read-font            -> ok | err <system code>      (opens the system's Segoe UI font file, reads 4 bytes)
//!   read-registry <hive> <key>
//!                        -> ok | err <system code>      (Windows: opens a registry key to read; hive is hklm, hkcu
//!                                                         or hkcr)
//!   capabilities         -> capabilities <n> <identifiers, comma-separated, sorted>  (Windows: the capabilities in
//!                                                         its own token, as S-1-15-3-... identifiers)
//!   hang                 -> hanging, then sleeps for ever
//!   spin                 -> spinning, then burns one core for ever
//!   exit <code>          -> exits with that code
//!
//! Numbers are decimal, handle values hexadecimal (as `setevent`); a grant's `wire` is decimal (it is a `u64`).
//!
//! `beebeeb-sandbox-probe parent sandboxed|plain` is the "parent that dies" of the tests: it starts a second
//! probe (confined through the crate, or plainly), makes it hang, prints `child <process id>` and sleeps.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

use beebeeb_office_sandbox::{Access, Granted, SandboxError, helper};

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
    for part in bytes[8..].chunks_exact(4) {
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
            "read-font" => read_font(),
            "read-registry" => read_registry(rest),
            "capabilities" => capabilities(),
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
