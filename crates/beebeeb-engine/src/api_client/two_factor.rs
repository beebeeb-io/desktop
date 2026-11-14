//! The two-factor status of the account (`GET /api/v1/me/two-factor`): whether it is on and, when it is, when it was
//! added, when it was last used and how many backup codes are left. One request answers all of it; a value Beebeeb
//! does not give (two-factor off, never used) is `None`, never a made-up one.

use super::*;

/// Beebeeb's own words for the account's two-factor state.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TwoFactorStatus {
    pub enabled: bool,
    /// RFC 3339.
    #[serde(default)]
    pub added_at: Option<String>,
    /// RFC 3339; `None`: not used since it was added.
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub backup_codes_left: Option<u32>,
    #[serde(default)]
    pub backup_codes_total: Option<u32>,
}

impl ApiClient {
    /// `GET /api/v1/me/two-factor`.
    pub async fn two_factor_status(&self) -> anyhow::Result<TwoFactorStatus> {
        Ok(self.get_typed("/api/v1/me/two-factor").await?)
    }
}
