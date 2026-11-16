//! Windows: what the app reads from a helper's process.
//!
//! - **Before it runs:** the facts [`crate::check_facts`] judges, read from the new, still suspended process: job
//!   membership, the AppContainer, the capabilities, the less-privileged container attribute, the integrity level and
//!   each process mitigation of its profile ([`read_facts`]). And its memory counters, once ([`commit`]): a Windows that
//!   cannot report a helper's shared commit cannot watch it, so the helper never runs there.
//! - **While it runs:** its private and shared commit, every 100 ms, by a watch thread of its own ([`Watch`]), which
//!   ends it as soon as [`crate::over_memory`] says so, or when the counters can no longer be read.
//!
//! As in the `windows` module, every `unsafe` block is one call into the system, with its invariant stated.
#![allow(unsafe_code)]

use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{
    ERROR_BAD_LENGTH, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, GetLastError, HANDLE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, PSID, SID_AND_ATTRIBUTES, TOKEN_GROUPS,
    TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenCapabilities, TokenIntegrityLevel,
    TokenIsAppContainer, TokenSecurityAttributes,
};
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows_sys::Win32::System::Threading::{
    GetProcessMitigationPolicy, OpenProcessToken, PROCESS_MITIGATION_POLICY, ProcessDynamicCodePolicy,
    ProcessExtensionPointDisablePolicy, ProcessImageLoadPolicy, ProcessSignaturePolicy, ProcessStrictHandleCheckPolicy,
    ProcessSystemCallDisablePolicy, WaitForSingleObject,
};

use crate::attributes;
use crate::block::same_members;
use crate::windows::{CapabilitySids, end, os, owned};
use crate::{Mitigations, SandboxError, TokenFacts, over_memory};

/// How often a helper's commit is read, in milliseconds.
const POLL_MS: u32 = 100;

/// The codes a Windows gives for a counters structure it does not know: the kernel's length check
/// (`ERROR_BAD_LENGTH`), and the two a library layer might answer instead.
const UNKNOWN_SIZE: [u32; 3] = [ERROR_BAD_LENGTH, ERROR_INVALID_PARAMETER, ERROR_INSUFFICIENT_BUFFER];

/// Reads what [`crate::check_facts`] judges from the new process. `expected` are the identifiers the system derived
/// from the profile's capability names.
pub(crate) fn read_facts(
    process: &OwnedHandle,
    job: &OwnedHandle,
    expected: &CapabilitySids,
) -> Result<TokenFacts, SandboxError> {
    let mut in_job = 0i32;
    // SAFETY: both handles are open; `in_job` is a valid place for the answer.
    if unsafe { IsProcessInJob(process.as_raw_handle(), job.as_raw_handle(), &mut in_job) } == 0 {
        return Err(os("job membership"));
    }
    let mut raw: HANDLE = null_mut();
    // SAFETY: `process` is open with query access; `raw` receives a new handle.
    if unsafe { OpenProcessToken(process.as_raw_handle(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(os("helper token"));
    }
    let token = owned(raw);
    let app_container = token_flag(&token, TokenIsAppContainer, "container read-back")?;
    Ok(TokenFacts {
        in_job: in_job != 0,
        app_container,
        // A token outside a container has no capability list, and is no less-privileged container whatever it carries.
        exact_capabilities: app_container && holds_exactly(&token, expected)?,
        less_privileged: app_container && no_all_application_packages(&token)?,
        integrity_rid: integrity_rid(&token)?,
        mitigations: read_mitigations(process)?,
    })
}

/// A token class whose answer is one 32-bit flag.
fn token_flag(token: &OwnedHandle, class: TOKEN_INFORMATION_CLASS, step: &'static str) -> Result<bool, SandboxError> {
    let mut value = 0u32;
    let mut returned = 0u32;
    // SAFETY: `token` is open with query access; `value` is writable for the 4 bytes these classes return.
    if unsafe { GetTokenInformation(token.as_raw_handle(), class, (&raw mut value).cast(), 4, &mut returned) } == 0 {
        return Err(os(step));
    }
    Ok(value != 0)
}

/// The most a token's security attributes may take: a container token carries a handful (about 200 bytes each).
const MOST_ATTRIBUTE_BYTES: u32 = 64 * 1024;

/// Whether the token carries the less-privileged container attribute, set (`WIN://NOALLAPPPKG`, see
/// [`attributes::less_privileged`]), read from its security attributes. This is the documented way to read it: the
/// token class `TokenIsLessPrivilegedAppContainer` is documented as reserved, and Windows 11 23H2 refuses it (system
/// code 87, measured).
fn no_all_application_packages(token: &OwnedHandle) -> Result<bool, SandboxError> {
    let mut needed = 0u32;
    // SAFETY: a null buffer of length 0 only asks for the size; `needed` is a valid place for it.
    let sized = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenSecurityAttributes,
            null_mut(),
            0,
            &mut needed,
        )
    };
    if sized == 0 {
        // SAFETY: reads the calling thread's last-error value; no pointer is involved.
        let code = unsafe { GetLastError() };
        if code != ERROR_INSUFFICIENT_BUFFER && code != ERROR_BAD_LENGTH {
            return Err(SandboxError::Os {
                step: "lpac read-back",
                code,
            });
        }
    }
    if needed == 0 || needed > MOST_ATTRIBUTE_BYTES {
        return Err(SandboxError::NotConfined(
            "the security-attribute read-back does not fit",
        ));
    }
    // u64 units: the answer's pointers and numbers are 8-byte aligned.
    let mut words = vec![0u64; (needed as usize).div_ceil(8)];
    let mut returned = 0u32;
    // SAFETY: the buffer is writable for the length passed (at least `needed`) and aligned for the structure.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenSecurityAttributes,
            words.as_mut_ptr().cast(),
            (words.len() * 8) as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return Err(os("lpac read-back"));
    }
    // The pointers in the answer point into `words`; the parse turns them into offsets into this copy of its bytes.
    let answer: Vec<u8> = words
        .iter()
        .flat_map(|word| word.to_ne_bytes())
        .take(returned as usize)
        .collect();
    attributes::less_privileged(&answer, words.as_ptr() as usize)
}

/// Whether the token holds exactly the identifiers in `expected`, each once.
fn holds_exactly(token: &OwnedHandle, expected: &CapabilitySids) -> Result<bool, SandboxError> {
    // The answer is a count followed by that many identifier-and-attributes entries whose identifiers point into the
    // same buffer; with no capability it is a count of zero. 4 KiB, pointer-aligned: room for far more than the five
    // capabilities a profile has (about 60 bytes each).
    let mut capabilities = [0u64; 512];
    let mut returned = 0u32;
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
    let groups = capabilities.as_ptr().cast::<TOKEN_GROUPS>();
    // SAFETY: the call succeeded, so the buffer starts with the structure; the count is read, not trusted (below).
    let count = unsafe { (*groups).GroupCount } as usize;
    let room =
        (size_of_val(&capabilities) - std::mem::offset_of!(TOKEN_GROUPS, Groups)) / size_of::<SID_AND_ATTRIBUTES>();
    if count > room {
        return Err(SandboxError::NotConfined("the capability read-back does not fit"));
    }
    // SAFETY: `count` entries follow the count inside the buffer (checked against its room above); each identifier
    // points into the same buffer, which lives until the comparison below is done.
    let held: Vec<PSID> = unsafe {
        let entries = (&raw const (*groups).Groups).cast::<SID_AND_ATTRIBUTES>();
        (0..count).map(|index| (*entries.add(index)).Sid).collect()
    };
    let equal = |a: &PSID, b: &PSID| {
        // SAFETY: both are valid identifiers: one from the read-back buffer above, one derived by the system.
        unsafe { EqualSid(*a, *b) != 0 }
    };
    Ok(same_members(&held, &expected.identifiers(), equal))
}

/// The relative identifier of the token's integrity level (the last part of its label's identifier).
fn integrity_rid(token: &OwnedHandle) -> Result<u32, SandboxError> {
    // The label and its identifier (S-1-16-<level>, 12 bytes) need about 30 bytes; u64 units align the structure.
    let mut label = [0u64; 16];
    let mut returned = 0u32;
    // SAFETY: `token` is open with query access; the buffer is writable for the length passed and aligned.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            label.as_mut_ptr().cast(),
            size_of_val(&label) as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return Err(os("integrity read-back"));
    }
    // SAFETY: the call succeeded, so the buffer starts with the label, whose identifier points into the same buffer.
    let sid = unsafe { (*label.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid };
    // SAFETY: `sid` is the valid identifier the system wrote above.
    let parts = unsafe { *GetSidSubAuthorityCount(sid) };
    if parts == 0 {
        return Err(SandboxError::NotConfined("the integrity label has no level"));
    }
    // SAFETY: `parts - 1` is the index of the last part of a valid identifier with `parts` parts.
    Ok(unsafe { *GetSidSubAuthority(sid, u32::from(parts) - 1) })
}

/// Each mitigation, read back from the process. Every policy read here is one 32-bit flags word (winnt.h, the
/// `PROCESS_MITIGATION_*_POLICY` unions); bit 0 is the setting this crate asks for, and the image-load policy's bits
/// 1 and 2 are the low-label and system-folder settings.
fn read_mitigations(process: &OwnedHandle) -> Result<Mitigations, SandboxError> {
    let flags = |policy: PROCESS_MITIGATION_POLICY, step: &'static str| -> Result<u32, SandboxError> {
        let mut value = 0u32;
        // SAFETY: `process` is open with query access; `value` is writable for the 4 bytes of the policy's flags.
        if unsafe { GetProcessMitigationPolicy(process.as_raw_handle(), policy, (&raw mut value).cast(), 4) } == 0 {
            return Err(os(step));
        }
        Ok(value)
    };
    let image = flags(ProcessImageLoadPolicy, "image-load policy read-back")?;
    Ok(Mitigations {
        no_window_system_calls: (flags(ProcessSystemCallDisablePolicy, "system-call policy read-back")? & 1) != 0,
        no_remote_images: (image & 1) != 0,
        no_low_label_images: (image & 2) != 0,
        prefer_system32_images: (image & 4) != 0,
        no_extension_points: (flags(ProcessExtensionPointDisablePolicy, "extension-point policy read-back")? & 1) != 0,
        no_dynamic_code: (flags(ProcessDynamicCodePolicy, "dynamic-code policy read-back")? & 1) != 0,
        strict_handle_checks: (flags(ProcessStrictHandleCheckPolicy, "handle-check policy read-back")? & 1) != 0,
        microsoft_signed_only: (flags(ProcessSignaturePolicy, "signature policy read-back")? & 1) != 0,
    })
}

/// A helper's commit as the system counts it, in bytes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Commit {
    /// Memory committed privately (what the job's cap counts).
    pub(crate) private: u64,
    /// Memory committed through shared sections (what the job's cap does not count).
    pub(crate) shared: u64,
}

/// Reads a process's commit. [`SandboxError::NeedsNewerWindows`] when this Windows does not report the shared commit
/// (the extended counters need Windows 10 22H2 or Windows 11 22H2 with the September 2023 update).
pub(crate) fn commit(process: HANDLE) -> Result<Commit, SandboxError> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        // A value no process has: a Windows that fills only the older, shorter structure leaves it standing.
        SharedCommitUsage: u64::MAX,
        ..Default::default()
    };
    // SAFETY: `process` is an open process handle with query access; `counters` is writable for `cb` bytes and starts
    // with the shorter structure the call is declared with.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            process,
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    if ok == 0 {
        // SAFETY: reads the calling thread's last-error value; no pointer is involved.
        let code = unsafe { GetLastError() };
        return Err(if UNKNOWN_SIZE.contains(&code) {
            SandboxError::NeedsNewerWindows
        } else {
            SandboxError::Os {
                step: "memory read",
                code,
            }
        });
    }
    if counters.SharedCommitUsage == u64::MAX {
        return Err(SandboxError::NeedsNewerWindows);
    }
    Ok(Commit {
        private: counters.PrivateUsage as u64,
        shared: counters.SharedCommitUsage,
    })
}

/// One helper's memory watch: a thread on its own duplicates of the helper's process and job handles. Ending the
/// watch (or dropping it) stops the thread and waits for it, at most one poll.
#[derive(Debug)]
pub(crate) struct Watch {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    /// Starts the watch. `granted` is the helper's counter of granted bytes still alive; `killed` is set before the
    /// watch ends the helper.
    pub(crate) fn start(
        process: &OwnedHandle,
        job: &OwnedHandle,
        limit: u64,
        granted: Arc<AtomicU64>,
        killed: Arc<AtomicBool>,
    ) -> Result<Self, SandboxError> {
        let duplicate = |handle: &OwnedHandle| {
            // Not inheritable, the same access: the thread's own handles, closed when it ends.
            handle.try_clone().map_err(|error| SandboxError::Os {
                step: "watch handle",
                code: error.raw_os_error().unwrap_or(0) as u32,
            })
        };
        let (process, job) = (duplicate(process)?, duplicate(job)?);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("helper-memory-watch".into())
            .spawn(move || watch(&process, &job, limit, &granted, &killed, &stopping))
            .map_err(|error| SandboxError::Os {
                step: "watch thread",
                code: error.raw_os_error().unwrap_or(0) as u32,
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// Stops the watch and waits for its thread.
    pub(crate) fn finish(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.finish();
    }
}

/// The watch thread: waits on the process (so it ends with it), reads its commit after every poll, and ends it when
/// it is over or cannot be read. Never stops watching a helper that still runs, except when asked to (`stop`).
fn watch(
    process: &OwnedHandle,
    job: &OwnedHandle,
    limit: u64,
    granted: &AtomicU64,
    killed: &AtomicBool,
    stop: &AtomicBool,
) {
    loop {
        // SAFETY: `process` is this thread's own open duplicate of the helper's process handle.
        match unsafe { WaitForSingleObject(process.as_raw_handle(), POLL_MS) } {
            WAIT_OBJECT_0 => return,
            WAIT_TIMEOUT if stop.load(Ordering::SeqCst) => return,
            WAIT_TIMEOUT => {}
            // The wait itself failed: the helper is not left unwatched.
            _ => {
                killed.store(true, Ordering::SeqCst);
                let _ = end(process, job);
                return;
            }
        }
        let over = match commit(process.as_raw_handle()) {
            Ok(now) => over_memory(now.private, now.shared, limit, granted.load(Ordering::SeqCst)),
            // A helper that ended by itself between the wait and the read was not ended for its memory.
            Err(_) if ended(process) => return,
            Err(_) => true,
        };
        if over {
            // Before the kill, so whoever sees the helper's end also sees why.
            killed.store(true, Ordering::SeqCst);
            let _ = end(process, job);
            return;
        }
    }
}

fn ended(process: &OwnedHandle) -> bool {
    // SAFETY: `process` is an open process handle; a zero wait only asks.
    unsafe { WaitForSingleObject(process.as_raw_handle(), 0) == WAIT_OBJECT_0 }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::System::JobObjects::CreateJobObjectW;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::*;
    use crate::{Profile, check_facts};

    /// The reader reads real values: this test process is in no job of ours, no container, and above low integrity;
    /// its own commit is read too. `check_facts` refuses it.
    #[test]
    fn read_facts_reads_a_process_that_is_not_confined() {
        // SAFETY: null attributes and name: an unnamed job nothing else can open.
        let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) });
        // SAFETY: plain call; the result is checked before use.
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                GetCurrentProcessId(),
            )
        };
        assert!(!raw.is_null(), "this process can be opened");
        let me = owned(raw);
        let expected = CapabilitySids::of(Profile::NoCapabilities).unwrap();
        let facts = read_facts(&me, &job, &expected).unwrap();
        assert!(!facts.in_job, "not in a job made just now");
        assert!(!facts.app_container);
        assert!(!facts.exact_capabilities && !facts.less_privileged);
        assert!(
            facts.integrity_rid >= 0x2000,
            "medium or higher: {:#x}",
            facts.integrity_rid
        );
        assert_eq!(
            check_facts(&facts, Profile::NoCapabilities),
            Err(SandboxError::NotConfined("not in the job"))
        );

        let now = commit(me.as_raw_handle()).unwrap();
        assert!(now.private > 0, "a running test has private commit");
    }

    /// The less-privileged attribute is read the documented way on this Windows (no refused token class), and a
    /// token that is no less-privileged container (this test's own) reads as not carrying it.
    #[test]
    fn the_lpac_attribute_reads_false_on_a_plain_token() {
        let mut raw: HANDLE = null_mut();
        // SAFETY: the pseudo-handle of this process is always valid; `raw` receives a new handle.
        let opened = unsafe {
            OpenProcessToken(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                TOKEN_QUERY,
                &mut raw,
            )
        };
        assert_ne!(opened, 0, "this process's token can be opened");
        let token = owned(raw);
        assert_eq!(no_all_application_packages(&token), Ok(false));
        assert!(!token_flag(&token, TokenIsAppContainer, "container read-back").unwrap());
    }
}
