//! URL-builder tests: chunk/file/metadata URLs and the master-key drop guard.

use super::*;

#[test]
fn test_chunk_url() {
    let client = ApiClient::new("https://api.beebeeb.io".into(), "tok".into(), [0u8; 32]);
    assert_eq!(
        client.chunk_url(FileId::new("file123"), 0),
        "https://api.beebeeb.io/api/v1/files/file123/chunks/0"
    );
}

/// Task 1538 Codex P1 (PR #49, lib.rs:1087 thread, "invalidate the
/// session key the engine holds"): once `EngineRunner::abort` confirms
/// the engine's task is actually gone, every `Arc<ApiClient>` it held
/// drops with it — this must actually wipe the master key from memory,
/// not just deallocate it holding its last plaintext value. Verified the
/// same way `lib.rs`'s `session_zeroizes_master_key_on_drop` verifies
/// `Session`: a compile-time guarantee (the type implements
/// `ZeroizeOnDrop`, which requires `Drop`) plus exercising the real drop
/// path without panicking.
#[test]
fn api_client_zeroizes_master_key_on_drop() {
    // Compile-time guarantee: if `Drop`/`ZeroizeOnDrop` for `ApiClient`
    // is ever removed, this stops compiling.
    fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
    assert_zeroize_on_drop::<ApiClient>();

    // Sanity-check the underlying operation the `Drop` impl performs.
    let mut key = [7u8; 32];
    key.zeroize();
    assert_eq!(key, [0u8; 32]);

    // Exercise the real drop path (no panic / double-free) on a
    // populated client; the wipe itself runs inside `Drop` where the
    // bytes are no longer observably aliased.
    let client = ApiClient::new("https://api.beebeeb.io".into(), "tok".into(), [9u8; 32]);
    drop(client);
}

#[test]
fn test_desktop_contract_urls_trim_base_and_encode_queries() {
    let client = ApiClient::new("https://api.beebeeb.io/".into(), "tok".into(), [0u8; 32]);
    assert_eq!(client.upload_init_url(), "https://api.beebeeb.io/api/v1/uploads/init");
    assert_eq!(
        client.upload_chunk_url(UploadSessionId::new("session/with space"), 2),
        "https://api.beebeeb.io/api/v1/uploads/session%2Fwith%20space/chunks/2"
    );
    assert_eq!(
        client.list_files_url(ListFilesScope {
            parent_id: Some(FolderId::new("parent/one")),
            shared_folder_id: Some(SharedFolderId::new("shared one")),
            trashed: Some(false),
            cursor: Some("cursor/one"),
        }),
        "https://api.beebeeb.io/api/v1/files?parent_id=parent%2Fone&shared_folder_id=shared%20one&trashed=false&cursor=cursor%2Fone"
    );
    assert_eq!(
        client.version_download_url(FileId::new("file/one"), VersionId::new("version two")),
        "https://api.beebeeb.io/api/v1/files/file%2Fone/versions/version%20two/download"
    );
    assert_eq!(
        client.thumbnail_upload_url(FileId::new("file/one"), "medium", Some("LEHV6nWB2yk8pyo0adR*.7kCMdnj")),
        "https://api.beebeeb.io/api/v1/files/file%2Fone/thumbnail?blurhash=LEHV6nWB2yk8pyo0adR%2A.7kCMdnj"
    );
    assert_eq!(
        client.thumbnail_upload_url(FileId::new("file/one"), "large", Some("ignored")),
        "https://api.beebeeb.io/api/v1/files/file%2Fone/thumbnail/large"
    );
}
#[test]
fn test_metadata_operation_urls() {
    let client = ApiClient::new("https://api.beebeeb.io".into(), "tok".into(), [0u8; 32]);
    assert_eq!(
        client.file_url(FileId::new("file one")),
        "https://api.beebeeb.io/api/v1/files/file%20one"
    );
    assert_eq!(
        client.restore_file_url(FileId::new("file one")),
        "https://api.beebeeb.io/api/v1/files/file%20one/restore"
    );
    assert_eq!(
        client.permanent_file_url(FileId::new("file one")),
        "https://api.beebeeb.io/api/v1/files/file%20one/permanent"
    );
    assert_eq!(
        client.folder_members_url(FolderId::new("folder/one")),
        "https://api.beebeeb.io/api/v1/shares/invites/folder-members/folder%2Fone"
    );
    assert_eq!(
        client.folder_keys_url(InviteId::new("invite/one")),
        "https://api.beebeeb.io/api/v1/shares/invites/invite%2Fone/folder-keys"
    );
    assert_eq!(
        client.shares_incoming_url(),
        "https://api.beebeeb.io/api/v1/shares/invites/incoming"
    );
}
