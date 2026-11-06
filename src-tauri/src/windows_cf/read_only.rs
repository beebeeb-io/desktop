//! LD-0113 Amendment 7 (Guus, 2026-11-06 19:30, "Makes sense, do it"): files under Backups are
//! read-only in Explorer. Every file placeholder at or under the Backups root carries
//! `FILE_ATTRIBUTE_READONLY`, so an editor opens it read-only (Word shows its Read-only bar and offers
//! Save As) and the file system itself refuses an open that writes. Folders are left alone: Explorer
//! ignores the attribute on a folder.
//!
//! The engine's own updates of such a file (a new version's emptying and resize, a dehydrate, a
//! hydrate, a pin, its own remove) need the write access the attribute refuses: [`Lifted`] clears it
//! for one operation and sets it again after. The attribute is changed through a handle with attribute
//! access only (no data access, so a cloud-only placeholder is never fetched), and a placeholder that
//! was in sync is in sync again afterwards.
//!
//! Amendment 8a (Guus, 20:04): the Backups root carries core's `MSG_BADGE` as a Cloud Files custom
//! state, which Explorer shows in its Status column ([`set_read_only_badge`]).
//!
//! This file uses `std`, `tracing` and the `windows` crate only, so it type-checks on its own.

#![cfg(target_os = "windows")]

use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::CloudFilters::{
    CF_IN_SYNC_STATE_IN_SYNC, CF_PLACEHOLDER_INFO_STANDARD, CF_PLACEHOLDER_STANDARD_INFO, CF_SET_IN_SYNC_FLAG_NONE,
    CfGetPlaceholderInfo, CfSetInSyncState,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY, FILE_BASIC_INFO, FILE_DISPOSITION_FLAG_DELETE,
    FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_INFO_EX_FLAGS,
    FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FileBasicInfo,
    FileDispositionInfo, FileDispositionInfoEx, GetFileInformationByHandleEx, SYNCHRONIZE, SetFileInformationByHandle, WRITE_DAC,
};

/// The id of the "Read-only" custom state among the Explorer registration's item property definitions
/// (the Status column's own definition is id 0).
pub(crate) const READ_ONLY_STATE_ID: i32 = 1;

/// The badge's icon: the padlock in `shell32.dll` (resource 48), present on every Windows version the
/// app runs on.
pub(crate) const READ_ONLY_STATE_ICON: &str = r"%SystemRoot%\System32\shell32.dll,-48";

/// The attributes `SetFileInformationByHandle(FileBasicInfo)` may set: `FILE_ATTRIBUTE_VALID_SET_FLAGS`
/// (read-only, hidden, system, archive, normal, temporary, offline, not-content-indexed), the
/// integrity and no-scrub bits, and the pin bits Cloud Files reads (`PINNED` 0x80000, `UNPINNED`
/// 0x100000).
const SETTABLE: u32 = 0x31A7 | 0x8000 | 0x2_0000 | 0x8_0000 | 0x10_0000;

/// A handle on `path` that reads and writes attributes and may set the in-sync state (`WRITE_DAC`, as
/// `CfSetInSyncState` requires), with no data access: an open of a cloud-only placeholder this way
/// fetches nothing. Every other handle may stay open meanwhile.
fn open_for_attributes(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0 | FILE_WRITE_ATTRIBUTES.0 | WRITE_DAC.0 | SYNCHRONIZE.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
}

/// The in-sync state of the placeholder behind `handle`; `None` for a plain file or an unreadable one.
fn in_sync(handle: HANDLE) -> Option<bool> {
    let mut buffer = vec![0u64; 1024];
    // SAFETY: `handle` is open for the call; the buffer is 8-byte aligned and its length is passed, so
    // the call writes only inside it (the identity is at most 4 KiB).
    unsafe {
        CfGetPlaceholderInfo(handle, CF_PLACEHOLDER_INFO_STANDARD, buffer.as_mut_ptr().cast(), (buffer.len() * 8) as u32, None)
    }
    .ok()?;
    // SAFETY: a successful CF_PLACEHOLDER_INFO_STANDARD call wrote that struct at the start.
    let info = unsafe { &*buffer.as_ptr().cast::<CF_PLACEHOLDER_STANDARD_INFO>() };
    Some(info.InSyncState == CF_IN_SYNC_STATE_IN_SYNC)
}

/// The attribute word of the file behind `handle`.
fn attributes(handle: HANDLE) -> windows::core::Result<u32> {
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: `basic` is a FILE_BASIC_INFO and its size is passed; `handle` is open for the call.
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    }?;
    Ok(basic.FileAttributes)
}

/// Whether the file at `path` carries `FILE_ATTRIBUTE_READONLY`; `None` when it cannot be read.
pub(crate) fn is_read_only(path: &Path) -> Option<bool> {
    let file = open_for_attributes(path).ok()?;
    attributes(HANDLE(file.as_raw_handle())).ok().map(|word| word & FILE_ATTRIBUTE_READONLY.0 != 0)
}

/// Set (`on`) or clear `FILE_ATTRIBUTE_READONLY` on the file at `path`, keeping its other attributes
/// (the pin state among them), its times, and its in-sync state: a placeholder that was in sync and
/// is not after the change is marked in sync again (only the connected provider may; elsewhere that
/// step fails and is reported). A folder is left alone. Returns whether the attribute changed.
pub(crate) fn set_read_only(path: &Path, on: bool) -> anyhow::Result<bool> {
    let file = open_for_attributes(path)?;
    let handle = HANDLE(file.as_raw_handle());
    let word = attributes(handle)?;
    if word & FILE_ATTRIBUTE_DIRECTORY.0 != 0 || (word & FILE_ATTRIBUTE_READONLY.0 != 0) == on {
        return Ok(false);
    }
    let was_in_sync = in_sync(handle) == Some(true);
    // Only the bits a caller may set go back (the others, a placeholder's reparse point and recall
    // bits among them, are the file system's and stay as they are); the pin bits are among them, so
    // the pin state is kept.
    let settable = word & SETTABLE;
    let wanted = if on { settable | FILE_ATTRIBUTE_READONLY.0 } else { settable & !FILE_ATTRIBUTE_READONLY.0 };
    // NORMAL is valid only alone.
    let wanted = match wanted & !FILE_ATTRIBUTE_NORMAL.0 {
        0 => FILE_ATTRIBUTE_NORMAL.0,
        others => others,
    };
    // Zero times leave the times as they are.
    let basic = FILE_BASIC_INFO { FileAttributes: wanted, ..Default::default() };
    // SAFETY: `basic` is a FILE_BASIC_INFO and its size is passed; `handle` has FILE_WRITE_ATTRIBUTES.
    unsafe {
        SetFileInformationByHandle(
            handle,
            FileBasicInfo,
            (&basic as *const FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    }?;
    if was_in_sync && in_sync(handle) == Some(false) {
        // SAFETY: `handle` is open and carries WRITE_DAC.
        if let Err(error) = unsafe { CfSetInSyncState(handle, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) } {
            // The attribute did change, so the caller still learns it did ([`Lifted`] sets it back).
            // Zero-knowledge: never the path.
            tracing::warn!(%error, "read-only: the attribute changed, but the in-sync mark could not be put back");
        }
    }
    Ok(true)
}

/// The read-only attribute of one file, cleared for one operation of the engine's own and set again
/// when this is dropped, if it was set before and the file is still there. A file without it (or a
/// folder) is left as it is, and so is one whose attribute cannot be read: the operation then runs and
/// fails or succeeds as it would have.
#[must_use = "the attribute is set again when the guard is dropped"]
pub(crate) struct Lifted {
    path: Option<PathBuf>,
}

impl Lifted {
    pub(crate) fn new(path: &Path) -> Self {
        match set_read_only(path, false) {
            Ok(true) => Self { path: Some(path.to_path_buf()) },
            Ok(false) => Self { path: None },
            Err(error) => {
                // Zero-knowledge: never the path.
                tracing::debug!(%error, "read-only: the attribute could not be cleared for the engine's own change");
                Self { path: None }
            }
        }
    }
}

impl Drop for Lifted {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if std::fs::symlink_metadata(&path).is_err() {
            // The operation removed it (the engine's own delete).
            return;
        }
        if let Err(error) = set_read_only(&path, true) {
            tracing::warn!(%error, "read-only: the attribute could not be set again after the engine's own change");
        }
    }
}

/// Mark the file or folder behind `handle` (opened with `DELETE` access) for deletion when it closes,
/// a read-only file included (`FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE`, Windows 10 1809 and
/// later). Where that class is not known, the plain disposition, which a read-only file refuses.
pub(crate) fn mark_for_deletion(handle: HANDLE) -> windows::core::Result<()> {
    let ex = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(FILE_DISPOSITION_FLAG_DELETE.0 | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0),
    };
    // SAFETY: `ex` is a FILE_DISPOSITION_INFO_EX and its size is passed; `handle` is open for the call.
    let with_ex = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfoEx,
            (&ex as *const FILE_DISPOSITION_INFO_EX).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    };
    match with_ex {
        Ok(()) => Ok(()),
        // ERROR_INVALID_PARAMETER (87) or ERROR_NOT_SUPPORTED (50): an older Windows.
        Err(e) if matches!(e.code().0 as u32, 0x8007_0057 | 0x8007_0032) => {
            let plain = FILE_DISPOSITION_INFO { DeleteFile: true.into() };
            // SAFETY: as above, with FILE_DISPOSITION_INFO.
            unsafe {
                SetFileInformationByHandle(
                    handle,
                    FileDispositionInfo,
                    (&plain as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            }
        }
        Err(e) => Err(e),
    }
}

/// The "Read-only" custom state's definition, for the Explorer registration (`label`: core's
/// `MSG_BADGE`). Only a new registration takes it: an existing one is kept as it is (LD-0105 S7).
pub(crate) fn read_only_state_definition(
    label: &str,
) -> windows::core::Result<windows::Storage::Provider::StorageProviderItemPropertyDefinition> {
    let definition = windows::Storage::Provider::StorageProviderItemPropertyDefinition::new()?;
    definition.SetId(READ_ONLY_STATE_ID)?;
    definition.SetDisplayNameResource(&windows::core::HSTRING::from(label))?;
    Ok(definition)
}

/// Show (`on`) or take away the "Read-only" custom state (`label`: core's `MSG_BADGE`) on the folder
/// at `path`, a placeholder of a registered sync root. It blocks on WinRT: call it off a callback.
pub(crate) fn set_read_only_badge(path: &Path, label: &str, on: bool) -> windows::core::Result<()> {
    use windows::Foundation::Collections::IIterable;
    use windows::Storage::Provider::{StorageProviderItemProperties, StorageProviderItemProperty};
    use windows::Storage::{IStorageItem, StorageFolder};
    use windows::core::{HSTRING, Interface};

    let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(&*path.to_string_lossy()))?.get()?;
    let item: IStorageItem = folder.cast()?;
    let mut values = Vec::new();
    if on {
        let property = StorageProviderItemProperty::new()?;
        property.SetId(READ_ONLY_STATE_ID)?;
        property.SetValue(&HSTRING::from(label))?;
        property.SetIconResource(&HSTRING::from(READ_ONLY_STATE_ICON))?;
        values.push(Some(property));
    }
    // An empty list takes the custom states away.
    let values = IIterable::<StorageProviderItemProperty>::try_from(values)?;
    StorageProviderItemProperties::SetAsync(&item, &values)?.get()
}
