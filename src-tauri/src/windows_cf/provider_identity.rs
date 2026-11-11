//! Who the Cloud Files provider is: its name, version and GUID, split out of `windows_cf/mod.rs` (file-size
//! ratchet). The name and GUID come from `app_identity`, so a test build registers as another provider.

use windows::core::GUID;

use crate::app_identity::AppIdentity;

/// Identifier shown to Windows for our sync provider ("Files synced by Beebeeb" in Sync settings). Keep
/// stable: changing this between releases would orphan existing placeholders.
pub const PROVIDER_ID: &str = AppIdentity::CURRENT.cloud_files_provider_name();

/// Provider version reported alongside `PROVIDER_ID`. It is written into the
/// Explorer SHELL registration (`StorageProviderSyncRootInfo::SetVersion`) when a root
/// is first registered.
///
/// LD-0105 S7 (0.8.22): bumping it NO LONGER re-registers anything. Up to 0.8.21 a
/// version that differed from (or could not be read back as) this constant made
/// `register_shell_sync_root` run the WinRT `Unregister`, and an unregister deletes
/// every cloud-only placeholder of the root. An existing registration is now kept as
/// it is ([`shell_step`]); a policy change reaches existing installs through the Win32
/// registration, which is re-applied with `CF_REGISTER_FLAG_UPDATE` on every start.
/// A change that really needs a new Explorer registration needs its own migration.
///
/// `1.0.0` → `1.1.0`: dropped `AutoDehydrationAllowed` from the hydration modifier.
pub const PROVIDER_VERSION: &str = "1.1.0";

/// Stable provider GUID for the sync root, from `app_identity` (the real app's is UUIDv4
/// `b33b33b0-5217-4c0a-9e3f-1f2a7c4d8e90`, minted once; a test build has its own). Hardcoded so the
/// sync-root identity is stable across releases and machines: `GUID::zeroed()` let Windows pick a
/// per-machine default, which would orphan placeholders if the default ever changed.
pub const BEEBEEB_PROVIDER_GUID: GUID = GUID::from_u128(AppIdentity::CURRENT.cloud_files_provider_guid());
