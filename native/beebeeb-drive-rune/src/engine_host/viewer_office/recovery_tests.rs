//! Data tests of the recovery store's door and of the editor's words (`frames/office/edit/recovery.rs`).

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;
use crate::frames::office::edit::{EditUi, recovery as words};

const FILE: &str = "00000000-0000-4000-8000-000000000001";
const OTHER: &str = "00000000-0000-4000-8000-000000000002";

/// A fake engine store: blobs by file id as (time, base, bytes), for one session.
#[derive(Default)]
struct Fake {
    blobs: Mutex<BTreeMap<String, (i64, i64, Vec<u8>)>>,
    turn: u64,
}

impl Blobs for Fake {
    fn put(&self, session: SessionGeneration, id: &str, base: i64, bytes: &Zeroizing<Vec<u8>>) -> Result<(), RecoveryRefused> {
        if session != SessionGeneration::for_test(self.turn) {
            return Err(RecoveryRefused::NotSignedIn);
        }
        // 14:32:07 UTC on a day.
        let at = 86_400 * 100 + 14 * 3600 + 32 * 60 + 7;
        self.blobs.lock().unwrap().insert(id.to_string(), (at, base, bytes.to_vec()));
        Ok(())
    }
    fn list(&self, session: SessionGeneration) -> Result<Vec<RecoveryEntry>, RecoveryRefused> {
        if session != SessionGeneration::for_test(self.turn) {
            return Err(RecoveryRefused::NotSignedIn);
        }
        Ok(self
            .blobs
            .lock()
            .unwrap()
            .iter()
            .map(|(id, (at, base, _))| RecoveryEntry { file_id: id.clone(), saved_at: *at, base: *base })
            .collect())
    }
    fn take(&self, _session: SessionGeneration, id: &str) -> Result<Zeroizing<Vec<u8>>, RecoveryRefused> {
        self.blobs.lock().unwrap().get(id).map(|b| Zeroizing::new(b.2.clone())).ok_or(RecoveryRefused::NotFound)
    }
    fn discard(&self, _session: SessionGeneration, id: &str) -> Result<(), RecoveryRefused> {
        self.blobs.lock().unwrap().remove(id).map(|_| ()).ok_or(RecoveryRefused::NotFound)
    }
}

fn store() -> (Arc<Fake>, Recovery) {
    let fake = Arc::new(Fake::default());
    (fake.clone(), Recovery::new(fake, SessionGeneration::for_test(0)))
}

fn bytes(text: &str) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(text.as_bytes().to_vec())
}

#[test]
fn recovery_restore_after_helper_kill_is_the_snapshots_text() {
    let (_fake, recovery) = store();
    assert!(recovery.keep(FILE, 3, &bytes("snapshot at 14:32:07")));
    // A newer snapshot replaces the older one; the helper then dies and the restore is the last snapshot's.
    assert!(recovery.keep(FILE, 3, &bytes("snapshot later")));
    assert_eq!(&**recovery.restore(FILE).unwrap(), b"snapshot later");
}

#[test]
fn recovery_restore_after_app_restart_opens_the_snapshots_bytes() {
    let (fake, first) = store();
    first.keep(FILE, 1, &bytes("kept text"));
    drop(first);
    // A new app run: a new store over the same blobs.
    let second = Recovery::new(fake, SessionGeneration::for_test(0));
    assert_eq!(second.offer(FILE).unwrap().at, "14:32");
    assert_eq!(&**second.restore(FILE).unwrap(), b"kept text");
    // The blob stays after a restore (a crash right after loses nothing).
    assert!(second.offer(FILE).is_some());
}

#[test]
fn recovery_discard_deletes_the_blob() {
    let (_fake, recovery) = store();
    recovery.keep(FILE, 1, &bytes("x"));
    recovery.discard(FILE);
    assert!(recovery.offer(FILE).is_none());
    assert!(recovery.restore(FILE).is_none());
}

#[test]
fn recovery_a_save_deletes_the_blob_and_only_that_files() {
    let (_fake, recovery) = store();
    recovery.keep(FILE, 1, &bytes("x"));
    recovery.keep(OTHER, 1, &bytes("y"));
    recovery.saved(FILE);
    assert!(recovery.offer(FILE).is_none());
    assert!(recovery.offer(OTHER).is_some());
}

#[test]
fn recovery_another_session_keeps_and_lists_nothing() {
    let (fake, _) = store();
    let stale = Recovery::new(fake, SessionGeneration::for_test(1));
    assert!(!stale.keep(FILE, 1, &bytes("x")));
    assert!(stale.pending().is_empty());
    assert!(stale.offer(FILE).is_none());
}

#[test]
fn recovery_pending_lists_the_accounts_blobs_for_the_notice() {
    let (_fake, recovery) = store();
    assert_eq!(words::unlocked_words(recovery.pending().len(), None), None);
    recovery.keep(FILE, 1, &bytes("x"));
    recovery.keep(OTHER, 1, &bytes("y"));
    let pending = recovery.pending();
    assert_eq!(pending.len(), 2);
    assert!(pending.iter().all(|p| p.at == "14:32"));
    assert_eq!(
        words::unlocked_words(2, None).unwrap(),
        "Unsaved changes to 2 files were kept on this PC."
    );
}

#[test]
fn recovery_words_follow_the_editor_state() {
    let mut edit = EditUi::new(beebeeb_office_protocol::office::EditKind::Docx);
    assert_eq!(words::banner(Some(&edit), None), None);
    let offer = RecoveryOffer { at: "14:32".to_string() };
    assert_eq!(
        words::banner(Some(&edit), Some(&offer)),
        Some(words::Banner::Offer { at: "14:32".to_string() })
    );
    assert_eq!(words::offer_words("Plan.docx", "14:32"), "Unsaved changes to Plan.docx from 14:32.");
    edit.restored = Some("14:32".to_string());
    let restored = words::banner(Some(&edit), Some(&offer)).unwrap();
    assert!(words::words(&restored, "Plan.docx").starts_with("The editor stopped. Your changes up to 14:32 are restored."));
    // Two crashes: read-only, with the restored copy as the Keep both path.
    edit.stopped = true;
    let read_only = words::banner(Some(&edit), None).unwrap();
    assert!(matches!(read_only, words::Banner::ReadOnly { at: Some(_) }));
    assert!(words::words(&read_only, "Plan.docx").contains("read-only"));
}
