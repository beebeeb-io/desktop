//! Ending a session on the server, as sign-out and the sign-in paths need it.

use std::time::Duration;

/// The longest a caller waits for the server to end a session (task 1886: "bounded and best effort"). Signing out
/// without a connection ends this long after the engine stopped, not after the client's 30 s
/// (`auth_transport::client_for`).
pub(crate) const REVOKE_BOUND: Duration = Duration::from_secs(3);

/// End `token`'s session on the server. Best effort, within [`REVOKE_BOUND`]: whatever the server answers, or when it
/// never answers, the caller goes on the same way. Never logs (a request error can carry the URL); keeps no copy of
/// `token`.
pub(crate) async fn revoke(client: &reqwest::Client, base: &str, token: &str) {
    let send = client
        .post(format!("{base}/api/v1/auth/logout"))
        .bearer_auth(token)
        // The request's own time-out replaces the client's 30 s, so the bound holds even when this future is polled
        // late; the outer one ends the wait itself.
        .timeout(REVOKE_BOUND)
        .send();
    let _ = tokio::time::timeout(REVOKE_BOUND, send).await;
}

#[cfg(test)]
#[path = "session_revoke_tests.rs"]
mod tests;
