//! Tasks 2270 / 2280 (desktop 0.8.20): the upload-finalization steps, the stamp pass and the promote pass on a real
//! Windows sync root, registered in a temporary directory and connected with a FETCH_DATA callback that serves known
//! bytes and counts every fetch by the file's identity. Each test asserts the spec's invariants on the file it
//! touches: I1 (attributes, last-write time and DACL as before, read-only included) and I2 (no fetch). The tests in
//! this module take turns (one sync root at a time), because every fetch reaches the same callback.
use super::*;
use crate::upload_finalization_plan::{Judged, judge};
use crate::windows_cf::placeholders::{self, Local, Restamped};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use windows::Win32::Foundation::HANDLE;
use windows::core::PCWSTR;

/// The identity of every file the provider was asked to fetch, in order.
static FETCHED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The bytes the provider serves, by identity. A fetch of any other identity fails.
static SERVED: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
static ONE_ROOT_AT_A_TIME: Mutex<()> = Mutex::new(());

unsafe extern "system" fn serving_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: both are valid for the duration of the callback (the Cloud Files contract).
    let info = unsafe { &*info };
    let fetch = unsafe { (*params).Anonymous.FetchData };
    // SAFETY: the identity bytes are valid for the duration of the callback.
    let identity = unsafe { std::slice::from_raw_parts(info.FileIdentity as *const u8, info.FileIdentityLength as usize) };
    let identity = String::from_utf8_lossy(identity).into_owned();
    FETCHED.lock().unwrap_or_else(PoisonError::into_inner).push(identity.clone());
    let bytes = SERVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(id, _)| *id == identity)
        .map(|(_, bytes)| bytes.clone());
    // SAFETY: the keys come from this live callback.
    unsafe {
        match bytes {
            Some(bytes) => {
                let _ = transfer_range(
                    info.ConnectionKey,
                    info.TransferKey,
                    info.RequestKey,
                    &bytes,
                    || false,
                    fetch.RequiredFileOffset,
                    fetch.RequiredLength,
                );
            }
            None => fail_transfer(info.ConnectionKey, info.TransferKey, info.RequestKey, fetch.RequiredFileOffset, fetch.RequiredLength),
        }
    }
}

/// How many fetches asked for a file whose identity starts with `prefix` (each test names its files' identities
/// with its own prefix).
fn fetches(prefix: &str) -> usize {
    FETCHED.lock().unwrap_or_else(PoisonError::into_inner).iter().filter(|id| id.starts_with(prefix)).count()
}

fn serve(identity: &str, bytes: &[u8]) {
    SERVED.lock().unwrap_or_else(PoisonError::into_inner).push((identity.into(), bytes.to_vec()));
}

/// Time for a fetch the platform started to reach the callback.
fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(500));
}

/// A temporary sync root, registered and connected; disconnected, unregistered and removed on drop.
struct Root {
    path: PathBuf,
    key: CF_CONNECTION_KEY,
    temp: tempfile::TempDir,
    _turn: MutexGuard<'static, ()>,
}

impl Root {
    fn new() -> Self {
        let turn = ONE_ROOT_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("root");
        std::fs::create_dir(&path).unwrap();
        crate::windows_cf::register_sync_root(&path).unwrap();
        let table = [
            CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_FETCH_DATA, Callback: Some(serving_fetch) },
            CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_NONE, Callback: None },
        ];
        let wide = wide(&path);
        // SAFETY: the path and the table outlive the call; the callback is a static function.
        let key = unsafe { CfConnectSyncRoot(PCWSTR(wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_NONE) }.unwrap();
        Self { path, key, temp, _turn: turn }
    }

    fn db(&self) -> crate::state_db::StateDb {
        crate::state_db::StateDb::open(self.temp.path().join("state.db")).unwrap()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        // SAFETY: the key of the connection made in `new`, disconnected once.
        let _ = unsafe { CfDisconnectSyncRoot(self.key) };
        if let Ok(entries) = std::fs::read_dir(&self.path) {
            for entry in entries.flatten() {
                set_attributes(&entry.path(), 0x80);
            }
        }
        let _ = crate::windows_cf::unregister_sync_root(&self.path);
    }
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn set_attributes(path: &Path, attributes: u32) {
    use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, SetFileAttributesW};
    let wide = wide(path);
    // SAFETY: `wide` is NUL-terminated and alive for the call.
    let _ = unsafe { SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(attributes)) };
}

/// Read-only and archive, as the 82 copies were.
fn make_read_only(path: &Path) {
    set_attributes(path, 0x21);
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// What spec I1 keeps: the attributes a person or a program sets (the placeholder-state bits the platform manages are
/// left out: a plain file that becomes a placeholder gains REPARSE_POINT), the last-write time and the DACL.
#[derive(Debug, PartialEq, Eq)]
struct Kept {
    attributes: u32,
    modified: std::time::SystemTime,
    /// The SHA-256 of the DACL's security descriptor: compared, and printed without the account SIDs it holds.
    dacl: String,
}

impl Kept {
    fn of(path: &Path) -> Self {
        use windows::Win32::Storage::FileSystem::GetFileAttributesW;
        // REPARSE_POINT, SPARSE, OFFLINE, RECALL_ON_OPEN, RECALL_ON_DATA_ACCESS.
        const PLACEHOLDER_STATE: u32 = 0x400 | 0x200 | 0x1000 | 0x0004_0000 | 0x0040_0000;
        let wide = wide(path);
        // SAFETY: `wide` is NUL-terminated and alive for the call.
        let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
        assert_ne!(attributes, u32::MAX, "attributes of {path:?}");
        Self {
            attributes: attributes & !PLACEHOLDER_STATE,
            modified: std::fs::symlink_metadata(path).unwrap().modified().unwrap(),
            dacl: sha256(&dacl(path)),
        }
    }
}

/// The file's DACL, as its self-relative security descriptor (the lead's note: nothing may set one through the
/// finalization handles, which carry WRITE_DAC).
fn dacl(path: &Path) -> Vec<u8> {
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, GetFileSecurityW, PSECURITY_DESCRIPTOR};
    let wide = wide(path);
    let mut needed = 0u32;
    // SAFETY: a size query: no buffer is passed, and the length needed is written to `needed`.
    let _ = unsafe { GetFileSecurityW(PCWSTR(wide.as_ptr()), DACL_SECURITY_INFORMATION.0, PSECURITY_DESCRIPTOR::default(), 0, &mut needed) };
    assert!(needed > 0, "the DACL size of {path:?}");
    let mut buffer = vec![0u8; needed as usize];
    // SAFETY: the buffer is `needed` bytes long and outlives the call.
    let read = unsafe {
        GetFileSecurityW(
            PCWSTR(wide.as_ptr()),
            DACL_SECURITY_INFORMATION.0,
            PSECURITY_DESCRIPTOR(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    };
    assert!(read.as_bool(), "the DACL of {path:?}");
    buffer
}

fn placeholder(on_disk: crate::unknown_check::OnDisk, in_sync: bool, identity: &str) -> crate::unknown_check::PlatformView {
    crate::unknown_check::PlatformView::Placeholder { on_disk, in_sync, identity: Some(identity.into()) }
}

/// A cloud-only placeholder carrying `identity`: written, converted in sync, then dehydrated (no fetch).
fn cloud_only(path: &Path, bytes: &[u8], identity: &str) {
    std::fs::write(path, bytes).unwrap();
    placeholders::convert_to_in_sync_placeholder(path, identity).unwrap();
    placeholders::dehydrate_placeholder(path).unwrap();
    assert_eq!(placeholders::platform_view(path), placeholder(crate::unknown_check::OnDisk::Nothing, true, identity));
}

fn never_judged(_: &str) -> Judged {
    panic!("the bytes of this file must not be read")
}

/// Task 2270, spec F7a: the 82. A read-only plain copy of the uploaded bytes finalizes in sync under the server
/// identity. 0.8.19 opened it for writing and failed with access denied (os error 5) on every tick (probe A2).
#[test]
fn regression_2270_a_read_only_plain_file_finalizes_in_sync_with_its_attributes_kept() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let path = root.path.join("a.txt");
    let bytes = b"the uploaded document".repeat(100);
    std::fs::write(&path, &bytes).unwrap();
    make_read_only(&path);
    let kept = Kept::of(&path);
    let mut judged_hash = None;
    let local = placeholders::finalize_wholly_local(&path, "t01-local", "t01-server", &mut |hash| {
        judged_hash = Some(hash.to_string());
        judge(hash, Some(&sha256(&bytes)), None)
    })
    .unwrap();
    assert_eq!(local, Local::Finalized(Judged::MarkInSync));
    assert_eq!(judged_hash, Some(sha256(&bytes)), "the bytes hashed are the file's");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t01-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
    assert_eq!(kept.attributes & 1, 1, "still read-only");
    settle();
    assert_eq!(fetches("t01-"), 0, "I2");
}

/// Spec F7a on a placeholder: a read-only unsynced placeholder with this upload's throwaway local identity finalizes
/// in sync under the server identity.
#[test]
fn a_read_only_unsynced_placeholder_finalizes_under_the_server_identity() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let path = root.path.join("b.txt");
    let bytes = b"uploaded while it was a placeholder".repeat(100);
    std::fs::write(&path, &bytes).unwrap();
    placeholders::convert_to_unsynced_placeholder(&path, "t02-local").unwrap();
    make_read_only(&path);
    let kept = Kept::of(&path);
    let local =
        placeholders::finalize_wholly_local(&path, "t02-local", "t02-server", &mut |hash| judge(hash, Some(&sha256(&bytes)), None))
            .unwrap();
    assert_eq!(local, Local::Finalized(Judged::MarkInSync));
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t02-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
    settle();
    assert_eq!(fetches("t02-"), 0, "I2");
}

/// Spec F5: a read-only cloud-only placeholder of this upload gets the server identity, in sync, through a handle
/// with no data access: nothing is fetched (probe K0: an open with read-data access fetches it). Afterwards it reads
/// as the bytes the server identity serves.
#[test]
fn a_read_only_cloud_only_placeholder_is_re_stamped_without_a_fetch() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let path = root.path.join("c.bin");
    let bytes = b"stored on the server".repeat(4000);
    cloud_only(&path, &bytes, "t03-local");
    serve("t03-server", &bytes);
    make_read_only(&path);
    let kept = Kept::of(&path);
    assert_eq!(placeholders::restamp_cloud_only(&path, "t03-local", "t03-server").unwrap(), Restamped::Done);
    settle();
    assert_eq!(fetches("t03-"), 0, "I2: nothing is fetched to re-stamp it");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Nothing, true, "t03-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "it reads as the server's bytes");
    assert!(fetches("t03-server") >= 1, "and that read is what fetched it");
}

/// Spec F7b (0.8.19's P3-3): the bytes are not the staged payload but a later edit that has since landed (the agreed
/// content). The row finalizes in sync instead of reporting "Local bytes changed" every tick.
#[test]
fn a_later_edit_that_landed_finalizes_in_sync() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let path = root.path.join("d.txt");
    std::fs::write(&path, b"the edit that landed").unwrap();
    make_read_only(&path);
    let kept = Kept::of(&path);
    let (payload, agreed) = (sha256(b"the uploaded original"), sha256(b"the edit that landed"));
    let local =
        placeholders::finalize_wholly_local(&path, "t04-local", "t04-server", &mut |hash| judge(hash, Some(&payload), Some(&agreed)))
            .unwrap();
    assert_eq!(local, Local::Finalized(Judged::MarkInSync));
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t04-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
    settle();
    assert_eq!(fetches("t04-"), 0, "I2");
}

/// Spec F7c, I3: bytes that are neither the payload nor the agreed content get the server identity only, never the
/// in-sync mark, whether the file is plain or a placeholder that already carries the server identity.
#[test]
fn bytes_that_are_neither_get_the_identity_only() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    for (name, starts_as_placeholder) in [("plain.txt", false), ("placeholder.txt", true)] {
        let path = root.path.join(name);
        std::fs::write(&path, b"an edit nobody has sent yet").unwrap();
        if starts_as_placeholder {
            placeholders::convert_to_unsynced_placeholder(&path, "t05-server").unwrap();
            assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, false, "t05-server"));
        }
        make_read_only(&path);
        let kept = Kept::of(&path);
        let local = placeholders::finalize_wholly_local(&path, "t05-local", "t05-server", &mut |hash| {
            judge(hash, Some(&sha256(b"the uploaded original")), None)
        })
        .unwrap();
        assert_eq!(local, Local::Finalized(Judged::IdentityOnly), "{name}");
        assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, false, "t05-server"), "{name}");
        assert_eq!(Kept::of(&path), kept, "I1: {name}");
    }
    settle();
    assert_eq!(fetches("t05-"), 0, "I2");
}

/// Spec F4, I5: a placeholder whose identity names another file is never changed, wholly on disk (F7's step) or cloud
/// only (F5's step): both read the identity again through their handle.
#[test]
fn a_placeholder_of_another_file_is_not_touched() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let local_path = root.path.join("other-local.txt");
    std::fs::write(&local_path, b"another file's bytes").unwrap();
    placeholders::convert_to_unsynced_placeholder(&local_path, "t06-other").unwrap();
    make_read_only(&local_path);
    let kept = Kept::of(&local_path);
    let local = placeholders::finalize_wholly_local(&local_path, "t06-local", "t06-server", &mut never_judged).unwrap();
    assert_eq!(local, Local::OtherFile);
    assert_eq!(placeholders::platform_view(&local_path), placeholder(OnDisk::Wholly, false, "t06-other"));
    assert_eq!(Kept::of(&local_path), kept, "I1");

    let cloud_path = root.path.join("other-cloud.bin");
    cloud_only(&cloud_path, &b"another file on the server".repeat(1000), "t06-other-cloud");
    make_read_only(&cloud_path);
    let kept = Kept::of(&cloud_path);
    assert_eq!(placeholders::restamp_cloud_only(&cloud_path, "t06-local", "t06-server").unwrap(), Restamped::OtherFile);
    assert_eq!(placeholders::platform_view(&cloud_path), placeholder(OnDisk::Nothing, true, "t06-other-cloud"));
    assert_eq!(Kept::of(&cloud_path), kept, "I1");
    settle();
    assert_eq!(fetches("t06-"), 0, "I2");
}

/// Spec F6, I2 (review focus): a placeholder that is only partly on disk when finalization reaches it is never opened
/// for its data, which would fetch the rest. Its second half is dehydrated through a handle with no data access.
#[test]
fn a_placeholder_partly_on_disk_when_opened_is_not_read() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let path = root.path.join("partly.bin");
    let bytes = vec![7u8; 256 * 1024];
    std::fs::write(&path, &bytes).unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, "t07-server").unwrap();
    serve("t07-server", &bytes);
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, SYNCHRONIZE, WRITE_DAC};
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0 | WRITE_DAC.0 | SYNCHRONIZE.0)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&path)
            .unwrap();
        // SAFETY: the handle belongs to `file`, open for the call.
        unsafe { CfDehydratePlaceholder(HANDLE(file.as_raw_handle()), 128 * 1024, 128 * 1024, CF_DEHYDRATE_FLAG_NONE, None) }.unwrap();
    }
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Partly, true, "t07-server"));
    make_read_only(&path);
    let kept = Kept::of(&path);
    settle();
    let before = fetches("t07-");
    let local = placeholders::finalize_wholly_local(&path, "t07-local", "t07-server", &mut never_judged).unwrap();
    assert_eq!(local, Local::NotWhollyLocal);
    settle();
    assert_eq!(fetches("t07-") - before, 0, "I2: the rest of it is not fetched");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Partly, true, "t07-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
}

/// Spec F8 (review focus): a file someone has open is a transient error (sharing violation), never a review; once it
/// is closed the next attempt finalizes it.
#[test]
fn an_open_file_is_transient() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let path = root.path.join("open.txt");
    let bytes = b"open in an editor".repeat(10);
    std::fs::write(&path, &bytes).unwrap();
    make_read_only(&path);
    let kept = Kept::of(&path);
    let reader = std::fs::File::open(&path).unwrap();
    let error = placeholders::finalize_wholly_local(&path, "t08-local", "t08-server", &mut never_judged).unwrap_err();
    assert!(placeholders::is_transient(&error), "a sharing violation is transient: {error:?}");
    assert!(!placeholders::is_access_denied(&error), "{error:?}");
    assert_eq!(placeholders::platform_view(&path), crate::unknown_check::PlatformView::PlainFile, "nothing changed");
    drop(reader);
    let local =
        placeholders::finalize_wholly_local(&path, "t08-local", "t08-server", &mut |hash| judge(hash, Some(&sha256(&bytes)), None))
            .unwrap();
    assert_eq!(local, Local::Finalized(Judged::MarkInSync));
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t08-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
}

fn track(db: &crate::state_db::StateDb, file_id: &str, local_hash: Option<String>) {
    use crate::state_db::{FileContractState, FileEntry, FileStatus, ItemKind, Namespace, PinState};
    db.upsert_file(&FileEntry {
        file_id: file_id.into(),
        path: format!("{file_id}.txt"),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    if local_hash.is_some() {
        db.set_file_contract_state(&FileContractState {
            file_id: file_id.into(),
            namespace: Namespace::MyFiles,
            parent_id: None,
            shared_root_id: None,
            share_id: None,
            owner_email: None,
            permission_bits: 1,
            item_kind: ItemKind::File,
            content_type: None,
            current_version: 1,
            current_object_version_id: None,
            local_base_version: 1,
            local_hash,
            cache_path: None,
            cache_bytes: 0,
            pin_state: PinState::Inherit,
            inherited_pin_state: PinState::Unpinned,
            last_sync_at: 1,
        })
        .unwrap();
    }
}

/// Spec S1: the stamp pass marks a read-only placeholder whose bytes are the agreed content (0.8.19: access denied),
/// and never one whose identity is not the file's own, however agreed its bytes.
#[test]
fn stamp_if_agreed_marks_a_read_only_file_and_refuses_another_identity() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let db = root.db();
    let bytes = b"the agreed content".as_slice();
    let own = root.path.join("own.txt");
    std::fs::write(&own, bytes).unwrap();
    placeholders::convert_to_unsynced_placeholder(&own, "t09-own").unwrap();
    track(&db, "t09-own", Some(sha256(bytes)));
    make_read_only(&own);
    let kept = Kept::of(&own);
    assert!(placeholders::stamp_if_agreed(&db, "t09-own", &own).unwrap(), "a read-only file with agreed bytes is marked");
    assert_eq!(placeholders::platform_view(&own), placeholder(OnDisk::Wholly, true, "t09-own"));
    assert_eq!(Kept::of(&own), kept, "I1");

    let foreign = root.path.join("foreign.txt");
    std::fs::write(&foreign, bytes).unwrap();
    placeholders::convert_to_unsynced_placeholder(&foreign, "t09-other").unwrap();
    track(&db, "t09-row", Some(sha256(bytes)));
    make_read_only(&foreign);
    assert!(!placeholders::stamp_if_agreed(&db, "t09-row", &foreign).unwrap(), "another file's placeholder is not marked");
    assert_eq!(placeholders::platform_view(&foreign), placeholder(OnDisk::Wholly, false, "t09-other"));
    settle();
    assert_eq!(fetches("t09-"), 0, "I2");
}

/// Spec S2: the promote pass reads a read-only file's bytes (0.8.19: access denied) and makes its fetched candidate the
/// agreed content.
#[test]
fn promote_reads_a_read_only_file() {
    let root = Root::new();
    let db = root.db();
    let bytes = b"version three, fetched earlier".as_slice();
    let path = root.path.join("fetched.txt");
    std::fs::write(&path, bytes).unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, "t10-server").unwrap();
    track(&db, "t10-server", None);
    assert!(db.mark_no_fetched_content("t10-server").unwrap());
    db.record_fetched_version("t10-server", 3, &sha256(bytes)).unwrap();
    make_read_only(&path);
    let kept = Kept::of(&path);
    assert!(placeholders::promote_fetched_content(&db, "t10-server", &path).unwrap(), "promoted");
    let contract = db.get_file_contract_state("t10-server").unwrap().unwrap();
    assert_eq!((contract.local_base_version, contract.local_hash), (3, Some(sha256(bytes))));
    assert_eq!(Kept::of(&path), kept, "I1");
    settle();
    assert_eq!(fetches("t10-"), 0, "I2");
}
