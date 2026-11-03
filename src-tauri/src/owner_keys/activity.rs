use beebeeb_core::kdf::{FileKey, MasterKey};
use serde_json::Value;

/// L16 readers for one shared-folder activity row (task 2275).
///
/// A bound envelope wins: the link label is sealed to the master key and bound
/// to the share id (L16a), and the actor name is sealed under a folder-key
/// subkey bound to the shared root (L16b). Until every writer ships L16, a
/// legacy folder-key name still reads behind the same `{`-prefixed gate the
/// 2236 request readers use; anything that opens under neither is not shown.
pub(super) fn open_labels(
    master: &MasterKey,
    folder_key: &FileKey,
    root: Option<uuid::Uuid>,
    row: &Value,
) -> (Value, Value) {
    let share = row["share_id"].as_str().and_then(|s| uuid::Uuid::parse_str(s).ok());
    let label = row["share_label_encrypted"]
        .as_str()
        .and_then(|s| {
            let bound = serde_json::from_str::<beebeeb_types::EncryptedBlob>(s)
                .ok()
                .zip(share)
                .and_then(|(blob, share)| beebeeb_core::kdf::unwrap_link_label(master, share.as_bytes(), &blob).ok());
            bound.or_else(|| legacy_name(folder_key, s))
        })
        .map(Value::String)
        .unwrap_or(Value::Null);
    let actor = row["actor_name_encrypted"]
        .as_str()
        .and_then(|s| {
            root.and_then(|root| beebeeb_core::kdf::open_folder_actor_name(folder_key, root.as_bytes(), s).ok())
                .or_else(|| legacy_name(folder_key, s))
        })
        .map(Value::String)
        .unwrap_or(Value::Null);
    (label, actor)
}

fn legacy_name(folder_key: &FileKey, text: &str) -> Option<String> {
    // The same gate the 2236 readers use: only an envelope shape is tried.
    text.starts_with('{')
        .then(|| beebeeb_core::encrypt::decrypt_name_metadata_with_key(folder_key, text).ok().map(|metadata| metadata.name))
        .flatten()
}
