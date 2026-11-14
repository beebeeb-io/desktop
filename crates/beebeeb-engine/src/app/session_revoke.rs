//! Ending a session on the server, as sign-out and the sign-in paths need it.

/// End `token`'s session on the server. Best effort; never logs.
pub(crate) async fn revoke(client: &reqwest::Client, base: &str, token: &str) {
    let _ = client
        .post(format!("{base}/api/v1/auth/logout"))
        .bearer_auth(token)
        .send()
        .await;
}
