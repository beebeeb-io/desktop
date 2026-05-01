use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_API_URL: &str = "http://localhost:3001";

/// Daemon configuration persisted to disk as JSON.
///
/// On Windows this lives at `%LOCALAPPDATA%\Beebeeb\config.json`.
/// On other platforms it uses the dirs crate's data-local directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Base URL of the Beebeeb API server.
    pub api_url: String,
    /// Bearer session token from login.
    pub session_token: Option<String>,
    /// Email of the logged-in user.
    pub email: Option<String>,
    /// Base64-encoded 32-byte master key derived during login.
    /// Used to derive per-file encryption keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_key: Option<String>,
    /// Local folder to sync. Overridden by CLI arg or env var.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_url: DEFAULT_API_URL.to_string(),
            session_token: None,
            email: None,
            master_key: None,
            sync_path: None,
        }
    }
}

/// Return the path to the daemon config file.
///
/// Windows: `%LOCALAPPDATA%\Beebeeb\config.json`
/// Linux/macOS: `$XDG_DATA_HOME/beebeeb/config.json` (via dirs crate)
fn config_path() -> PathBuf {
    // Try LOCALAPPDATA first (Windows), then fall back to dirs crate.
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Beebeeb")
            .join("config.json");
    }

    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("beebeeb")
        .join("config.json")
}

pub fn load_config() -> Config {
    let path = config_path();
    if !path.exists() {
        return Config::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

#[allow(dead_code)]
pub fn save_config(config: &Config) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create config directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("failed to serialize config: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("failed to write config: {e}"))?;
    Ok(())
}
