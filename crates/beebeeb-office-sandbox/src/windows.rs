//! Windows: an AppContainer with its profile's capabilities only, inside a job object, from a restricted token.
//!
//! | Layer | What it takes away |
//! |---|---|
//! | AppContainer, opted out of the "all application packages" group, with the profile's capabilities only (none by default) | network (also loopback), the user's files, every writable place |
//! | Job object: 1 active process, kill on close, memory cap, UI restrictions | child processes; outliving the parent; unbounded private memory; clipboard, desktop and global-atom access |
//! | Memory watch (the `watch` module): private plus shared commit, every 100 ms | unbounded memory through sections the helper makes itself |
//! | Restricted token: privileges removed, Administrators deny-only | whatever an elevated parent's token would add |
//! | Handle list | every handle of the parent except the two pipe ends |
//! | Process mitigations, per profile (`Profile::mitigations`) | the window system, dynamic code, foreign and remote libraries, extension points, unchecked handle use |
//!
//! Read back from the suspended process before it runs (`watch::read_facts`, judged by `crate::check_facts`): the
//! job, the container, the capabilities, the less-privileged container flag, the integrity level (low) and every
//! mitigation of the profile. Its memory counters are read once too: a Windows that cannot report a helper's shared
//! commit cannot watch it, so the helper does not run there (`SandboxError::NeedsNewerWindows`).
//!
//! **Shared memory.** A [`crate::Section`] is an unnamed section backed by the paging file; its handle is not
//! inheritable. A grant duplicates it into the running helper with exactly the rights asked for (map-read, or
//! map-read and map-write), never "the same access" as the app's handle. The helper maps it with `map` below, which
//! owns that handle from then on. Dropping a section writes zeros over all of it (volatile writes) before it is
//! unmapped, so a helper still holding a view sees zeros. The job's memory cap does not count the app's sections.
//!
//! **The container profile exists only while the helper is suspended.** Windows refuses to create an AppContainer
//! process for an identity that has no registered profile (system code 2, measured), and a registered profile comes
//! with a folder and registry entries of its own, meant for its process. So each helper gets a profile of its own
//! under a random name; the helper is created suspended; the profile, its folder and its registry entries are
//! deleted; the deletion is checked; only then does the helper run. The helper never executes while its profile
//! exists, two helpers never share an identity (so one cannot open the other), and nothing of the profile
//! outlives the spawn call. A test checks that no profile folder of ours exists while a helper answers. Whether
//! the helper could have written to that folder was not measured: it does not learn where the folder is (the
//! variables Windows sets for it are built from the placeholder below, not from the user's profile path). If
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
//! **Capabilities.** A [`Profile`] names the capabilities by their Windows names; the system derives one identifier
//! per name, the helper is created with exactly those, and before it runs its token is read back: the identifiers it
//! holds must be the derived ones, each once, no more and no fewer. Windows accepts any capability name (a misspelled
//! one too), so the names themselves are pinned by a test of `Profile::capabilities`.
//!
//! This module and `watch` are the only ones with `unsafe`. Every block is one call into the system, with its
//! invariant stated.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fs::File;
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Foundation::{
    DuplicateHandle, GetLastError, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE, DeriveCapabilitySidsFromName, FreeSid, PSID,
    SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_QUERY, WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS,
    JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
    JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READWRITE,
    UnmapViewOfFile,
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
    GetProcessHandleCount, InitializeProcThreadAttributeList, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::block::{checked_path, command_line, environment_block, is_our_profile_name, profile_name};
use crate::watch::{Watch, commit, read_facts};
use crate::{
    Access, ExitStatus, Granted, KILLED_EXIT_CODE, LaunchGuard, Limits, Mitigations, Pipes, Profile, SandboxError,
    check_facts,
};

/// `SE_GROUP_ENABLED`: the attribute a granted capability carries.
const SE_GROUP_ENABLED: u32 = 4;

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

// The `PROCESS_CREATION_MITIGATION_POLICY_*_ALWAYS_ON` bits, from WinBase.h of the Windows SDK (10.0.26100); windows-sys
// 0.61 does not carry them. All eight are in the attribute's first 64-bit word.
const STRICT_HANDLE_CHECKS_ALWAYS_ON: u64 = 1 << 24;
const WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON: u64 = 1 << 28;
const EXTENSION_POINT_DISABLE_ALWAYS_ON: u64 = 1 << 32;
const PROHIBIT_DYNAMIC_CODE_ALWAYS_ON: u64 = 1 << 36;
const BLOCK_NON_MICROSOFT_BINARIES_ALWAYS_ON: u64 = 1 << 44;
const IMAGE_LOAD_NO_REMOTE_ALWAYS_ON: u64 = 1 << 52;
const IMAGE_LOAD_NO_LOW_LABEL_ALWAYS_ON: u64 = 1 << 56;
const IMAGE_LOAD_PREFER_SYSTEM32_ALWAYS_ON: u64 = 1 << 60;

/// The creation attribute's word for a profile's mitigations.
fn creation_mitigations(wanted: Mitigations) -> u64 {
    [
        (wanted.no_window_system_calls, WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON),
        (wanted.no_remote_images, IMAGE_LOAD_NO_REMOTE_ALWAYS_ON),
        (wanted.no_low_label_images, IMAGE_LOAD_NO_LOW_LABEL_ALWAYS_ON),
        (wanted.prefer_system32_images, IMAGE_LOAD_PREFER_SYSTEM32_ALWAYS_ON),
        (wanted.no_extension_points, EXTENSION_POINT_DISABLE_ALWAYS_ON),
        (wanted.no_dynamic_code, PROHIBIT_DYNAMIC_CODE_ALWAYS_ON),
        (wanted.strict_handle_checks, STRICT_HANDLE_CHECKS_ALWAYS_ON),
        (wanted.microsoft_signed_only, BLOCK_NON_MICROSOFT_BINARIES_ALWAYS_ON),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .fold(0, |word, (_, bit)| word | bit)
}

pub(crate) fn os(step: &'static str) -> SandboxError {
    // SAFETY: reads the calling thread's last-error value; no pointer is involved.
    SandboxError::Os {
        step,
        code: unsafe { GetLastError() },
    }
}

/// Takes ownership of a handle the system just returned.
pub(crate) fn owned(handle: HANDLE) -> OwnedHandle {
    // SAFETY: every caller passes a handle that a system call returned as open and that nothing else owns; it is
    // closed exactly once, by the returned value.
    unsafe { OwnedHandle::from_raw_handle(handle) }
}

/// A confined helper: its process handle, its job and its memory watch. Dropping it ends the helper.
#[derive(Debug)]
pub(crate) struct ChildInner {
    process: OwnedHandle,
    job: OwnedHandle,
    pid: u32,
    /// The bytes of the sections granted to this helper that are still alive in the app (`crate::GrantedTo`).
    granted: Arc<AtomicU64>,
    /// Set by the memory watch before it ends the helper.
    killed_for_memory: Arc<AtomicBool>,
    /// Started right before the helper runs.
    watch: Option<Watch>,
}

/// Ends a helper: its job first (every process in it), then the process. Fine on a helper that already ended. The
/// one path of `Child::kill`, of dropping a `Child` and of the memory watch.
pub(crate) fn end(process: &OwnedHandle, job: &OwnedHandle) -> Result<(), SandboxError> {
    // SAFETY: `job` is an open job handle. Ends every process in the job.
    let job_ended = unsafe { TerminateJobObject(job.as_raw_handle(), KILLED_EXIT_CODE) } != 0;
    // SAFETY: `process` is an open process handle.
    let process_ended = unsafe { TerminateProcess(process.as_raw_handle(), KILLED_EXIT_CODE) } != 0;
    if job_ended || process_ended {
        return Ok(());
    }
    let failure = os("kill");
    // Both calls fail on a process that has already ended: that is not an error.
    // SAFETY: `process` is an open process handle; a zero wait only asks.
    if unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } == WAIT_OBJECT_0 {
        Ok(())
    } else {
        Err(failure)
    }
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
        end(&self.process, &self.job)
    }

    pub(crate) fn granted_bytes(&self) -> &Arc<AtomicU64> {
        &self.granted
    }

    pub(crate) fn killed_for_memory(&self) -> bool {
        self.killed_for_memory.load(Ordering::SeqCst)
    }
}

impl Drop for ChildInner {
    fn drop(&mut self) {
        // The job handle closes right after this and would end the helper too; killing first leaves no window.
        let _ = self.kill();
        // The helper is ending, so the watch's wait returns at once; its thread is joined before the handles close.
        if let Some(watch) = &mut self.watch {
            watch.finish();
        }
    }
}

impl ChildInner {
    /// Places a handle to `section` in the helper with exactly the rights of `access`, not inheritable.
    pub(crate) fn grant(&self, section: &SectionInner, access: Access) -> Result<Granted, SandboxError> {
        let mut wire: HANDLE = null_mut();
        // SAFETY: `section.mapping` is an open section handle; `process` is the helper's open process handle,
        // returned by process creation with full access (so with the right to place a handle in it); `wire` receives
        // the value in the helper. Not inheritable; the rights are exactly `granted_rights(access)`, no options (so
        // not "the same access" as the app's handle).
        let ok = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                section.mapping.as_raw_handle(),
                self.process.as_raw_handle(),
                &mut wire,
                granted_rights(access),
                0,
                0,
            )
        };
        if ok == 0 {
            return Err(os("grant"));
        }
        Ok(Granted {
            wire: wire as usize as u64,
            len: section.len as u64,
        })
    }

    pub(crate) fn handle_count(&self) -> Result<u32, SandboxError> {
        let mut count = 0u32;
        // SAFETY: `process` is the helper's open process handle, returned by process creation with full access (so
        // with query access); `count` is a valid place for one u32.
        if unsafe { GetProcessHandleCount(self.process.as_raw_handle(), &mut count) } == 0 {
            return Err(os("handle count"));
        }
        Ok(count)
    }
}

/// The rights a grant carries into the helper: exactly these, never the rights of the app's own handle.
fn granted_rights(access: Access) -> u32 {
    match access {
        Access::Read => FILE_MAP_READ,
        Access::ReadWrite => FILE_MAP_READ | FILE_MAP_WRITE,
    }
}

/// An unnamed section backed by the paging file, and the app's read-write view of all of it.
pub(crate) struct SectionInner {
    mapping: OwnedHandle,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
}

// SAFETY: the view is memory of this process that only this value unmaps; a shared reference lends it for reading
// and an exclusive one for writing, as a `Vec<u8>` would.
unsafe impl Send for SectionInner {}
// SAFETY: as above.
unsafe impl Sync for SectionInner {}

impl SectionInner {
    /// `len` is 1 byte to 4 GiB (checked by the caller).
    pub(crate) fn new(len: usize) -> Result<Self, SandboxError> {
        let size = len as u64;
        let (high, low) = ((size >> 32) as u32, size as u32);
        // SAFETY: no file (the paging file backs it); null attributes: default security and a handle that is not
        // inheritable; read-write pages; the size; and a null name, so no other process can open it by name.
        let raw = unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, null(), PAGE_READWRITE, high, low, null()) };
        if raw.is_null() {
            return Err(os("section"));
        }
        let mapping = owned(raw);
        // SAFETY: `mapping` is the open section just made, `len` bytes long; the view is unmapped once, in `drop`.
        let view = unsafe { MapViewOfFile(mapping.as_raw_handle(), FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, len) };
        if view.Value.is_null() {
            return Err(os("section view"));
        }
        Ok(Self { mapping, view, len })
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        // SAFETY: the view is mapped read-write for `len` bytes until `drop`; `&self` lends it for reading only.
        unsafe { std::slice::from_raw_parts(self.view.Value.cast(), self.len) }
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as above, and `&mut self` makes this the only reference into the view.
        unsafe { std::slice::from_raw_parts_mut(self.view.Value.cast(), self.len) }
    }

    pub(crate) fn raw_handle(&self) -> HANDLE {
        self.mapping.as_raw_handle()
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Copies the first `out.len()` bytes of the view into `out`, read from the view's address with volatile reads.
    /// `Section::copy_to` hands over exactly `len` bytes; a longer `out` is a bug of ours and panics here before
    /// anything is read.
    pub(crate) fn copy_into(&self, out: &mut [u8]) {
        assert!(out.len() <= self.len, "a copy longer than the section");
        let source = self.view.Value.cast::<u8>().cast_const();
        let (words, tail) = out.as_chunks_mut::<8>();
        let done = words.len() * 8;
        for (index, word_out) in words.iter_mut().enumerate() {
            // SAFETY: the view is mapped readable for `self.len` bytes until `drop`, and `index * 8 + 8 <= out.len()
            // <= self.len`; the view starts on a page boundary, so each word read is aligned. The read goes through
            // the raw address: no reference to the shared bytes is made, so a helper writing them at this moment can
            // leave a torn value in `out` but never changes memory a reference points to. Volatile, so each word is
            // read exactly once and nothing is assumed about bytes another process may change.
            let word = unsafe { std::ptr::read_volatile(source.add(index * 8).cast::<u64>()) };
            *word_out = word.to_ne_bytes();
        }
        for (index, byte) in tail.iter_mut().enumerate() {
            // SAFETY: as above, one byte inside the view: `done + index < out.len() <= self.len`.
            *byte = unsafe { std::ptr::read_volatile(source.add(done + index)) };
        }
    }
}

impl Drop for SectionInner {
    fn drop(&mut self) {
        zeroize(self.as_mut_slice());
        #[cfg(test)]
        tests::record_zeroed(self.as_slice());
        // SAFETY: the address `MapViewOfFile` returned in `new`, unmapped once, here. The section's handle closes
        // right after, when `mapping` is dropped.
        unsafe { UnmapViewOfFile(self.view) };
    }
}

/// Writes zero over every byte with volatile writes, which the compiler may not leave out.
fn zeroize(bytes: &mut [u8]) {
    // SAFETY: every bit pattern is a valid `u64`, so the aligned middle of the bytes may be written as words.
    let (head, words, tail) = unsafe { bytes.align_to_mut::<u64>() };
    for word in words {
        // SAFETY: `word` is a valid, aligned and exclusive reference.
        unsafe { std::ptr::write_volatile(word, 0) };
    }
    for byte in head.iter_mut().chain(tail) {
        // SAFETY: `byte` is a valid and exclusive reference.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// A granted section, mapped in the helper: its view and the grant's handle, both released on drop.
pub(crate) struct MappedInner {
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
    writable: bool,
    _grant: OwnedHandle,
}

// SAFETY: as for `SectionInner`: the view is memory of this process that only this value unmaps.
unsafe impl Send for MappedInner {}
// SAFETY: as above.
unsafe impl Sync for MappedInner {}

impl MappedInner {
    pub(crate) fn as_slice(&self) -> &[u8] {
        // SAFETY: the view is mapped readable for `len` bytes until `drop`; `&self` lends it for reading only.
        unsafe { std::slice::from_raw_parts(self.view.Value.cast(), self.len) }
    }

    pub(crate) fn as_mut_slice(&mut self) -> Option<&mut [u8]> {
        if !self.writable {
            return None;
        }
        // SAFETY: a view mapped read-write for `len` bytes until `drop`; `&mut self` makes this the only reference.
        let bytes = unsafe { std::slice::from_raw_parts_mut(self.view.Value.cast(), self.len) };
        Some(bytes)
    }
}

impl Drop for MappedInner {
    fn drop(&mut self) {
        // SAFETY: the address `MapViewOfFile` returned in `map`, unmapped once, here. The grant's handle closes right
        // after, when `_grant` is dropped.
        unsafe { UnmapViewOfFile(self.view) };
    }
}

/// The helper's side: maps the section behind `wire`, a handle the app placed in this process with `grant`.
pub(crate) fn map(wire: u64, len: usize, access: Access) -> Result<MappedInner, SandboxError> {
    let value = usize::try_from(wire).map_err(|_| SandboxError::Limits("a grant that does not fit this platform"))?;
    // SAFETY: the value is the handle the app placed in this process for this one grant; nothing else here owns it,
    // and it is closed once, by this value. In a confined helper a value that names no handle of this process ends
    // the process at its first use (strict handle checks), before any memory is touched.
    let grant = unsafe { OwnedHandle::from_raw_handle(value as HANDLE) };
    let rights = if access == Access::ReadWrite {
        FILE_MAP_READ | FILE_MAP_WRITE
    } else {
        FILE_MAP_READ
    };
    // SAFETY: `grant` is a section handle of this process (see above); the view is `len` bytes, the length of the
    // app's section; it is unmapped once, in `drop`.
    let view = unsafe { MapViewOfFile(grant.as_raw_handle(), rights, 0, 0, len) };
    if view.Value.is_null() {
        return Err(os("map"));
    }
    Ok(MappedInner {
        view,
        len,
        writable: access == Access::ReadWrite,
        _grant: grant,
    })
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

/// The capability identifiers of one [`Profile`], derived by the system from the profile's names. Everything the
/// system allocated for them is freed when this is dropped, after the helper's token was read back.
pub(crate) struct CapabilitySids {
    /// One enabled entry per capability, in the profile's order.
    granted: Vec<SID_AND_ATTRIBUTES>,
    /// Every allocation of the system's: each identifier, then each array that held them. Freed once.
    allocations: Vec<*mut c_void>,
}

impl CapabilitySids {
    pub(crate) fn of(profile: Profile) -> Result<Self, SandboxError> {
        let mut sids = Self {
            granted: Vec::new(),
            allocations: Vec::new(),
        };
        for name in profile.capabilities() {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let mut group_sids: *mut PSID = null_mut();
            let mut group_count = 0u32;
            let mut capability_sids: *mut PSID = null_mut();
            let mut capability_count = 0u32;
            // SAFETY: `wide` is a terminated UTF-16 name; the four out-pointers are valid places for the answers.
            let ok = unsafe {
                DeriveCapabilitySidsFromName(
                    wide.as_ptr(),
                    &mut group_sids,
                    &mut group_count,
                    &mut capability_sids,
                    &mut capability_count,
                )
            };
            if ok == 0 {
                return Err(os("capability identifier"));
            }
            // SAFETY: on success the system returned arrays of exactly these lengths (or null for none); each entry
            // and each array is an allocation of its own, recorded here once so `drop` frees it once.
            unsafe {
                for index in 0..group_count as usize {
                    sids.allocations.push(*group_sids.add(index));
                }
                for index in 0..capability_count as usize {
                    sids.allocations.push(*capability_sids.add(index));
                }
            }
            sids.allocations.push(group_sids.cast());
            sids.allocations.push(capability_sids.cast());
            if capability_count == 0 || capability_sids.is_null() {
                return Err(SandboxError::NotConfined("a capability name gave no identifier"));
            }
            // SAFETY: the array holds at least one entry (checked above). The first is the capability's own
            // identifier, as the media spike used and measured.
            let sid = unsafe { *capability_sids };
            sids.granted.push(SID_AND_ATTRIBUTES {
                Sid: sid,
                Attributes: SE_GROUP_ENABLED,
            });
        }
        Ok(sids)
    }

    /// The list for process creation: null when the profile has no capability.
    fn as_mut_ptr(&mut self) -> *mut SID_AND_ATTRIBUTES {
        if self.granted.is_empty() {
            null_mut()
        } else {
            self.granted.as_mut_ptr()
        }
    }

    fn count(&self) -> u32 {
        self.granted.len() as u32
    }

    pub(crate) fn identifiers(&self) -> Vec<PSID> {
        self.granted.iter().map(|entry| entry.Sid).collect()
    }
}

impl Drop for CapabilitySids {
    fn drop(&mut self) {
        for pointer in self.allocations.drain(..) {
            if !pointer.is_null() {
                // SAFETY: an allocation the system made in `of`, recorded once, freed once.
                unsafe { LocalFree(pointer) };
            }
        }
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

/// Reads the confinement back from the new, still suspended process and judges it against `profile`: in the job, an
/// AppContainer holding exactly the capabilities in `expected`, less-privileged, at low integrity, under every
/// mitigation of the profile (`crate::check_facts`).
fn confirm_confined(
    process: &OwnedHandle,
    job: &OwnedHandle,
    expected: &CapabilitySids,
    profile: Profile,
) -> Result<(), SandboxError> {
    check_facts(&read_facts(process, job, expected)?, profile)
}

/// `launch` is the app's launch lock, taken by the caller before anything here: between "the pipe ends are
/// inheritable" and "the helper exists" no other process of the app is created, so no launch receives a helper's
/// pipe ends. It is released as soon as the helper's ends are closed in the parent.
pub(crate) fn spawn(
    helper_exe: &Path,
    limits: Limits,
    profile: Profile,
    launch: LaunchGuard,
) -> Result<(ChildInner, Pipes), SandboxError> {
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
    // Lives until the end of this function: the attribute list points at its entries, and the read-back compares
    // the helper's token with them.
    let mut granted = CapabilitySids::of(profile)?;

    let capabilities = [SECURITY_CAPABILITIES {
        AppContainerSid: container.sid,
        Capabilities: granted.as_mut_ptr(),
        CapabilityCount: granted.count(),
        Reserved: 0,
    }];
    let packages_policy = [ALL_APPLICATION_PACKAGES_OPT_OUT];
    let mitigation_policy = [creation_mitigations(profile.mitigations())];
    let inherited: [HANDLE; 2] = [helper_stdin.as_raw_handle(), helper_stdout.as_raw_handle()];
    let jobs: [HANDLE; 1] = [job.as_raw_handle()];

    let mut attributes = AttributeList::new(5)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &packages_policy)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, &mitigation_policy)?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = helper_stdin.as_raw_handle();
    startup.StartupInfo.hStdOutput = helper_stdout.as_raw_handle();
    startup.StartupInfo.hStdError = null_mut();
    startup.lpAttributeList = attributes.buffer.as_mut_ptr().cast();

    let mut created = PROCESS_INFORMATION::default();
    let flags = CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | DETACHED_PROCESS | EXTENDED_STARTUPINFO_PRESENT;

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
    drop(launch);
    if let Some(failure) = failure {
        return Err(failure);
    }

    // From here every early return drops `child`, which ends the suspended process.
    let mut child = ChildInner {
        process: owned(created.hProcess),
        job,
        pid: created.dwProcessId,
        granted: Arc::new(AtomicU64::new(0)),
        killed_for_memory: Arc::new(AtomicBool::new(false)),
        watch: None,
    };
    let thread = owned(created.hThread);
    // The helper has not run yet. Its profile folder goes now; if it cannot be removed, the helper never runs.
    container.unregister()?;
    confirm_confined(&child.process, &child.job, &granted, profile)?;
    // A Windows that cannot report the helper's shared commit cannot watch it: the helper never runs there.
    commit(child.process.as_raw_handle())?;
    let watch = Watch::start(
        &child.process,
        &child.job,
        limits.memory_bytes,
        Arc::clone(&child.granted),
        Arc::clone(&child.killed_for_memory),
    )?;
    child.watch = Some(watch);
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
    use std::cell::Cell;

    use super::*;

    const ALREADY_EXISTS: u32 = 0x8007_00B7;

    thread_local! {
        /// How many bytes of the last section dropped on this thread were zero right before it was unmapped.
        static ZEROED_AT_DROP: Cell<Option<usize>> = const { Cell::new(None) };
    }

    /// The test-only hook of `SectionInner::drop`.
    pub(super) fn record_zeroed(view: &[u8]) {
        ZEROED_AT_DROP.set(Some(view.iter().filter(|&&byte| byte == 0).count()));
    }

    #[test]
    fn a_dropped_section_is_zeroed() {
        // Neither a multiple of eight nor of the page size, so the zeroing has a tail to cover.
        let len = 3 * 4096 + 5;
        let mut section = crate::Section::new(len).unwrap();
        section.as_mut_slice().fill(0xA5);
        assert_eq!(
            section.as_slice().iter().filter(|&&byte| byte == 0).count(),
            0,
            "the instrument can fail: before the drop no byte is zero"
        );
        ZEROED_AT_DROP.set(None);
        drop(section);
        assert_eq!(
            ZEROED_AT_DROP.get(),
            Some(len),
            "every byte was zero when the memory was released"
        );
    }

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
