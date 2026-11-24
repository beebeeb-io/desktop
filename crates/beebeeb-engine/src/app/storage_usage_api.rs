//! Storage use and quota from the server, for the engine handle.

use super::{CredentialStores, Engine};

impl<C: CredentialStores> Engine<C> {
    /// Storage use and quota from the server (`/api/v1/billing/usage`).
    pub fn storage_info(&self) -> Result<(i64, i64), String> {
        let token = self.session_token()?;
        self.runtime.block_on(async {
            #[derive(serde::Deserialize)]
            struct Usage {
                used_bytes: i64,
                quota_bytes: i64,
            }
            let resp = self
                .client
                .get(format!("{}/api/v1/billing/usage", self.config.api_base))
                .bearer_auth(token.as_str())
                .send()
                .await
                .map_err(|_| "offline".to_string())?
                .error_for_status()
                .map_err(|_| "could not read storage".to_string())?;
            let usage: Usage = resp.json().await.map_err(|_| "invalid storage response".to_string())?;
            Ok((usage.used_bytes.max(0), usage.quota_bytes.max(0)))
        })
    }
}
