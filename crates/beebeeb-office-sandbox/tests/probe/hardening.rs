//! The probe's verbs for the hardened confinement (process mitigations, the token's flags, memory and kept grants):
//! a module of its own so `main.rs` stays under the 1,500-line limit for tests. See `main.rs` for the requests.

use beebeeb_office_sandbox::{Access, Granted, helper};

use crate::refusal;

/// The code of the last system call that failed on this thread.
#[cfg(windows)]
fn last_error() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// This process's private and shared commit, through the extended counters.
#[cfg(windows)]
fn own_commit() -> Result<(u64, u64), String> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        SharedCommitUsage: u64::MAX,
        ..Default::default()
    };
    // SAFETY: the pseudo-handle of this process is always valid; `counters` is writable for `cb` bytes.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    if ok == 0 {
        return Err(format!("err counters {}", last_error()));
    }
    if counters.SharedCommitUsage == u64::MAX {
        return Err("err counters unfilled".into());
    }
    Ok((counters.PrivateUsage as u64, counters.SharedCommitUsage))
}

#[cfg(windows)]
pub(crate) fn section_commit(mebibytes: &str) -> String {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile,
    };
    let Ok(mebibytes) = mebibytes.parse::<u64>() else {
        return "err parse".into();
    };
    let size = mebibytes * 1024 * 1024;
    let Ok(len) = usize::try_from(size) else {
        return "err parse".into();
    };
    // SAFETY: no file (the paging file backs it), default security, read-write pages, the size, no name.
    let section = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            std::ptr::null(),
            PAGE_READWRITE,
            (size >> 32) as u32,
            size as u32,
            std::ptr::null(),
        )
    };
    if section.is_null() {
        return format!("err section {}", last_error());
    }
    // SAFETY: the section just made, `len` bytes long; unmapped below, once.
    let view = unsafe { MapViewOfFile(section, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, len) };
    if view.Value.is_null() {
        return format!("err view {}", last_error());
    }
    let base = view.Value.cast::<u8>();
    for offset in (0..len).step_by(4096) {
        // SAFETY: `offset < len`, inside the read-write view.
        unsafe { base.add(offset).write_volatile(1) };
    }
    // SAFETY: the address the call above returned, unmapped once. The section's handle is kept (never closed), so its
    // memory stays committed.
    unsafe { UnmapViewOfFile(view) };
    match own_commit() {
        Ok((_, shared)) => format!("ok {shared}"),
        Err(answer) => answer,
    }
}

#[cfg(not(windows))]
pub(crate) fn section_commit(_mebibytes: &str) -> String {
    "err unsupported".into()
}

/// Every grant `hold-grants` mapped, kept until the process ends.
static KEPT: std::sync::Mutex<Vec<helper::Mapped>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn hold_grants(rest: &str) -> String {
    let words: Vec<&str> = rest.split(' ').filter(|word| !word.is_empty()).collect();
    if words.is_empty() || !words.len().is_multiple_of(2) {
        return "err parse".into();
    }
    let mut held = 0;
    for pair in words.chunks(2) {
        let (Ok(wire), Ok(len)) = (pair[0].parse::<u64>(), pair[1].parse::<u64>()) else {
            return "err parse".into();
        };
        match helper::map(Granted { wire, len }, Access::Read) {
            Ok(mapped) => {
                // Never unmapped and the grant's handle never closed: what a helper that keeps its grants does.
                KEPT.lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(mapped);
                held += 1;
            }
            Err(error) => return refusal(error),
        }
    }
    format!("ok {held}")
}

/// A top-level window and one screen pixel, through the window libraries loaded at run time: an import of them would
/// stop this program from starting at all under the window-system lockdown, and every other verb with it.
#[cfg(windows)]
pub(crate) fn window() -> String {
    use std::ffi::c_void;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type CreateWindowExW = unsafe extern "system" fn(
        u32,
        *const u16,
        *const u16,
        u32,
        i32,
        i32,
        i32,
        i32,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const c_void,
    ) -> *mut c_void;
    type DestroyWindow = unsafe extern "system" fn(*mut c_void) -> i32;
    type GetDc = unsafe extern "system" fn(*mut c_void) -> *mut c_void;
    type ReleaseDc = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
    type GetPixel = unsafe extern "system" fn(*mut c_void, i32, i32) -> u32;
    /// `WS_POPUP`: a window without a parent or a frame, top-level.
    const WS_POPUP: u32 = 0x8000_0000;

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    // SAFETY: a terminated UTF-16 name; the result is checked before use.
    let user32 = unsafe { LoadLibraryW(wide("user32.dll").as_ptr()) };
    if user32.is_null() {
        return format!("err load {}", last_error());
    }
    // SAFETY: as above.
    let gdi32 = unsafe { LoadLibraryW(wide("gdi32.dll").as_ptr()) };
    if gdi32.is_null() {
        return format!("err load {}", last_error());
    }
    // SAFETY: each name is a terminated ASCII string and each module is loaded; each function is turned into the
    // signature the system documents for it, and only when it was found.
    let (create, destroy, get_dc, release_dc, get_pixel) = unsafe {
        let (Some(create), Some(destroy), Some(get_dc), Some(release_dc), Some(get_pixel)) = (
            GetProcAddress(user32, c"CreateWindowExW".as_ptr().cast()),
            GetProcAddress(user32, c"DestroyWindow".as_ptr().cast()),
            GetProcAddress(user32, c"GetDC".as_ptr().cast()),
            GetProcAddress(user32, c"ReleaseDC".as_ptr().cast()),
            GetProcAddress(gdi32, c"GetPixel".as_ptr().cast()),
        ) else {
            return format!("err lookup {}", last_error());
        };
        (
            std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateWindowExW>(create),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, DestroyWindow>(destroy),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, GetDc>(get_dc),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, ReleaseDc>(release_dc),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, GetPixel>(get_pixel),
        )
    };
    let class = wide("STATIC");
    // SAFETY: a system class name, no title, a 1 x 1 top-level window, no parent, menu, instance or parameter.
    let handle = unsafe {
        create(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if handle.is_null() {
        return format!("err window {}", last_error());
    }
    // SAFETY: a null window asks for the screen's drawing context.
    let screen = unsafe { get_dc(std::ptr::null_mut()) };
    let answer = if screen.is_null() {
        format!("err screen {}", last_error())
    } else {
        // SAFETY: `screen` is the context just obtained; the pixel's value is not needed, only that it was read.
        let _ = unsafe { get_pixel(screen, 0, 0) };
        // SAFETY: the context obtained above, released once.
        unsafe { release_dc(std::ptr::null_mut(), screen) };
        "ok".to_string()
    };
    // SAFETY: the window made above, destroyed once.
    unsafe { destroy(handle) };
    answer
}

#[cfg(not(windows))]
pub(crate) fn window() -> String {
    "err unsupported".into()
}

#[cfg(windows)]
pub(crate) fn jit() -> String {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
    };
    // SAFETY: a new page anywhere; the result is checked before use.
    let page = unsafe { VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
    if page.is_null() {
        return format!("err {}", last_error());
    }
    // SAFETY: the page is committed and writable; `ret` is one byte.
    unsafe { page.cast::<u8>().write_volatile(0xC3) };
    // SAFETY: the page allocated above, released once.
    unsafe { VirtualFree(page, 0, MEM_RELEASE) };
    "ok".into()
}

#[cfg(not(windows))]
pub(crate) fn jit() -> String {
    "err unsupported".into()
}

/// The handle value `bad-handle` uses: far above any value a process's handle table reaches.
#[cfg(windows)]
const NOT_A_HANDLE: usize = 0x7FFF_FFF8;

/// Uses a handle value this process does not hold. First the proof that it holds none: closing the value fails
/// (`err live` if it does not). A close never ends a process, strict handle checks or not: they act on a handle that
/// is USED, not closed (measured on the Windows test machine, 22631.6199: a close of such a value answers code 6 in a confined probe
/// too). Then a wait on it, which uses it: under strict handle checks the process ends here (0xC0000008); without
/// them the wait fails, `ok refused <code>`.
#[cfg(windows)]
pub(crate) fn bad_handle() -> String {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED};
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    // SAFETY: any value may be passed; one this process does not hold makes the call fail.
    if unsafe { CloseHandle(NOT_A_HANDLE as HANDLE) } != 0 {
        return "err live".into();
    }
    // SAFETY: as above; under strict handle checks the system ends the process instead of failing the call.
    let waited = unsafe { WaitForSingleObject(NOT_A_HANDLE as HANDLE, 0) };
    if waited == WAIT_FAILED {
        format!("ok refused {}", last_error())
    } else {
        format!("err waited {waited:#x}")
    }
}

/// This process's own strict-handle-check policy: `ok raise=<0|1> permanent=<0|1>` (an exception on a handle that
/// is used but not held; and whether that can never be turned off), or `err <system code>`.
#[cfg(windows)]
pub(crate) fn strict_handles() -> String {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessMitigationPolicy, ProcessStrictHandleCheckPolicy,
    };
    let mut flags = 0u32;
    // SAFETY: the pseudo-handle of this process is always valid; `flags` is writable for the policy's 4 bytes.
    let ok = unsafe {
        GetProcessMitigationPolicy(
            GetCurrentProcess(),
            ProcessStrictHandleCheckPolicy,
            (&raw mut flags).cast(),
            4,
        )
    };
    if ok == 0 {
        return format!("err {}", last_error());
    }
    format!("ok raise={} permanent={}", flags & 1, (flags >> 1) & 1)
}

#[cfg(not(windows))]
pub(crate) fn strict_handles() -> String {
    "err unsupported".into()
}

#[cfg(not(windows))]
pub(crate) fn bad_handle() -> String {
    "err unsupported".into()
}

#[cfg(windows)]
pub(crate) fn token() -> String {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
        TokenIntegrityLevel,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle of this process is always valid; `token` receives a new handle, closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return format!("err token {}", last_error());
    }
    let lpac = lpac_attribute(token);
    let mut returned = 0u32;
    let mut label = [0u64; 16];
    // SAFETY: the buffer is writable for the length passed and aligned for the structure.
    let label_ok = unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            label.as_mut_ptr().cast(),
            size_of_val(&label) as u32,
            &mut returned,
        )
    } != 0;
    let rid = if label_ok {
        // SAFETY: the call succeeded, so the buffer starts with the label, whose identifier points into the buffer;
        // the last part is read only when there is one.
        unsafe {
            let sid = (*label.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
            let parts = *GetSidSubAuthorityCount(sid);
            if parts == 0 {
                "none".to_string()
            } else {
                format!("{:#x}", *GetSidSubAuthority(sid, u32::from(parts) - 1))
            }
        }
    } else {
        format!("err{}", last_error())
    };
    // SAFETY: the token opened above, closed once.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(token) };
    format!("token lpac={lpac} rid={rid}")
}

/// `1` when this token carries the less-privileged container attribute `WIN://NOALLAPPPKG` set to a number other
/// than zero, `0` when it does not; `err<code>` when the read fails, `bad` when the answer does not fit, `bad-type<n>`
/// when the attribute is not one number. Read from the token's security attributes (the documented way: the class
/// `TokenIsLessPrivilegedAppContainer` is reserved and refused by Windows 11 23H2), here with a walk of its own over
/// the answer, independent of the crate's parse.
#[cfg(windows)]
fn lpac_attribute(token: windows_sys::Win32::Foundation::HANDLE) -> String {
    use windows_sys::Win32::Security::{GetTokenInformation, TokenSecurityAttributes};
    // The layout of the answer (64-bit): read through a pointer, never built, some fields never read.
    /// A counted string: length in bytes, room, characters.
    #[allow(dead_code)]
    #[repr(C)]
    struct Name {
        length: u16,
        maximum: u16,
        characters: *const u16,
    }
    #[allow(dead_code)]
    #[repr(C)]
    struct Entry {
        name: Name,
        value_type: u16,
        reserved: u16,
        flags: u32,
        value_count: u32,
        values: *const u64,
    }
    #[allow(dead_code)]
    #[repr(C)]
    struct Header {
        version: u16,
        reserved: u16,
        count: u32,
        entries: *const Entry,
    }
    let mut needed = 0u32;
    // SAFETY: a null buffer of length 0 only asks for the size; `needed` is a valid place for it.
    unsafe { GetTokenInformation(token, TokenSecurityAttributes, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 || needed > 64 * 1024 {
        return format!("err{}", last_error());
    }
    let mut words = vec![0u64; (needed as usize).div_ceil(8)];
    let mut returned = 0u32;
    // SAFETY: the buffer is writable for the length passed and aligned for the structures.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenSecurityAttributes,
            words.as_mut_ptr().cast(),
            (words.len() * 8) as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return format!("err{}", last_error());
    }
    let start = words.as_ptr() as usize;
    let end = start + (returned as usize).min(words.len() * 8);
    let within = |at: usize, length: usize| at >= start && at.checked_add(length).is_some_and(|stop| stop <= end);
    if !within(start, size_of::<Header>()) {
        return "bad".into();
    }
    // SAFETY: the answer starts with the header, which fits (checked above).
    let header = unsafe { &*words.as_ptr().cast::<Header>() };
    if header.version != 1 {
        return "bad".into();
    }
    if header.count == 0 {
        return "0".into();
    }
    let Some(length) = (header.count as usize).checked_mul(size_of::<Entry>()) else {
        return "bad".into();
    };
    if !within(header.entries as usize, length) {
        return "bad".into();
    }
    for index in 0..header.count as usize {
        // SAFETY: every entry lies inside the answer (checked above).
        let entry = unsafe { &*header.entries.add(index) };
        let length = usize::from(entry.name.length);
        if length == 0 {
            continue;
        }
        if !within(entry.name.characters as usize, length) {
            return "bad".into();
        }
        // SAFETY: the characters lie inside the answer (checked above), 2-byte units as the system writes them.
        let name = unsafe { std::slice::from_raw_parts(entry.name.characters, length / 2) };
        if !String::from_utf16_lossy(name).eq_ignore_ascii_case("WIN://NOALLAPPPKG") {
            continue;
        }
        if entry.value_type != 2 || entry.value_count == 0 || !within(entry.values as usize, 8) {
            return format!("bad-type{}", entry.value_type);
        }
        // SAFETY: the first value lies inside the answer (checked above).
        return if unsafe { *entry.values } != 0 { "1" } else { "0" }.into();
    }
    "0".into()
}

#[cfg(not(windows))]
pub(crate) fn token() -> String {
    "err unsupported".into()
}

#[cfg(windows)]
pub(crate) fn memory() -> String {
    use windows_sys::Win32::System::JobObjects::{
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOBOBJECT_LIMIT_VIOLATION_INFORMATION, JobObjectExtendedLimitInformation,
        JobObjectLimitViolationInformation, QueryInformationJobObject,
    };
    let (private, shared) = match own_commit() {
        Ok(commit) => commit,
        Err(answer) => return answer,
    };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    let mut violation = JOBOBJECT_LIMIT_VIOLATION_INFORMATION::default();
    // SAFETY: a null job handle means "the job of this process"; each buffer is writable for the size passed, the
    // size of the class asked for. A process in no job gets an error, answered as `none`.
    let in_job = unsafe {
        QueryInformationJobObject(
            std::ptr::null_mut(),
            JobObjectExtendedLimitInformation,
            (&raw mut limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            std::ptr::null_mut(),
        ) != 0
            && QueryInformationJobObject(
                std::ptr::null_mut(),
                JobObjectLimitViolationInformation,
                (&raw mut violation).cast(),
                size_of::<JOBOBJECT_LIMIT_VIOLATION_INFORMATION>() as u32,
                std::ptr::null_mut(),
            ) != 0
    };
    let (job, peak) = if in_job {
        (violation.JobMemory.to_string(), limits.PeakJobMemoryUsed.to_string())
    } else {
        ("none".to_string(), "none".to_string())
    };
    format!("memory private={private} shared={shared} job={job} peak-job={peak}")
}

#[cfg(not(windows))]
pub(crate) fn memory() -> String {
    "err unsupported".into()
}

/// Windows' decoders starting in this process, through libraries loaded at run time: COM (multithreaded), Media
/// Foundation, Microsoft's AAC decoder and the WIC imaging factory (HEIC and camera RAW pictures), each made and let go.
/// `ok`, or the first step that failed with its code (`err load|lookup <system code>`, `err <step> <hresult hex>`).
/// The system-decoder profile must answer `ok` (it holds `lpacCom`); the default profile must not.
#[cfg(windows)]
pub(crate) fn decoders() -> String {
    use std::ffi::c_void;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::core::GUID;
    type CoInitializeEx = unsafe extern "system" fn(*const c_void, u32) -> i32;
    type CoCreateInstance =
        unsafe extern "system" fn(*const GUID, *mut c_void, u32, *const GUID, *mut *mut c_void) -> i32;
    type MfStartup = unsafe extern "system" fn(u32, u32) -> i32;
    type MfShutdown = unsafe extern "system" fn() -> i32;
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    /// `MF_VERSION` (SDK 2, API 0x70) and `MFSTARTUP_NOSOCKET`, as the media helper starts it.
    const MF_VERSION: u32 = 0x0002_0070;
    const MFSTARTUP_NOSOCKET: u32 = 1;
    const COINIT_MULTITHREADED: u32 = 0;
    const CLSCTX_INPROC_SERVER: u32 = 1;
    /// `IUnknown`.
    const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
    /// `CLSID_CMSAACDecMFT`: Microsoft's AAC decoder.
    const CLSID_AAC_DECODER: GUID = GUID::from_u128(0x32d186a7_218f_4c75_8876_dd77273a8999);
    /// `CLSID_WICImagingFactory`.
    const CLSID_WIC_FACTORY: GUID = GUID::from_u128(0xcacaf262_9370_4615_a13b_9f5539da4c0a);

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    // SAFETY: terminated UTF-16 names; each result is checked before use.
    let (ole32, mfplat) = unsafe { (LoadLibraryW(wide("ole32.dll").as_ptr()), LoadLibraryW(wide("mfplat.dll").as_ptr())) };
    if ole32.is_null() || mfplat.is_null() {
        return format!("err load {}", last_error());
    }
    // SAFETY: each name is a terminated ASCII string and each module is loaded; each function is turned into the
    // signature the system documents for it, and only when it was found.
    let (co_initialize, co_create, mf_startup, mf_shutdown) = unsafe {
        let (Some(init), Some(create), Some(startup), Some(shutdown)) = (
            GetProcAddress(ole32, c"CoInitializeEx".as_ptr().cast()),
            GetProcAddress(ole32, c"CoCreateInstance".as_ptr().cast()),
            GetProcAddress(mfplat, c"MFStartup".as_ptr().cast()),
            GetProcAddress(mfplat, c"MFShutdown".as_ptr().cast()),
        ) else {
            return format!("err lookup {}", last_error());
        };
        (
            std::mem::transmute::<_, CoInitializeEx>(init),
            std::mem::transmute::<_, CoCreateInstance>(create),
            std::mem::transmute::<_, MfStartup>(startup),
            std::mem::transmute::<_, MfShutdown>(shutdown),
        )
    };
    // SAFETY: no reserved pointer; this thread joins the multithreaded apartment (and stays in it until exit).
    let hr = unsafe { co_initialize(std::ptr::null(), COINIT_MULTITHREADED) };
    if hr < 0 {
        return format!("err com {:08x}", hr as u32);
    }
    // SAFETY: the version these constants describe; no sockets.
    let hr = unsafe { mf_startup(MF_VERSION, MFSTARTUP_NOSOCKET) };
    if hr < 0 {
        return format!("err mf {:08x}", hr as u32);
    }
    for (step, class) in [("aac", CLSID_AAC_DECODER), ("wic", CLSID_WIC_FACTORY)] {
        let mut object: *mut c_void = std::ptr::null_mut();
        // SAFETY: valid GUIDs, no outer object, an in-process server; `object` receives one reference on success.
        let hr = unsafe { co_create(&class, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_IUNKNOWN, &mut object) };
        if hr < 0 || object.is_null() {
            // SAFETY: Media Foundation was started above.
            unsafe { mf_shutdown() };
            return format!("err {step} {:08x}", hr as u32);
        }
        // SAFETY: `object` is a live COM object; slot 2 of every COM vtable is `Release`, called once for the one
        // reference this call took.
        unsafe {
            let vtable = *object.cast::<*const Release>();
            (*vtable.add(2))(object);
        }
    }
    // SAFETY: Media Foundation was started above.
    unsafe { mf_shutdown() };
    "ok".into()
}

#[cfg(not(windows))]
pub(crate) fn decoders() -> String {
    "err unsupported".into()
}
