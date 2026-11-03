use beebeeb_core::kdf::{FileKey, MasterKey};
use serde_json::Value;

/// L16 readers for one shared-folder activity row (task 2275).
///
/// A bound envelope wins. L16/L16c have no legacy label reader: a label sealed
/// the old way (a folder-key name) opens as nothing, exactly as L16 says. The
/// actor name keeps its L16b reader only; it never falls back either.
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
            serde_json::from_str::<beebeeb_types::EncryptedBlob>(s)
                .ok()
                .zip(share)
                .and_then(|(blob, share)| beebeeb_core::kdf::unwrap_link_label(master, share.as_bytes(), &blob).ok())
        })
        .map(Value::String)
        .unwrap_or(Value::Null);
    let actor = row["actor_name_encrypted"]
        .as_str()
        .and_then(|s| {
            root.and_then(|root| beebeeb_core::kdf::open_folder_actor_name(folder_key, root.as_bytes(), s).ok())
        })
        .map(Value::String)
        .unwrap_or(Value::Null);
    (label, actor)
}
