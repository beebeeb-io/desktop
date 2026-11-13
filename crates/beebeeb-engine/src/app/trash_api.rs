//! The account's Trash, for a host without a UI toolkit (the native Trash window): every item in it, with its name
//! decrypted here, in the engine, with the session's vault key, from the server only (never this computer's local
//! state, which may belong to an earlier account), bound to the session that asked. Moved from the Tauri shell's `desktop_trash_list`
//! (`src-tauri/src/account_commands.rs`) so the native app needs no shell code for it; the Tauri command is left
//! as it was.
//!
//! [`Engine::trash_listing`] BLOCKS on the handle's runtime, as every method of the handle does: call it from a worker
//! thread, never from the UI thread. A failure is an [`AccountCallError`] (fixed words, never a server's body).
//!
//! Must not: log a name, an id or anything the server sends; hand the vault key to anything but the name decryption
//! (the API client of the call gets none: listing the Trash decrypts nothing on the wire).

use zeroize::Zeroizing;

use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::account_dto::DesktopTrashItem;
use crate::api_client::ApiClient;
use crate::owner_keys;

/// The account's Trash as the server listed it, and the session it was listed for.
#[derive(Clone)]
pub struct TrashListing {
    /// The session the listing belongs to: a host compares it with [`Engine::session_turn`] and drops a listing
    /// whose session is gone (a lock, a sign-out, another account's sign-in), before it draws a name of it.
    pub session: SessionGeneration,
    pub items: Vec<DesktopTrashItem>,
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
        let (token, key, session) = {
            let guard = self.acct.session.lock().map_err(|_| AccountCallError::NotSignedIn)?;
            let current = guard.as_ref().ok_or(AccountCallError::NotSignedIn)?;
            (
                current.token.clone(),
                Zeroizing::new(current.master_key),
                self.acct.session_generation(),
            )
        };
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        let raw = self
            .runtime
            .block_on(api.list_trashed_files_all())
            .map_err(|error| kind_of(&error))?;
        drop(api);
        if !self.acct.session_unchanged_since(session) {
            return Err(AccountCallError::NotSignedIn);
        }
        let items = raw.iter().filter_map(|meta| trash_item_from_meta(meta, &key)).collect();
        // The hand-over: only while the session in memory is the one the token and the key were copied from, asked
        // with no session change half done. A listing made for a session that is ending is dropped here.
        if !self.session_is_still(session, &key) {
            return Err(AccountCallError::NotSignedIn);
        }
        Ok(TrashListing { session, items })
    }

    /// Whether the session in memory is, now, the very one `generation` and `key` were copied from: asked with no
    /// session change half done (the session-write lock is held for the question), so an answer made for a session
    /// that a lock or a sign-out is ending is never taken for the current one. Never waits: while a session change
    /// is under way the answer is no.
    fn session_is_still(&self, generation: SessionGeneration, key: &[u8; 32]) -> bool {
        let _writes = match self.writes.try_read() {
            Ok(writes) => writes,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return false,
        };
        if !self.acct.session_unchanged_since(generation) {
            return false;
        }
        let guard = self.acct.session.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().is_some_and(|session| {
            // Every byte is compared, whatever the first difference.
            session
                .master_key
                .iter()
                .zip(key.iter())
                .fold(0u8, |differs, (a, b)| differs | (a ^ b))
                == 0
        })
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

    /// Wake the host's loop, as the engine does when its own view changed: for a host's worker that has an answer
    /// for the window. Nothing is queued.
    pub fn wake_host(&self) {
        self.host.wake();
    }
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

    #[test]
    fn a_row_without_an_id_is_left_out() {
        assert!(trash_item_from_meta(&json!({ "name_encrypted": "x" }), &key()).is_none());
        assert!(trash_item_from_meta(&json!({}), &key()).is_none());
    }
}
