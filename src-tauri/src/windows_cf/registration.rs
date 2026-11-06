//! The sync root's registration, split out of `windows_cf/mod.rs` (LD-0105: the file-size ratchet):
//! who registers (the app's provider, or a test's [`QA_PROVIDER`]), the Win32 hydration policy and
//! its read-back (LD-0105 S7), the Explorer registration's stable Id, and whether an existing
//! Explorer registration is kept ([`shell_step`]: always).

use std::os::windows::ffi::OsStrExt;

use windows::Win32::Storage::CloudFilters::*;
use windows::core::*;

use super::{BEEBEEB_PROVIDER_GUID, PROVIDER_ID, PROVIDER_VERSION};

/// Who registers a sync root: the name and version Windows shows, and the provider GUID.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SyncProvider {
    pub name: &'static str,
    pub version: &'static str,
    pub guid: GUID,
}

/// The app's own provider identity. Never used by a test (LD-0105 rule: a test registers its own
/// root with [`QA_PROVIDER`], so nothing a test does can touch the person's Beebeeb registration).
pub(crate) const BEEBEEB_PROVIDER: SyncProvider = SyncProvider {
    name: PROVIDER_ID,
    version: PROVIDER_VERSION,
    guid: BEEBEEB_PROVIDER_GUID,
};

/// The identity every test sync root registers with: its own name and GUID (minted 2026-11-05 for
/// LD-0105, `2e0cdc60-1a9c-48f3-81c9-3e402af235b0`), never the app's.
#[cfg(test)]
pub(crate) const QA_PROVIDER: SyncProvider = SyncProvider {
    name: "BeebeebQA",
    version: PROVIDER_VERSION,
    guid: GUID::from_u128(0x2e0cdc60_1a9c_48f3_81c9_3e402af235b0),
};

/// Register a TEST sync root (tests only): [`QA_PROVIDER`], today's policies.
#[cfg(test)]
pub(crate) fn register_test_sync_root(sync_root_path: &std::path::Path) -> anyhow::Result<()> {
    super::register_sync_root_with(sync_root_path, &QA_PROVIDER, HYDRATION_POLICY)
}

/// LD-0105 S7 (W1): the Win32 hydration policy. `PROGRESSIVE` completes a reader's IO as soon as its
/// bytes arrive and keeps hydrating in the background; `PARTIAL`, used up to 0.8.21, is documented
/// "not currently supported". Re-registering with `CF_REGISTER_FLAG_UPDATE` on every engine start
/// moves an existing root over in place; no unregister, no provider-version bump (a bump would run
/// the WinRT Unregister, and an unregister deletes every cloud-only placeholder).
pub(crate) const HYDRATION_POLICY: CF_HYDRATION_POLICY_PRIMARY = CF_HYDRATION_POLICY_PROGRESSIVE;

/// LD-0105 S7: the Win32 hydration policy read back from Windows after both registrations, so a
/// later step (the streaming path, W2) can tell whether `PROGRESSIVE` really is in effect. `None`
/// until [`connect_root`] has read it; a root whose policy is anything else keeps the whole-file path.
pub(super) static HYDRATION_IN_EFFECT: std::sync::Mutex<Option<(u16, u16)>> = std::sync::Mutex::new(None);

/// LD-0105 S7: is the connected root's hydration policy, as Windows reports it, `PROGRESSIVE` with
/// no modifier? `false` when it was not read.
#[allow(dead_code)] // read by the streaming path (W2)
pub(crate) fn progressive_in_effect() -> bool {
    hydration_policy_is_ours(
        *HYDRATION_IN_EFFECT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// Pure: is `(primary, modifier)` the policy this build registers?
pub(crate) fn hydration_policy_is_ours(read_back: Option<(u16, u16)>) -> bool {
    read_back == Some((HYDRATION_POLICY.0, CF_HYDRATION_POLICY_MODIFIER_NONE.0))
}

/// The Win32 hydration policy `(primary, modifier)` Windows holds for the sync root at `path`
/// (`CfGetSyncRootInfoByPath`, standard info). Reads only; fetches nothing.
pub(crate) fn hydration_policy_of(path: &std::path::Path) -> anyhow::Result<(u16, u16)> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // The standard info is a fixed struct followed by the variable-length sync-root identity; 8 KiB
    // holds both (our identity is empty). u64 elements keep the buffer aligned for the struct.
    let mut buffer = vec![0u64; 1024];
    // SAFETY: `wide` is NUL-terminated and outlives the call; the buffer is writable for its stated length.
    unsafe {
        CfGetSyncRootInfoByPath(
            PCWSTR(wide.as_ptr()),
            CF_SYNC_ROOT_INFO_STANDARD,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            None,
        )
    }
    .map_err(|e| anyhow::anyhow!("CfGetSyncRootInfoByPath (standard): {e}"))?;
    // SAFETY: on success the buffer starts with a CF_SYNC_ROOT_STANDARD_INFO, and it is 8-byte aligned.
    let info = unsafe { &*(buffer.as_ptr() as *const CF_SYNC_ROOT_STANDARD_INFO) };
    Ok((info.HydrationPolicy.Primary.0, info.HydrationPolicy.Modifier.0))
}

/// Stable, deterministic 64-bit FNV-1a hash of the sync-root path bytes.
///
/// Used only to mint the WinRT shell-registration `Id` (see
/// [`shell_sync_root_id`]). FNV-1a is chosen because it is tiny, dependency
/// free, and — critically — STABLE across releases and machines (a fixed
/// algorithm with fixed constants), which is exactly what the shell `Id`
/// requires: the same sync root must hash to the same Id on every launch so
/// re-registration is idempotent and [`unregister_shell_sync_root`] can
/// reconstruct the Id from the persisted sync-root path alone.
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The stable WinRT shell-registration `Id` for a given sync root.
///
/// ## Id scheme — `Beebeeb!<fnv1a64(sync_root)>` (chosen + why)
///
/// `StorageProviderSyncRootManager::Register` keys every shell entry by this
/// string Id; `Unregister` takes the same Id. So the Id MUST be:
///
/// 1. **Stable across releases/launches** — a re-register of the same root must
///    hit the same entry (idempotent), and sign-out must be able to unregister
///    it. We derive it purely from the sync-root path, which is persisted in
///    `desktop.toml` (`DesktopConfig.sync_root`) and is the one piece of
///    identity reachable from BOTH `register_shell_sync_root(sync_root)` and the
///    sign-out path (`clear_session`, which has only the config, not the running
///    engine). A fixed FNV-1a of the path is fully deterministic.
/// 2. **Forward-compatible with multi-account** — each account maps to its own
///    sync-root folder, so distinct accounts naturally hash to distinct Ids
///    without any schema change here. When real multi-account lands, the same
///    function keeps working per-root; if a SID/account component is later wanted
///    for nav-pane grouping it can be appended as a third `!`-segment
///    (`Beebeeb!<sid>!<account>`) without disturbing existing single-account Ids.
///
/// We deliberately do NOT embed the Windows user SID here (the OneDrive
/// `OneDrive!<sid>!Business1` form): pulling the SID needs extra Win32 token +
/// authorization features and an unsafe `GetTokenInformation`/`ConvertSidToStringSidW`
/// dance, and `Register` accepts a plain `provider!stableid` Id for a
/// single-account provider. The path hash gives the same stability with zero
/// extra deps and trivial reconstruction at sign-out.
///
/// The `Beebeeb!` prefix namespaces the provider (mirrors OneDrive/ProtonDrive's
/// `<Provider>!…` convention) and matches the Win32 [`PROVIDER_ID`] name so the
/// two registrations read as the same provider in tooling.
pub(super) fn shell_sync_root_id(sync_root: &std::path::Path) -> String {
    shell_sync_root_id_as(sync_root, PROVIDER_ID)
}

/// [`shell_sync_root_id`] for the provider called `provider_name` (a test root uses its own).
pub(crate) fn shell_sync_root_id_as(sync_root: &std::path::Path, provider_name: &str) -> String {
    // Normalise the path string the same way `register_sync_root` does (lossy
    // UTF-8 of the OS path) so the Id is computed from a canonical byte form and
    // both register + unregister agree.
    let canonical = sync_root.to_string_lossy();
    format!("{}!{:016x}", provider_name, fnv1a64(canonical.as_bytes()))
}

/// LD-0105 S7: what to do about the Explorer (WinRT) registration of a root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellStep {
    /// No registration for this root yet: register (first install, or after sign-out).
    Register,
    /// A registration exists: keep it as it is. The word says which case, for the log.
    Keep(&'static str),
}

/// LD-0105 S7 (review N-P2-1 (a)): `registered` is `None` when the root has no Explorer registration,
/// else its recorded version, `None` inside when it cannot be read. An existing registration is
/// NEVER unregistered here: `Unregister` deletes every cloud-only placeholder of the root (Microsoft
/// Learn, `CfUnregisterSyncRoot`), so neither an unreadable version (before: `""` ≠ the current
/// version, then Unregister on any start) nor an older one may cost the person their placeholders.
/// The policies an existing registration carries are not in effect anyway: the Win32 registration,
/// re-applied with UPDATE on every start, is (measured on the Legion: modifier 0, 2026-11-05).
pub(crate) fn shell_step(registered: Option<Option<&str>>) -> ShellStep {
    match registered {
        None => ShellStep::Register,
        Some(Some(version)) if version == PROVIDER_VERSION => ShellStep::Keep("current"),
        Some(Some(_)) => ShellStep::Keep("other_version_kept"),
        Some(None) => ShellStep::Keep("version_unreadable_kept"),
    }
}
