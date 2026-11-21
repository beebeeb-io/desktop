//! Ending a session on the server, as sign-out and the sign-in paths need it.

use crate::keychain::owed_revocations::OwedRevocations;
use std::sync::Arc;
use std::time::Duration;

/// The longest a caller waits for the server to end a session (task 1886: "bounded and best effort"). Signing out
/// without a connection ends this long after the engine stopped, not after the client's 30 s
/// (`auth_transport::client_for`).
pub(crate) const REVOKE_BOUND: Duration = Duration::from_secs(3);

/// The list of owed revocations this process records into and pays from, set once at engine start
/// ([`install_owed`]). Without it (tests, a platform with no store) a revoke is the bounded try alone.
static OWED: std::sync::OnceLock<Arc<OwedRevocations>> = std::sync::OnceLock::new();

/// Install the process's owed list (the first call wins).
#[cfg_attr(test, allow(dead_code))] // a test build keeps one list per handle (`Engine::open`)
pub(crate) fn install_owed(owed: Arc<OwedRevocations>) {
    let _ = OWED.set(owed);
}

/// One logout request, within [`REVOKE_BOUND`]. `true`: the server ANSWERED for the token (it is ended, or it was
/// never live: any status below 500). A time-out, a failed connection or a 5xx leaves it live: `false`. Never logs
/// (a request error can carry the URL).
async fn send_logout(client: &reqwest::Client, base: &str, token: &str) -> bool {
    let send = client
        .post(format!("{base}/api/v1/auth/logout"))
        .bearer_auth(token)
        // The request's own time-out replaces the client's 30 s, so the bound holds even when this future is polled
        // late; the outer one ends the wait itself.
        .timeout(REVOKE_BOUND)
        .send();
    matches!(
        tokio::time::timeout(REVOKE_BOUND, send).await,
        Ok(Ok(response)) if !response.status().is_server_error()
    )
}

/// End `token`'s session on the server. Best effort, within [`REVOKE_BOUND`]: whatever the server answers, or when it
/// never answers, the caller goes on the same way. When the process has an owed list the token is recorded first and
/// stays there until the server answers (LD-0028). Never logs; keeps no copy of `token`.
pub(crate) async fn revoke(client: &reqwest::Client, base: &str, token: &str) {
    match OWED.get() {
        Some(owed) => revoke_owing(client, base, token, owed).await,
        None => {
            send_logout(client, base, token).await;
        }
    }
}

/// [`revoke`] against an explicit owed list: record, ask, settle on an answer. A list that cannot record (a locked
/// store) leaves the bounded try alone, as before.
pub(crate) async fn revoke_owing(client: &reqwest::Client, base: &str, token: &str, owed: &OwedRevocations) {
    let recorded = owed.owe(token).is_ok();
    if send_logout(client, base, token).await && recorded {
        let _ = owed.settle(token);
    }
}

/// Pay what is owed: one request per token; each the server answered is settled, the rest stays for the next time.
pub(crate) async fn pay_owed(client: &reqwest::Client, base: &str, owed: &OwedRevocations) {
    for token in owed.owed() {
        if send_logout(client, base, &token).await {
            let _ = owed.settle(&token);
        }
    }
}

/// [`revoke`] as a task of its own: the sign-out's result never waits on the network (task 1886, LD-0026 item 5). The
/// token is recorded before this returns, so a process that ends first still owes it. Needs a tokio runtime.
pub(crate) fn revoke_detached(client: reqwest::Client, base: String, token: String, owed: Arc<OwedRevocations>) {
    let _ = owed.owe(&token);
    tokio::spawn(async move { revoke_owing(&client, &base, &token, &owed).await });
}

/// [`pay_owed`] as a task of its own, for the engine start and for the network coming back.
pub(crate) fn pay_owed_detached(client: reqwest::Client, base: String, owed: Arc<OwedRevocations>) {
    tokio::spawn(async move { pay_owed(&client, &base, &owed).await });
}

#[cfg(test)]
#[path = "session_revoke_tests.rs"]
mod tests;
