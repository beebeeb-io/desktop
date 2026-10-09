use std::ffi::CStr;
use std::os::raw::c_char;
use std::time::Duration;

use crate::finder_setup::core::DomainState;
use crate::finder_setup::error::{FpError, FpErrorCode, app_code};

/// Mirror of `BeebeebFpError` in `src-tauri/macos/FileProviderBridge.m` (spec §6.1). The C side
/// `_Static_assert`s the same size and field offsets, and the `bridge_error_tests` module compares
/// `sizeof`, `alignof` and every `offsetof` exported from Objective-C against this struct at
/// runtime, so a one-sided edit fails a test and not only a compile.
#[repr(C)]
pub struct BeebeebFpErrorC {
    pub code: i64,
    pub underlying_code: i64,
    pub has_underlying: i32,
    pub domain: [c_char; 128],
    pub underlying_domain: [c_char; 128],
    pub message: [c_char; 1024],
}

const _: () = assert!(std::mem::size_of::<BeebeebFpErrorC>() == 1304);

impl BeebeebFpErrorC {
    /// Every call starts from an all-zero struct, so a bridge path that fails without filling the
    /// error is seen as "no domain" and becomes an app error, never as stale or garbage data.
    pub fn zeroed() -> Self {
        Self {
            code: 0,
            underlying_code: 0,
            has_underlying: 0,
            domain: [0; 128],
            underlying_domain: [0; 128],
            message: [0; 1024],
        }
    }

    pub fn into_fp_error(self) -> FpError {
        let domain = c_array_to_string(&self.domain);
        if domain.is_empty() {
            return FpError::app(
                app_code::UNEXPECTED_BRIDGE_RETURN,
                "the File Provider bridge failed without describing the error",
            );
        }
        FpError {
            domain,
            code: self.code,
            message: c_array_to_string(&self.message),
            underlying: (self.has_underlying != 0).then(|| FpErrorCode {
                domain: c_array_to_string(&self.underlying_domain),
                code: self.underlying_code,
            }),
        }
    }
}

/// Bounded read: never past the array, even if the C side forgot the NUL.
fn c_array_to_string(array: &[c_char]) -> String {
    let bytes: Vec<u8> = array.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

unsafe extern "C" {
    fn beebeeb_fp_visible_url(url_buffer: *mut c_char, url_buffer_len: usize, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_domain_user_enabled(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_add_domain(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_wait_for_domain_ready(timeout_seconds: f64, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_remove(
        location_buffer: *mut c_char,
        location_buffer_len: usize,
        kept_state: *mut i32,
        out_error: *mut BeebeebFpErrorC,
    ) -> i32;
    fn beebeeb_fp_kept_folder_state_for_path(path: *const c_char) -> i32;
    fn beebeeb_fp_signal_working_set(out_error: *mut BeebeebFpErrorC) -> i32;
}

/// Room for the folder macOS reports after a removal that kept files (task 1882). File-system
/// paths on macOS are limited to `PATH_MAX` (1024) bytes, well under this.
const PRESERVED_LOCATION_BUFFER_LEN: usize = 4096;

/// Run one bridge primitive; a negative return carries the filled error.
fn call(function: unsafe extern "C" fn(*mut BeebeebFpErrorC) -> i32) -> Result<i32, FpError> {
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { function(&mut out) };
    if code >= 0 { Ok(code) } else { Err(out.into_fp_error()) }
}

/// `NSFileProviderDomain.userEnabled` for the Beebeeb domain, read from the OS's own live
/// registry (`getDomainsWithCompletionHandler`), not a locally-constructed
/// `NSFileProviderDomain` object (which carries no system state). `NotRegistered` is a fresh
/// install, or the state after a clean `removeDomain`: not itself evidence of anything wrong.
pub fn domain_state() -> Result<DomainState, FpError> {
    match call(beebeeb_fp_domain_user_enabled)? {
        1 => Ok(DomainState::Enabled),
        0 => Ok(DomainState::Disabled),
        2 => Ok(DomainState::NotRegistered),
        other => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("beebeeb_fp_domain_user_enabled returned {other}"),
        )),
    }
}

/// `+[NSFileProviderManager addDomain:completionHandler:]` only; does not wait for stabilization.
pub fn add_domain() -> Result<(), FpError> {
    call(beebeeb_fp_add_domain).map(|_| ())
}

pub fn wait_for_domain_ready(timeout: Duration) -> Result<(), FpError> {
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_fp_wait_for_domain_ready(timeout.as_secs_f64(), &mut out) };
    if code < 0 { Err(out.into_fp_error()) } else { Ok(()) }
}

pub fn visible_url() -> Result<Option<String>, FpError> {
    let mut url_buffer = [0 as c_char; 2048];
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_fp_visible_url(url_buffer.as_mut_ptr(), url_buffer.len(), &mut out) };
    match code {
        c if c < 0 => Err(out.into_fp_error()),
        0 => Ok(None),
        _ => Ok(Some(c_array_to_string(&url_buffer)).filter(|s| !s.is_empty())),
    }
}

/// The bridge's own folder check on a path (the same function the removal runs on the URL macOS
/// returned). `stat` always answers in the sandbox, so a folder that appeared since is seen.
/// A path that cannot be handed over (an interior NUL) is shown, never hidden.
fn kept_state_for_path(path: &str) -> i32 {
    match std::ffi::CString::new(path) {
        Ok(c_path) => unsafe { beebeeb_fp_kept_folder_state_for_path(c_path.as_ptr()) },
        Err(_) => crate::finder_removal::KEPT_UNCHECKED,
    }
}

/// A removal the bridge reported as failed: the OS error as spec 2026-10-06 §6.1 carries it (the
/// reconciler classifies it, and callers see its domain and code only), and the folder macOS kept
/// all the same (task 1882, review M2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRemovalFailure {
    pub error: FpError,
    pub kept: crate::finder_removal::KeptFolder,
}

impl BridgeRemovalFailure {
    /// Like `finder_removal::RemovalFailure::kept_location`: the folder to show, if any; logs never
    /// carry it.
    pub fn kept_location(self, context: &'static str) -> Option<String> {
        crate::finder_removal::DomainRemoval { kept: self.kept }.kept_location(context)
    }
}

/// Decodes the bridge's reply to a removal, for EVERY removal (`remove()` and the sweep). A folder
/// that reads as missing is looked at again first (re-review D1, `settle_kept_state`), so the one
/// early look inside the completion handler cannot hide files macOS puts there a moment later.
/// `finder_removal`'s source pin keeps this the only place that decodes a reply.
fn decode_removal(
    code: i32,
    kept_state: i32,
    location_buffer: &[c_char],
    error_buffer: BeebeebFpErrorC,
) -> Result<crate::finder_removal::DomainRemoval, BridgeRemovalFailure> {
    decode_removal_with(
        code,
        kept_state,
        location_buffer,
        error_buffer,
        kept_state_for_path,
        std::thread::sleep,
    )
}

/// [`decode_removal`] with the look and the wait injected, so a test drives the real decode
/// without sleeping. The error crosses as spec 2026-10-06 §6.1's structured error; a return code
/// the bridge does not document is an app error, as for every other primitive.
fn decode_removal_with(
    code: i32,
    kept_state: i32,
    location_buffer: &[c_char],
    error_buffer: BeebeebFpErrorC,
    check: impl FnMut(&str) -> i32,
    wait: impl FnMut(std::time::Duration),
) -> Result<crate::finder_removal::DomainRemoval, BridgeRemovalFailure> {
    let location = buffer_to_exact_string(location_buffer);
    let kept_state = crate::finder_removal::settle_kept_state(kept_state, location.as_deref(), check, wait);
    let error = (code < 0).then(|| error_buffer.into_fp_error());
    crate::finder_removal::removal_from_bridge(
        code,
        kept_state,
        location,
        error.as_ref().map(|error| error.message.clone()),
    )
    .map_err(|failure| BridgeRemovalFailure {
        error: error.unwrap_or_else(|| FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, failure.message)),
        kept: failure.kept,
    })
}

/// Removes our Finder location, keeping the files that never reached the server (task 1882,
/// `NSFileProviderDomainRemovalModePreserveDirtyUserData`). The result says whether macOS kept
/// anything, checked on disk (round 2, spec §4), and names the folder if so.
pub fn remove() -> Result<crate::finder_removal::DomainRemoval, BridgeRemovalFailure> {
    let mut location_buffer = [0 as c_char; PRESERVED_LOCATION_BUFFER_LEN];
    let mut kept_state: i32 = crate::finder_removal::KEPT_NONE_REPORTED;
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe {
        beebeeb_fp_remove(
            location_buffer.as_mut_ptr(),
            location_buffer.len(),
            &mut kept_state,
            &mut out,
        )
    };
    decode_removal(code, kept_state, &location_buffer, out)
}

/// Result of a best-effort working-set signal (task 1697).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkingSetSignalOutcome {
    /// The system accepted the signal (or the domain is not registered —
    /// nothing to signal is not a failure; sync continues regardless).
    Delivered,
    /// The signal itself failed (timeout, NSFileProviderManager error). Logged
    /// by the caller; never blocks the sync tick that produced it.
    Failed,
}

/// Task 1698 part 3 (closes 1696, audit G8): the ONE domain we own. The
/// zombie `io.beebeeb.desktop.FileProvider` domain from the pre-rename bundle
/// id (task 1696 forensics) keeps its "Beebeeb-Drive" volume in
/// `~/Library/CloudStorage` until a SIGNED context enumerates and removes it
/// (`NSFileProviderManager.getDomains` needs app identity — an unsigned CLI
/// gets error -2001). Must stay identical to `BeebeebDomainIdentifier` in
/// `src-tauri/macos/FileProviderBridge.m`.
///
/// **Correction — 2026-10-06 (spec `docs/specs/2026-10-06-macos-finder-setup-reconciler.md` §2):**
/// the paragraph above was wrong on two counts. The zombie's folder is `Beebeeb-Beebeeb`, not
/// `Beebeeb-Drive`. And no signed `io.beebeeb.app` context can enumerate it, because
/// `NSFileProviderManager.getDomains` returns only the CALLING provider's domains, so the
/// sweep below can never see `io.beebeeb.desktop.FileProvider`. The orphan is cleared by the
/// one-off helper of spec §11. The 1696 forensics read the zombie's display name ("Beebeeb"),
/// and 1698 renamed our domain to match it, which created the folder collision.
pub const DOMAIN_IDENTIFIER: &str = "io.beebeeb.app.domain";

/// Pure filter: which of the system's registered domains must be REMOVED?
/// Every domain whose identifier is not EXACTLY ours is stale — a zombie
/// from an older bundle id, a renamed registration, or another app's domain
/// that must never have been created in our app group context. Our own
/// identifier is never stale (the cleanup can never remove the live domain,
/// even if the system reports it twice). The comparison is exact and
/// case-sensitive: `io.beebeeb.app.domain` ≠ `IO.BEEBEEB.APP.DOMAIN`.
pub fn stale_domain_identifiers<'a>(domains: &'a [String], ours: &str) -> Vec<&'a str> {
    domains
        .iter()
        .map(String::as_str)
        .filter(|identifier| *identifier != ours)
        .collect()
}

/// What one app-start cleanup run did (logged honestly; also surfaced for
/// tests and diagnostics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleDomainCleanup {
    /// Identifiers that were present and stale, with their removal result:
    /// Ok(()) = removed, Err(error) = the system refused (logged, NOT
    /// retried here — the next app start retries the whole sweep).
    pub removals: Vec<(String, Result<(), FpError>)>,
    /// Task 1882: the folders where macOS kept un-synced files of a removed
    /// domain, one per such removal. Shown to the person, never logged.
    pub preserved_locations: Vec<String>,
    /// Domains found foreign but not attempted (enumeration/other errors).
    pub skipped: Vec<String>,
    /// Our own domain was present (or not) — informational only; it is never
    /// touched.
    pub ours_present: bool,
}

impl StaleDomainCleanup {
    pub fn removed_count(&self) -> usize {
        self.removals.iter().filter(|(_, result)| result.is_ok()).count()
    }
}

#[cfg(target_os = "macos")]
mod cleanup_ffi {
    use super::{BeebeebFpErrorC, c_array_to_string};
    use crate::finder_setup::error::FpError;
    use std::os::raw::c_char;

    unsafe extern "C" {
        fn beebeeb_fp_list_domains(
            ids_buffer: *mut c_char,
            ids_buffer_len: usize,
            out_error: *mut BeebeebFpErrorC,
        ) -> i32;
        fn beebeeb_fp_remove_domain_by_id(
            identifier: *const c_char,
            location_buffer: *mut c_char,
            location_buffer_len: usize,
            kept_state: *mut i32,
            out_error: *mut BeebeebFpErrorC,
        ) -> i32;
    }

    /// Enumerate every registered domain identifier via the ObjC bridge.
    /// Newline-separated in the buffer; returns them split, or the bridge's error.
    pub fn list_domains() -> Result<Vec<String>, FpError> {
        let mut ids_buffer = [0 as c_char; 8192];
        let mut out = BeebeebFpErrorC::zeroed();
        let count = unsafe { beebeeb_fp_list_domains(ids_buffer.as_mut_ptr(), ids_buffer.len(), &mut out) };
        if count < 0 {
            return Err(out.into_fp_error());
        }
        Ok(c_array_to_string(&ids_buffer)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Task 1882: keeps the domain's un-synced files, like every removal.
    pub fn remove_domain(
        identifier: &str,
    ) -> Result<crate::finder_removal::DomainRemoval, super::BridgeRemovalFailure> {
        let c_id = std::ffi::CString::new(identifier).map_err(|_| super::BridgeRemovalFailure {
            error: FpError::app(
                crate::finder_setup::error::app_code::UNEXPECTED_BRIDGE_RETURN,
                "domain identifier contained a NUL byte",
            ),
            kept: crate::finder_removal::KeptFolder::default(),
        })?;
        let mut location_buffer = [0 as c_char; super::PRESERVED_LOCATION_BUFFER_LEN];
        let mut kept_state: i32 = crate::finder_removal::KEPT_NONE_REPORTED;
        let mut out = BeebeebFpErrorC::zeroed();
        let code = unsafe {
            beebeeb_fp_remove_domain_by_id(
                c_id.as_ptr(),
                location_buffer.as_mut_ptr(),
                location_buffer.len(),
                &mut kept_state,
                &mut out,
            )
        };
        super::decode_removal(code, kept_state, &location_buffer, out)
    }
}

/// App-start stale-domain sweep (task 1698 part 3 / 1696 acceptance). Runs in
/// the SIGNED app process only — the Tauri app holds the Developer ID
/// identity the NSFileProviderManager calls require; an unsigned CLI context
/// gets -2001, which surfaces here as Err and is logged by the caller
/// WITHOUT failing anything (the engine does not depend on this).
///
/// Idempotent by construction: a second run finds no foreign domains and
/// removes nothing. Our own domain is NEVER a removal candidate (the pure
/// filter above guarantees it). Per-domain removal failures are recorded and
/// skipped — one stubborn zombie never blocks the rest or the app.
#[cfg(target_os = "macos")]
pub fn cleanup_stale_domains() -> Result<StaleDomainCleanup, FpError> {
    let domains = match cleanup_ffi::list_domains() {
        Ok(domains) => domains,
        Err(error) => return Err(error),
    };
    let ours_present = domains.iter().any(|identifier| identifier == DOMAIN_IDENTIFIER);
    let stale = stale_domain_identifiers(&domains, DOMAIN_IDENTIFIER);
    let mut cleanup = StaleDomainCleanup {
        removals: Vec::new(),
        preserved_locations: Vec::new(),
        skipped: Vec::new(),
        ours_present,
    };
    for identifier in stale {
        match cleanup_ffi::remove_domain(identifier) {
            Ok(removal) => {
                if let Some(location) = removal.kept_location("stale-domain sweep") {
                    cleanup.preserved_locations.push(location);
                }
                cleanup.removals.push((identifier.to_string(), Ok(())));
            }
            Err(failure) => {
                tracing::warn!(identifier = %identifier, error = %failure.error, "stale-domain removal failed; the next app start retries");
                // Review M2: a folder kept with the error still reaches the person.
                if let Some(location) = failure.kept_location("stale-domain sweep") {
                    cleanup.preserved_locations.push(location);
                }
                cleanup.skipped.push(identifier.to_string());
            }
        }
    }
    Ok(cleanup)
}

/// Pure decision core for the runner's signal path: an operation batch that
/// changed items always asks for a working-set signal (Replicated honors ONLY
/// `.workingSet`); anything else never touches the bridge.
pub fn should_signal_working_set(changed_item_ids: &[String]) -> bool {
    !changed_item_ids.is_empty()
}

/// Task 1697: call the FFI bridge to signal the replica's working set after
/// the daemon applied a change batch. Best-effort by contract — a failure is
/// returned for logging and must never fail the sync tick that produced it.
pub fn signal_working_set() -> Result<WorkingSetSignalOutcome, FpError> {
    match call(beebeeb_fp_signal_working_set)? {
        0 => Ok(WorkingSetSignalOutcome::Delivered),
        other => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("beebeeb_fp_signal_working_set returned {other}"),
        )),
    }
}

/// Never trims, unlike `c_array_to_string`: a folder the system reported is shown byte for byte
/// (task 1882). `None` when the buffer is empty.
fn buffer_to_exact_string(buffer: &[c_char]) -> Option<String> {
    let text = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Task 1698 part 3: stale domain cleanup (closes 1696, audit G8) ─────

    #[test]
    fn test_1698_stale_domain_filter_removes_foreign_domains_keeps_ours() {
        // The zombie from the 0.8.6 era (io.beebeeb.desktop.FileProvider) and
        // anything else foreign must be removed; the CURRENT domain never.
        let domains = vec![
            "io.beebeeb.desktop.FileProvider".to_string(),
            "io.beebeeb.app.domain".to_string(),
            "com.other.zombie".to_string(),
        ];
        let stale = stale_domain_identifiers(&domains, "io.beebeeb.app.domain");
        assert_eq!(
            stale,
            vec!["io.beebeeb.desktop.FileProvider", "com.other.zombie"],
            "every non-Beebeeb domain is stale"
        );
    }

    #[test]
    fn test_1698_stale_domain_filter_is_idempotent_and_safe() {
        // A clean registry removes nothing.
        assert_eq!(
            stale_domain_identifiers(&["io.beebeeb.app.domain".to_string()], "io.beebeeb.app.domain"),
            Vec::<&str>::new()
        );
        // An empty registry removes nothing.
        assert_eq!(
            stale_domain_identifiers(&[], "io.beebeeb.app.domain"),
            Vec::<&str>::new()
        );
        // NEVER a partial match: only exact foreign identifiers are stale.
        assert_eq!(
            stale_domain_identifiers(&["io.beebeeb.app.domain.stale".to_string()], "io.beebeeb.app.domain"),
            vec!["io.beebeeb.app.domain.stale"]
        );
        // Case matters: the identifier is compared exactly.
        assert_eq!(
            stale_domain_identifiers(&["IO.BEEBEEB.APP.DOMAIN".to_string()], "io.beebeeb.app.domain"),
            vec!["IO.BEEBEEB.APP.DOMAIN"],
            "a case-mangled copy of our id is NOT ours and must be cleaned"
        );
    }
}

/// Spec §13.1 "Bridge": a real NSError built in Objective-C crosses the FFI and Rust
/// receives its domain and code unchanged. Also pins the C struct layout and the bridge's
/// synthesized error codes against their Rust mirrors (lead ruling T1-2), and the memory-safety
/// properties of the buffer copies (bounded, NUL-terminated, zeroed before filling).
#[cfg(test)]
mod bridge_error_tests {
    use super::*;
    use crate::finder_setup::error::{APP_DOMAIN, BRIDGE_DOMAIN, FpErrorCode, app_code, bridge_code};
    use std::ffi::CString;
    use std::mem::{align_of, offset_of, size_of};

    unsafe extern "C" {
        fn beebeeb_fp_test_fill_error(
            domain: *const c_char,
            code: i64,
            message: *const c_char,
            underlying_domain: *const c_char,
            underlying_code: i64,
            out_error: *mut BeebeebFpErrorC,
        );
        fn beebeeb_fp_test_fill_bridge_error(code: i64, message: *const c_char, out_error: *mut BeebeebFpErrorC);
        fn beebeeb_fp_test_error_size() -> usize;
        fn beebeeb_fp_test_error_align() -> usize;
        /// code, underlying_code, has_underlying, domain, underlying_domain, message.
        fn beebeeb_fp_test_error_offsets(out: *mut usize);
        /// ManagerUnavailable, StabilizationTimeout, ResolveUrlTimeout, SignalTimeout, NoIdentifier.
        fn beebeeb_fp_test_bridge_codes(out: *mut i64);
        // The one real bridge entry point that fails before it touches the system (task 1882: its
        // removal reply carries the kept folder too).
        fn beebeeb_fp_remove_domain_by_id(
            identifier: *const c_char,
            location_buffer: *mut c_char,
            location_buffer_len: usize,
            kept_state: *mut i32,
            out_error: *mut BeebeebFpErrorC,
        ) -> i32;
    }

    fn through_bridge(domain: &str, code: i64, message: &str, underlying: Option<(&str, i64)>) -> FpError {
        let domain = CString::new(domain).unwrap();
        let message = CString::new(message).unwrap();
        let underlying_domain = underlying.map(|(d, _)| CString::new(d).unwrap());
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe {
            beebeeb_fp_test_fill_error(
                domain.as_ptr(),
                code,
                message.as_ptr(),
                underlying_domain.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                underlying.map_or(0, |(_, c)| c),
                &mut out,
            );
        }
        out.into_fp_error()
    }

    #[test]
    fn the_error_struct_has_the_same_size_on_both_sides() {
        assert_eq!(std::mem::size_of::<BeebeebFpErrorC>(), 1304);
        assert_eq!(
            unsafe { beebeeb_fp_test_error_size() },
            std::mem::size_of::<BeebeebFpErrorC>()
        );
    }

    #[test]
    fn every_field_sits_at_the_same_offset_on_both_sides() {
        let mut c_offsets = [usize::MAX; 6];
        unsafe { beebeeb_fp_test_error_offsets(c_offsets.as_mut_ptr()) };
        let rust_offsets = [
            offset_of!(BeebeebFpErrorC, code),
            offset_of!(BeebeebFpErrorC, underlying_code),
            offset_of!(BeebeebFpErrorC, has_underlying),
            offset_of!(BeebeebFpErrorC, domain),
            offset_of!(BeebeebFpErrorC, underlying_domain),
            offset_of!(BeebeebFpErrorC, message),
        ];
        assert_eq!(c_offsets, rust_offsets, "ObjC offsetof vs Rust offset_of!");
        assert_eq!(rust_offsets, [0, 8, 16, 20, 148, 276], "the layout the spec describes");
        assert_eq!(unsafe { beebeeb_fp_test_error_align() }, align_of::<BeebeebFpErrorC>());
        assert_eq!(size_of::<BeebeebFpErrorC>() % align_of::<BeebeebFpErrorC>(), 0);
    }

    #[test]
    fn the_bridge_error_codes_are_the_rust_constants() {
        let mut c_codes = [i64::MIN; 5];
        unsafe { beebeeb_fp_test_bridge_codes(c_codes.as_mut_ptr()) };
        assert_eq!(
            c_codes,
            [
                bridge_code::MANAGER_UNAVAILABLE,
                bridge_code::STABILIZATION_TIMEOUT,
                bridge_code::RESOLVE_URL_TIMEOUT,
                bridge_code::SIGNAL_TIMEOUT,
                bridge_code::NO_IDENTIFIER,
            ],
            "BeebeebBridge* enum in FileProviderBridge.m vs finder_setup::error::bridge_code"
        );
        assert_eq!(c_codes, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn every_classified_domain_and_code_crosses_the_bridge_unchanged() {
        let cases: &[(&str, i64)] = &[
            ("NSFileProviderErrorDomain", -2001),
            ("NSFileProviderErrorDomain", -2002),
            ("NSFileProviderErrorDomain", -2003),
            ("NSFileProviderErrorDomain", -2004),
            ("NSFileProviderErrorDomain", -2005),
            ("NSFileProviderErrorDomain", -2011),
            ("NSFileProviderErrorDomain", -2012),
            ("NSFileProviderErrorDomain", -2013),
            ("NSFileProviderErrorDomain", -2014),
            ("NSCocoaErrorDomain", 516),
            ("NSPOSIXErrorDomain", 17),
        ];
        for (domain, code) in cases {
            let error = through_bridge(domain, *code, "fixture", None);
            assert_eq!((error.domain.as_str(), error.code), (*domain, *code));
            assert_eq!(error.message, "fixture");
            assert_eq!(error.underlying, None, "{domain} {code}");
        }
    }

    #[test]
    fn the_underlying_error_crosses_with_its_own_domain_and_code() {
        let error = through_bridge("NSCocoaErrorDomain", 516, "exists", Some(("NSPOSIXErrorDomain", 17)));
        assert_eq!(
            error.underlying,
            Some(FpErrorCode {
                domain: "NSPOSIXErrorDomain".into(),
                code: 17
            })
        );
    }

    #[test]
    fn bridge_synthesized_errors_carry_the_bridge_domain() {
        let message =
            CString::new("Timed out waiting for the Beebeeb File Provider domain to become available").unwrap();
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe { beebeeb_fp_test_fill_bridge_error(bridge_code::STABILIZATION_TIMEOUT, message.as_ptr(), &mut out) };
        let error = out.into_fp_error();
        assert_eq!(
            (error.domain.as_str(), error.code),
            (BRIDGE_DOMAIN, bridge_code::STABILIZATION_TIMEOUT)
        );
        assert_eq!(error.underlying, None);
    }

    #[test]
    fn a_real_bridge_entry_point_reports_its_synthesized_code() {
        // No system call happens before the NULL check, so this runs the real call site of
        // BeebeebBridgeNoIdentifier on any machine.
        let mut out = BeebeebFpErrorC::zeroed();
        let code = unsafe {
            beebeeb_fp_remove_domain_by_id(
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut out,
            )
        };
        assert_eq!(code, -1);
        let error = out.into_fp_error();
        assert_eq!(
            (error.domain.as_str(), error.code),
            (BRIDGE_DOMAIN, bridge_code::NO_IDENTIFIER)
        );
        assert!(!error.message.is_empty());
    }

    #[test]
    fn a_long_message_is_cut_not_overflowed() {
        // 6000 UTF-8 bytes into a 1024-byte field: strlcpy may cut inside a code point, which
        // from_utf8_lossy turns into U+FFFD, so count chars, not bytes.
        let error = through_bridge("NSCocoaErrorDomain", 1, &"é".repeat(3000), None);
        assert!(
            error.message.chars().count() <= 1023,
            "{}",
            error.message.chars().count()
        );
        assert_eq!(error.domain, "NSCocoaErrorDomain");
    }

    #[test]
    fn an_overlong_ascii_message_fills_exactly_1023_bytes_and_the_last_byte_is_a_nul() {
        let domain = CString::new("NSCocoaErrorDomain").unwrap();
        let message = CString::new("m".repeat(5000)).unwrap();
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe { beebeeb_fp_test_fill_error(domain.as_ptr(), 1, message.as_ptr(), std::ptr::null(), 0, &mut out) };
        assert_eq!(out.message[1022], b'm' as c_char);
        assert_eq!(out.message[1023], 0, "strlcpy must terminate inside the field");
        assert_eq!(out.into_fp_error().message.len(), 1023);
    }

    #[test]
    fn an_overlong_domain_is_cut_and_terminated_without_touching_its_neighbours() {
        let domain = CString::new("d".repeat(300)).unwrap();
        let message = CString::new("kept").unwrap();
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe { beebeeb_fp_test_fill_error(domain.as_ptr(), 1, message.as_ptr(), std::ptr::null(), 0, &mut out) };
        assert_eq!(out.domain[126], b'd' as c_char);
        assert_eq!(out.domain[127], 0, "strlcpy must terminate inside the field");
        assert!(
            out.underlying_domain.iter().all(|c| *c == 0),
            "the domain overran into underlying_domain"
        );
        let error = out.into_fp_error();
        assert_eq!(error.domain.len(), 127);
        assert_eq!(error.message, "kept");
    }

    #[test]
    fn filling_clears_whatever_the_struct_held_before() {
        let mut out = BeebeebFpErrorC {
            code: -1,
            underlying_code: -1,
            has_underlying: 1,
            domain: [0x7f; 128],
            underlying_domain: [0x7f; 128],
            message: [0x7f; 1024],
        };
        let domain = CString::new("NSCocoaErrorDomain").unwrap();
        let message = CString::new("short").unwrap();
        unsafe { beebeeb_fp_test_fill_error(domain.as_ptr(), 4, message.as_ptr(), std::ptr::null(), 0, &mut out) };
        assert_eq!((out.has_underlying, out.underlying_code), (0, 0));
        assert!(
            out.underlying_domain.iter().all(|c| *c == 0),
            "stale underlying_domain survived the fill"
        );
        assert!(
            out.message[6..].iter().all(|c| *c == 0),
            "stale message bytes survived the fill"
        );
        let error = out.into_fp_error();
        assert_eq!(
            (error.domain.as_str(), error.code, error.message.as_str()),
            ("NSCocoaErrorDomain", 4, "short")
        );
        assert_eq!(error.underlying, None);
    }

    #[test]
    fn a_missing_nul_is_read_only_up_to_the_end_of_its_own_array() {
        let mut out = BeebeebFpErrorC::zeroed();
        out.code = 9;
        out.domain = [b'a' as c_char; 128];
        out.underlying_domain = [b'b' as c_char; 128];
        out.message = [b'm' as c_char; 1024];
        let error = out.into_fp_error();
        assert_eq!(
            error.domain,
            "a".repeat(128),
            "a read past the array would append the b bytes"
        );
        assert_eq!(error.message, "m".repeat(1024));
    }

    #[test]
    fn an_unfilled_error_becomes_an_app_error_not_an_empty_one() {
        let error = BeebeebFpErrorC::zeroed().into_fp_error();
        assert_eq!(
            (error.domain.as_str(), error.code),
            (APP_DOMAIN, app_code::UNEXPECTED_BRIDGE_RETURN)
        );
    }
}

/// Task 1882 round 2 (device K-F2, review I3, spec §4/§8): the bridge's own folder check, run on
/// a real disk through its test-only entry point, which builds the same `NSURL` and calls the
/// same function the removal calls.
#[cfg(test)]
mod kept_folder_check_tests {
    use crate::finder_removal::{KEPT_EMPTY, KEPT_HAS_ENTRIES, KEPT_MISSING, KEPT_NONE_REPORTED, KEPT_UNCHECKED};
    use std::ffi::CString;
    use std::os::unix::fs::PermissionsExt;

    use super::beebeeb_fp_kept_folder_state_for_path;

    fn state_of(path: &std::path::Path) -> i32 {
        let c_path = CString::new(path.as_os_str().as_encoded_bytes()).expect("no NUL in a temp path");
        unsafe { beebeeb_fp_kept_folder_state_for_path(c_path.as_ptr()) }
    }

    #[test]
    fn test_1882_r2_bridge_check_a_missing_folder_is_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            state_of(&dir.path().join("Beebeeb-Drive (10-10-2026 10:52)")),
            KEPT_MISSING
        );
        assert_eq!(
            unsafe { beebeeb_fp_kept_folder_state_for_path(std::ptr::null()) },
            KEPT_NONE_REPORTED,
            "no URL is 'none reported'"
        );
    }

    #[test]
    fn test_1882_r2_bridge_check_an_empty_folder_is_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let kept = dir.path().join("Beebeeb-Beebeeb (10-10-2026 10:50)");
        std::fs::create_dir(&kept).expect("kept folder");
        assert_eq!(state_of(&kept), KEPT_EMPTY);
    }

    #[test]
    fn test_1882_r2_bridge_check_a_folder_with_one_item_has_entries() {
        let dir = tempfile::tempdir().expect("temp dir");
        let kept = dir.path().join("Beebeeb-Beebeeb (10-10-2026 10:50)");
        std::fs::create_dir(&kept).expect("kept folder");
        std::fs::write(kept.join("k1882.txt"), b"never uploaded").expect("kept file");
        assert_eq!(state_of(&kept), KEPT_HAS_ENTRIES);
    }

    #[test]
    fn test_1882_r2_bridge_check_a_hidden_item_or_a_single_file_counts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let hidden_only = dir.path().join("hidden-only");
        std::fs::create_dir(&hidden_only).expect("folder");
        std::fs::write(hidden_only.join(".env"), b"a dotfile is the person's data too").expect("dotfile");
        assert_eq!(state_of(&hidden_only), KEPT_HAS_ENTRIES);

        let single_file = dir.path().join("kept-file.bin");
        std::fs::write(&single_file, b"one kept file").expect("file");
        assert_eq!(state_of(&single_file), KEPT_HAS_ENTRIES);
    }

    #[test]
    fn test_1882_r2_bridge_check_a_folder_it_may_not_list_is_unchecked() {
        let dir = tempfile::tempdir().expect("temp dir");
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).expect("folder");
        std::fs::write(locked.join("k1882.txt"), b"x").expect("file");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod 000");
        let state = state_of(&locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).expect("chmod back");
        assert_eq!(
            state, KEPT_UNCHECKED,
            "an existing folder the app may not list is shown (spec §4's fallback), never hidden"
        );
    }

    // ── round 3 (re-review D1): a missing folder is looked at again, on a real disk ──────────

    /// The production settle, with the real check on a real disk, and a wait that does not sleep:
    /// it runs `on_wait` (what macOS might do meanwhile) and counts the waits.
    fn settle_on_disk(path: &std::path::Path, mut on_wait: impl FnMut(usize)) -> (i32, usize) {
        let mut waits = 0usize;
        let state =
            crate::finder_removal::settle_kept_state(KEPT_MISSING, path.to_str(), super::kept_state_for_path, |_| {
                waits += 1;
                on_wait(waits);
            });
        (state, waits)
    }

    #[test]
    fn test_1882_r3_a_folder_filled_after_the_completion_handler_is_seen() {
        let dir = tempfile::tempdir().expect("temp dir");
        let kept = dir.path().join("Beebeeb-Beebeeb (10-10-2026 10:50)");
        assert_eq!(state_of(&kept), KEPT_MISSING, "missing when the handler runs");
        let (state, waits) = settle_on_disk(&kept, |wait| {
            if wait == 2 {
                std::fs::create_dir(&kept).expect("kept folder");
                std::fs::write(kept.join("k1882.txt"), b"never uploaded").expect("kept file");
            }
        });
        assert_eq!(
            state, KEPT_HAS_ENTRIES,
            "the files that appeared during the second wait are found"
        );
        assert_eq!(waits, 2, "and it stops looking once it has seen them");
    }

    #[test]
    fn test_1882_r3_a_folder_that_exists_but_is_still_empty_is_seen() {
        let dir = tempfile::tempdir().expect("temp dir");
        let kept = dir.path().join("Beebeeb-Beebeeb (10-10-2026 10:50)");
        let (state, _) = settle_on_disk(&kept, |wait| {
            if wait == 1 {
                std::fs::create_dir(&kept).expect("kept folder");
            }
        });
        assert_eq!(state, KEPT_EMPTY, "an existing folder is never reported missing");
    }

    #[test]
    fn test_1882_r3_a_folder_missing_throughout_stays_missing_after_a_bounded_look() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (state, waits) = settle_on_disk(&dir.path().join("Beebeeb-Drive (10-10-2026 10:52)"), |_| {});
        assert_eq!(state, KEPT_MISSING);
        assert_eq!(waits, crate::finder_removal::KEPT_MISSING_RECHECKS as usize);
    }

    fn buffer_of(text: &str) -> [std::os::raw::c_char; 4096] {
        let mut buffer = [0 as std::os::raw::c_char; 4096];
        for (slot, byte) in buffer.iter_mut().zip(text.bytes()) {
            *slot = byte as std::os::raw::c_char;
        }
        buffer
    }

    /// The bridge's structured error (spec 2026-10-06 §6.1) as `BeebeebFillError` leaves it for an
    /// NSError `NSFileProviderErrorDomain -1001` whose description is `message`. Blank: no error.
    fn bridge_error(message: &str) -> super::BeebeebFpErrorC {
        let mut out = super::BeebeebFpErrorC::zeroed();
        if !message.is_empty() {
            out.code = -1001;
            for (slot, byte) in out.domain.iter_mut().zip("NSFileProviderErrorDomain".bytes()) {
                *slot = byte as std::os::raw::c_char;
            }
            for (slot, byte) in out.message.iter_mut().zip(message.bytes()) {
                *slot = byte as std::os::raw::c_char;
            }
        }
        out
    }

    /// The real decode (the one `remove()` and the sweep both call), on a real disk, with the
    /// wait injected: files that appear after the completion handler reach the person.
    #[test]
    fn test_1882_r3_the_decode_uses_what_the_settle_found() {
        let dir = tempfile::tempdir().expect("temp dir");
        let kept = dir.path().join("Beebeeb-Beebeeb (10-10-2026 10:50)");
        let location = kept.to_str().expect("utf-8 temp path").to_string();
        for (code, error) in [(0, ""), (-1, "busy")] {
            let _ = std::fs::remove_dir_all(&kept);
            let mut waits = 0;
            let decoded = super::decode_removal_with(
                code,
                KEPT_MISSING,
                &buffer_of(&location),
                bridge_error(error),
                super::kept_state_for_path,
                |_| {
                    waits += 1;
                    if waits == 3 {
                        std::fs::create_dir(&kept).expect("kept folder");
                        std::fs::write(kept.join("k1882.txt"), b"never uploaded").expect("kept file");
                    }
                },
            );
            let kept_by_it = match decoded {
                Ok(removal) => removal.kept_location("test"),
                Err(failure) => {
                    assert_eq!(
                        (
                            failure.error.domain.as_str(),
                            failure.error.code,
                            failure.error.message.as_str()
                        ),
                        ("NSFileProviderErrorDomain", -1001, error),
                        "the error is reported as before, as spec 2026-10-06 §6.1's structured error"
                    );
                    failure.kept_location("test")
                }
            };
            assert_eq!(
                kept_by_it,
                Some(location.clone()),
                "code {code}: the late files are kept"
            );
            assert_eq!(waits, 3, "code {code}: it stopped looking once it saw them");
        }
    }

    #[test]
    fn test_1882_r3_the_decode_reports_nothing_kept_for_a_folder_missing_throughout() {
        let dir = tempfile::tempdir().expect("temp dir");
        let location = dir.path().join("Beebeeb-Drive (10-10-2026 10:52)");
        let location = location.to_str().expect("utf-8 temp path");
        let mut waits = 0;
        let decoded = super::decode_removal_with(
            0,
            KEPT_MISSING,
            &buffer_of(location),
            super::BeebeebFpErrorC::zeroed(),
            super::kept_state_for_path,
            |_| waits += 1,
        )
        .expect("the removal succeeded");
        assert_eq!(decoded.kept_location("test"), None);
        assert_eq!(waits, crate::finder_removal::KEPT_MISSING_RECHECKS as usize);
    }
}
