//! Task 2240 slice 2: the placeholder reconcile through folder listings, on real Windows.
//!
//! Spec: `docs/superpowers/specs/2026-10-28-2240-reconcile-from-listing-design.md` (workspace repo), T10–T16, and
//! the source pins on the wiring. Registered roots are confined to a fresh temporary directory and revoked on drop,
//! as in `signout_tests.rs`; no provider is connected (spec premise P5).
use super::*;
use crate::os_state_reconcile::{Delta, OS_STATE_MASK};
use crate::source_pin::squeeze;
use crate::state_db::{FileContractState, FileEntry, FileStatus, ItemKind, Namespace, PinState, StateDb};
use std::os::windows::fs::OpenOptionsExt;
use std::sync::atomic::AtomicUsize;

const RECALL: u32 = 0x0040_0000;
const PINNED: u32 = 0x0008_0000;
const UNPINNED: u32 = 0x0010_0000;
/// `FILE_READ_ATTRIBUTES`: a handle that reads attributes only, as Explorer and indexers hold.
const FILE_READ_ATTRIBUTES: u32 = 0x80;

/// The text of `fn <name>(` up to the end of its body, with LF line endings on every checkout.
fn fn_text(source: &str, name: &str) -> String {
    let start = source
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("fn {name} is missing"));
    let end = source[start..].find("\n}\n").expect("fn body end");
    source[start..start + end].to_string()
}

#[test]
fn task_2240_reconcile_reads_no_attributes_per_row() {
    let source = include_str!("mod.rs").replace("\r\n", "\n");
    let with = squeeze(&fn_text(&source, "reconcile_placeholder_state_with"));
    assert!(
        with.contains("plan_reconcile("),
        "the pass must be decided by os_state_reconcile"
    );
    assert!(
        with.contains("placeholders::list_dir_attributes("),
        "attributes come from one listing per folder"
    );
    assert_eq!(
        with.matches("read_os_placeholder_state").count(),
        1,
        "the direct read appears once: in the read_direct closure"
    );
    assert!(with.contains("|rel|placeholders::read_os_placeholder_state("));
    let public = squeeze(&fn_text(&source, "reconcile_placeholder_state"));
    assert!(
        !public.contains("read_os_placeholder_state"),
        "the per-row loop is gone from the public wrapper"
    );
}

/// A registered scratch sync root with its own state DB.
struct Root {
    connected: Option<CF_CONNECTION_KEY>,
    db: StateDb,
    root: std::path::PathBuf,
    _temp: tempfile::TempDir,
}

impl Root {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        crate::windows_cf::register_sync_root(&root).unwrap();
        Self {
            connected: None,
            db,
            root,
            _temp: temp,
        }
    }

    /// P5: dehydration needs a running provider, so this root is connected, with no callbacks registered
    /// (the pattern of the 1640 test at `callbacks.rs:1100-1130`).
    fn connected() -> Self {
        let mut r = Self::new();
        let table = [CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NONE,
            Callback: None,
        }];
        let root_wide: Vec<u16> = r.root.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: the path and the table outlive the call; no callback is registered.
        let key = unsafe { CfConnectSyncRoot(PCWSTR(root_wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_NONE) }
            .expect("connect the scratch root");
        r.connected = Some(key);
        r
    }

    fn path(&self, rel: &str) -> std::path::PathBuf {
        self.root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    /// A file row (file id = its path) with `status` and its own `pin`; it inherits "unpinned".
    fn track(&self, rel: &str, status: FileStatus, pin: PinState) {
        self.db
            .upsert_file(&FileEntry {
                file_id: rel.into(),
                path: rel.into(),
                status,
                size_bytes: 0,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 1,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        self.db
            .set_file_contract_state(&FileContractState {
                file_id: rel.into(),
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
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: pin,
                inherited_pin_state: PinState::Unpinned,
                last_sync_at: 1,
            })
            .unwrap();
    }

    /// A hydrated, in-sync file placeholder at `on_disk` (as an upload leaves it), identified as `file_id`.
    fn hydrated(&self, on_disk: &str, file_id: &str) -> std::path::PathBuf {
        let path = self.path(on_disk);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"2240 slice 2 bytes").unwrap();
        placeholders::convert_to_unsynced_placeholder(&path, "temporary-local-id").unwrap();
        placeholders::convert_to_in_sync_placeholder(&path, file_id).unwrap();
        path
    }

    fn pinned_in_db(&self, rel: &str) -> bool {
        let rows = self.db.file_overview_rows().unwrap();
        rows.into_iter().find(|(e, _)| e.file_id == rel).unwrap().1
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        if let Some(key) = self.connected.take() {
            // SAFETY: the key of the connection made in `connected`, disconnected once.
            let _ = unsafe { CfDisconnectSyncRoot(key) };
        }
        crate::windows_cf::unregister_sync_root(&self.root).unwrap();
    }
}

/// The three decoded bits of `path` as its folder's listing shows them.
fn listing_bits(path: &std::path::Path) -> u32 {
    let name = path.file_name().unwrap().to_str().unwrap();
    let listing = placeholders::list_dir_attributes(path.parent().unwrap()).unwrap();
    listing[name] & OS_STATE_MASK
}

/// The three decoded bits of `path` as `GetFileAttributesW` reads them.
fn direct_bits(path: &std::path::Path) -> u32 {
    placeholders::read_os_placeholder_state(path).unwrap() & OS_STATE_MASK
}

/// Opens an attribute-only handle on `path`, sharing everything.
fn attribute_handle(path: &std::path::Path) -> std::fs::File {
    std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(0x7)
        .open(path)
        .unwrap()
}

#[test]
fn t10_listing_bits_equal_direct_bits_on_a_hydrated_placeholder() {
    let r = Root::new();
    let path = r.hydrated("a/file.txt", "a/file.txt");
    assert_eq!(listing_bits(&path), direct_bits(&path));
    assert_eq!(direct_bits(&path) & RECALL, 0, "a hydrated placeholder has its data");
}

#[test]
fn t11_pin_then_unpin_is_visible_in_the_listing_at_once() {
    let r = Root::new();
    let path = r.hydrated("a/file.txt", "a/file.txt");
    placeholders::set_pin_state(&path, true, false).unwrap();
    assert_eq!(direct_bits(&path) & (PINNED | UNPINNED), PINNED);
    assert_eq!(listing_bits(&path) & (PINNED | UNPINNED), PINNED);
    placeholders::set_pin_state(&path, false, false).unwrap();
    assert_eq!(direct_bits(&path) & (PINNED | UNPINNED), UNPINNED);
    assert_eq!(listing_bits(&path) & (PINNED | UNPINNED), UNPINNED);
}

#[test]
fn t12_dehydration_is_visible_in_the_listing_at_once() {
    let r = Root::connected();
    let path = r.hydrated("a/file.txt", "a/file.txt");
    placeholders::set_pin_state(&path, false, false).unwrap();
    placeholders::dehydrate_placeholder(&path).unwrap();
    assert_eq!(direct_bits(&path) & RECALL, RECALL);
    assert_eq!(listing_bits(&path) & RECALL, RECALL);
}

#[test]
fn t13_listing_catches_up_after_the_last_handle_closes() {
    let r = Root::new();
    let path = r.hydrated("a/file.txt", "a/file.txt");
    placeholders::set_pin_state(&path, false, false).unwrap();
    let handle = attribute_handle(&path);
    placeholders::set_pin_state(&path, true, false).expect("pin with an attribute-only handle open");
    assert_eq!(direct_bits(&path) & (PINNED | UNPINNED), PINNED);
    let while_open = listing_bits(&path) & (PINNED | UNPINNED);
    // Recorded for the PR, not asserted: whether the folder listing showed the pin while the handle was open.
    println!("lag_while_open={}", while_open != PINNED);
    drop(handle);
    assert_eq!(
        listing_bits(&path) & (PINNED | UNPINNED),
        PINNED,
        "the listing catches up after close"
    );
}

#[test]
fn t14_no_unpin_is_written_while_a_handle_hides_the_pin() {
    let r = Root::new();
    let path = r.hydrated("a/file.txt", "a/file.txt");
    placeholders::set_pin_state(&path, false, false).unwrap();
    let handle = attribute_handle(&path);
    // The in-app pin: the DB row and the placeholder together, while another process holds a handle.
    r.track("a/file.txt", FileStatus::Local, PinState::Pinned);
    placeholders::set_pin_state(&path, true, false).unwrap();
    let plan = reconcile_placeholder_state_with(&r.db, &r.root, &AtomicUsize::new(0));
    assert!(
        plan.deltas
            .iter()
            .all(|(id, _, pin)| id != "a/file.txt" || *pin != Some(PinState::Unpinned)),
        "the app's pin was written back to unpinned: {:?}",
        plan.deltas
    );
    assert!(r.pinned_in_db("a/file.txt"));
    drop(handle);
}

/// Today's per-file pass (`windows_cf/mod.rs:1279-1320` at `60bd1eb`), copied as the reference: the deltas it
/// would write, without writing them.
fn per_file_reference(db: &StateDb, sync_root: &std::path::Path) -> Vec<Delta> {
    use crate::state_db::decode_os_state;
    let mut deltas: Vec<Delta> = Vec::new();
    for (entry, pinned) in db.file_overview_rows().unwrap() {
        if entry.is_dir() {
            continue;
        }
        let rel = entry.path.trim_matches('/');
        if rel.is_empty() {
            continue;
        }
        let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let Some(attrs) = placeholders::read_os_placeholder_state(&local_path) else {
            continue;
        };
        let (desired_status, desired_pin) = decode_os_state(attrs, entry.status.clone());
        let status_delta = match desired_status {
            Some(s) if s != entry.status => Some(s),
            _ => None,
        };
        let pin_delta = match desired_pin {
            Some(PinState::Pinned) if !pinned => Some(PinState::Pinned),
            Some(PinState::Unpinned) if pinned => Some(PinState::Unpinned),
            _ => None,
        };
        if status_delta.is_some() || pin_delta.is_some() {
            deltas.push((entry.file_id, status_delta, pin_delta));
        }
    }
    deltas
}

#[test]
fn t15_end_to_end_deltas_equal_the_per_file_reference() {
    let r = Root::connected();
    // Each DB row is one step off its placeholder.
    let pinned_on_disk = r.hydrated("a/pinned.txt", "a/pinned.txt");
    placeholders::set_pin_state(&pinned_on_disk, true, false).unwrap();
    r.track("a/pinned.txt", FileStatus::Local, PinState::Inherit);

    let unpinned_on_disk = r.hydrated("a/unpinned.txt", "a/unpinned.txt");
    placeholders::set_pin_state(&unpinned_on_disk, false, false).unwrap();
    r.track("a/unpinned.txt", FileStatus::Local, PinState::Pinned);

    let freed = r.hydrated("b/freed.txt", "b/freed.txt");
    placeholders::set_pin_state(&freed, false, false).unwrap();
    placeholders::dehydrate_placeholder(&freed).unwrap();
    r.track("b/freed.txt", FileStatus::Local, PinState::Inherit);

    // On disk as "case.txt", in the DB as "Case.txt": the listing misses it, the direct read finds it (D3).
    let case = r.hydrated("b/case.txt", "b/Case.txt");
    placeholders::set_pin_state(&case, true, false).unwrap();
    r.track("b/Case.txt", FileStatus::Local, PinState::Inherit);

    r.track("b/missing.txt", FileStatus::Local, PinState::Inherit);

    let reference = per_file_reference(&r.db, &r.root);
    assert_eq!(
        reference.len(),
        4,
        "the fixture must give the reference something to write: {reference:?}"
    );
    let plan = reconcile_placeholder_state_with(&r.db, &r.root, &AtomicUsize::new(0));
    assert_eq!(plan.deltas, reference);
    assert_eq!(
        plan.counts.fallbacks, 2,
        "Case.txt and missing.txt are not in their folder's listing"
    );
    assert_eq!(plan.counts.listing_hits, 3);
}
