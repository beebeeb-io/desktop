//! Windows: an AppContainer with no capabilities, inside a job object, from a restricted token.
//!
//! | Layer | What it takes away |
//! |---|---|
//! | AppContainer, 0 capabilities, opted out of the "all application packages" group | network (also loopback), the user's files, every writable place |
//! | Job object: 1 active process, kill on close, memory cap, UI restrictions | child processes; outliving the parent; unbounded memory; clipboard, desktop and global-atom access |
//! | Restricted token: privileges removed, Administrators deny-only | whatever an elevated parent's token would add |
//! | Handle list | every handle of the parent except the two pipe ends |
//!
//! **The container profile exists only while the helper is suspended.** Windows refuses to create an AppContainer
//! process for an identity that has no registered profile (system code 2, measured), and a registered profile comes
//! with a folder its process may write to. So each helper gets a profile of its own under a random name; the
//! helper is created suspended; the profile, its folder and its registry entries are deleted; the deletion is
//! checked; only then does the helper run. It never executes while a place it could write to exists, two helpers
//! never share an identity (so one cannot open the other), and nothing of the profile outlives the spawn call. If
//! the parent is killed inside that window (a few system calls long), one empty profile under the name prefix
//! `beebeeb.office.` stays behind and the helper, still suspended, is ended by its job. The next `Sandbox::new`
//! deletes such a left-over: it lists this user's registered profiles, and deletes those whose name has exactly
//! our shape (the prefix and 32 hexadecimal digits) and that are older than a minute, so a profile another
//! process of ours is using right now is not touched. That list is the one input read from outside; it can only
//! select which of OUR empty profiles to delete, never change how a helper is confined.
//!
//! The job handle is held by the parent only and is not inheritable: when the parent ends, for any reason, the
//! handle closes and the system ends the helper.
//!
//! This is the only module with `unsafe`. Every block is one call into the system, with its invariant stated.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fs::File;
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Foundation::{
    GetLastError, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE, FreeSid, GetTokenInformation, PSID,
    SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_QUERY, TokenCapabilities, TokenIsAppContainer, WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS,
    JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
    JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows_sys::Win32::System::SystemInformation::{
    GetSystemDirectoryW, GetSystemTimeAsFileTime, GetSystemWindowsDirectoryW,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, DETACHED_PROCESS,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenProcessToken, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::block::{checked_path, command_line, environment_block, is_our_profile_name, profile_name};
use crate::{ExitStatus, KILLED_EXIT_CODE, Limits, Pipes, SandboxError};

/// `PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT`: the helper's token is not a member of the "all application
/// packages" group, so the files that group may read on every PC are closed to it as well.
const ALL_APPLICATION_PACKAGES_OPT_OUT: u32 = 1;

/// Every UI restriction a job object has (the eight documented classes).
const UI_RESTRICTIONS: u32 = JOB_OBJECT_UILIMIT_DESKTOP
    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
    | JOB_OBJECT_UILIMIT_EXITWINDOWS
    | JOB_OBJECT_UILIMIT_GLOBALATOMS
    | JOB_OBJECT_UILIMIT_HANDLES
    | JOB_OBJECT_UILIMIT_READCLIPBOARD
    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;

const JOB_LIMIT_FLAGS: u32 = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
    | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
    | JOB_OBJECT_LIMIT_JOB_MEMORY;

/// Between "the pipe ends are inheritable" and "the helper exists" no other helper of ours is created, so one
/// helper never receives another's pipe ends.
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

fn os(step: &'static str) -> SandboxError {
    // SAFETY: reads the calling thread's last-error value; no pointer is involved.
    SandboxError::Os {
        step,
        code: unsafe { GetLastError() },
    }
}

/// Takes ownership of a handle the system just returned.
fn owned(handle: HANDLE) -> OwnedHandle {
    // SAFETY: every caller passes a handle that a system call returned as open and that nothing else owns; it is
    // closed exactly once, by the returned value.
    unsafe { OwnedHandle::from_raw_handle(handle) }
}

/// A confined helper: its process handle and its job. Dropping it ends the helper.
#[derive(Debug)]
pub(crate) struct ChildInner {
    process: OwnedHandle,
    job: OwnedHandle,
    pid: u32,
}

impl ChildInner {
    pub(crate) fn id(&self) -> u32 {
        self.pid
    }

    pub(crate) fn wait(&self, bound: Duration) -> Result<Option<ExitStatus>, SandboxError> {
        // `u32::MAX` means "for ever" to the system; a bound is never that.
        let millis = u32::try_from(bound.as_millis())
            .unwrap_or(u32::MAX - 1)
            .min(u32::MAX - 1);
        // SAFETY: `process` is an open process handle owned by `self`.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), millis) } {
            WAIT_OBJECT_0 => {
                let mut code = 0u32;
                // SAFETY: `process` is open; `code` is a valid place for one u32.
                if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
                    return Err(os("exit code"));
                }
                Ok(Some(ExitStatus { code }))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(os("wait")),
        }
    }

    pub(crate) fn kill(&self) -> Result<(), SandboxError> {
        // SAFETY: `job` is an open job handle owned by `self`. Ends every process in the job.
        let job_ended = unsafe { TerminateJobObject(self.job.as_raw_handle(), KILLED_EXIT_CODE) } != 0;
        // SAFETY: `process` is an open process handle owned by `self`.
        let process_ended = unsafe { TerminateProcess(self.process.as_raw_handle(), KILLED_EXIT_CODE) } != 0;
        if job_ended || process_ended {
            return Ok(());
        }
        let failure = os("kill");
        // Both calls fail on a process that has already ended: that is not an error.
        match self.wait(Duration::ZERO) {
            Ok(Some(_)) => Ok(()),
            _ => Err(failure),
        }
    }
}

impl Drop for ChildInner {
    fn drop(&mut self) {
        // The job handle closes right after this and would end the helper too; killing first leaves no window.
        let _ = self.kill();
    }
}

/// A process-creation attribute list. The values it points to are borrowed for `'a`, so they outlive it.
struct AttributeList<'a> {
    buffer: Vec<usize>,
    values: PhantomData<&'a ()>,
}

impl<'a> AttributeList<'a> {
    fn new(count: u32) -> Result<Self, SandboxError> {
        let mut size = 0usize;
        // SAFETY: a null list with a valid size pointer is the documented way to ask for the needed size.
        unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size) };
        if size == 0 {
            return Err(os("attribute list size"));
        }
        let mut buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
        // SAFETY: `buffer` is at least `size` bytes, pointer-aligned, and lives as long as the list.
        if unsafe { InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), count, 0, &mut size) } == 0 {
            return Err(os("attribute list"));
        }
        Ok(Self {
            buffer,
            values: PhantomData,
        })
    }

    fn set<T>(&mut self, attribute: u32, value: &'a [T]) -> Result<(), SandboxError> {
        // SAFETY: the list was initialised in `new`; `value` is valid for `size_of_val(value)` bytes and is
        // borrowed for `'a`, which outlives the list, so the pointer the list keeps stays valid.
        let ok = unsafe {
            UpdateProcThreadAttribute(
                self.buffer.as_mut_ptr().cast(),
                0,
                attribute as usize,
                value.as_ptr().cast::<c_void>(),
                size_of_val(value),
                null_mut(),
                null(),
            )
        };
        if ok == 0 { Err(os("attribute")) } else { Ok(()) }
    }
}

impl Drop for AttributeList<'_> {
    fn drop(&mut self) {
        // SAFETY: the list was initialised in `new` and is deleted once.
        unsafe { DeleteProcThreadAttributeList(self.buffer.as_mut_ptr().cast()) };
    }
}

/// One helper's AppContainer profile. Registered by `register`, deleted by `unregister`; `drop` deletes it if an
/// early return skipped that, and frees the identity.
struct Container {
    name: Vec<u16>,
    sid: PSID,
    registered: bool,
}

impl Container {
    /// Registers a profile with no capabilities under a name made of 128 random bits.
    fn register() -> Result<Self, SandboxError> {
        let mut bytes = [0u8; 16];
        // SAFETY: `bytes` is valid for 16 bytes; a null algorithm with this flag selects the system generator.
        let status = unsafe { BCryptGenRandom(null_mut(), bytes.as_mut_ptr(), 16, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        if status != 0 {
            return Err(SandboxError::Os {
                step: "random",
                code: status as u32,
            });
        }
        let name: Vec<u16> = profile_name(bytes).encode_utf16().chain([0]).collect();
        let mut sid: PSID = null_mut();
        // SAFETY: `name` is a terminated UTF-16 string, used as name, display name and description; the
        // capability list is empty; `sid` receives an allocation freed in `drop`.
        let result =
            unsafe { CreateAppContainerProfile(name.as_ptr(), name.as_ptr(), name.as_ptr(), null(), 0, &mut sid) };
        if result != 0 || sid.is_null() {
            return Err(SandboxError::Os {
                step: "container profile",
                code: result as u32,
            });
        }
        Ok(Self {
            name,
            sid,
            registered: true,
        })
    }

    /// Deletes the profile: its folder and its registry entries.
    fn unregister(&mut self) -> Result<(), SandboxError> {
        // SAFETY: `name` is the terminated UTF-16 name the profile was registered under.
        let result = unsafe { DeleteAppContainerProfile(self.name.as_ptr()) };
        if result != 0 {
            return Err(SandboxError::Os {
                step: "container profile removal",
                code: result as u32,
            });
        }
        self.registered = false;
        Ok(())
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        if self.registered {
            let _ = self.unregister();
        }
        // SAFETY: the pointer came from `CreateAppContainerProfile` and is freed once.
        unsafe { FreeSid(self.sid) };
    }
}

/// Where Windows lists this user's registered container profiles (one key per profile, its name in `Moniker`).
const PROFILE_LIST: &str =
    r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings";

/// A profile of ours this old was left behind: the window in which one exists is a few system calls long.
const LEFT_OVER_AFTER: Duration = Duration::from_secs(60);

pub(crate) fn remove_left_over_profiles() {
    remove_profiles_older_than(LEFT_OVER_AFTER);
}

/// Deletes this user's container profiles whose name has our shape and whose list entry is older than `age`.
/// Returns how many were deleted. A list that cannot be read deletes nothing.
fn remove_profiles_older_than(age: Duration) -> usize {
    let list: Vec<u16> = PROFILE_LIST.encode_utf16().chain([0]).collect();
    let value_name: Vec<u16> = "Moniker".encode_utf16().chain([0]).collect();
    let mut key: HKEY = null_mut();
    // SAFETY: `list` is a terminated UTF-16 string; `key` receives a handle closed below.
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, list.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
        return 0;
    }
    let ticks = |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    let mut now = FILETIME::default();
    // SAFETY: `now` is a valid place for the time.
    unsafe { GetSystemTimeAsFileTime(&mut now) };
    let age_ticks = u64::try_from(age.as_nanos() / 100).unwrap_or(u64::MAX);

    let mut ours: Vec<Vec<u16>> = Vec::new();
    for index in 0u32.. {
        let mut entry = [0u16; 256];
        let mut entry_length = entry.len() as u32;
        let mut written = FILETIME::default();
        // SAFETY: `key` is open; `entry` is writable for `entry_length` units; `written` is a valid place for
        // the entry's time; the unused out-parameters are null, which the call allows.
        let result = unsafe {
            RegEnumKeyExW(
                key,
                index,
                entry.as_mut_ptr(),
                &mut entry_length,
                null(),
                null_mut(),
                null_mut(),
                &mut written,
            )
        };
        if result != 0 {
            break;
        }
        if ticks(now).saturating_sub(ticks(written)) < age_ticks {
            continue;
        }
        let mut name = [0u16; 128];
        let mut name_bytes = size_of_val(&name) as u32;
        // SAFETY: `key` is open; `entry` was terminated by the call above; `value_name` is terminated; `name`
        // is writable for `name_bytes` bytes; only a text value is accepted.
        let result = unsafe {
            RegGetValueW(
                key,
                entry.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                name.as_mut_ptr().cast(),
                &mut name_bytes,
            )
        };
        if result != 0 {
            continue;
        }
        let length = name.iter().position(|&unit| unit == 0).unwrap_or(name.len());
        if is_our_profile_name(&name[..length]) {
            ours.push(name[..length].iter().copied().chain([0]).collect());
        }
    }
    // SAFETY: `key` was opened above and is closed once.
    unsafe { RegCloseKey(key) };

    let mut deleted = 0;
    for name in ours {
        // SAFETY: `name` is a terminated UTF-16 string that passed `is_our_profile_name`.
        if unsafe { DeleteAppContainerProfile(name.as_ptr()) } == 0 {
            deleted += 1;
        }
    }
    deleted
}

/// An unnamed pipe: (read end, write end). Neither end is inheritable.
fn pipe() -> Result<(OwnedHandle, OwnedHandle), SandboxError> {
    let mut read: HANDLE = null_mut();
    let mut write: HANDLE = null_mut();
    // SAFETY: both out-pointers are valid; null attributes mean "not inheritable, default security".
    if unsafe { CreatePipe(&mut read, &mut write, null(), 0) } == 0 {
        return Err(os("pipe"));
    }
    Ok((owned(read), owned(write)))
}

/// The job: one process, ended when its last handle closes, memory capped, no UI access. Read back before use.
fn job(limits: Limits) -> Result<OwnedHandle, SandboxError> {
    let memory = usize::try_from(limits.memory_bytes)
        .map_err(|_| SandboxError::Limits("the memory limit does not fit this platform"))?;
    // SAFETY: null attributes and a null name: an unnamed job nothing else can open, handle not inheritable.
    let raw = unsafe { CreateJobObjectW(null(), null()) };
    if raw.is_null() {
        return Err(os("job"));
    }
    let job = owned(raw);

    let mut wanted = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    wanted.BasicLimitInformation.LimitFlags = JOB_LIMIT_FLAGS;
    wanted.BasicLimitInformation.ActiveProcessLimit = 1;
    wanted.JobMemoryLimit = memory;
    let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
    // SAFETY: `wanted` is a fully initialised structure of the class and size passed.
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const wanted).cast(),
            size,
        )
    } == 0
    {
        return Err(os("job limits"));
    }
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: UI_RESTRICTIONS,
    };
    let ui_size = size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32;
    // SAFETY: `ui` is a fully initialised structure of the class and size passed.
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectBasicUIRestrictions,
            (&raw const ui).cast(),
            ui_size,
        )
    } == 0
    {
        return Err(os("job UI restrictions"));
    }

    let mut got = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    // SAFETY: `got` is writable for `size` bytes, the size of the class asked for.
    if unsafe {
        QueryInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw mut got).cast(),
            size,
            null_mut(),
        )
    } == 0
    {
        return Err(os("job limits read-back"));
    }
    let mut got_ui = JOBOBJECT_BASIC_UI_RESTRICTIONS::default();
    // SAFETY: `got_ui` is writable for `ui_size` bytes, the size of the class asked for.
    if unsafe {
        QueryInformationJobObject(
            job.as_raw_handle(),
            JobObjectBasicUIRestrictions,
            (&raw mut got_ui).cast(),
            ui_size,
            null_mut(),
        )
    } == 0
    {
        return Err(os("job UI read-back"));
    }
    if got.BasicLimitInformation.LimitFlags & JOB_LIMIT_FLAGS != JOB_LIMIT_FLAGS
        || got.BasicLimitInformation.ActiveProcessLimit != 1
        || got.JobMemoryLimit != memory
        || got_ui.UIRestrictionsClass & UI_RESTRICTIONS != UI_RESTRICTIONS
    {
        return Err(SandboxError::NotConfined("job limits"));
    }
    Ok(job)
}

/// The parent's token with every privilege that can be removed removed and Administrators made deny-only.
fn restricted_token() -> Result<OwnedHandle, SandboxError> {
    let access = TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY | TOKEN_ADJUST_DEFAULT | TOKEN_ADJUST_SESSIONID;
    let mut raw: HANDLE = null_mut();
    // SAFETY: the pseudo-handle of this process is always valid; `raw` receives a new handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut raw) } == 0 {
        return Err(os("own token"));
    }
    let own = owned(raw);

    // 68 bytes is the largest identifier the system has; u32 units give it the alignment it needs.
    let mut administrators = [0u32; 17];
    let mut length = size_of_val(&administrators) as u32;
    // SAFETY: the buffer is writable for `length` bytes and aligned for an identifier.
    if unsafe {
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            null_mut(),
            administrators.as_mut_ptr().cast(),
            &mut length,
        )
    } == 0
    {
        return Err(os("administrators identifier"));
    }
    let deny_only = [SID_AND_ATTRIBUTES {
        Sid: administrators.as_mut_ptr().cast(),
        Attributes: 0,
    }];

    let mut restricted: HANDLE = null_mut();
    // SAFETY: `own` is an open token with duplicate access; `deny_only` holds one valid entry whose identifier
    // lives until after the call; the other lists are empty; `restricted` receives a new handle.
    let ok = unsafe {
        CreateRestrictedToken(
            own.as_raw_handle(),
            DISABLE_MAX_PRIVILEGE,
            1,
            deny_only.as_ptr(),
            0,
            null(),
            0,
            null(),
            &mut restricted,
        )
    };
    if ok == 0 {
        return Err(os("restricted token"));
    }
    Ok(owned(restricted))
}

/// A system directory as UTF-16 without a terminator.
fn directory(
    step: &'static str,
    ask: unsafe extern "system" fn(*mut u16, u32) -> u32,
) -> Result<Vec<u16>, SandboxError> {
    let mut buffer = vec![0u16; 512];
    // SAFETY: `buffer` is writable for 512 units, the length passed.
    let length = unsafe { ask(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(os(step));
    }
    buffer.truncate(length);
    Ok(buffer)
}

/// Reads the confinement back from the new, still suspended process.
fn confirm_confined(process: &OwnedHandle, job: &OwnedHandle) -> Result<(), SandboxError> {
    let mut in_job = 0i32;
    // SAFETY: both handles are open; `in_job` is a valid place for the answer.
    if unsafe { IsProcessInJob(process.as_raw_handle(), job.as_raw_handle(), &mut in_job) } == 0 {
        return Err(os("job membership"));
    }
    if in_job == 0 {
        return Err(SandboxError::NotConfined("not in the job"));
    }

    let mut raw: HANDLE = null_mut();
    // SAFETY: `process` is open with full access; `raw` receives a new handle.
    if unsafe { OpenProcessToken(process.as_raw_handle(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(os("helper token"));
    }
    let token = owned(raw);

    let mut is_container = 0u32;
    let mut returned = 0u32;
    // SAFETY: `is_container` is writable for the 4 bytes this class returns.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIsAppContainer,
            (&raw mut is_container).cast(),
            4,
            &mut returned,
        )
    } == 0
    {
        return Err(os("container read-back"));
    }
    if is_container == 0 {
        return Err(SandboxError::NotConfined("not an AppContainer"));
    }

    // The answer is a count followed by that many entries; with no capability it is a count of zero. 512 bytes,
    // pointer-aligned.
    let mut capabilities = [0u64; 64];
    // SAFETY: the buffer is writable for the length passed and aligned for the structure.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenCapabilities,
            capabilities.as_mut_ptr().cast(),
            size_of_val(&capabilities) as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return Err(os("capabilities read-back"));
    }
    let count = capabilities[0] as u32;
    if count != 0 {
        return Err(SandboxError::NotConfined("has a capability"));
    }
    Ok(())
}

pub(crate) fn spawn(helper_exe: &Path, limits: Limits) -> Result<(ChildInner, Pipes), SandboxError> {
    let units: Vec<u16> = helper_exe.as_os_str().encode_wide().collect();
    let application = checked_path(&units)?;
    if !std::fs::metadata(helper_exe).map(|m| m.is_file()).unwrap_or(false) {
        return Err(SandboxError::HelperPath("not a file"));
    }
    let mut command = command_line(&application);
    let environment = environment_block(&directory("windows directory", GetSystemWindowsDirectoryW)?)?;
    let mut current = directory("system directory", GetSystemDirectoryW)?;
    current.push(0);

    let (helper_stdin, to_helper) = pipe()?;
    let (from_helper, helper_stdout) = pipe()?;
    let job = job(limits)?;
    let token = restricted_token()?;
    let mut container = Container::register()?;

    let capabilities = [SECURITY_CAPABILITIES {
        AppContainerSid: container.sid,
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    }];
    let packages_policy = [ALL_APPLICATION_PACKAGES_OPT_OUT];
    let inherited: [HANDLE; 2] = [helper_stdin.as_raw_handle(), helper_stdout.as_raw_handle()];
    let jobs: [HANDLE; 1] = [job.as_raw_handle()];

    let mut attributes = AttributeList::new(4)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &packages_policy)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = helper_stdin.as_raw_handle();
    startup.StartupInfo.hStdOutput = helper_stdout.as_raw_handle();
    startup.StartupInfo.hStdError = null_mut();
    startup.lpAttributeList = attributes.buffer.as_mut_ptr().cast();

    let mut created = PROCESS_INFORMATION::default();
    let flags = CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | DETACHED_PROCESS | EXTENDED_STARTUPINFO_PRESENT;

    let guard = SPAWN_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for end in inherited {
        // SAFETY: `end` is one of the two open pipe ends owned by this function.
        if unsafe { SetHandleInformation(end, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
            return Err(os("inheritable pipe end"));
        }
    }
    // SAFETY: `token` is an open primary token; `application`, `command`, `environment` and `current` are
    // terminated UTF-16 buffers that live past the call (`command` is writable, as the call requires);
    // `startup` is an extended startup structure whose attribute list and the values it points to are alive;
    // `created` receives the new handles. Handles are inherited, but only the two in the handle list.
    let ok = unsafe {
        CreateProcessAsUserW(
            token.as_raw_handle(),
            application.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            flags,
            environment.as_ptr().cast(),
            current.as_ptr(),
            &raw const startup.StartupInfo,
            &mut created,
        )
    };
    let failure = if ok == 0 { Some(os("create process")) } else { None };
    // The helper's ends are closed in the parent whatever happened: after this, only the helper holds them, so a
    // read from a dead helper ends instead of waiting for ever.
    drop(helper_stdin);
    drop(helper_stdout);
    drop(guard);
    if let Some(failure) = failure {
        return Err(failure);
    }

    // From here every early return drops `child`, which ends the suspended process.
    let child = ChildInner {
        process: owned(created.hProcess),
        job,
        pid: created.dwProcessId,
    };
    let thread = owned(created.hThread);
    // The helper has not run yet. Its profile folder goes now; if it cannot be removed, the helper never runs.
    container.unregister()?;
    confirm_confined(&child.process, &child.job)?;
    // SAFETY: `thread` is the open handle of the helper's suspended first thread.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(os("resume"));
    }
    Ok((
        child,
        Pipes {
            to_helper: File::from(to_helper),
            from_helper: File::from(from_helper),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALREADY_EXISTS: u32 = 0x8007_00B7;

    fn wide(name: &str) -> Vec<u16> {
        name.encode_utf16().chain([0]).collect()
    }

    /// Registers a profile with no capabilities; the result code.
    fn register(name: &str) -> u32 {
        let name = wide(name);
        let mut sid: PSID = null_mut();
        // SAFETY: `name` is a terminated UTF-16 string; no capabilities; `sid` receives an allocation freed below.
        let result =
            unsafe { CreateAppContainerProfile(name.as_ptr(), name.as_ptr(), name.as_ptr(), null(), 0, &mut sid) };
        if !sid.is_null() {
            // SAFETY: the pointer came from the call above and is freed once.
            unsafe { FreeSid(sid) };
        }
        result as u32
    }

    fn delete(name: &str) -> u32 {
        // SAFETY: a terminated UTF-16 string.
        (unsafe { DeleteAppContainerProfile(wide(name).as_ptr()) }) as u32
    }

    /// Whether a profile of that name is registered: registering it again is refused as "already exists".
    fn exists(name: &str) -> bool {
        match register(name) {
            ALREADY_EXISTS => true,
            0 => {
                assert_eq!(delete(name), 0);
                false
            }
            other => panic!("unexpected result {other:#x}"),
        }
    }

    /// A parent killed while starting a helper leaves a profile of ours behind. The clean-up deletes it, and only
    /// it: a fresh one (possibly in use by another process of ours) and one with a foreign name stay.
    #[test]
    fn a_left_over_profile_of_ours_is_deleted_and_nothing_else_is() {
        let unique = std::process::id();
        let ours = profile_name([0xEE; 16]);
        let foreign = format!("beebeeb.officetest.{unique:08x}");
        let _ = (delete(&ours), delete(&foreign));
        assert_eq!((register(&ours), register(&foreign)), (0, 0), "both planted");
        assert!(
            exists(&ours) && exists(&foreign),
            "the instrument sees a planted profile"
        );

        // Younger than a minute: what `Sandbox::new` does. Nothing of the two goes.
        remove_left_over_profiles();
        assert!(exists(&ours), "a fresh profile of ours is left alone");
        assert!(exists(&foreign));

        // Any age: ours goes, the foreign one stays.
        assert!(remove_profiles_older_than(Duration::ZERO) >= 1);
        assert!(!exists(&ours), "the left-over profile of ours is gone");
        assert!(exists(&foreign), "a profile that is not ours is untouched");

        assert_eq!(delete(&foreign), 0, "the test removes what it planted");
        assert!(!exists(&foreign));
    }
}
