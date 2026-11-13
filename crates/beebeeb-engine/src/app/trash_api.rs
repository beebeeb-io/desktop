//! The account's Trash, for a host without a UI toolkit (the native Trash window): every item in it, with its name
//! decrypted here, in the engine, with the session's vault key. Moved from the Tauri shell's `desktop_trash_list`
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
use crate::account_dto::DesktopTrashItem;
use crate::api_client::ApiClient;
use crate::owner_keys;
use crate::state_db::StateDb;

impl<C: CredentialStores> Engine<C> {
    /// Every item in the account's Trash (`GET /api/v1/files?trashed=true`, every page), each name decrypted with the
    /// session's vault key, or the name this computer's local state holds for it. A name that cannot be decrypted is
    /// never guessed from a server field: it reads "Encrypted item <id>…" (a shared item's: `blocked_name`).
    ///
    /// No session in memory (signed out, or locked): [`AccountCallError::NotSignedIn`], and nothing is asked.
    pub fn trash_listing(&self) -> Result<Vec<DesktopTrashItem>, AccountCallError> {
        // The token and the key leave the session's lock as copies of their own, wiped when this call ends (the
        // token inside the call's client, which wipes it on drop; the key in a `Zeroizing`).
        let (token, key) = {
            let session = self.acct.session.lock().map_err(|_| AccountCallError::NotSignedIn)?;
            let session = session.as_ref().ok_or(AccountCallError::NotSignedIn)?;
            (session.token.clone(), Zeroizing::new(session.master_key))
        };
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        let raw = self
            .runtime
            .block_on(api.list_trashed_files_all())
            .map_err(|error| kind_of(&error))?;
        drop(api);
        let db = self.state_db().ok();
        Ok(raw
            .iter()
            .filter_map(|meta| trash_item(meta, &key, db.as_ref()))
            .collect())
    }

    /// Wake the host's loop, as the engine does when its own view changed: for a host's worker that has an answer
    /// for the window (the native Trash and Search read theirs when the window draws next). Nothing is queued.
    pub fn wake_host(&self) {
        self.host.wake();
    }
}

/// One row of the server's Trash listing as the Trash view's item: the name this computer's local state holds for it
/// when there is one (`owner_keys::stored_name`), else the decrypted name, else the fixed words of
/// [`undecrypted_name`]. `None`: a row without an id.
pub fn trash_item(meta: &serde_json::Value, master_key: &[u8; 32], db: Option<&StateDb>) -> Option<DesktopTrashItem> {
    let mut item = trash_item_from_meta(meta, master_key)?;
    if let Some(db) = db
        && let Ok(owner_keys::StoredName::Name(name)) = owner_keys::stored_name(db, &item.id, master_key)
    {
        item.name = name;
    }
    Some(item)
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
        assert!(trash_item(&json!({}), &key(), None).is_none());
    }
}
