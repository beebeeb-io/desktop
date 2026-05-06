//! Placeholder creation — drops 0-byte cloud-only stubs into the sync
//! root so files appear in Explorer before any bytes have been
//! downloaded.
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 2 Task 6.
//!
//! ## When this is called
//!
//! `engine_bridge::sync_tick` walks the server's file list each tick.
//! For each remote file the local DB has marked `CloudOnly`, the bridge
//! calls [`create_placeholder`] with the parent directory and the
//! file's plaintext name. Subsequent ticks are no-ops because Windows
//! returns ERROR_ALREADY_EXISTS, which we map to `Ok(())`.
//!
//! ## File identity
//!
//! The `FileIdentity` blob is the file's UUID stored as UTF-8 bytes.
//! On the way back in (when Windows fires the fetch callback), we
//! decode the same bytes back into a UUID string and pass it to
//! `EngineBridge::hydrate_file`.
//!
//! ## What gets stored on disk
//!
//! - A reparse point at `<parent>/<name>` — looks like a normal file in
//!   Explorer, has the right size + modified-at metadata, but contains
//!   no real bytes until the first hydration.
//! - The reparse point carries our `FileIdentity` blob, which Windows
//!   passes back to our fetch callback verbatim.

#![cfg(target_os = "windows")]

use windows::core::PCWSTR;
use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::Storage::FileSystem::FILE_BASIC_INFO;

/// Create a single cloud-only placeholder under `parent_dir`. The file
/// shows up in Explorer immediately with the cloud-icon overlay; bytes
/// are fetched lazily by [`super::callbacks::fetch_data_callback`].
///
/// # Parameters
///
/// - `parent_dir`: directory inside the registered sync root. Must
///   already exist.
/// - `file_id`: the server's UUID for this file. Round-trips through
///   the FileIdentity blob and resurfaces in the fetch callback.
/// - `name`: plaintext filename Explorer should show. The encrypted
///   filename never reaches Windows.
/// - `size_bytes`: plaintext size — what the user expects to see in
///   the size column.
/// - `_modified_at`: kept for API parity; we don't currently mint a
///   `FILETIME` from it (Windows is happy enough with the 0 default,
///   and the engine_bridge has the real value if we need to backfill).
pub fn create_placeholder(
    parent_dir: &std::path::Path,
    file_id: &str,
    name: &str,
    size_bytes: i64,
    _modified_at: i64,
) -> anyhow::Result<()> {
    let parent_wide: Vec<u16> = parent_dir
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // FileIdentity is opaque to Windows — we use raw UTF-8 bytes of
    // the file_id so the fetch callback can decode it directly.
    let identity: Vec<u8> = file_id.as_bytes().to_vec();

    // SAFETY: `entry`, `parent_wide`, `name_wide`, `identity` all live
    // on this stack frame for the duration of the FFI call. The
    // `CfCreatePlaceholders` API copies what it needs before returning
    // and does not retain raw pointers afterwards.
    unsafe {
        let mut entry = CF_PLACEHOLDER_CREATE_INFO {
            RelativeFileName: PCWSTR(name_wide.as_ptr()),
            FsMetadata: CF_FS_METADATA {
                BasicInfo: FILE_BASIC_INFO {
                    // 0x20 = FILE_ATTRIBUTE_NORMAL. Anything other
                    // than 0 is required, otherwise Cloud Files
                    // refuses the placeholder. We don't carry over
                    // hidden/system bits; we have no use for them.
                    FileAttributes: 0x20,
                    ..Default::default()
                },
                FileSize: size_bytes,
            },
            FileIdentity: identity.as_ptr() as *const _,
            FileIdentityLength: identity.len() as u32,
            // MARK_IN_SYNC = "this placeholder reflects the latest
            // server state." Without this Windows treats every new
            // placeholder as locally modified relative to the cloud,
            // which kicks off a spurious upload conflict.
            Flags: CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
            // Result is filled in by Windows on return. Initialised to
            // S_OK so a no-op (already exists) reads sensibly.
            Result: windows::core::HRESULT(0),
            CreateUsn: 0,
        };

        // The 1-element slice + count == 1 is the canonical "create one
        // placeholder" call shape. Batch creation is possible by
        // passing a longer slice, but we pace creation per-file as the
        // engine bridge discovers them, so single-entry is fine.
        let entries = std::slice::from_mut(&mut entry);
        CfCreatePlaceholders(
            PCWSTR(parent_wide.as_ptr()),
            entries,
            entries.len() as u32,
            CF_CREATE_FLAG_NONE,
            None,
        )
        .map_err(|e| anyhow::anyhow!("CfCreatePlaceholders: {e}"))?;
    }

    tracing::debug!(
        file_id = %file_id,
        parent = %parent_dir.display(),
        name = %name,
        "cloud placeholder created"
    );
    Ok(())
}
