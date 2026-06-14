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

use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::Storage::FileSystem::FILE_BASIC_INFO;
use windows::core::PCWSTR;

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
/// - `modified_at`: unix timestamp (seconds; millis tolerated — see
///   [`unix_to_filetime`]) of the file's last modification. Converted to
///   a Windows `FILETIME` tick count and written to the placeholder's
///   `LastWriteTime` / `ChangeTime` / `CreationTime` so Explorer shows a
///   real modified date instead of a blank/epoch one.
pub fn create_placeholder(
    parent_dir: &std::path::Path,
    file_id: &str,
    name: &str,
    size_bytes: i64,
    modified_at: i64,
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

    // Convert the file's unix modified timestamp into the Windows
    // FILETIME tick count (100-ns intervals since 1601-01-01) that
    // FILE_BASIC_INFO stores as an i64.
    let modified_ticks = unix_to_filetime(modified_at);

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
                    // Real timestamps so Explorer's Date Modified column
                    // is populated. We only have a single modified time
                    // from the server, so we mirror it across
                    // creation/write/change. LastAccessTime is left at 0
                    // (Windows treats 0 as "don't change / unknown").
                    CreationTime: modified_ticks,
                    LastWriteTime: modified_ticks,
                    ChangeTime: modified_ticks,
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
        // windows 0.58 takes the placeholder array as a `&mut [..]` slice
        // (the length is derived from it) plus an optional
        // `entriesprocessed` out-pointer — four args total, not five.
        let entries = std::slice::from_mut(&mut entry);
        if let Err(e) = CfCreatePlaceholders(PCWSTR(parent_wide.as_ptr()), entries, CF_CREATE_FLAG_NONE, None) {
            // A placeholder that already exists is the steady state on every
            // tick after the first — `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`
            // (0x800700B7). Treat it as success: the desired end state holds.
            const ALREADY_EXISTS: windows::core::HRESULT = windows::core::HRESULT(0x800700B7u32 as i32);
            if e.code() == ALREADY_EXISTS {
                return Ok(());
            }
            return Err(anyhow::anyhow!("CfCreatePlaceholders: {e}"));
        }
    }

    tracing::debug!(
        file_id = %file_id,
        parent = %parent_dir.display(),
        name = %name,
        "cloud placeholder created"
    );
    Ok(())
}

/// Number of seconds between the Windows FILETIME epoch (1601-01-01) and
/// the Unix epoch (1970-01-01).
const FILETIME_UNIX_EPOCH_OFFSET_SECS: i64 = 11_644_473_600;

/// Convert a Unix timestamp into a Windows FILETIME tick count: the
/// number of 100-nanosecond intervals since 1601-01-01, as stored in
/// `FILE_BASIC_INFO`'s i64 timestamp fields.
///
/// The desktop DB stores `modified_at` in **seconds** (see
/// `engine_bridge::now_secs`). As a defensive measure we also accept
/// millisecond values: anything implausibly large to be seconds (beyond
/// ~year 5000) is treated as milliseconds. A non-positive or zero input
/// yields 0, which Windows renders as "no date".
fn unix_to_filetime(modified_at: i64) -> i64 {
    if modified_at <= 0 {
        return 0;
    }

    // Year ~5000 in unix seconds. A value larger than this is almost
    // certainly milliseconds, not seconds.
    const SECONDS_SANITY_CEILING: i64 = 95_617_584_000;
    let unix_secs = if modified_at > SECONDS_SANITY_CEILING {
        modified_at / 1000
    } else {
        modified_at
    };

    // Shift to the 1601 epoch, then scale seconds → 100-ns ticks.
    // saturating_* keeps a pathological value from overflowing into a
    // negative (which Explorer would render as a bogus 1601 date).
    unix_secs
        .saturating_add(FILETIME_UNIX_EPOCH_OFFSET_SECS)
        .saturating_mul(10_000_000)
}
