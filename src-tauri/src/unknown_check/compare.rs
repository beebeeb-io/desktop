//! Comparison only: no baseline write and no destructive operation.
use crate::download_pin::PinnedVersion;
use crate::state_db::UnknownCheck;
use serde_json::Value;
use std::path::Path;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Halt {
    Paused,
    Daily,
    Offline,
}
impl std::fmt::Display for Halt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("comparison deferred")
    }
}
impl std::error::Error for Halt {}

#[derive(Debug, Clone, PartialEq)]
pub enum Comparison {
    Matched {
        version: i64,
        hash: String,
        hash_mode: bool,
    },
    NoMatch,
    Undecided,
    FileGone,
}

pub(super) trait Source {
    async fn begin_candidate(&mut self, _hash_mode: bool, _wire: u64) -> anyhow::Result<()> {
        Ok(())
    }
    fn finish_candidate(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn metadata(&mut self, version: Option<&str>) -> anyhow::Result<Value>;
    async fn history(&mut self) -> anyhow::Result<Value>;
    async fn chunk(
        &mut self,
        index: u32,
        pin: Option<&PinnedVersion>,
        max_wire: u64,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>>;
}

/// Only requests control-flow errors leave the candidate loop. A decrypt/manifest failure
/// makes the result undecided, even if every other candidate differs.
#[derive(Debug)]
pub(crate) struct HttpFailure(pub u16, pub u64);
impl std::fmt::Display for HttpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "check request status {}", self.0)
    }
}
impl std::error::Error for HttpFailure {}

pub(super) async fn compare<S: Source>(
    source: &mut S,
    record: &mut UnknownCheck,
    version: i64,
    path: &Path,
) -> anyhow::Result<Comparison> {
    let current = match source.metadata(None).await {
        Err(e) if e.downcast_ref::<HttpFailure>().is_some_and(|e| e.0 == 404) => return Ok(Comparison::FileGone),
        result => result?,
    };
    let (size, mtime) = super::record::stamp(path)?;
    let hash_mode = record.recorded_size != Some(size) || record.recorded_mtime != Some(mtime);
    let target_size = record
        .recorded_size
        .ok_or_else(|| anyhow::anyhow!("missing recording"))?;
    let mut undecided = false;
    match candidate(source, record, &current, version, path, hash_mode).await? {
        r @ Comparison::Matched { .. } => return Ok(r),
        Comparison::Undecided => undecided = true,
        _ => {}
    }
    // Missing field means an old server; null means a legacy current on a pin-aware server.
    if current.get("download_version").is_some() {
        let history = match source.history().await {
            Err(e) if e.downcast_ref::<HttpFailure>().is_some_and(|e| e.0 == 404) => Value::Null,
            result => result?,
        };
        let current_id = current["download_version"]["object_version_id"].as_str();
        let mut versions: Vec<_> = history["versions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| {
                v["object_version_id"].as_str().is_some_and(|id| Some(id) != current_id)
                    && v["size_bytes"]
                        .as_i64()
                        .zip(v["chunk_count"].as_i64())
                        .is_some_and(|(s, n)| n >= 0 && s.checked_sub(n.saturating_mul(28)) == Some(target_size))
            })
            .collect();
        versions.sort_by_key(|v| std::cmp::Reverse(v["version_number"].as_i64().unwrap_or(0)));
        let mut seen = std::collections::HashSet::new();
        for v in versions
            .into_iter()
            .filter(|v| seen.insert(v["object_version_id"].as_str().unwrap()))
            .take(9)
        {
            let id = v["object_version_id"].as_str().unwrap();
            let meta = match source.metadata(Some(id)).await {
                Err(e) if e.downcast_ref::<HttpFailure>().is_some_and(|e| e.0 == 409) => continue,
                result => result?,
            };
            if meta["download_version"]["object_version_id"].as_str() != Some(id) {
                break;
            }
            match candidate(
                source,
                record,
                &meta,
                v["version_number"].as_i64().unwrap_or(0),
                path,
                hash_mode,
            )
            .await?
            {
                r @ Comparison::Matched { .. } => return Ok(r),
                Comparison::Undecided => undecided = true,
                _ => {}
            }
        }
    }
    Ok(if undecided {
        Comparison::Undecided
    } else {
        Comparison::NoMatch
    })
}

// V1 may carry base64 JSON, while v2 is a raw nonce/ciphertext/tag frame.
fn legacy_wire_bound() -> u64 {
    (beebeeb_core::chunk_stream::MAX_CHUNK_SIZE + 28)
        .saturating_mul(4)
        .div_ceil(3)
        + 4096
}

async fn candidate<S: Source>(
    source: &mut S,
    record: &mut UnknownCheck,
    meta: &Value,
    fallback_version: i64,
    path: &Path,
    hash_mode: bool,
) -> anyhow::Result<Comparison> {
    let result = candidate_inner(source, record, meta, fallback_version, path, hash_mode).await;
    let finished = source.finish_candidate();
    if result.is_ok() {
        finished?;
    }
    result
}

async fn candidate_inner<S: Source>(
    source: &mut S,
    record: &mut UnknownCheck,
    meta: &Value,
    fallback_version: i64,
    path: &Path,
    hash_mode: bool,
) -> anyhow::Result<Comparison> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    let dv = &meta["download_version"];
    let version = dv["version_number"].as_i64().unwrap_or(fallback_version);
    let empty = dv["chunk_count"].as_u64() == Some(0)
        && dv["plaintext_size_bytes"].as_u64() == Some(0)
        && dv["ciphertext_size_bytes"].as_u64() == Some(0)
        && dv["chunks"].as_array().is_some_and(Vec::is_empty)
        && dv["object_version_id"]
            .as_str()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok());
    let pin = if empty {
        None
    } else {
        match PinnedVersion::from_metadata(meta) {
            Ok(p) => p,
            Err(_) => return Ok(Comparison::Undecided),
        }
    };
    let count = if empty {
        0
    } else if let Some(p) = &pin {
        p.chunk_count
    } else {
        match meta["chunk_count"].as_u64().and_then(|v| u32::try_from(v).ok()) {
            Some(n) => n,
            None => return Ok(Comparison::Undecided),
        }
    };
    if (empty || pin.is_some()) && dv["plaintext_size_bytes"].as_i64() != record.recorded_size {
        return Ok(Comparison::NoMatch);
    }
    let mut local = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let candidate_id = pin
        .as_ref()
        .map(|p| p.object_version_id.clone())
        .unwrap_or_else(|| format!("current:{version}"));
    // Persist lengths with the chained digest: legacy manifests do not provide a chunk
    // stride. Never guess one from size/count when validating a resumed prefix.
    let mut lengths: Vec<u64> = Vec::new();
    let mut chain: Vec<u8> = Vec::new();
    let mut first = 0;
    if !hash_mode
        && record.resume_candidate.as_deref() == Some(&candidate_id)
        && let Some(saved) = record
            .resume_digest
            .as_deref()
            .and_then(|s| serde_json::from_str::<(Vec<u64>, Vec<u8>)>(s).ok())
        && record.resume_chunk == Some(saved.0.len() as i64)
        && saved.0.len() <= count as usize
    {
        let mut valid = true;
        for len in &saved.0 {
            if *len > beebeeb_core::chunk_stream::MAX_CHUNK_SIZE {
                valid = false;
                break;
            }
            let mut bytes = Zeroizing::new(vec![0; *len as usize]);
            if local.read_exact(&mut bytes).is_err() {
                valid = false;
                break;
            }
            hash.update(&*bytes);
            let mut h = Sha256::new();
            h.update(&chain);
            h.update(&*bytes);
            chain = h.finalize().to_vec();
            total += *len;
        }
        if valid && chain == saved.1 {
            first = saved.0.len() as u32;
            lengths = saved.0;
        } else {
            local.seek(SeekFrom::Start(0))?;
            hash = Sha256::new();
            chain.clear();
            total = 0;
        }
    }
    let wire = pin
        .as_ref()
        .map(|p| p.ciphertext_size_bytes)
        .unwrap_or((count as u64).saturating_mul(legacy_wire_bound()));
    source.begin_candidate(hash_mode, wire).await?;
    for i in first..count {
        let max_wire = pin.as_ref().and_then(|p| p.wire_size(i)).unwrap_or(legacy_wire_bound());
        let plain = match source.chunk(i, pin.as_ref(), max_wire).await {
            Err(e) if e.downcast_ref::<HttpFailure>().is_some_and(|e| e.0 == 409) => return Ok(Comparison::NoMatch),
            Err(e) if e.downcast_ref::<Halt>().is_some() || e.downcast_ref::<HttpFailure>().is_some() => {
                if !hash_mode {
                    record.resume_candidate = Some(candidate_id.clone());
                    record.resume_chunk = Some(i as i64);
                    record.resume_digest = Some(serde_json::to_string(&(lengths, chain))?);
                }
                return Err(e);
            }
            Err(_) => return Ok(Comparison::Undecided),
            Ok(p) => p,
        };
        if let Some(p) = &pin
            && p.plaintext_size_of(i..i + 1) != Some(plain.len() as u64)
        {
            return Ok(Comparison::Undecided);
        }
        if !hash_mode {
            let mut bytes = Zeroizing::new(vec![0; plain.len()]);
            if local.read_exact(&mut bytes).is_err() || *bytes != *plain {
                return Ok(Comparison::NoMatch);
            }
            hash.update(&*bytes);
            let mut h = Sha256::new();
            h.update(&chain);
            h.update(&*bytes);
            chain = h.finalize().to_vec();
            lengths.push(bytes.len() as u64);
        } else {
            hash.update(&*plain);
        }
        total = total.saturating_add(plain.len() as u64);
    }
    if !hash_mode && local.read(&mut [0u8; 1])? != 0 {
        return Ok(Comparison::NoMatch);
    }
    let hash = format!("{:x}", hash.finalize());
    if Some(hash.as_str()) != record.recorded_hash.as_deref() || Some(total as i64) != record.recorded_size {
        return Ok(if hash_mode {
            Comparison::NoMatch
        } else {
            Comparison::Undecided
        });
    }
    if version <= 0 {
        return Ok(Comparison::Undecided);
    }
    Ok(Comparison::Matched {
        version,
        hash,
        hash_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    struct Mock {
        bodies: Vec<Vec<Vec<u8>>>,
        calls: Vec<String>,
        legacy: bool,
        old: bool,
        corrupt: bool,
        version: i64,
        stop_at: Option<u32>,
    }
    impl Source for Mock {
        async fn metadata(&mut self, version: Option<&str>) -> anyhow::Result<Value> {
            self.calls.push(format!("meta:{}", version.unwrap_or("current")));
            let n = if version.is_some() { 1 } else { 0 };
            let mut value = json!({"chunk_count":self.bodies[n].len(), "chunk_size_bytes":4});
            if !self.old {
                value["download_version"] = if self.legacy {
                    Value::Null
                } else {
                    let sizes: Vec<_> = self.bodies[n].iter().map(|b| b.len() as u64).collect();
                    let mut m = crate::download_pin::tests::manifest(
                        if n == 0 {
                            "11111111-1111-4111-8111-111111111111"
                        } else {
                            "22222222-2222-4222-8222-222222222222"
                        },
                        4,
                        &sizes,
                    );
                    m["version_number"] = json!(if n == 0 { self.version } else { 1 });
                    m
                };
            }
            Ok(value)
        }
        async fn history(&mut self) -> anyhow::Result<Value> {
            self.calls.push("history".into());
            Ok(
                json!({"versions":[{"object_version_id":"22222222-2222-4222-8222-222222222222","version_number":1,
                "size_bytes":self.bodies[1].iter().map(|b|b.len()+28).sum::<usize>(),"chunk_count":self.bodies[1].len()}]}),
            )
        }
        async fn chunk(
            &mut self,
            index: u32,
            pin: Option<&PinnedVersion>,
            _max_wire: u64,
        ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
            if self.stop_at == Some(index) {
                return Err(Halt::Daily.into());
            }
            let n = usize::from(pin.is_some_and(|p| p.object_version_id.starts_with('2')));
            self.calls.push(format!("chunk:{n}:{index}"));
            if self.corrupt && n == 0 {
                anyhow::bail!("decrypt");
            }
            Ok(Zeroizing::new(self.bodies[n][index as usize].clone()))
        }
    }
    fn setup(bytes: &[u8]) -> (tempfile::TempDir, UnknownCheck, Mock) {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("f");
        std::fs::write(&path, bytes).unwrap();
        let m = std::fs::metadata(&path).unwrap();
        let r = UnknownCheck {
            recorded_hash: Some(format!("{:x}", Sha256::digest(bytes))),
            recorded_size: Some(bytes.len() as i64),
            recorded_mtime: Some(
                m.modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as i64,
            ),
            ..Default::default()
        };
        let mock = Mock {
            bodies: vec![vec![b"same".to_vec()], vec![b"past".to_vec()]],
            calls: vec![],
            legacy: false,
            old: false,
            corrupt: false,
            version: 2,
            stop_at: None,
        };
        (d, r, mock)
    }
    #[tokio::test]
    async fn task_2191_s2_current_match_requests_zero_histories() {
        let (d, mut r, mut s) = setup(b"same");
        let result = compare(&mut s, &mut r, 2, &d.path().join("f")).await.unwrap();
        assert!(matches!(
            result,
            Comparison::Matched {
                version: 2,
                hash_mode: false,
                ..
            }
        ));
        assert_eq!(s.calls, vec!["meta:current", "chunk:0:0"]);
    }
    #[tokio::test]
    async fn task_2191_s2_history_is_lazy_pinned_and_stops_on_first_difference() {
        let (d, mut r, mut s) = setup(b"past");
        assert!(matches!(
            compare(&mut s, &mut r, 2, &d.path().join("f")).await.unwrap(),
            Comparison::Matched { version: 1, .. }
        ));
        assert_eq!(
            s.calls,
            vec![
                "meta:current",
                "chunk:0:0",
                "history",
                "meta:22222222-2222-4222-8222-222222222222",
                "chunk:1:0"
            ]
        );
    }
    #[tokio::test]
    async fn task_2191_s2_v1_and_old_servers_keep_current_without_size_filter() {
        for old in [false, true] {
            let (d, mut r, mut s) = setup(b"same");
            s.old = old;
            s.legacy = true;
            assert!(matches!(
                compare(&mut s, &mut r, 7, &d.path().join("f")).await.unwrap(),
                Comparison::Matched { version: 7, .. }
            ));
            assert!(!s.calls.contains(&"history".into()));
        }
    }
    #[tokio::test]
    async fn task_2191_s2_decrypt_failure_is_undecided_unless_history_matches() {
        let (d, mut r, mut s) = setup(b"else");
        s.corrupt = true;
        assert_eq!(
            compare(&mut s, &mut r, 2, &d.path().join("f")).await.unwrap(),
            Comparison::Undecided
        );
        let (d, mut r, mut s) = setup(b"past");
        s.corrupt = true;
        assert!(matches!(
            compare(&mut s, &mut r, 2, &d.path().join("f")).await.unwrap(),
            Comparison::Matched { version: 1, .. }
        ));
    }
    #[tokio::test]
    async fn task_2191_s2_zero_version_never_matches_and_zero_bytes_need_no_chunk() {
        let (d, mut r, mut s) = setup(b"");
        s.bodies[0] = vec![];
        assert!(matches!(
            compare(&mut s, &mut r, 2, &d.path().join("f")).await.unwrap(),
            Comparison::Matched { version: 2, .. }
        ));
        assert_eq!(s.calls, vec!["meta:current"]);
        let (d, mut r, mut s) = setup(b"same");
        s.version = 0;
        assert_eq!(
            compare(&mut s, &mut r, 0, &d.path().join("f")).await.unwrap(),
            Comparison::Undecided
        );
    }
    #[tokio::test]
    async fn task_2191_s2_hash_mode_uses_the_recorded_content() {
        let (d, mut r, mut s) = setup(b"past");
        std::fs::write(d.path().join("f"), b"edited since recording").unwrap();
        assert!(matches!(
            compare(&mut s, &mut r, 2, &d.path().join("f")).await.unwrap(),
            Comparison::Matched {
                version: 1,
                hash_mode: true,
                ..
            }
        ));
    }
    #[tokio::test]
    async fn task_2191_s2_all_decided_differences_are_no_match() {
        let (d, mut r, mut s) = setup(b"else");
        assert_eq!(
            compare(&mut s, &mut r, 2, &d.path().join("f")).await.unwrap(),
            Comparison::NoMatch
        );
    }
    #[tokio::test]
    async fn task_2191_s2_resume_rechecks_local_prefix_and_skips_downloaded_chunks() {
        let (d, mut r, mut s) = setup(b"aaaabbbbcccc");
        s.bodies[0] = vec![b"aaaa".to_vec(), b"bbbb".to_vec(), b"cccc".to_vec()];
        s.stop_at = Some(1);
        let path = d.path().join("f");
        assert!(compare(&mut s, &mut r, 2, &path).await.unwrap_err().is::<Halt>());
        assert_eq!(r.resume_chunk, Some(1));
        let checkpoint = r.clone();
        s.stop_at = None;
        s.calls.clear();
        assert!(matches!(
            compare(&mut s, &mut r, 2, &path).await.unwrap(),
            Comparison::Matched { .. }
        ));
        assert!(!s.calls.contains(&"chunk:0:0".into()));
        std::fs::write(&path, b"xxxxbbbbcccc").unwrap();
        r = checkpoint;
        r.recorded_mtime = Some(super::super::record::stamp(&path).unwrap().1);
        s.calls.clear();
        assert!(!matches!(
            compare(&mut s, &mut r, 2, &path).await.unwrap(),
            Comparison::Matched { .. }
        ));
        assert!(s.calls.contains(&"chunk:0:0".into()));
    }
    #[tokio::test]
    async fn task_2191_s2_early_difference_downloads_only_one_of_eight_chunks() {
        let (d, mut r, mut s) = setup(b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
        s.bodies[0] = vec![b"aaaa".to_vec(); 8];
        assert_eq!(
            compare(&mut s, &mut r, 2, &d.path().join("f")).await.unwrap(),
            Comparison::NoMatch
        );
        assert_eq!(s.calls.iter().filter(|c| c.starts_with("chunk:")).count(), 1);
    }
    struct HistoryCase {
        calls: Vec<String>,
        missing: bool,
        lie: bool,
        gone: bool,
    }
    impl Source for HistoryCase {
        async fn metadata(&mut self, id: Option<&str>) -> anyhow::Result<Value> {
            if self.gone {
                return Err(HttpFailure(404, 0).into());
            }
            self.calls.push(id.unwrap_or("current").into());
            let current = "00000000-0000-4000-8000-000000000000";
            let mut manifest =
                crate::download_pin::tests::manifest(if self.lie { current } else { id.unwrap_or(current) }, 4, &[4]);
            manifest["version_number"] = json!(20);
            Ok(json!({"download_version":manifest}))
        }
        async fn history(&mut self) -> anyhow::Result<Value> {
            if self.missing {
                return Err(HttpFailure(404, 0).into());
            }
            let versions: Vec<_> = (0..14)
                .map(|n| {
                    json!({"object_version_id":format!("00000000-0000-4000-8000-{n:012}"),
                "version_number":n,"size_bytes":if n==13 {999} else {32},"chunk_count":1})
                })
                .collect();
            Ok(json!({"versions":versions}))
        }
        async fn chunk(
            &mut self,
            _i: u32,
            _pin: Option<&PinnedVersion>,
            _wire: u64,
        ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
            Ok(Zeroizing::new(b"diff".to_vec()))
        }
    }
    #[tokio::test]
    async fn task_2191_s2_history_caps_ten_prefilters_sizes_and_excludes_current() {
        let (d, mut r, _) = setup(b"same");
        let mut s = HistoryCase {
            calls: vec![],
            missing: false,
            lie: false,
            gone: false,
        };
        assert_eq!(
            compare(&mut s, &mut r, 20, &d.path().join("f")).await.unwrap(),
            Comparison::NoMatch
        );
        assert_eq!(s.calls.len(), 10);
        assert_eq!(s.calls[1], "00000000-0000-4000-8000-000000000012");
        assert!(
            !s.calls
                .iter()
                .skip(1)
                .any(|id| id.ends_with("000000000000") || id.ends_with("000000000013"))
        );
    }
    #[tokio::test]
    async fn task_2191_s2_shared_history_404_is_current_only_but_metadata_404_is_gone() {
        let (d, mut r, _) = setup(b"same");
        let mut s = HistoryCase {
            calls: vec![],
            missing: true,
            lie: false,
            gone: false,
        };
        assert_eq!(
            compare(&mut s, &mut r, 20, &d.path().join("f")).await.unwrap(),
            Comparison::NoMatch
        );
        assert_eq!(s.calls.len(), 1);
        s.gone = true;
        assert_eq!(
            compare(&mut s, &mut r, 20, &d.path().join("f")).await.unwrap(),
            Comparison::FileGone
        );
    }
    #[tokio::test]
    async fn task_2191_s2_server_ignoring_pin_is_never_historical_evidence() {
        let (d, mut r, _) = setup(b"same");
        let mut s = HistoryCase {
            calls: vec![],
            missing: false,
            lie: true,
            gone: false,
        };
        assert_eq!(
            compare(&mut s, &mut r, 20, &d.path().join("f")).await.unwrap(),
            Comparison::NoMatch
        );
        assert_eq!(s.calls.len(), 2);
    }
}
