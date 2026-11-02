//! Error-mapping tests: every branch of `check_status` and the text each
//! `DesktopApiError` shows keep the shape callers and logs already read.

use super::*;

/// Serve one request with the given status line and body, and hand back the
/// response exactly as `reqwest` read it — the input `check_status` maps.
async fn answered(status: &'static str, body: String) -> reqwest::Response {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    reqwest::Client::new().get(&url).send().await.unwrap()
}

#[tokio::test]
async fn a_2xx_answer_passes_through() {
    let response = answered("200 OK", "{}".to_string()).await;
    let passed = ApiClient::check_status(response).await.expect("2xx is not an error");
    assert_eq!(passed.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn a_401_answer_becomes_http_status_with_the_body_head() {
    let response = answered("401 Unauthorized", r#"{"error":"token_expired"}"#.to_string()).await;
    let error = ApiClient::check_status(response).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        r#"HTTP 401 Unauthorized: {"error":"token_expired"}"#
    );
    let DesktopApiError::HttpStatus { status, .. } = &error else {
        panic!("a 401 is a plain status error, got {error:?}");
    };
    assert_eq!(*status, reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_500_answer_keeps_its_status_and_trims_the_body_to_300_chars() {
    // A big body must not reach a log line or a view: the snippet is capped,
    // not the status.
    let body = "x".repeat(400);
    let response = answered("500 Internal Server Error", body).await;
    let error = ApiClient::check_status(response).await.unwrap_err();
    let DesktopApiError::HttpStatus { status, snippet } = &error else {
        panic!("a 500 is a plain status error, got {error:?}");
    };
    assert_eq!(*status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(snippet.chars().count(), 300);
    assert!(error.to_string().starts_with("HTTP 500 Internal Server Error: xxx"));
}

#[tokio::test]
async fn a_409_upgrade_required_is_the_typed_refusal() {
    let response = answered("409 Conflict", r#"{"error":"upgrade_required"}"#.to_string()).await;
    let error = ApiClient::check_status(response).await.unwrap_err();
    assert!(matches!(error, DesktopApiError::UpgradeRequired(_)));
    // The text older call sites read is unchanged.
    assert_eq!(
        error.to_string(),
        "the owner must upgrade this folder before its keys can be shared here"
    );
    // And the state is recognised on the typed error as well as through anyhow.
    assert!(upgrade_required(&error));
}

#[tokio::test]
async fn a_409_with_another_body_is_a_plain_conflict() {
    let response = answered("409 Conflict", r#"{"error":"key_version_conflict"}"#.to_string()).await;
    let error = ApiClient::check_status(response).await.unwrap_err();
    let DesktopApiError::HttpStatus { status, .. } = &error else {
        panic!("a conflict with another body is a plain status error, got {error:?}");
    };
    assert_eq!(*status, reqwest::StatusCode::CONFLICT);
    assert!(!upgrade_required(&error));
}

#[tokio::test]
async fn a_request_with_no_answer_is_a_transport_error_with_its_cause() {
    let error = reqwest::Client::new()
        .get("http://127.0.0.1:1/")
        .send()
        .await
        .expect_err("nothing listens on port 1");
    let error = DesktopApiError::from(error);
    assert!(matches!(error, DesktopApiError::Transport(_)));
    assert!(!error.to_string().is_empty());
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn an_init_conflict_keeps_its_class_text_and_its_cause() {
    let error = DesktopApiError::InitConflict(InitConflict {
        class: InitConflictClass::StaleBase,
        refusal: None,
    });
    assert_eq!(error.to_string(), "upload init refused with 409 Conflict (stale_base)");
    assert!(matches!(
        std::error::Error::source(&error).and_then(|cause| cause.downcast_ref::<InitConflict>()),
        Some(conflict) if conflict.class == InitConflictClass::StaleBase
    ));
}

#[test]
fn a_shard_error_names_the_shard_it_refused() {
    assert_eq!(
        DesktopApiError::EmptyShardBlob.to_string(),
        "search shard blob must not be empty"
    );
    assert_eq!(
        DesktopApiError::ShardBlobTooLarge {
            bucket: 3,
            page: 7,
            bytes: 200_000
        }
        .to_string(),
        "search shard 3/7 blob 200000B exceeds cap"
    );
}

#[test]
fn a_listing_that_outgrows_its_cap_names_the_listing() {
    assert_eq!(
        DesktopApiError::ListingTooLarge {
            listing: Listing::SharedFolder,
            cap: 50_000
        }
        .to_string(),
        "shared folder listing exceeded hard cap of 50000"
    );
    assert_eq!(
        DesktopApiError::ListingTooLarge {
            listing: Listing::Trashed,
            cap: 50_000
        }
        .to_string(),
        "trashed file listing exceeded hard cap of 50000"
    );
}

#[test]
fn an_adoption_answer_without_a_boolean_is_its_own_failure() {
    assert_eq!(
        DesktopApiError::InvalidAdoptionResponse.to_string(),
        "invalid adoption response"
    );
}
