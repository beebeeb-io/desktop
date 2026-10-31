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
/// The identity of every fetch the provider refused because this process asked for it inside a no-fetch scope.
static REFUSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Task 2284 (CI fix): the identity of every FETCH_DATA callback the provider received, whatever it answered. A
/// callback is either refused (`REFUSED`) or served (`FETCHED`).
static CALLED: Mutex<Vec<String>> = Mutex::new(Vec::new());
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
    CALLED.lock().unwrap_or_else(PoisonError::into_inner).push(identity.clone());
    // Task 2284: the production refusal, as the provider's callback runs it (the root is connected with
    // `CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO`, as production does).
    // SAFETY: `info` is this live callback's info.
    if unsafe { super::refuse_own_scoped_fetch(info, &identity, fetch.RequiredFileOffset, fetch.RequiredLength) } {
        REFUSED.lock().unwrap_or_else(PoisonError::into_inner).push(identity);
        return;
    }
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

/// How many of this process's own fetches the provider refused inside a no-fetch scope, by identity prefix.
fn refused(prefix: &str) -> usize {
    REFUSED.lock().unwrap_or_else(PoisonError::into_inner).iter().filter(|id| id.starts_with(prefix)).count()
}

/// How many FETCH_DATA callbacks the provider received for a file whose identity starts with `prefix`.
fn callbacks(prefix: &str) -> usize {
    CALLED.lock().unwrap_or_else(PoisonError::into_inner).iter().filter(|id| id.starts_with(prefix)).count()
}

/// Task 2284 (CI fix): the refusal half of a dehydration test. Whether the data open of a dehydrated placeholder
/// reaches the provider at all depends on the platform: on the Legion (probe K0) the open alone asks for the file,
/// and the no-fetch scope refuses it (os 389); on the GitHub Windows runner the open did not call FETCH_DATA (run
/// 41032979318), so there was nothing to refuse. Either way nothing may be served (the caller asserts
/// `fetches(prefix) == 0` first), and every callback that did fire must be a refusal.
/// Which case ran is written to the real stderr (not the captured one), so a passing CI log shows it too.
fn assert_every_own_fetch_was_refused(prefix: &str, test: &str) {
    use std::io::Write;
    let (called, refused) = (callbacks(prefix), refused(prefix));
    let line = if called == 0 {
        format!("task 2284 {test}: 0 FETCH_DATA callbacks: the platform never routed the data open to the provider\n")
    } else {
        format!("task 2284 {test}: {called} FETCH_DATA callback(s), {refused} refused by the no-fetch scope\n")
    };
    let _ = std::io::stderr().write_all(line.as_bytes());
    assert_eq!(
        refused, called,
        "every FETCH_DATA callback for this file was this process's own fetch, refused inside the scope"
    );
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
        let key = unsafe { CfConnectSyncRoot(PCWSTR(wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO) }.unwrap();
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

/// Task 2284 (CI fix): a self-relative DACL security descriptor as text, without the account SIDs it holds: the
/// control word, then each ACE's type, flags and access mask, in order. For assert messages only.
fn dacl_shape(descriptor: &[u8]) -> String {
    let u16_at = |at: usize| descriptor.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| descriptor.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (Some(control), Some(offset)) = (u16_at(2), u32_at(16)) else {
        return format!("<{} bytes, too short>", descriptor.len());
    };
    let mut shape = format!("control {control:#06x}");
    if offset == 0 {
        return shape + ", no DACL";
    }
    let acl = offset as usize;
    let Some(count) = u16_at(acl + 4) else {
        return shape + ", <DACL past the end>";
    };
    let mut ace = acl + 8;
    for _ in 0..count {
        let (Some(&kind), Some(&flags), Some(size), Some(mask)) =
            (descriptor.get(ace), descriptor.get(ace + 1), u16_at(ace + 2), u32_at(ace + 4))
        else {
            return shape + ", <ACE past the end>";
        };
        shape += &format!(", ace(type {kind}, flags {flags:#04x}, mask {mask:#010x})");
        ace += usize::from(size.max(4));
    }
    shape
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

fn never_judged(_: &str) -> anyhow::Result<Judged> {
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
        Ok(judge(hash, Some(&sha256(&bytes)), None))
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
        placeholders::finalize_wholly_local(&path, "t02-local", "t02-server", &mut |hash| Ok(judge(hash, Some(&sha256(&bytes)), None)))
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
        placeholders::finalize_wholly_local(&path, "t04-local", "t04-server", &mut |hash| Ok(judge(hash, Some(&payload), Some(&agreed))))
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
            Ok(judge(hash, Some(&sha256(b"the uploaded original")), None))
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
        placeholders::finalize_wholly_local(&path, "t08-local", "t08-server", &mut |hash| Ok(judge(hash, Some(&sha256(&bytes)), None)))
            .unwrap();
    assert_eq!(local, Local::Finalized(Judged::MarkInSync));
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t08-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
}

fn track(db: &crate::state_db::StateDb, file_id: &str, local_hash: Option<String>) {
    track_at(db, file_id, &format!("{file_id}.txt"), local_hash);
}

/// The server row `file_id` at `path`, with its agreed content when `local_hash` is given.
fn track_at(db: &crate::state_db::StateDb, file_id: &str, path: &str, local_hash: Option<String>) {
    use crate::state_db::{FileContractState, FileEntry, FileStatus, ItemKind, Namespace, PinState};
    db.upsert_file(&FileEntry {
        file_id: file_id.into(),
        path: path.into(),
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

/// A finalization journal row for the upload `prefix` of the file at `target` (relative to the root).
fn journal(db: &crate::state_db::StateDb, prefix: &str, target: &str, payload: &Path) {
    db.put_upload_finalization(&crate::state_db::UploadFinalization {
        op_id: format!("{prefix}-op"),
        local_file_id: format!("{prefix}-local"),
        server_file_id: format!("{prefix}-server"),
        target_path: target.into(),
        payload_path: payload.to_string_lossy().into_owned(),
        stamped: false,
        review: None,
    })
    .unwrap();
}

/// Spec R1, R2, I4, I5: a row held for review is left alone by every later tick (no retry, no warn per tick) and
/// tried again once per engine start, when it goes through F0-F9 again. Its file is not changed.
#[test]
fn review_rows_are_retried_once_per_start() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let db = root.db();
    let path = root.path.join("r.txt");
    std::fs::write(&path, b"another file's bytes").unwrap();
    placeholders::convert_to_unsynced_placeholder(&path, "t11-other").unwrap();
    track_at(&db, "t11-server", "r.txt", None);
    let payload = root.temp.path().join("t11-payload");
    std::fs::write(&payload, b"the uploaded bytes").unwrap();
    journal(&db, "t11", "r.txt", &payload);

    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    let rows = db.upload_finalizations().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].review.as_deref(), Some("other_file"), "held for review (F4)");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, false, "t11-other"), "I5: not changed");
    assert!(payload.exists(), "the staged copy is kept while the row is in review");

    // Its file goes. Later ticks of the same start do not look at the row again: it is still in review.
    std::fs::remove_file(&path).unwrap();
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    let rows = db.upload_finalizations().unwrap();
    assert_eq!(rows.len(), 1, "a row in review is not retried every tick");
    assert_eq!(rows[0].review.as_deref(), Some("other_file"));

    // The next start tries it again: nothing is left at the target (F2), so the row ends.
    crate::windows_cf::upload_finalization::forget_review_retry(&root.path);
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    assert!(db.upload_finalizations().unwrap().is_empty(), "the next start finishes it");
    assert!(!payload.exists(), "and removes its staged copy");
}

/// Spec F7b with no staged copy (review focus: a crash after its unlink, or a cleanup): the bytes are the agreed
/// content, so the row ends in sync. 0.8.19 failed on the payload's open every tick.
#[test]
fn a_finalization_with_its_payload_gone_still_finishes() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let db = root.db();
    let path = root.path.join("p.txt");
    let bytes = b"the agreed bytes".repeat(50);
    std::fs::write(&path, &bytes).unwrap();
    make_read_only(&path);
    track_at(&db, "t12-server", "p.txt", Some(sha256(&bytes)));
    journal(&db, "t12", "p.txt", &root.temp.path().join("t12-payload-already-removed"));
    let kept = Kept::of(&path);
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    assert!(db.upload_finalizations().unwrap().is_empty(), "the row ends");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t12-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
    settle();
    assert_eq!(fetches("t12-"), 0, "I2");
}

/// Spec F8 (review focus): a file open in another program stays pending, not in review, and finishes on the next
/// tick once it is closed.
#[test]
fn an_open_file_stays_pending_and_finishes_next_tick() {
    use crate::unknown_check::{OnDisk, PlatformView};
    let root = Root::new();
    let db = root.db();
    let path = root.path.join("o.txt");
    let bytes = b"open in an editor".repeat(50);
    std::fs::write(&path, &bytes).unwrap();
    make_read_only(&path);
    track_at(&db, "t13-server", "o.txt", None);
    let payload = root.temp.path().join("t13-payload");
    std::fs::write(&payload, &bytes).unwrap();
    journal(&db, "t13", "o.txt", &payload);
    let reader = std::fs::File::open(&path).unwrap();
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    let rows = db.upload_finalizations().unwrap();
    assert_eq!(rows.len(), 1, "pending");
    assert_eq!((rows[0].stamped, rows[0].review.as_deref()), (false, None), "not in review, not stamped");
    assert_eq!(placeholders::platform_view(&path), PlatformView::PlainFile, "nothing changed");
    drop(reader);
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    assert!(db.upload_finalizations().unwrap().is_empty(), "the next tick finishes it");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t13-server"));
    assert!(!payload.exists(), "its staged copy is removed");
}

/// Spec F7c, I3 (self-review of #195): a placeholder of this upload that is already marked in sync, whose bytes are
/// neither the payload nor the agreed content, gets the server identity and is left NOT in sync. Keeping the old mark
/// would claim the server holds bytes it may not have, and "Free up space" could then drop them.
#[test]
fn an_in_sync_placeholder_whose_bytes_are_neither_is_left_not_in_sync() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let path = root.path.join("s.txt");
    std::fs::write(&path, b"bytes the server may not have").unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, "t15-local").unwrap();
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t15-local"));
    make_read_only(&path);
    let kept = Kept::of(&path);
    let local = placeholders::finalize_wholly_local(&path, "t15-local", "t15-server", &mut |hash| {
        Ok(judge(hash, Some(&sha256(b"the uploaded original")), None))
    })
    .unwrap();
    assert_eq!(local, Local::Finalized(Judged::IdentityOnly));
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, false, "t15-server"), "F7c: not in sync");
    assert_eq!(Kept::of(&path), kept, "I1");
    settle();
    assert_eq!(fetches("t15-"), 0, "I2");
}

/// Spec F8 for a row in review (self-review of #195): when the start's retry of a row in review ends pending (its file
/// is open in another program), the row leaves review ("no review"), so a later tick of the same start finishes it
/// once the file is closed, and the tray stops counting it.
#[test]
fn a_review_row_that_comes_back_pending_leaves_review_and_finishes_next_tick() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let db = root.db();
    let path = root.path.join("rp.txt");
    let bytes = b"held, then open in an editor".repeat(50);
    std::fs::write(&path, &bytes).unwrap();
    make_read_only(&path);
    track_at(&db, "t14-server", "rp.txt", None);
    let payload = root.temp.path().join("t14-payload");
    std::fs::write(&payload, &bytes).unwrap();
    journal(&db, "t14", "rp.txt", &payload);
    db.mark_upload_finalization_review("t14-op", "platform_error", 1).unwrap();
    let reader = std::fs::File::open(&path).unwrap();
    // The first retry of this start: the row in review goes through F0-F9 again and meets the open file (F8).
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    let rows = db.upload_finalizations().unwrap();
    assert_eq!(rows.len(), 1, "pending");
    assert_eq!((rows[0].stamped, rows[0].review.as_deref()), (false, None), "pending is not review (F8)");
    drop(reader);
    // A later tick of the same start.
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    assert!(db.upload_finalizations().unwrap().is_empty(), "the next tick finishes it");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t14-server"));
    assert!(!payload.exists(), "its staged copy is removed");
}

/// Self-review of #195 (0.8.19 read the staged copy only after the file opened): a row whose file is open in another
/// program stays pending without reading its staged copy, which is as large as the upload. The copy is read once,
/// when the file itself is open for its hash.
#[test]
fn a_pending_row_does_not_read_its_staged_copy() {
    use crate::windows_cf::upload_finalization::{payloads_read, retry};
    let root = Root::new();
    let db = root.db();
    let path = root.path.join("big.txt");
    let bytes = b"a large upload".repeat(1000);
    std::fs::write(&path, &bytes).unwrap();
    track_at(&db, "t16-server", "big.txt", None);
    let payload = root.temp.path().join("t16-payload");
    std::fs::write(&payload, &bytes).unwrap();
    journal(&db, "t16", "big.txt", &payload);
    let reader = std::fs::File::open(&path).unwrap();
    let before = payloads_read();
    retry(&db, &root.path);
    retry(&db, &root.path);
    assert_eq!(db.upload_finalizations().unwrap().len(), 1, "pending");
    assert_eq!(payloads_read() - before, 0, "the staged copy is not read while the file is open elsewhere");
    drop(reader);
    retry(&db, &root.path);
    assert!(db.upload_finalizations().unwrap().is_empty(), "the next tick finishes it");
    assert_eq!(payloads_read() - before, 1, "read once, with the file open for its hash");
}

/// Task 2284 (probe K5): dehydrate `path`'s range through a handle with no data access, so nothing is fetched.
fn dehydrate_fully(path: &Path, offset: i64, length: i64, share: u32) -> Result<(), String> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, SYNCHRONIZE, WRITE_DAC};
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0 | WRITE_DAC.0 | SYNCHRONIZE.0)
        .share_mode(share)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .map_err(|e| format!("open os {:?}", e.raw_os_error()))?;
    // SAFETY: the handle belongs to `file`, open for the call.
    unsafe { CfDehydratePlaceholder(HANDLE(file.as_raw_handle()), offset, length, CF_DEHYDRATE_FLAG_NONE, None) }
        .map_err(|e| format!("dehydrate {:?}", e.code()))
}

/// Task 2284: dehydrate `length` bytes fully, right before the next data open of a finalization, stamp or promote pass.
fn dehydrate_before_the_data_open(length: usize) {
    placeholders::BEFORE_DATA_OPEN.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path: &Path| dehydrate_fully(path, 0, length as i64, 0).unwrap()));
    });
}

/// Task 2284 S1: the platform refuses to dehydrate a placeholder that is not in sync (HRESULT 0x80070179), so
/// `stamp_if_agreed`'s window opens only when this process marks the file in sync (another pass) and it is then
/// dehydrated, between the pre-check and the data open. This hook does both, in that order.
fn mark_in_sync_then_dehydrate_before_the_data_open(length: usize) {
    placeholders::BEFORE_DATA_OPEN.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path: &Path| {
            use std::os::windows::fs::OpenOptionsExt;
            use windows::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES, SYNCHRONIZE};
            let file = std::fs::OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES.0 | FILE_WRITE_ATTRIBUTES.0 | SYNCHRONIZE.0)
                .share_mode(0)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(path)
                .unwrap();
            // SAFETY: the handle belongs to `file`, open for the call.
            unsafe { CfSetInSyncState(HANDLE(file.as_raw_handle()), CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) }.unwrap();
            drop(file);
            dehydrate_fully(path, 0, length as i64, 0).unwrap();
        }));
    });
}

/// A folder `icacls` denies to Everyone (probe K6). On drop the deny is removed, then the DACL of the folder and of
/// everything that was in it is written back exactly as it was read before the deny. Task 2284 (CI fix): `icacls
/// /remove:d /T` alone re-propagates inheritance, and on the GitHub Windows runner the file's DACL came back with other
/// bytes than it had before the deny (run 41032979318), so I1 compared the harness's change, not the product's.
struct DeniedFolder {
    dir: PathBuf,
    /// Every path under the folder (the folder first) and its self-relative DACL security descriptor, read before
    /// the deny.
    saved: Vec<(PathBuf, Vec<u8>)>,
}

impl Drop for DeniedFolder {
    fn drop(&mut self) {
        let _ = std::process::Command::new("icacls").arg(&self.dir).arg("/remove:d").arg("*S-1-1-0").arg("/T").output();
        for (path, descriptor) in &self.saved {
            // Never panic in a drop (it may run while a failed assert unwinds); a DACL not written back is caught by
            // the test's own I1 comparison.
            if let Err(error) = set_dacl(path, descriptor) {
                eprintln!("task 2284: the DACL of {path:?} was not written back: {error}");
            }
        }
    }
}

/// `path` and everything under it, the folder before what it holds.
fn walk(path: &Path, into: &mut Vec<PathBuf>) {
    into.push(path.to_path_buf());
    if std::fs::symlink_metadata(path).unwrap().is_dir() {
        for entry in std::fs::read_dir(path).unwrap() {
            walk(&entry.unwrap().path(), into);
        }
    }
}

/// Write `descriptor` (as [`dacl`] read it) back as `path`'s DACL. `SetFileSecurityW` sets this one object's
/// descriptor and propagates nothing to what a folder holds.
fn set_dacl(path: &Path, descriptor: &[u8]) -> Result<(), String> {
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SetFileSecurityW};
    let wide = wide(path);
    // SAFETY: `wide` is NUL-terminated and `descriptor` is a self-relative security descriptor; both outlive the call,
    // which only reads them.
    let written = unsafe {
        SetFileSecurityW(
            PCWSTR(wide.as_ptr()),
            DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR(descriptor.as_ptr().cast_mut().cast()),
        )
    };
    if written.as_bool() { Ok(()) } else { Err(format!("{:?}", windows::core::Error::from_win32().code())) }
}

fn deny_folder(dir: &Path) -> DeniedFolder {
    let mut paths = Vec::new();
    walk(dir, &mut paths);
    let mut saved = Vec::new();
    for path in paths {
        let descriptor = dacl(&path);
        saved.push((path, descriptor));
    }
    let output = std::process::Command::new("icacls").arg(dir).arg("/deny").arg("*S-1-1-0:(OI)(CI)(RX)").output().unwrap();
    assert!(output.status.success(), "icacls /deny on {dir:?}");
    DeniedFolder { dir: dir.to_path_buf(), saved }
}

/// A wholly local in-sync placeholder carrying `identity`, whose bytes the provider serves: `(path, bytes)`.
fn wholly_local(root: &Root, name: &str, identity: &str, length: usize) -> (PathBuf, Vec<u8>) {
    let path = root.path.join(name);
    let bytes: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, identity).unwrap();
    serve(identity, &bytes);
    (path, bytes)
}

/// Run `body` with a tracing subscriber of its own, and return the WARN lines it logged on this thread.
fn warn_lines(body: impl FnOnce()) -> Vec<String> {
    #[derive(Clone)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = Capture(buffer.clone());
    // tracing caches a callsite's interest process-wide; a second live dispatcher keeps registration off the
    // one-subscriber fast path, as the crate's other log captures do.
    let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    tracing::subscriber::with_default(subscriber, body);
    let bytes = buffer.lock().unwrap().clone();
    String::from_utf8_lossy(&bytes).lines().map(str::to_owned).collect()
}

/// Task 2284 A1.1 (probe K6): a target in a folder this account may not read is unreadable, not missing (which would
/// end the row as done): held for review, its staged copy kept, its file untouched.
#[test]
fn a_target_in_a_denied_folder_goes_to_review_and_keeps_its_staged_copy() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let db = root.db();
    let dir = root.path.join("denied");
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("a.txt");
    let bytes = b"the uploaded document".repeat(100);
    std::fs::write(&path, &bytes).unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, "t17-server").unwrap();
    make_read_only(&path);
    let kept = Kept::of(&path);
    let kept_shape = dacl_shape(&dacl(&path));
    track_at(&db, "t17-server", "denied/a.txt", None);
    let payload = root.temp.path().join("t17-payload");
    std::fs::write(&payload, &bytes).unwrap();
    journal(&db, "t17", "denied/a.txt", &payload);
    let denied = deny_folder(&dir);
    // Task 2284 (CI fix): the DACL the pass meets is the one with the deny inherited into it. Read it, and read it
    // again after the pass, under the same deny: nothing the harness does lies between the two reads. (The owner may
    // read a DACL that denies it everything else.)
    let under_deny = dacl(&path);
    crate::windows_cf::upload_finalization::forget_review_retry(&root.path);
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    let rows = db.upload_finalizations().unwrap();
    assert_eq!(rows.len(), 1, "held, not done");
    assert_eq!(rows[0].review.as_deref(), Some("unreadable"), "a denied folder is Unreadable, never Missing");
    assert!(payload.exists(), "the staged copy is kept while the row is in review");
    let after_pass = dacl(&path);
    assert_eq!(
        sha256(&after_pass),
        sha256(&under_deny),
        "I1: the pass did not change the DACL (read under the deny): before {}, after {}",
        dacl_shape(&under_deny),
        dacl_shape(&after_pass)
    );
    drop(denied);
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, true, "t17-server"), "I5: not changed");
    // The attributes and the last-write time as before the deny; the DACL as the harness wrote it back.
    let now = Kept::of(&path);
    assert_eq!(now, kept, "I1; the DACL before {kept_shape}, now {}", dacl_shape(&dacl(&path)));
}

/// Task 2284 A1.1 (the 2191 guard, probe K6): the guard cannot read a file in a denied folder, so its mark stays pending for the next
/// tick, and a second pass adds no warn line (I4).
#[test]
fn the_guard_keeps_a_mark_pending_for_a_file_in_a_denied_folder() {
    use crate::state_db::{FileEntry, FileStatus, ItemKind};
    let root = Root::new();
    let db = root.db();
    let dir = root.path.join("denied");
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("g.txt");
    std::fs::write(&path, b"a file of unknown content").unwrap();
    placeholders::convert_to_in_sync_placeholder(&path, "t18").unwrap();
    db.upsert_file(&FileEntry {
        file_id: "t18".into(),
        path: "denied/g.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    let rows = db.list_guarded().unwrap();
    assert_eq!(rows.len(), 1);
    let denied = deny_folder(&dir);
    let summary = crate::unknown_check::guard_rows(
        &db,
        &root.path,
        &rows,
        &placeholders::platform_view,
        &mut |_: &Path| -> anyhow::Result<()> { panic!("a file in a denied folder is not marked") },
        &mut crate::unknown_check::GuardControl { stop: &|| false, progress: &mut |_| {} },
    )
    .unwrap();
    assert_eq!(summary.mark_failed, 1, "the mark could not be read, let alone made");
    assert!(db.mark_pending_ids().unwrap().contains("t18"), "still mark-pending");
    let warns = warn_lines(|| {
        let second = crate::unknown_check::guard_rows(
            &db,
            &root.path,
            &rows,
            &placeholders::platform_view,
            &mut |_: &Path| -> anyhow::Result<()> { panic!("a file in a denied folder is not marked") },
            &mut crate::unknown_check::GuardControl { stop: &|| false, progress: &mut |_| {} },
        )
        .unwrap();
        assert_eq!(second.mark_failed, 1, "the next tick retries the mark");
    });
    assert!(warns.is_empty(), "no warn line per tick (I4): {warns:?}");
    assert!(db.mark_pending_ids().unwrap().contains("t18"), "still mark-pending after the second pass");
    drop(denied);
}

/// Task 2284 A1.2 / F7d (probe K5a): a dehydration between the state check and the data open is refused at the
/// provider, so nothing is fetched and the row stays pending; the next tick re-stamps it in sync as F5, still unfetched.
/// Refused where the platform routes that open to the provider at all ([`assert_every_own_fetch_was_refused`]).
#[test]
fn a_dehydration_before_the_data_open_fetches_nothing_and_stays_pending() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let db = root.db();
    let (path, bytes) = wholly_local(&root, "t20.bin", "t20-server", 256 * 1024);
    track_at(&db, "t20-server", "t20.bin", None);
    let payload = root.temp.path().join("t20-payload");
    std::fs::write(&payload, &bytes).unwrap();
    journal(&db, "t20", "t20.bin", &payload);
    let kept = Kept::of(&path);
    dehydrate_before_the_data_open(bytes.len());
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    let rows = db.upload_finalizations().unwrap();
    assert_eq!(rows.len(), 1, "pending (F7d), not done");
    assert_eq!((rows[0].stamped, rows[0].review.as_deref()), (false, None), "pending is not review");
    settle();
    assert_eq!(fetches("t20-"), 0, "I2: the dehydrated file is not fetched");
    assert_every_own_fetch_was_refused("t20-", "a_dehydration_before_the_data_open_fetches_nothing_and_stays_pending");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Nothing, true, "t20-server"));
    assert_eq!(Kept::of(&path), kept, "I1");
    // The next tick takes F5: a cloud-only placeholder is re-stamped without opening its data.
    crate::windows_cf::upload_finalization::retry(&db, &root.path);
    assert!(db.upload_finalizations().unwrap().is_empty(), "the next tick is done");
    assert_eq!(fetches("t20-"), 0, "still nothing served");
}

/// Task 2284 A1.2 / S1: a mark in sync by another pass and a dehydration between the state check and the data open are
/// refused, so the pass does not mark the file from a fetch it would have made.
/// Refused where the platform routes that open to the provider at all ([`assert_every_own_fetch_was_refused`]).
#[test]
fn stamp_if_agreed_after_a_dehydration_fetches_nothing() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let db = root.db();
    let path = root.path.join("t21.bin");
    let bytes: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    placeholders::convert_to_unsynced_placeholder(&path, "t21-server").unwrap();
    serve("t21-server", &bytes);
    track(&db, "t21-server", Some(sha256(&bytes)));
    mark_in_sync_then_dehydrate_before_the_data_open(bytes.len());
    assert!(!placeholders::stamp_if_agreed(&db, "t21-server", &path).unwrap(), "not marked: not wholly on disk any more");
    settle();
    assert_eq!(fetches("t21-"), 0, "I2");
    assert_every_own_fetch_was_refused("t21-", "stamp_if_agreed_after_a_dehydration_fetches_nothing");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Nothing, true, "t21-server"));
}

/// Task 2284 S1 (platform fact): a placeholder that is not in sync cannot be dehydrated, so `stamp_if_agreed`'s data
/// open races a dehydration only after a mark in sync.
#[test]
fn an_unsynced_placeholder_cannot_be_dehydrated() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let path = root.path.join("t25.bin");
    let bytes: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    placeholders::convert_to_unsynced_placeholder(&path, "t25-server").unwrap();
    let error = dehydrate_fully(&path, 0, bytes.len() as i64, 0).expect_err("the platform refuses");
    assert!(error.contains("0x80070179"), "ERROR_CLOUD_FILE_NOT_IN_SYNC: {error}");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Wholly, false, "t25-server"));
}

/// Task 2284 A1.2 / S2: a dehydration between the state check and the data open is refused, so the pass does not
/// promote a file from a fetch it would have made.
/// Refused where the platform routes that open to the provider at all ([`assert_every_own_fetch_was_refused`]).
#[test]
fn promote_after_a_dehydration_fetches_nothing() {
    use crate::unknown_check::OnDisk;
    let root = Root::new();
    let db = root.db();
    let (path, bytes) = wholly_local(&root, "t22.bin", "t22-server", 256 * 1024);
    track_at(&db, "t22-server", "t22.bin", None);
    assert!(db.mark_no_fetched_content("t22-server").unwrap());
    db.record_fetched_version("t22-server", 3, &sha256(&bytes)).unwrap();
    dehydrate_before_the_data_open(bytes.len());
    assert!(!placeholders::promote_fetched_content(&db, "t22-server", &path).unwrap(), "not promoted: not wholly on disk any more");
    settle();
    assert_eq!(fetches("t22-"), 0, "I2");
    assert_every_own_fetch_was_refused("t22-", "promote_after_a_dehydration_fetches_nothing");
    assert_eq!(placeholders::platform_view(&path), placeholder(OnDisk::Nothing, true, "t22-server"));
}

/// Task 2284 A1.2 (the lead's PA1): a scope refuses only this process's own fetches. Another program that opens the
/// scoped file is served.
#[test]
fn another_program_is_still_served_while_a_scope_is_open() {
    let root = Root::new();
    let path = root.path.join("t23.bin");
    let bytes = b"another program reads this".repeat(1000);
    cloud_only(&path, &bytes, "t23-server");
    serve("t23-server", &bytes);
    let scope = crate::no_fetch_scope::enter(&["t23-server"]);
    let status = std::process::Command::new("cmd")
        .arg("/c")
        .arg("type")
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    drop(scope);
    settle();
    assert_eq!(status.code(), Some(0), "the other program reads the file");
    assert!(fetches("t23-") >= 1, "another program's fetch is served");
    assert_eq!(refused("t23-"), 0, "only this process's own fetches are refused");
}

/// Task 2284 A1.2 (the lead's PA1): this process's own open of a cloud-only file inside a scope fails with os 389
/// (ERROR_CLOUD_FILE_UNSUCCESSFUL), never with Ok — a zero-byte read would pass for a real one.
#[test]
fn our_own_open_of_a_scoped_cloud_only_file_fails_with_os_error_389() {
    let root = Root::new();
    let path = root.path.join("t24.bin");
    let bytes = b"scoped and cloud only".repeat(1000);
    cloud_only(&path, &bytes, "t24-server");
    serve("t24-server", &bytes);
    let scope = crate::no_fetch_scope::enter(&["t24-server"]);
    let error = std::fs::read(&path).expect_err("the provider refuses this process's own fetch");
    assert_eq!(error.raw_os_error(), Some(389), "ERROR_CLOUD_FILE_UNSUCCESSFUL: {error:?}");
    assert_eq!(fetches("t24-"), 0, "nothing was served to this process");
    assert!(refused("t24-") >= 1, "the provider refused it");
    drop(scope);
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "without the scope, the bytes are served");
}
