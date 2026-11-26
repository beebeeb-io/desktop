//! The account's Trash, for a host without a UI toolkit (the native Trash window): every item in it, with its name
//! decrypted here, in the engine, with the session's vault key, from the server only (never this computer's local
//! state, which may belong to an earlier account), bound to the session that asked. Moved from the Tauri shell's `desktop_trash_list`
//! (`src-tauri/src/account_commands.rs`) so the native app needs no shell code for it; the Tauri command went with
//! that shell.
//!
//! [`Engine::restore_items_for`] puts items back and [`Engine::trash_items_for`] moves items to Trash, each bound to
//! the session of the press (nothing is sent under another). There is no unbound form of either.
//!
//! [`Engine::trash_listing`] BLOCKS on the handle's runtime, as every method of the handle does: call it from a worker
//! thread, never from the UI thread. A failure is an [`AccountCallError`] (fixed words, never a server's body).
//!
//! Must not: log a name, an id or anything the server sends; hand the vault key to anything but the name decryption
//! (the API client of the call gets none: listing the Trash decrypts nothing on the wire).

use zeroize::Zeroizing;

use super::settings_api::{kind_of, AccountCallError};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::account_dto::DesktopTrashItem;
use crate::api_client::{ApiClient, ProtectedFolderRefused};
use crate::owner_keys;

/// The account's Trash as the server listed it, and the session it was listed for.
#[derive(Clone)]
pub struct TrashListing {
    /// The session the listing belongs to: a host compares it with [`Engine::session_turn`] and drops a listing
    /// whose session is gone (a lock, a sign-out, another account's sign-in), before it draws a name of it.
    pub session: SessionGeneration,
    pub items: Vec<DesktopTrashItem>,
}

/// Why an item was not moved to Trash ([`Engine::trash_items_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotTrashed {
    /// Beebeeb refuses it for what the item is (the Backups folder): asking again changes nothing.
    Protected,
    /// The call failed, by its kind. `NotSignedIn`: the session of the press is gone, and nothing was sent.
    Call(AccountCallError),
}

/// How many items, never one of them: a listing holds decrypted names, and a `{:?}` must not print one.
impl std::fmt::Debug for TrashListing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrashListing")
            .field("session", &self.session)
            .field("items", &self.items.len())
            .finish()
    }
}

#[cfg(test)]
#[path = "trash_api_tests.rs"]
mod local_mark_tests;

impl<C: CredentialStores> Engine<C> {
    /// Every item in the account's Trash (`GET /api/v1/files?trashed=true`, every page), from the server only, for
    /// the session in memory, each name decrypted with that session's vault key. Nothing is read from this
    /// computer's local state (it may still belong to an earlier account). A name that cannot be decrypted is never
    /// guessed from a server field: it reads "Encrypted item <id>…" (a shared item's: `blocked_name`).
    ///
    /// No session in memory (signed out, or locked): [`AccountCallError::NotSignedIn`], and nothing is asked. A
    /// session that changed while the call ran (a lock, a sign-out, another sign-in): `NotSignedIn` too, and the
    /// answer is dropped here, never handed over.
    pub fn trash_listing(&self) -> Result<TrashListing, AccountCallError> {
        // The token, the key and the session's generation leave the session's lock together, as copies of their own,
        // wiped when this call ends (the token inside the call's client, which wipes it on drop; the key in a
        // `Zeroizing`).
        let (token, key, session) = self
            .person_session(|current, generation| {
                (current.token.clone(), Zeroizing::new(current.master_key), generation)
            })
            .map_err(|_| AccountCallError::NotSignedIn)?;
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        let raw = self
            .runtime
            .block_on(api.list_trashed_files_all())
            .map_err(|error| kind_of(&error))?;
        drop(api);
        if !self.acct.session_unchanged_since(session) {
            return Err(AccountCallError::NotSignedIn);
        }
        let items = trash_roots(raw.iter().filter_map(|meta| trash_item_from_meta(meta, &key)).collect());
        // The hand-over: only while the session in memory is the one the token and the key were copied from, asked
        // with no session change half done. A listing made for a session that is ending is dropped here.
        if !self.session_is_still(session, &key) {
            return Err(AccountCallError::NotSignedIn);
        }
        Ok(TrashListing { session, items })
    }

    /// How many items are in the account's Trash (`GET /api/v1/files/count`, the server's own count), for the session
    /// `session` names and no other: the token is copied only while that session is still the one in memory, and
    /// nothing is sent otherwise ([`AccountCallError::NotSignedIn`]). Reads nothing locally and decrypts nothing: the
    /// sidebar's badge, which must not read the whole Trash. BLOCKS on the handle's runtime: call it from a worker.
    pub fn trash_count_for(&self, session: SessionGeneration) -> Result<u64, AccountCallError> {
        self.account_call_for(session, |api| async move { api.trashed_file_count().await })
    }

    /// How many moves to or from Trash this handle made (each [`Engine::trash_items_for`] or
    /// [`Engine::restore_items_for`] that moved at least one item). A window that keeps the Trash count compares it
    /// with the number it last saw: a change means the count may have moved. Never waits, sends nothing.
    pub fn trash_moves(&self) -> u64 {
        self.trash_moves.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether a person's call may be sent now: `false` behind a locked screen or a closed vault (core's answer).
    /// Never waits, sends nothing.
    pub fn person_calls_allowed(&self) -> bool {
        self.check_call(super::lock_api::Call::Person(super::lock_api::LockedAction::ViewStatus))
            .is_ok()
    }

    /// WRITES: put items back from Trash (`POST /api/v1/files/{id}/restore`, one call each, in order), for the
    /// session a person pressed under and no other. `session` is [`Engine::session_turn`] as the host read it AT
    /// THE PRESS (on its window's thread, before any worker started). Before EACH item the token is copied only
    /// while the session in memory is still that one, asked with no session change half done
    /// (`AccountRuntime::while_session_is`); a session that moved on since (a lock, a sign-out, another sign-in,
    /// even back to the same account): [`AccountCallError::NotSignedIn`], and nothing (more) is sent. Stops at the
    /// first item that fails: the ones before it are restored. The only form there is: an id chosen under one
    /// account must never be sent with another session's token.
    ///
    /// BLOCKS on the handle's runtime: call it from a worker thread.
    pub fn restore_items_for(&self, session: SessionGeneration, file_ids: &[&str]) -> Result<(), AccountCallError> {
        let mut restored = false;
        let mut outcome = Ok(());
        for id in file_ids {
            let id = id.to_string();
            outcome = self.account_call_for(session, move |api| async move { api.restore_file_sent(&id).await });
            if outcome.is_err() {
                break;
            }
            restored = true;
        }
        if restored {
            self.trash_moves.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Ask the sync engine to take the server's list again (the signed-in account's local state only; asks
            // nothing of a state that is not the session's), also when a later item failed.
            self.request_resnapshot();
        }
        outcome
    }

    /// WRITES: move items to Trash (`DELETE /api/v1/files/{id}`, one call each, in order; never the permanent
    /// delete, never a confirm token), for the session a person pressed under and no other. `session` is
    /// [`Engine::session_turn`] as the host read it AT THE PRESS; the rule is [`Engine::restore_items_for`]'s:
    /// before EACH item the token is copied only while the session in memory is still that one, and a session that
    /// moved on since is [`AccountCallError::NotSignedIn`] with nothing (more) sent. Stops at the first item that
    /// fails: the ones before it are in Trash. Any 2xx is a trash that was made: the body is not read.
    ///
    /// BLOCKS on the handle's runtime: call it from a worker thread.
    pub fn trash_items_for(&self, session: SessionGeneration, file_ids: &[&str]) -> Result<(), NotTrashed> {
        let mut trashed = false;
        let mut outcome = Ok(());
        for id in file_ids {
            let id_str = *id;
            let id = id.to_string();
            let sent = self.account_call_for(session, move |api| async move {
                match api.trash_file_sent(&id).await {
                    Ok(()) => Ok(true),
                    Err(error) if error.chain().any(|cause| cause.is::<ProtectedFolderRefused>()) => Ok(false),
                    Err(error) => Err(error),
                }
            });
            outcome = match sent {
                Ok(true) => {
                    self.mark_trashed_locally(id_str);
                    Ok(())
                }
                Ok(false) => Err(NotTrashed::Protected),
                Err(kind) => Err(NotTrashed::Call(kind)),
            };
            if outcome.is_err() {
                break;
            }
            trashed = true;
        }
        if trashed {
            self.trash_moves.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // As after a restore: the signed-in account's local state only, also when a later item failed.
            self.request_resnapshot();
        }
        outcome
    }

    /// After the server accepted a trash: park the item (and a folder's subtree) `Trashing` in this account's
    /// local state, so the Files view (which reads it) drops it now instead of waiting for a re-snapshot, and the
    /// snapshot's `Trashing` guard keeps it from coming back. Best effort: no local state, no row: nothing.
    fn mark_trashed_locally(&self, id: &str) {
        if let Ok(state) = self.local_state_for_changes() {
            let _ = state.with(|db| db.mark_subtree_trashing(id));
        }
    }

    /// Whether the session in memory is, now, the very one `generation` and `key` were copied from: asked with no
    /// session change half done (the session-write lock is held for the question), so an answer made for a session
    /// that a lock or a sign-out is ending is never taken for the current one. Never waits: while a session change
    /// is under way the answer is no.
    pub(super) fn session_is_still(&self, generation: SessionGeneration, key: &[u8; 32]) -> bool {
        self.acct
            .while_session_is(&self.writes, generation, |session| {
                // Every byte is compared, whatever the first difference.
                session
                    .master_key
                    .iter()
                    .zip(key.iter())
                    .fold(0u8, |differs, (a, b)| differs | (a ^ b))
                    == 0
            })
            .unwrap_or(false)
    }

    /// Test builds only: `hook` runs once, in the middle of the next lock or sign-out (the session is ending and
    /// not yet gone from memory), on the thread that locks.
    #[cfg(any(test, feature = "test-support"))]
    pub fn during_next_forget_for_test(&self, hook: Box<dyn FnOnce() + Send>) {
        *self.forget_hook_for_test.lock().unwrap_or_else(|p| p.into_inner()) = Some(hook);
    }

    /// The session's generation now: it moves on at every sign-in, unlock, lock and sign-out. In memory, never
    /// waits: a window may ask on every frame.
    pub fn session_turn(&self) -> SessionGeneration {
        self.acct.session_generation()
    }

    /// Whether a session is in memory (signed in and unlocked). Never waits on the network or a credential store.
    pub fn has_session(&self) -> bool {
        self.unlocked()
    }

    /// How far the listing of the account's files has come while a snapshot is being ingested (`(listed, total)`):
    /// `None` when none is, or when the host keeps no transfer board. In memory, never waits.
    pub fn first_listing(&self) -> Option<(u64, u64)> {
        self.host.transfer_board()?.first_listing()
    }

    /// w31: where the account's first file list on this PC is, as the running engine last said it; `Here` when the
    /// host keeps no transfer board. In memory, never waits: a window asks on every frame.
    pub fn first_list(&self) -> crate::transfer_progress::FirstList {
        self.host
            .transfer_board()
            .map(|board| board.first_list())
            .unwrap_or_default()
    }

    /// w31: Try again on a first file list that failed: the running engine pulls now. Never waits; no engine: nothing.
    pub fn retry_first_list(&self) {
        if let Some(board) = self.host.transfer_board() {
            board.ask_pull_now();
        }
    }

    /// Wake the host's loop, as the engine does when its own view changed: for a host's worker that has an answer
    /// for the window. Nothing is queued.
    pub fn wake_host(&self) {
        self.host.wake();
    }
}

/// What a person trashed, as Trash lists it: each row once, and never a row whose parent folder is itself in Trash
/// (it went with that folder, comes back with its restore and goes with its delete for good). The server's flat
/// Trash holds every cascade-trashed row: listed as it came, a folder that holds a folder of the same name showed
/// twice (RD-d2). A row whose parent is not in the listing (a live folder, the root, or a server that sends no
/// parent) is kept. Order is kept.
pub fn trash_roots(items: Vec<DesktopTrashItem>) -> Vec<DesktopTrashItem> {
    let folders: std::collections::HashSet<String> = items
        .iter()
        .filter(|item| item.is_folder)
        .map(|item| item.id.clone())
        .collect();
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| !item.parent_id.as_ref().is_some_and(|parent| folders.contains(parent)))
        .filter(|item| seen.insert(item.id.clone()))
        .collect()
}

/// The server's row with its name decrypted (`owner_keys::display_name`), without the local state.
pub fn trash_item_from_meta(meta: &serde_json::Value, master_key: &[u8; 32]) -> Option<DesktopTrashItem> {
    let id = meta.get("id").and_then(|v| v.as_str())?.to_string();
    let name = owner_keys::display_name(meta, &id, master_key).unwrap_or_else(|| undecrypted_name(meta, &id));
    let updated_at = meta
        .get("updated_at")
        .and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .unwrap_or_default();
    Some(DesktopTrashItem {
        is_folder: meta
            .get("is_folder")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| meta.get("kind").and_then(|v| v.as_str()) == Some("folder")),
        size_bytes: meta
            .get("size_bytes")
            .and_then(|v| v.as_i64())
            .or_else(|| meta.get("size").and_then(|v| v.as_i64()))
            .unwrap_or(0),
        updated_at,
        parent_id: meta.get("parent_id").and_then(|v| v.as_str()).map(str::to_string),
        id,
        name,
    })
}

/// The words for a row whose name cannot be decrypted: a shared or foreign row's `blocked_name`, else "Encrypted item"
/// and the start of its id. Unlike the Tauri shell's fallback, no plaintext field the server sends is ever shown.
pub fn undecrypted_name(meta: &serde_json::Value, id: &str) -> String {
    if !meta["acted_via_share_id"].is_null()
        || !meta["uploaded_by"].is_null()
        || !meta["owner_wrapped_file_key"].is_null()
    {
        return owner_keys::blocked_name(id);
    }
    let short = id.get(..8).unwrap_or(id);
    format!("Encrypted item {short}\u{2026}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "a0000000-0000-4000-8000-0000000000aa";

    fn key() -> [u8; 32] {
        [7; 32]
    }

    fn encrypted(id: &str, name: &str, key: &[u8; 32]) -> String {
        let master = beebeeb_core::kdf::MasterKey::from_bytes(*key);
        let file_key = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
        beebeeb_core::encrypt::encrypt_name_for(
            &file_key,
            name,
            None,
            beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
        )
        .expect("encrypt name")
    }

    #[test]
    fn a_row_is_read_with_its_name_decrypted_with_the_vault_key() {
        let meta = json!({
            "id": ID,
            "name_encrypted": encrypted(ID, "An invented contract.pdf", &key()),
            "is_folder": false,
            "size_bytes": 1_200,
            "updated_at": "2026-11-02T10:00:00Z",
            "parent_id": "a0000000-0000-4000-8000-000000000001",
        });
        let item = trash_item_from_meta(&meta, &key()).expect("a row with an id");
        assert_eq!(item.id, ID);
        assert_eq!(item.name, "An invented contract.pdf");
        assert!(!item.is_folder);
        assert_eq!(item.size_bytes, 1_200);
        assert_eq!(item.updated_at, "2026-11-02T10:00:00Z");
        assert_eq!(item.parent_id.as_deref(), Some("a0000000-0000-4000-8000-000000000001"));
    }

    #[test]
    fn a_folder_by_kind_a_size_by_its_other_field_and_a_time_in_seconds() {
        let meta = json!({
            "id": ID,
            "name_encrypted": encrypted(ID, "Invented folder", &key()),
            "kind": "folder",
            "size": 42,
            "updated_at": 1_700_000_001,
        });
        let item = trash_item_from_meta(&meta, &key()).expect("a row with an id");
        assert!(item.is_folder);
        assert_eq!(item.size_bytes, 42);
        assert_eq!(item.updated_at, "1700000001");
        assert_eq!(item.parent_id, None);
    }

    #[test]
    fn a_name_that_does_not_decrypt_is_never_a_plaintext_field_of_the_server() {
        let meta = json!({
            "id": ID,
            "name_encrypted": encrypted(ID, "An invented secret name", &[9; 32]),
            "name": "a plaintext field",
            "display_name": "another plaintext field",
        });
        let item = trash_item_from_meta(&meta, &key()).expect("a row with an id");
        assert_eq!(item.name, "Encrypted item a0000000\u{2026}");
        assert!(!item.name.contains("secret") && !item.name.contains("plaintext"));
    }

    #[test]
    fn a_shared_row_that_does_not_decrypt_is_its_blocked_name() {
        let meta = json!({ "id": ID, "uploaded_by": "someone" });
        let item = trash_item_from_meta(&meta, &key()).expect("a row with an id");
        assert_eq!(item.name, owner_keys::blocked_name(ID));
    }

    fn listed(id: &str, name: &str, is_folder: bool, parent: Option<&str>) -> DesktopTrashItem {
        DesktopTrashItem {
            id: id.into(),
            name: name.into(),
            is_folder,
            size_bytes: 0,
            updated_at: String::new(),
            parent_id: parent.map(str::to_string),
        }
    }

    /// RD-d2: a folder trashed with a same-named folder inside it ("Film (2025)/Film (2025)/film.mkv") listed the
    /// name twice: the server's flat Trash holds every cascade-trashed row, and the view drew each one.
    #[test]
    fn what_a_trashed_folder_holds_is_listed_once_under_it_never_as_rows_of_its_own() {
        let items = vec![
            listed("f-outer", "An invented film (2025)", true, None),
            listed("f-inner", "An invented film (2025)", true, Some("f-outer")),
            listed("film", "An invented film (2025).mkv", false, Some("f-inner")),
            listed("loose", "An invented loose file.pdf", false, Some("a-live-folder")),
            listed("top", "An invented top file.txt", false, None),
        ];
        let ids: Vec<String> = trash_roots(items).into_iter().map(|item| item.id).collect();
        assert_eq!(ids, ["f-outer", "loose", "top"]);
    }

    #[test]
    fn the_same_row_twice_is_listed_once() {
        let items = vec![
            listed("a", "An invented note.txt", false, None),
            listed("a", "An invented note.txt", false, None),
        ];
        assert_eq!(trash_roots(items).len(), 1);
    }

    /// A listing without parents (an older server) keeps every row: nothing is hidden on a guess.
    #[test]
    fn rows_without_a_parent_are_all_kept() {
        let items = vec![
            listed("a", "An invented folder", true, None),
            listed("b", "An invented folder", true, None),
        ];
        assert_eq!(trash_roots(items).len(), 2);
    }

    #[test]
    fn a_row_without_an_id_is_left_out() {
        assert!(trash_item_from_meta(&json!({ "name_encrypted": "x" }), &key()).is_none());
        assert!(trash_item_from_meta(&json!({}), &key()).is_none());
    }
}
