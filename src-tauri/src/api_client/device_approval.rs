//! Device-link relay calls for approving a sign-in from this (already signed-in) desktop with the typed code.
//! Paths mirror `beebeeb-api` `proposed.rs`; the crypto lives in `beebeeb_core::device_link`.

use serde::{Deserialize, Serialize};

use super::ApiClient;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PendingLink {
    pub id: String,
    pub device_name: String,
    pub expires_at: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct PendingLinks {
    #[serde(default)]
    links: Vec<PendingLink>,
}

#[derive(Debug, Deserialize)]
pub struct LinkDetail {
    pub session_id: String,
    pub device_name: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct PakeResult {
    status: String,
    #[serde(default)]
    message: Option<String>,
}

fn enc(id: &str) -> String {
    id.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl ApiClient {
    fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.header("Authorization", format!("Bearer {}", self.token))
    }

    pub async fn pending_device_links(&self) -> anyhow::Result<Vec<PendingLink>> {
        Ok(self.get_typed::<PendingLinks>("/api/v1/devices/links/pending").await?.links)
    }

    pub async fn device_link_detail(&self, id: &str) -> anyhow::Result<LinkDetail> {
        Ok(self.get_typed(&format!("/api/v1/devices/link/{}", enc(id))).await?)
    }

    pub async fn send_link_pake_approver(&self, id: &str, message_b64: &str) -> anyhow::Result<()> {
        let url = self.url(&format!("/api/v1/devices/link/{}/pake", enc(id)));
        let body = serde_json::json!({ "role": "approver", "message": message_b64 });
        let resp = self.authed(self.client.post(&url)).json(&body).send().await?;
        Self::check_status(resp).await?;
        Ok(())
    }

    /// `Some(base64 message)` once the new device answered, `None` while pending.
    pub async fn get_link_pake_new_device(&self, id: &str) -> anyhow::Result<Option<String>> {
        let r: PakeResult = self
            .get_typed(&format!("/api/v1/devices/link/{}/pake?role=new_device", enc(id)))
            .await?;
        match r.status.as_str() {
            "pending" => Ok(None),
            "ready" => Ok(Some(r.message.ok_or_else(|| anyhow::anyhow!("empty answer"))?)),
            _ => Err(anyhow::anyhow!("link is no longer pending")),
        }
    }

    pub async fn approve_link(&self, id: &str, nonce_b64: &str, ciphertext_b64: &str) -> anyhow::Result<()> {
        let url = self.url(&format!("/api/v1/devices/link/{}/approve", enc(id)));
        let body = serde_json::json!({ "nonce": nonce_b64, "ciphertext": ciphertext_b64 });
        let resp = self.authed(self.client.post(&url)).json(&body).send().await?;
        Self::check_status(resp).await?;
        Ok(())
    }

    pub async fn deny_link(&self, id: &str) -> anyhow::Result<()> {
        let url = self.url(&format!("/api/v1/devices/link/{}/deny", enc(id)));
        let resp = self.authed(self.client.post(&url)).send().await?;
        Self::check_status(resp).await?;
        Ok(())
    }
}
