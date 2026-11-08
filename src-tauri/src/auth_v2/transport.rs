//! Sends one core `auth_flow` request over reqwest and hands the answer back. The base URL, the session token and the
//! confirm token stay here, in Rust.
use beebeeb_core::auth_flow::{HttpRequest, HttpResponse, Method, RequestAuth};

/// A network failure (no answer at all): the UI's offline message.
pub(crate) const OFFLINE: &str = "offline";

pub(crate) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        // A stuck request ends after 30 s with the offline message (DW-A18).
        .timeout(std::time::Duration::from_secs(30))
        .default_headers(crate::api_client::provenance_headers())
        .build()
        .map_err(|_| OFFLINE.to_string())
}

/// Send `req`; `session` is put on a [`RequestAuth::Session`] request. A body that is not JSON reads as `Null`; the
/// body is never logged or passed on.
pub(crate) async fn send(
    client: &reqwest::Client,
    session: Option<&str>,
    req: HttpRequest,
) -> Result<HttpResponse, String> {
    let url = format!("{}{}", crate::runner::api_base_url(), req.path);
    let mut builder = match req.method {
        Method::Get => client.get(&url),
        Method::Post => client.post(&url),
        Method::Delete => client.delete(&url),
        Method::Put => client.put(&url),
    };
    if req.auth == RequestAuth::Session {
        let token = session.ok_or_else(|| "locked".to_string())?;
        builder = builder.bearer_auth(token);
    }
    if let Some(confirm) = req.confirm_token.as_ref() {
        builder = builder.header("X-Confirm-Token", confirm.as_str());
    }
    if let Some(body) = req.body.as_ref() {
        builder = builder.json(body);
    }
    let resp = builder.send().await.map_err(|_| OFFLINE.to_string())?;
    let status = resp.status().as_u16();
    let retry_after_secs = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let text = zeroize::Zeroizing::new(resp.text().await.unwrap_or_default());
    Ok(HttpResponse::from_text(status, &text, retry_after_secs))
}
