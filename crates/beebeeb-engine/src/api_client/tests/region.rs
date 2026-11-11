//! `set_preferred_region` tests, driven by the facade's `region_set_fixture`.

use super::*;

#[test]
fn set_preferred_region_puts_region_and_parses_response() {
    let server = RegionMockServer::start(vec![(
        "200 OK".to_string(),
        serde_json::json!({ "preferred_region": "de", "region_strict": true }),
    )]);
    let client = ApiClient::new(server.base_url.clone(), "tok".into(), [0u8; 32]);

    let result = run_async(client.set_preferred_region(Some("de"), Some(true))).unwrap();

    assert_eq!(result.preferred_region.as_deref(), Some("de"));
    assert!(result.region_strict);
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].path, "/api/v1/me/region");
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer tok"));
    let body: serde_json::Value = serde_json::from_slice(requests[0].body.as_bytes()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({ "preferred_region": "de", "region_strict": true })
    );
}

#[test]
fn set_preferred_region_leaves_strict_out_when_the_caller_does_not_state_it() {
    let server = RegionMockServer::start(vec![(
        "200 OK".to_string(),
        serde_json::json!({ "preferred_region": "europe", "region_strict": false }),
    )]);
    let client = ApiClient::new(server.base_url.clone(), "tok".into(), [0u8; 32]);

    let result = run_async(client.set_preferred_region(Some("europe"), None)).unwrap();

    assert!(!result.region_strict);
    let requests = server.finish();
    let body: serde_json::Value = serde_json::from_slice(requests[0].body.as_bytes()).unwrap();
    assert_eq!(body, serde_json::json!({ "preferred_region": "europe" }));
}

#[test]
fn set_preferred_region_surfaces_server_error_for_frontend_rollback() {
    let server = RegionMockServer::start(vec![(
        "400 Bad Request".to_string(),
        serde_json::json!({
            "error": "region_unavailable",
            "continent": "mars"
        }),
    )]);
    let client = ApiClient::new(server.base_url.clone(), "tok".into(), [0u8; 32]);

    let err = run_async(client.set_preferred_region(Some("mars"), None))
        .unwrap_err()
        .to_string();

    assert!(err.contains("400 Bad Request"));
    assert!(err.contains("region_unavailable"));
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(requests[0].body.as_bytes()).unwrap();
    assert_eq!(body, serde_json::json!({ "preferred_region": "mars" }));
}
