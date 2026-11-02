//! Wire framing of the daemon IPC socket: replies stay delimited, split and
//! unframed requests still parse, and protocol errors never kill the connection.

use super::*;

#[test]
fn a_reply_larger_than_64_kib_arrives_intact_and_delimited() {
    let fx = IpcFixture::start(|db| seed_many_top_level_files(db, 400));
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(b"{\"ListFileProviderItems\":{\"container_id\":\"__fp_root__\"}}\n")
            .await
            .unwrap();
        let line = read_line(&mut client).await;
        assert!(
            line.len() > 65536,
            "fixture must exceed one 64 KiB read to prove anything (got {} bytes)",
            line.len()
        );
        let reply = parse(&line);
        let items = reply["FileProviderItems"]["items"].as_array().expect("items array");
        assert_eq!(items.len(), 400, "every seeded item must be in the reply");
    });
}

#[test]
fn a_request_split_across_two_writes_is_parsed_as_one_request() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client.write_all(b"{\"ListFileProviderItems\":{\"contai").await.unwrap();
        client.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        client.write_all(b"ner_id\":\"__fp_root__\"}}\n").await.unwrap();
        let reply = parse(&read_line(&mut client).await);
        let items = reply["FileProviderItems"]["items"]
            .as_array()
            .unwrap_or_else(|| panic!("expected FileProviderItems, got {reply}"));
        assert_eq!(
            items.len(),
            0,
            "the root container enumerates the REAL vault tree (1701): an empty vault lists nothing — the four synthetic namespaces are gone"
        );
    });
}

#[test]
fn several_requests_on_one_connection_each_get_exactly_one_delimited_reply() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let client = fx.connect().await;
        let mut lines = LineReader::new(client);
        // Three requests in ONE write: the pre-fix daemon parsed the whole
        // buffer as a single request and failed.
        let batch = b"\"GetSyncSummary\"\n\
            {\"ListFileProviderItems\":{\"container_id\":\"__fp_root__\"}}\n\
            {\"GetFileStatus\":{\"file_id\":\"does-not-exist\"}}\n";
        lines.client.write_all(batch).await.unwrap();

        let first = parse(&lines.next_line().await);
        assert!(first.get("SyncSummary").is_some(), "reply 1 must answer GetSyncSummary: {first}");
        let second = parse(&lines.next_line().await);
        assert!(
            second.get("FileProviderItems").is_some(),
            "reply 2 must answer ListFileProviderItems: {second}"
        );
        let third = parse(&lines.next_line().await);
        assert_eq!(third["Error"]["message"], "not found", "reply 3 must answer GetFileStatus: {third}");
        assert!(
            lines.stays_quiet_for(Duration::from_millis(300)).await,
            "exactly one reply per request: nothing more may follow"
        );
    });
}

#[test]
fn an_unframed_request_from_an_old_extension_is_still_answered() {
    // The 0.8.6 extension wrote its request in one write() with no delimiter
    // and then waited for the reply without closing. The daemon must answer
    // without waiting for a newline or EOF.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(br#"{"ListFileProviderItems":{"container_id":"__fp_root__"}}"#)
            .await
            .unwrap();
        let reply = parse(&read_line(&mut client).await);
        assert!(reply.get("FileProviderItems").is_some(), "got {reply}");
    });
}

#[test]
fn a_malformed_line_gets_an_error_and_the_connection_keeps_working() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let client = fx.connect().await;
        let mut lines = LineReader::new(client);
        lines.client.write_all(b"{this is not json}\n").await.unwrap();
        let err = parse(&lines.next_line().await);
        assert!(err["Error"]["message"].is_string(), "got {err}");
        lines.client.write_all(b"\"GetSyncSummary\"\n").await.unwrap();
        let ok = parse(&lines.next_line().await);
        assert!(ok.get("SyncSummary").is_some(), "connection must survive a bad line: {ok}");
    });
}

#[test]
fn an_oversized_request_is_refused_with_an_error_then_the_connection_closes() {
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        // 2 MiB with no newline and never valid JSON.
        let junk = vec![b'x'; 2 * 1024 * 1024];
        // The daemon may close mid-write once it has seen enough; ignore that.
        let _ = client.write_all(&junk).await;
        let reply = parse(&read_line(&mut client).await);
        let message = reply["Error"]["message"].as_str().unwrap_or_default().to_string();
        assert!(message.contains("limit"), "expected a size-limit error, got {reply}");
    });
}

#[test]
fn success_is_an_object_on_the_wire_not_a_bare_string() {
    // JSONSerialization on the Swift side rejects a top-level string, which is
    // what turned every SUCCESSFUL hydrate into "daemon response was not valid
    // JSON". Pin the shape both in isolation and over the socket. (The probe
    // was `SetFileStatus` until task 1699 retired it; `ReportMaterialized`
    // with an empty list is the remaining `Ok {}` RPC.)
    assert_eq!(
        serde_json::to_string(&crate::ipc_socket::IpcResponse::Ok {}).unwrap(),
        r#"{"Ok":{}}"#
    );
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client.write_all(b"{\"ReportMaterialized\":{\"container_ids\":[]}}\n").await.unwrap();
        let line = read_line(&mut client).await;
        assert_eq!(String::from_utf8(line).unwrap(), r#"{"Ok":{}}"#);
    });
}

#[test]
fn a_rejected_write_keeps_the_connection_open_for_the_next_request() {
    // `QueueFinderCreate` into "Shared with me" used to `return` from the
    // connection handler after writing its error, silently dropping the peer.
    let fx = IpcFixture::start(|_| {});
    fx.rt.block_on(async {
        let client = fx.connect().await;
        let mut lines = LineReader::new(client);
        lines
            .client
            .write_all(
                b"{\"QueueFinderCreate\":{\"parent_id\":\"namespace:shared_with_me\",\"filename\":\"a.txt\",\"kind\":\"file\",\"contents_path\":null,\"content_type\":null}}\n",
            )
            .await
            .unwrap();
        let err = parse(&lines.next_line().await);
        assert!(err["Error"]["message"].as_str().unwrap_or_default().contains("read-only"), "got {err}");
        lines.client.write_all(b"\"GetSyncSummary\"\n").await.unwrap();
        let next = parse(&lines.next_line().await);
        assert!(next.get("SyncSummary").is_some(), "connection must still be usable: {next}");
    });
}
