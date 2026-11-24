//! The search-index blob transport: the shard manifest and per-shard get,
//! put and delete.
//!
//! Owns: the shard blob cap and the "a shard may be absent" rule — the server
//! stores the blob opaquely, so nothing below this layer reads it. Must not:
//! build the shard URLs (see `client`) or encrypt (the caller owns the key).
//! Tests: `api_client/tests/`.

use super::*;

impl ApiClient {
    /// `GET /api/v1/search-index/shards` — web-compatible shard manifest.
    /// Returns `[]` when the server has no shards or omits the optional wrapper.
    pub async fn fetch_search_shard_manifest(&self) -> anyhow::Result<Vec<SearchShardRef>> {
        let url = self.search_shard_manifest_url();
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        Ok(resp.json::<SearchShardManifest>().await?.shards)
    }

    /// `GET /api/v1/search-index/shards/{bucket}/{page}` — raw encrypted blob.
    /// A 404 races a concurrent delete and is treated the same way web does:
    /// skip the missing shard and let a later rebuild repair the cache.
    pub async fn get_search_shard(&self, bucket: u32, page: u32) -> anyhow::Result<Option<Vec<u8>>> {
        let url = self.search_shard_url(bucket, page);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = Self::check_status(resp).await?;
        Ok(Some(resp.bytes().await?.to_vec()))
    }

    /// `PUT /api/v1/search-index/shards/{bucket}/{page}` — raw octet-stream
    /// encrypted shard upload. The body is never JSON.
    pub async fn put_search_shard(
        &self,
        shard: &beebeeb_core::search_index::EncryptedShard,
    ) -> anyhow::Result<SearchShardRef> {
        if shard.blob.is_empty() {
            return Err(DesktopApiError::EmptyShardBlob.into());
        }
        if shard.blob.len() > MAX_SEARCH_SHARD_BLOB_BYTES {
            return Err(DesktopApiError::ShardBlobTooLarge {
                bucket: shard.bucket,
                page: shard.page,
                bytes: shard.blob.len(),
            }
            .into());
        }
        let resp = self
            .client
            .put(self.search_shard_url(shard.bucket, shard.page))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/octet-stream")
            .body(shard.blob.clone())
            .send()
            .await?;
        let resp = Self::check_status(resp).await?;
        Ok(resp.json::<SearchShardRef>().await?)
    }

    /// `DELETE /api/v1/search-index/shards/{bucket}/{page}` — idempotent stale
    /// shard removal. 204 and 404 both mean the coordinate is gone.
    pub async fn delete_search_shard(&self, bucket: u32, page: u32) -> anyhow::Result<()> {
        let resp = self
            .client
            .delete(self.search_shard_url(bucket, page))
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Self::check_status(resp).await?;
        Ok(())
    }
}
