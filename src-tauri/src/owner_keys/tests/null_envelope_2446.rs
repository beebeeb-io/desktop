//! Task 2446: a request field the server sends as JSON null is not a carried envelope (`request_envelope`).
//! The snapshot-level proof is `engine_bridge::tests::snapshot_names_2446`.

use super::*;

#[test]
fn only_a_non_null_request_field_counts_as_carried() {
    use requests::request_envelope;
    let all_null =
        serde_json::json!({"file_request_id": null, "sender_ephemeral_pubkey": null, "wrapped_content_key": null});
    assert!(
        request_envelope(&all_null).unwrap().is_none(),
        "three nulls: no envelope"
    );
    assert!(
        request_envelope(&serde_json::json!({})).unwrap().is_none(),
        "three absent: no envelope"
    );
    let mixed = serde_json::json!({"file_request_id": "0f000000-0000-4000-8000-0000000000e2", "sender_ephemeral_pubkey": null, "wrapped_content_key": null});
    assert!(
        request_envelope(&mixed).is_err(),
        "one carried, two null: malformed, still refused"
    );
}
