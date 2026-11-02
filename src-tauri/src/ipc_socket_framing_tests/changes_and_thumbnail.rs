//! ListChanges paging over the socket, plus thumbnail variant and destination
//! guards.

use super::*;

#[test]
fn list_changes_returns_created_items_and_a_nonempty_anchor() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(seed_two_changes(&fx));
    fx.rt.block_on(async {
        let reply = send_one(&fx, list_changes_request(None)).await;
        let payload = reply
            .get("FileProviderChanges")
            .unwrap_or_else(|| panic!("expected FileProviderChanges, got {reply}"));
        let changes = payload["changes"].as_array().expect("changes array");
        assert_eq!(changes.len(), 2, "both created items: {payload}");
        assert_eq!(changes[0]["file_id"], "fp-a");
        assert_eq!(changes[0]["kind"], "created");
        let anchor = payload["next_anchor"].as_str().expect("next_anchor string");
        assert!(!anchor.is_empty(), "the anchor after real changes is never empty");
        assert!(anchor.len() <= 500, "Apple caps the anchor at 500 bytes");
    });
}

#[test]
fn list_changes_from_the_fresh_anchor_delivers_nothing_and_keeps_the_anchor() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(seed_two_changes(&fx));
    fx.rt.block_on(async {
        let first = send_one(&fx, list_changes_request(None)).await;
        let anchor = first["FileProviderChanges"]["next_anchor"].as_str().unwrap().to_string();
        let second = send_one(&fx, list_changes_request(Some(&anchor))).await;
        let payload = &second["FileProviderChanges"];
        assert_eq!(payload["changes"].as_array().map(Vec::len), Some(0), "up to date: {second}");
        assert_eq!(payload["next_anchor"].as_str(), Some(anchor.as_str()), "a no-op poll must not move the anchor");
    });
}

#[test]
fn list_changes_pages_with_resume_tokens_without_losing_or_repeating() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        for i in 0..250 {
            let id = format!("fp-{i:03}");
            fx.db.upsert_file(&fp_item(&id, &format!("/{id}"), ItemKind::File)).unwrap();
        }
    });
    fx.rt.block_on(async {
        let mut seen = std::collections::HashSet::new();
        let mut pages = 0;
        let mut cursor: Option<String> = None;
        loop {
            let reply = send_one(&fx, list_changes_request(cursor.as_deref())).await;
            let payload = &reply["FileProviderChanges"];
            let changes = payload["changes"].as_array().expect("changes array");
            assert!(changes.len() <= 100, "page size must be honored, got {}", changes.len());
            for change in changes {
                let key = (
                    change["file_id"].as_str().unwrap().to_string(),
                    change["kind"].as_str().unwrap().to_string(),
                );
                assert!(seen.insert(key), "a change repeated across pages: {change}");
            }
            pages += 1;
            let next = payload["next_anchor"].as_str().map(str::to_string);
            match next {
                Some(a) if Some(&a) == cursor.as_ref() => break, // up to date
                Some(a) => {
                    assert!(a > cursor.clone().unwrap_or_default(), "resume tokens strictly increase");
                    cursor = Some(a);
                }
                None => break,
            }
            assert!(pages < 10, "250 changes at 100/page must take 3 pages");
        }
        assert_eq!(seen.len(), 250, "every change delivered exactly once");
        // 3 delivery pages + 1 final round trip that reports "current" (the
        // echoed anchor breaks the loop).
        assert_eq!(pages, 4, "3 delivery pages + 1 up-to-date round trip, got {pages}");
    });
}

#[test]
fn list_changes_reports_deletes_with_the_old_parent() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        fx.db.upsert_file(&fp_item("fp-gone", "/gone.txt", ItemKind::File)).unwrap();
        // delete_file records the Deleted change itself, capturing the row's
        // parent BEFORE the delete (task 1697 wiring). Put the row under
        // parent-9 first so the change carries it.
        fx.db.set_file_contract_state(&crate::state_db::FileContractState {
            file_id: "fp-gone".into(),
            parent_id: Some("parent-9".into()),
            ..crate::state_db::FileContractState {
                file_id: String::new(),
                namespace: crate::state_db::Namespace::MyFiles,
                parent_id: None,
                shared_root_id: None,
                share_id: None,
                owner_email: None,
                permission_bits: 0,
                item_kind: ItemKind::File,
                content_type: None,
                current_version: 0,
                current_object_version_id: None,
                local_base_version: 0,
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: crate::state_db::PinState::Inherit,
                inherited_pin_state: crate::state_db::PinState::Unpinned,
                last_sync_at: 0,
            }
        })
        .unwrap();
        fx.db.delete_file("fp-gone").unwrap();
        let reply = send_one(&fx, list_changes_request(None)).await;
        let changes = reply["FileProviderChanges"]["changes"].as_array().unwrap();
        let deletion = changes.iter().find(|c| c["kind"] == "deleted").expect("the delete");
        assert_eq!(deletion["file_id"], "fp-gone");
        assert_eq!(deletion["new_parent_id"], "parent-9", "the materialized filter needs the old parent");
    });
}

#[test]
fn list_changes_reports_both_parents_for_reparents() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        fx.db.upsert_file(&fp_item("fp-moved", "/old/moved.txt", ItemKind::File)).unwrap();
        fx.db.record_file_change("fp-moved", crate::state_db::FpChangeKind::Created, None).unwrap();
        fx.db
            .record_file_change("fp-moved", crate::state_db::FpChangeKind::Reparented, Some("old-parent".into()))
            .unwrap();
        let reply = send_one(&fx, list_changes_request(None)).await;
        let changes = reply["FileProviderChanges"]["changes"].as_array().unwrap();
        let moved = changes.iter().find(|c| c["kind"] == "reparented").expect("the reparent");
        assert_eq!(moved["old_parent_id"], "old-parent");
    });
}

#[test]
fn an_unparsable_anchor_gets_an_error_not_a_crash() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let reply = send_one(&fx, list_changes_request(Some("garbage-anchor"))).await;
        assert!(reply.get("Error").is_some(), "an expired/garbage anchor is an error: {reply}");
    });
}

#[test]
fn retired_rpcs_are_refused_and_the_connection_survives() {
    // Task 1699 audit G13: SetFileStatus (a no-op stub the extension never
    // sent), RecordOpenedFile and EnforceSmartCache (smart-cache bookkeeping
    // with no File Provider caller — the daemon enforces the cache limit
    // internally) are RETIRED from the wire. An unknown variant must be
    // refused per-request (Error reply) without taking the connection down,
    // exactly like any other deserialization failure.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let client = fx.connect().await;
        let mut lines = LineReader::new(client);
        for request in [
            r#"{"SetFileStatus":{"file_id":"x","status":"local"}}"#,
            r#"{"RecordOpenedFile":{"file_id":"x","cache_path":"/tmp/x","cache_bytes":1}}"#,
            r#"{"EnforceSmartCache":{"max_unpinned_cache_bytes":null,"disk_pressure_min_free_bytes":null}}"#,
        ] {
            lines.client.write_all(format!("{request}\n").as_bytes()).await.unwrap();
            let reply = parse(&lines.next_line().await);
            let message = reply["Error"]["message"].as_str().unwrap_or_default().to_string();
            assert!(message.contains("unknown variant"), "retired RPC must be refused, got {reply}");
        }
        lines.client.write_all(b"\"GetSyncSummary\"\n").await.unwrap();
        let next = parse(&lines.next_line().await);
        assert!(next.get("SyncSummary").is_some(), "connection must still be usable: {next}");
    });
}

#[test]
fn thumbnail_variant_buckets_match_the_windows_picker() {
    // The variant picker mirrors windows_cf/thumbnail_provider.rs: a
    // requested max dimension of <=96 px gets the "small" server variant,
    // <=256 px "medium", anything larger "large".
    assert_eq!(crate::ipc_socket::thumbnail_variant(1), "small");
    assert_eq!(crate::ipc_socket::thumbnail_variant(96), "small");
    assert_eq!(crate::ipc_socket::thumbnail_variant(97), "medium");
    assert_eq!(crate::ipc_socket::thumbnail_variant(256), "medium");
    assert_eq!(crate::ipc_socket::thumbnail_variant(257), "large");
    assert_eq!(crate::ipc_socket::thumbnail_variant(u32::MAX), "large");
}

#[test]
fn persist_thumbnail_plaintext_writes_atomically_inside_allowed_roots() {
    let dir = tempfile::tempdir().unwrap();
    let roots = vec![dir.path().to_path_buf()];
    let dest = dir.path().join("thumb.bin");
    let bytes = b"thumbnail-plaintext";
    let written = crate::ipc_socket::persist_thumbnail_plaintext(&dest, &roots, bytes).unwrap();
    assert_eq!(written, bytes.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), bytes, "the renamed file must carry the plaintext");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "thumbnail staging plaintext must be owner-only");
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(entries.len(), 1, "no `.part` staging leftovers may remain");
}

#[test]
fn persist_thumbnail_plaintext_rejects_destinations_outside_allowed_roots() {
    let dir = tempfile::tempdir().unwrap();
    let outside = std::env::temp_dir().join("1699-persist-outside-thumb.bin");
    let roots = vec![dir.path().to_path_buf()];
    let err = crate::ipc_socket::persist_thumbnail_plaintext(&outside, &roots, b"x").unwrap_err();
    assert!(err.to_string().contains("allowed root"), "got {err}");
    assert!(!outside.exists(), "nothing may be written outside an allowed root");
}

// Task 1699 review (PR #103, thread PRRT_kwDOSLX6Xs6oeNfW, P1): the staging
// write must never FOLLOW a pre-created `<dest>.part` symlink. The old plain
// `File::create(&staging)` did: a caller that can reach the IPC socket plants
// `<dest>.part` as a symlink to an arbitrary daemon-accessible file, the
// parent-only containment check passes, and the plaintext truncates/overwrites
// the target outside every allowed root. The anchored staging writer (task
// 1247 / 1670-round-3 pattern: O_NOFOLLOW descent + fresh temp leaf +
// anchored renameat) can only REPLACE a planted path, never write through it.
#[test]
fn persist_thumbnail_plaintext_never_writes_through_a_planted_staging_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let outside_dir = tempfile::tempdir().unwrap();
    let outside = outside_dir.path().join("victim.txt");
    std::fs::write(&outside, b"DO-NOT-TOUCH").unwrap();
    let dest = dir.path().join("thumb.bin");
    let staging = dir.path().join("thumb.bin.part");
    std::os::unix::fs::symlink(&outside, &staging).unwrap();
    let roots = vec![dir.path().to_path_buf()];
    let written = crate::ipc_socket::persist_thumbnail_plaintext(&dest, &roots, b"payload")
        .expect("a normal dest inside an allowed root must still succeed");
    assert_eq!(written, b"payload".len() as u64);
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"DO-NOT-TOUCH",
        "the pre-created staging symlink must never divert the write outside the allowed roots"
    );
    let meta = std::fs::symlink_metadata(&dest).unwrap();
    assert!(
        meta.file_type().is_file(),
        "the published dest must be a REAL file, not the planted symlink moved into place"
    );
    assert_eq!(std::fs::read(&dest).unwrap(), b"payload");
}

#[test]
fn fetch_thumbnail_rejects_destination_outside_allowed_roots() {
    // The dest_path arrives straight off the wire (untrusted), exactly like
    // HydrateFile's: it must be bounded to the allowed roots BEFORE the
    // daemon does anything else with the request.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(
                b"{\"FetchThumbnail\":{\"file_id\":\"00000000-0000-0000-0000-000000000001\",\"dest_path\":\"/tmp/1699-fpfs-outside/thumb.bin\",\"max_dimension\":256}}\n",
            )
            .await
            .unwrap();
        let reply = parse(&read_line(&mut client).await);
        let message = reply["Error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("allowed root"), "got {reply}");
    });
}

#[test]
fn fetch_thumbnail_reports_daemon_side_failures_without_writing_the_dest() {
    // The fixture's ApiClient points at an unreachable port, so a request
    // that survives identifier + destination validation must surface the
    // daemon-side failure as an Error reply and leave NO file at dest —
    // the extension must never read a stale or partial staging file.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let dest = std::env::temp_dir().join("1699-thumb-should-not-exist.bin");
        let _ = std::fs::remove_file(&dest);
        let request = format!(
            "{{\"FetchThumbnail\":{{\"file_id\":\"00000000-0000-0000-0000-000000000001\",\"dest_path\":\"{}\",\"max_dimension\":256}}}}\n",
            dest.display()
        );
        let mut client = fx.connect().await;
        client.write_all(request.as_bytes()).await.unwrap();
        let reply = parse(&read_line(&mut client).await);
        let message = reply["Error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("thumbnail") && !message.contains("unknown variant"),
            "the arm must reach the daemon-side fetch, got {reply}"
        );
        assert!(!dest.exists(), "a failed fetch must not leave a staging file");
        let _ = std::fs::remove_file(&dest);
    });
}

#[test]
fn list_changes_ride_along_full_item_payloads_for_updates() {
    // The replica's didUpdateItems needs the FULL item; a second GetFileStatus
    // round-trip per change would triple the polling cost. The change payload
    // carries it for created/modified/reparented rows while the row still
    // exists, and nothing for deletions (or for rows that vanished between
    // the change and the poll — the Swift enumerator skips those).
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        fx.db.upsert_file(&fp_item("fp-ride", "/ride.txt", ItemKind::File)).unwrap();
        fx.db.upsert_file(&fp_item("fp-live", "/live.txt", ItemKind::File)).unwrap();
        fx.db.delete_file("fp-ride").unwrap();
        let reply = send_one(&fx, list_changes_request(None)).await;
        let changes = reply["FileProviderChanges"]["changes"].as_array().unwrap();
        let updated = changes.iter().find(|c| c["kind"] == "created" && c["file_id"] == "fp-live").expect("the live created change");
        let item = updated["item"].as_object().expect("created rows carry the full item payload");
        assert_eq!(item["identifier"], "fp-live");
        assert_eq!(item["status"], "local");
        // A created change whose row has since been deleted rides no item.
        let vanished = changes.iter().find(|c| c["kind"] == "created" && c["file_id"] == "fp-ride").expect("the vanished created change");
        assert!(vanished["item"].is_null(), "a row gone since the change yields no item");
        let deleted = changes.iter().find(|c| c["kind"] == "deleted").expect("the deleted change");
        assert!(deleted["item"].is_null(), "deletions carry no item payload");
    });
}
