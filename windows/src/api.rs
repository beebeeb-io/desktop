use reqwest::Client;
use serde_json::Value;

/// HTTP client for the Beebeeb API.
///
/// Adapted from the CLI's `api.rs` — carries the base URL and an optional
/// bearer token for authenticated requests.
pub struct ApiClient {
    client: Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: String, token: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url,
            token,
        }
    }

    pub fn require_auth(&self) -> Result<&str, String> {
        self.token
            .as_deref()
            .ok_or_else(|| "Not logged in. Run `bb login` first.".to_string())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// List files, optionally filtered to a parent folder.
    pub async fn list_files(&self, parent_id: Option<&str>) -> Result<Value, String> {
        let token = self.require_auth()?;
        let mut url = self.url("/api/v1/files");
        if let Some(pid) = parent_id {
            url = format!("{url}?parent_id={pid}");
        }
        let resp = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        parse_response(resp).await
    }

    /// Upload an encrypted file via multipart.
    ///
    /// The server expects:
    /// - A "metadata" text field with JSON (name_encrypted, parent_id, mime_type, size_bytes)
    /// - One or more "chunk_N" binary fields with the encrypted chunk data
    pub async fn upload_encrypted(
        &self,
        metadata_json: &str,
        encrypted_chunks: &[(u32, Vec<u8>)],
    ) -> Result<Value, String> {
        let token = self.require_auth()?;

        let mut form = reqwest::multipart::Form::new()
            .text("metadata", metadata_json.to_string());

        for (idx, data) in encrypted_chunks {
            let part = reqwest::multipart::Part::bytes(data.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| format!("mime error: {e}"))?;
            form = form.part(format!("chunk_{idx}"), part);
        }

        let resp = self
            .client
            .post(self.url("/api/v1/files/upload"))
            .bearer_auth(token)
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        parse_response(resp).await
    }

    /// Create a folder on the server.
    #[allow(dead_code)]
    pub async fn create_folder(
        &self,
        name_encrypted: &str,
        parent_id: Option<uuid::Uuid>,
        folder_id: Option<uuid::Uuid>,
    ) -> Result<Value, String> {
        let token = self.require_auth()?;
        let mut body = serde_json::json!({ "name_encrypted": name_encrypted });
        if let Some(pid) = parent_id {
            body["parent_id"] = serde_json::json!(pid);
        }
        if let Some(fid) = folder_id {
            body["folder_id"] = serde_json::json!(fid);
        }
        let resp = self
            .client
            .post(self.url("/api/v1/files/folder"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        parse_response(resp).await
    }

    /// Verify the session token is still valid by calling /auth/me.
    pub async fn verify_session(&self) -> Result<Value, String> {
        let token = self.require_auth()?;
        let resp = self
            .client
            .get(self.url("/api/v1/auth/me"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        parse_response(resp).await
    }
}

async fn parse_response(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("failed to read response: {e}"))?;

    if !status.is_success() {
        let msg = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
            .unwrap_or_else(|| format!("{status}: {body}"));
        return Err(msg);
    }

    serde_json::from_str(&body).map_err(|e| format!("invalid JSON: {e}"))
}
