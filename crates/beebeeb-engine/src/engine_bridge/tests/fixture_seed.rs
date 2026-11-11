pub(super) fn seed_bridge_row(
    bridge: &EngineBridge,
    file_id: &str,
    path: &str,
    parent_id: Option<&str>,
    status: FileStatus,
    cache_bytes: i64,
) {
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status,
            size_bytes: cache_bytes,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: parent_id.map(str::to_string),
            item_kind: ItemKind::File,
        })
        .unwrap();
    let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
    contract.parent_id = parent_id.map(str::to_string);
    contract.cache_bytes = cache_bytes;
    bridge.db.set_file_contract_state(&contract).unwrap();
}
