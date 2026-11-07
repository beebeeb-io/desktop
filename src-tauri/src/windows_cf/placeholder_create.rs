//! `create_placeholder`: mint one cloud-only placeholder (LD-0113: a file under Backups read-only).
//! Moved here verbatim from `placeholders.rs` to keep that file under the size ratchet.

#![cfg(target_os = "windows")]

use super::*;

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
/// - `is_dir`: when `true`, mint a **directory** placeholder
///   (`FILE_ATTRIBUTE_DIRECTORY`) that is a real, openable folder in
///   Explorer — children land inside it on disk. When `false`, mint a file
///   placeholder (`FILE_ATTRIBUTE_NORMAL`) whose bytes are hydrated on open.
pub fn create_placeholder(
    parent_dir: &std::path::Path,
    file_id: &str,
    name: &str,
    size_bytes: i64,
    modified_at: i64,
    is_dir: bool,
) -> anyhow::Result<bool> {
    create_placeholder_with(parent_dir, file_id, name, size_bytes, modified_at, is_dir, false)
}

/// [`create_placeholder`], a FILE minted with `FILE_ATTRIBUTE_READONLY` when `read_only` (LD-0113
/// Amendment 7: a file at or under the Backups root), in the same call, so it is never writable in
/// between. A folder ignores it.
pub fn create_placeholder_with(
    parent_dir: &std::path::Path,
    file_id: &str,
    name: &str,
    size_bytes: i64,
    modified_at: i64,
    is_dir: bool,
    read_only: bool,
) -> anyhow::Result<bool> {
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

    // Directory vs file placeholder.
    //  - DIRECTORY: a real, openable folder. Its children are the on-disk
    //    placeholders we seed inside it; a directory has no data stream so
    //    `FileSize` is 0.
    //  - FILE (NORMAL): a hydrate-on-open stub carrying the plaintext size.
    let file_attributes = if is_dir {
        FILE_ATTRIBUTE_DIRECTORY.0
    } else if read_only {
        FILE_ATTRIBUTE_READONLY.0
    } else {
        FILE_ATTRIBUTE_NORMAL.0
    };
    let file_size = if is_dir { 0 } else { size_bytes };

    // Placeholder-create flags:
    //  - MARK_IN_SYNC: this placeholder reflects the latest server state.
    //    Without it Windows treats a new placeholder as locally-modified
    //    relative to the cloud and starts a spurious upload conflict.
    //  - DISABLE_ON_DEMAND_POPULATION (directories only): the per-directory
    //    analog of the root's CF_REGISTER_FLAG_DISABLE_ON_DEMAND_POPULATION_ON_ROOT
    //    + CF_POPULATION_POLICY_ALWAYS_FULL. It tells Windows this directory is
    //    ALREADY fully enumerated by its on-disk children, so Explorer must NOT
    //    block on a FETCH_PLACEHOLDERS callback (which this in-process provider
    //    never registers — see mod.rs). This is what keeps nested folders from
    //    reintroducing the enumeration hang while staying in the eager model.
    //  - PIN STATE (inherit): we deliberately set NO pin flag here. A freshly
    //    created placeholder has pin state `CF_PIN_STATE_INHERIT` by default,
    //    which resolves to the EFFECTIVE pin state of its parent directory. So a
    //    child minted under a directory the user pinned ("Always keep on this
    //    device", i.e. `set_pin_state(parent, pinned=true, recurse=true)`)
    //    automatically inherits `pinned` and is kept resident — no per-child
    //    CfSetPinState needed (the recursive parent pin also stamps existing
    //    children; inherit covers children created LATER). Adding an explicit
    //    `CF_PIN_STATE_PINNED` here would be wrong: it would force every new
    //    placeholder resident regardless of the parent, defeating cloud-only.
    let mut create_flags = CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC;
    if is_dir {
        create_flags |= CF_PLACEHOLDER_CREATE_FLAG_DISABLE_ON_DEMAND_POPULATION;
    }

    // SAFETY: `entry`, `parent_wide`, `name_wide`, `identity` all live
    // on this stack frame for the duration of the FFI call. The
    // `CfCreatePlaceholders` API copies what it needs before returning
    // and does not retain raw pointers afterwards.
    unsafe {
        let mut entry = CF_PLACEHOLDER_CREATE_INFO {
            RelativeFileName: PCWSTR(name_wide.as_ptr()),
            FsMetadata: CF_FS_METADATA {
                BasicInfo: FILE_BASIC_INFO {
                    // FILE_ATTRIBUTE_DIRECTORY (0x10) for folders, else
                    // FILE_ATTRIBUTE_NORMAL (0x80). A non-zero attribute is
                    // required or Cloud Files refuses the placeholder. (The
                    // previous file path hardcoded 0x20 / ARCHIVE, which also
                    // worked; NORMAL is the semantically-correct file attr.) We
                    // don't carry over hidden/system bits; we have no use for them.
                    FileAttributes: file_attributes,
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
                FileSize: file_size,
            },
            FileIdentity: identity.as_ptr() as *const _,
            FileIdentityLength: identity.len() as u32,
            Flags: create_flags,
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
        // A placeholder that already exists is the steady state on every
        // tick after the first — `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`
        // (0x800700B7). Treat it as success: the desired end state holds. It
        // is `Ok(false)`, not created: task 1640 marks a placeholder "nothing
        // fetched on disk" only when it really was created.
        const ALREADY_EXISTS: windows::core::HRESULT = windows::core::HRESULT(0x800700B7u32 as i32);
        let entries = std::slice::from_mut(&mut entry);
        if let Err(e) = CfCreatePlaceholders(PCWSTR(parent_wide.as_ptr()), entries, CF_CREATE_FLAG_NONE, None) {
            if e.code() == ALREADY_EXISTS {
                return Ok(false);
            }
            return Err(anyhow::anyhow!("CfCreatePlaceholders: {e}"));
        }
        // Task 1640 (self-review #3): the entry's own result decides, not only the call's.
        if entries[0].Result == ALREADY_EXISTS {
            return Ok(false);
        }
        if entries[0].Result.is_err() {
            return Err(anyhow::anyhow!("CfCreatePlaceholders entry: {:?}", entries[0].Result));
        }
    }

    // Zero-knowledge: log the file_id + kind only. The decrypted plaintext
    // filename (and the parent path, which can carry decrypted folder names)
    // must never reach the logs.
    tracing::debug!(file_id = %file_id, is_dir, "cloud placeholder created");
    Ok(true)
}
