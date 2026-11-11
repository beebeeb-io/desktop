//! Link-monitor tests: every typed GET records how the server answered.

use super::*;

#[test]
fn a_typed_get_that_is_answered_records_an_answer() {
    let base = one_shot_server(Some(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
    ));
    let client = link_client(base);
    let link = client.link();
    run_async(client.get_typed::<serde_json::Value>("/x")).unwrap();
    let snap = link.snapshot();
    assert!(snap.last_ok_at.is_some(), "a 200 is an answer");
    assert_eq!(snap.failure, None);
}

#[test]
fn a_typed_get_answered_with_503_records_the_server_not_answering() {
    let base = one_shot_server(Some(
        "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 4\r\nconnection: close\r\n\r\ndown",
    ));
    let client = link_client(base);
    let link = client.link();
    let err = run_async(client.get_typed::<serde_json::Value>("/x")).unwrap_err();
    let (failure, _) = link.snapshot().failure.expect("a 503 is a link failure");
    assert_eq!(failure.kind, crate::link_health::LinkFailureKind::ServerDidNotAnswer);
    assert_eq!(failure.reason, crate::link_health::LinkReason::Http5xx(503));
    assert_eq!(link.snapshot().last_ok_at, None);
    // The error the caller gets is unchanged (tick code keys on its text).
    assert!(err.to_string().starts_with("HTTP 503"), "{err}");
}

#[test]
fn a_typed_get_answered_with_401_is_an_answer_not_a_link_failure() {
    let base = one_shot_server(Some(
        "HTTP/1.1 401 Unauthorized\r\ncontent-length: 2\r\nconnection: close\r\n\r\nno",
    ));
    let client = link_client(base);
    let link = client.link();
    let err = run_async(client.get_typed::<serde_json::Value>("/x")).unwrap_err();
    assert!(err.to_string().starts_with("HTTP 401"), "{err}");
    assert_eq!(link.snapshot().failure, None, "a 401 means the server answered");
    assert!(link.snapshot().last_ok_at.is_some());
}

#[test]
fn a_typed_get_with_nothing_listening_records_offline() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let client = link_client(format!("http://127.0.0.1:{port}"));
    let link = client.link();
    run_async(client.get_typed::<serde_json::Value>("/x")).unwrap_err();
    let (failure, _) = link.snapshot().failure.expect("connection refused is a link failure");
    assert_eq!(failure.kind, crate::link_health::LinkFailureKind::Offline);
}

#[test]
fn a_failure_then_an_answer_leaves_the_link_up() {
    // Two requests on one client: the second one's answer clears the first one's
    // failure (the latest outcome wins).
    let link_url = one_shot_server(Some(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
    ));
    let client = link_client(link_url);
    let link = client.link();
    link.note_failure(
        crate::link_health::LinkFailure {
            kind: crate::link_health::LinkFailureKind::Offline,
            reason: crate::link_health::LinkReason::Connect,
        },
        1,
    );
    run_async(client.get_typed::<serde_json::Value>("/x")).unwrap();
    assert_eq!(link.snapshot().failure, None);
}
