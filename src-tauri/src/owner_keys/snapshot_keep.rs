//! A snapshot row whose name already opens is kept when its detail is missing.
//! Dropping it would make the mirror prune a file the snapshot still listed.

use beebeeb_core::kdf::MasterKey;
use beebeeb_sync::key_resolution as keys;
use serde_json::Value;

pub(super) fn name_opens(master: &MasterKey, id: &str, row: &Value) -> bool {
    row["name_encrypted"]
        .as_str()
        .is_some_and(|name| keys::resolve_owned_key(master, id, super::row_kind(row), None, None, name, None).is_ok())
}
