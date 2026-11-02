//! Upload tests: refusal classification and chunk response parsing.

use super::*;

    /// Spec §8.4: a refused upload request keeps the server's code by type, matched exactly, and
    /// its text is the status error's alone: the body (its code, its numbers, its message) never
    /// reaches the error's text, which is what a log line or the queue's `last_error` could show.
    #[tokio::test]
    async fn an_upload_refusal_keeps_the_servers_code_and_never_its_body() {
        use std::io::{Read, Write};
        async fn refused(status: &'static str, body: &'static str) -> anyhow::Error {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            });
            let response = reqwest::Client::new().get(&url).send().await.unwrap();
            let error = upload_response(response).await.unwrap_err();
            server.join().unwrap();
            error
        }
        let quota = refused(
            "413 Payload Too Large",
            r#"{"error":"quota_exceeded","limit_bytes":5368709120,"used_bytes":5368709120,"is_trial_cap":false}"#,
        )
        .await;
        let refusal = quota.downcast_ref::<UploadRefused>().expect("typed");
        assert_eq!(refusal.code, Some(UploadRefusalCode::QuotaExceeded));
        assert_eq!(refusal.status(), Some(413));
        let text = quota.to_string();
        assert!(text.contains("413 Payload Too Large"), "{text}");
        assert!(
            !text.contains("quota_exceeded") && !text.contains("5368709120"),
            "the body never reaches the text: {text}"
        );
        assert!(
            quota
                .chain()
                .any(|cause| cause.downcast_ref::<reqwest::Error>().is_some()),
            "the status error stays in the chain"
        );
        // Matched exactly: another spelling, or another code, is no code.
        for body in [
            r#"{"error":"Quota_Exceeded"}"#,
            r#"{"error":"quota exceeded"}"#,
            r#"{"error":"payload_too_large"}"#,
            "not json",
        ] {
            let error = refused("413 Payload Too Large", body).await;
            assert_eq!(
                error.downcast_ref::<UploadRefused>().expect("typed").code,
                None,
                "{body}"
            );
        }
        for (code, expected) in [
            ("object_budget_exceeded", UploadRefusalCode::ObjectBudgetExceeded),
            ("billing_read_only", UploadRefusalCode::BillingReadOnly),
            ("account_lapsed", UploadRefusalCode::AccountLapsed),
            ("plan_required", UploadRefusalCode::PlanRequired),
            ("trial_cancelled_read_only", UploadRefusalCode::TrialCancelledReadOnly),
        ] {
            assert_eq!(UploadRefusalCode::from_code(code), Some(expected));
        }
        // The share gate's `trial_ended` is no upload refusal: no upload route sends it.
        assert_eq!(UploadRefusalCode::from_code("trial_ended"), None);
    }
