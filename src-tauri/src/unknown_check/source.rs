//! The check's only network adapter. Every metadata, key, history and chunk request
//! passes through the same account ledger; the normal hydration transport is untouched.
use super::{
    compare::{Halt, HttpFailure, Source},
    job::Scope,
};
use crate::download_pin::PinnedVersion;
use zeroize::Zeroizing;

pub(super) struct Remote<'a> {
    pub scope: &'a Scope,
    pub id: &'a str,
    key: Option<beebeeb_core::kdf::FileKey>,
    reserved: u64,
    used: u64,
    limit: u64,
}
impl<'a> Remote<'a> {
    pub fn new(scope: &'a Scope, id: &'a str) -> Self {
        Self {
            scope,
            id,
            key: None,
            reserved: 0,
            used: 0,
            limit: 0,
        }
    }
    fn notice<T>(&self, result: anyhow::Result<T>) -> anyhow::Result<T> {
        if let Err(e) = &result
            && let Some(HttpFailure(429, retry)) = e.downcast_ref::<HttpFailure>()
        {
            self.scope.commit(|| {
                self.scope
                    .bridge
                    .db()
                    .update_unknown_budget(|l| l.rate_limited(chrono::Utc::now().timestamp_millis(), *retry, 0))
            })?;
        }
        result
    }
}
impl Source for Remote<'_> {
    async fn metadata(&mut self, version: Option<&str>) -> anyhow::Result<serde_json::Value> {
        let mut url = self.scope.bridge.api().file_url(self.id);
        if let Some(id) = version {
            url.push_str(&format!("?object_version_id={}", urlencoding::encode(id)));
        }
        self.notice(self.scope.json(&url).await)
    }
    async fn history(&mut self) -> anyhow::Result<serde_json::Value> {
        self.notice(self.scope.json(&self.scope.bridge.api().versions_url(self.id)).await)
    }
    async fn begin_candidate(&mut self, hash_mode: bool, wire: u64) -> anyhow::Result<()> {
        if hash_mode && wire > 0 {
            self.limit = self.scope.before(wire, true).await?;
            self.reserved = wire;
            self.used = 0;
        }
        Ok(())
    }
    fn finish_candidate(&mut self) -> anyhow::Result<()> {
        if self.reserved > 0 {
            self.scope.commit(|| {
                self.scope
                    .bridge
                    .db()
                    .update_unknown_budget(|l| l.settle(self.reserved, self.used))
            })?;
            self.reserved = 0;
            self.used = 0;
        }
        Ok(())
    }
    async fn chunk(
        &mut self,
        index: u32,
        pin: Option<&PinnedVersion>,
        max_wire: u64,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        if self.key.is_none() {
            self.key = Some(self.notice(self.scope.bridge.file_key_for_check(self.id, Some(self.scope)).await)?);
        }
        if self.reserved > 0 && super::budget::allowance(super::cost::unmetered()) < self.limit {
            return Err(Halt::Daily.into());
        }
        self.scope
            .before(if self.reserved > 0 { 0 } else { max_wire }, false)
            .await?;
        let result = self
            .scope
            .request(
                self.scope
                    .bridge
                    .unknown_chunk(self.id, index, self.key.as_ref().unwrap(), pin, max_wire),
            )
            .await;
        // A failed request may have transferred its full bounded body: charge the reservation.
        let used = result.as_ref().map(|(_, n)| *n).unwrap_or(max_wire);
        if self.reserved > 0 {
            self.used = self.used.saturating_add(used);
        } else {
            self.scope.commit(|| {
                self.scope
                    .bridge
                    .db()
                    .update_unknown_budget(|l| l.settle(max_wire, used))
            })?;
        }
        self.notice(result).map(|(plain, _)| plain)
    }
}
