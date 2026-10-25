//! Task 1643: one download, one immutable version.
//!
//! A file's metadata (`GET /api/v1/files/:id`) carries `download_version`: the id and the exact
//! manifest of the version that is current right now (server contract "Version-bound downloads",
//! `.claude/skills/beebeeb-api.md`). A download pins every chunk request to that id with
//! `?object_version_id=`, so a replacement that completes mid-download cannot slip its chunks in:
//! the server serves the pinned version or answers 409. Every chunk's wire size and the final
//! plaintext size are checked against the manifest, and a 409 means "start again from fresh
//! metadata", never "retry without the pin".
//!
//! A file whose metadata has no `download_version` (a legacy object without a complete v2
//! manifest) keeps today's unpinned per-chunk download: the server says there is no version to pin,
//! and a client must not invent one.

use serde_json::Value;

/// AEAD overhead per chunk on the wire (`nonce(12) + tag(16)`), defined once in core.
const CHUNK_OVERHEAD: u64 = beebeeb_core::chunk_stream::CHUNK_OVERHEAD;

/// The version a download is bound to, with its manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PinnedVersion {
    pub object_version_id: String,
    pub chunk_count: u32,
    /// Plaintext stride: every chunk but the last holds exactly this many plaintext bytes.
    pub chunk_size_bytes: u64,
    pub plaintext_size_bytes: u64,
    pub ciphertext_size_bytes: u64,
    /// Exact wire size (ciphertext + overhead) of chunk `i`, in index order.
    chunk_sizes: Vec<u64>,
}

/// The pinned version is gone (the server answered 409 on a pinned request, or the version a
/// transfer landed on is not the one the metadata named). Nothing of the attempt is kept; the
/// download starts again from fresh metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VersionGone;

impl std::fmt::Display for VersionGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the file changed while it was downloading")
    }
}

impl std::error::Error for VersionGone {}

/// Why a `download_version` could not be used. The download is refused: a manifest the client cannot
/// check is never downloaded against, and never silently replaced by an unpinned download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BadManifest(&'static str);

impl std::fmt::Display for BadManifest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the server's download manifest is not usable: {}", self.0)
    }
}

impl std::error::Error for BadManifest {}

fn field_u64(v: &Value, name: &'static str) -> Result<u64, BadManifest> {
    v.get(name).and_then(Value::as_u64).ok_or(BadManifest(name))
}

impl PinnedVersion {
    /// The pin a download of this file must use, from its metadata. `Ok(None)`: the field is absent
    /// or null (a legacy file, or an older server): download unpinned, as before.
    pub(crate) fn from_metadata(meta: &Value) -> Result<Option<Self>, BadManifest> {
        let dv = match meta.get("download_version") {
            None | Some(Value::Null) => return Ok(None),
            Some(dv) => dv,
        };
        let id = dv
            .get("object_version_id")
            .and_then(Value::as_str)
            .ok_or(BadManifest("object_version_id"))?;
        let object_version_id = uuid::Uuid::parse_str(id)
            .map_err(|_| BadManifest("object_version_id is not a UUID"))?
            .to_string();
        let chunk_count = u32::try_from(field_u64(dv, "chunk_count")?).map_err(|_| BadManifest("chunk_count"))?;
        let chunk_size_bytes = field_u64(dv, "chunk_size_bytes")?;
        let plaintext_size_bytes = field_u64(dv, "plaintext_size_bytes")?;
        let ciphertext_size_bytes = field_u64(dv, "ciphertext_size_bytes")?;
        let chunks = dv.get("chunks").and_then(Value::as_array).ok_or(BadManifest("chunks"))?;
        if chunk_count == 0 || chunks.len() != chunk_count as usize {
            return Err(BadManifest("chunk_count does not match the chunk list"));
        }
        let mut chunk_sizes = Vec::with_capacity(chunks.len());
        let mut wire_total: u64 = 0;
        for (position, chunk) in chunks.iter().enumerate() {
            if field_u64(chunk, "index")? != position as u64 {
                return Err(BadManifest("chunks are not in index order"));
            }
            let size = field_u64(chunk, "size_bytes")?;
            // The geometry every real version has (the server builds the manifest by the same
            // rule): every chunk carries the overhead, every chunk but the last is a full stride,
            // the last is at most one.
            let plaintext = size.checked_sub(CHUNK_OVERHEAD).ok_or(BadManifest("a chunk below the overhead"))?;
            let last = position + 1 == chunks.len();
            if plaintext > chunk_size_bytes || (!last && plaintext != chunk_size_bytes) {
                return Err(BadManifest("a chunk size the stride does not allow"));
            }
            wire_total = wire_total.checked_add(size).ok_or(BadManifest("sizes overflow"))?;
            chunk_sizes.push(size);
        }
        let overhead = CHUNK_OVERHEAD
            .checked_mul(u64::from(chunk_count))
            .ok_or(BadManifest("sizes overflow"))?;
        if wire_total != ciphertext_size_bytes || plaintext_size_bytes.checked_add(overhead) != Some(wire_total) {
            return Err(BadManifest("the totals do not match the chunks"));
        }
        Ok(Some(PinnedVersion {
            object_version_id,
            chunk_count,
            chunk_size_bytes,
            plaintext_size_bytes,
            ciphertext_size_bytes,
            chunk_sizes,
        }))
    }

    /// Exact wire size of chunk `index`, `None` outside the version.
    pub(crate) fn wire_size(&self, index: u32) -> Option<u64> {
        self.chunk_sizes.get(index as usize).copied()
    }

    /// Plaintext bytes chunks `range` decrypt to. (Its caller, the range read, is Windows and tests only.)
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub(crate) fn plaintext_size_of(&self, range: std::ops::Range<u32>) -> Option<u64> {
        range.map(|i| self.wire_size(i).map(|s| s - CHUNK_OVERHEAD)).sum()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// A `download_version` exactly as the server builds it.
    pub(crate) fn manifest(id: &str, stride: u64, plaintext_sizes: &[u64]) -> Value {
        let chunks: Vec<Value> = plaintext_sizes
            .iter()
            .enumerate()
            .map(|(i, p)| json!({"index": i, "size_bytes": p + 28}))
            .collect();
        let plaintext: u64 = plaintext_sizes.iter().sum();
        json!({
            "object_version_id": id,
            "version_number": 1,
            "chunk_count": plaintext_sizes.len(),
            "chunk_size_bytes": stride,
            "plaintext_size_bytes": plaintext,
            "ciphertext_size_bytes": plaintext + 28 * plaintext_sizes.len() as u64,
            "chunks": chunks,
        })
    }

    const ID: &str = "11111111-2222-4333-8444-555555555555";

    #[test]
    fn a_manifest_is_read_with_every_size() {
        let meta = json!({"id": "f", "download_version": manifest(ID, 10, &[10, 10, 3])});
        let pin = PinnedVersion::from_metadata(&meta).unwrap().unwrap();
        assert_eq!(pin.object_version_id, ID);
        assert_eq!((pin.chunk_count, pin.chunk_size_bytes), (3, 10));
        assert_eq!((pin.plaintext_size_bytes, pin.ciphertext_size_bytes), (23, 23 + 84));
        assert_eq!((pin.wire_size(0), pin.wire_size(2), pin.wire_size(3)), (Some(38), Some(31), None));
        assert_eq!(pin.plaintext_size_of(1..3), Some(13));
        assert_eq!(pin.plaintext_size_of(0..4), None, "outside the version");
        // The empty file: one chunk of overhead only.
        let empty = json!({"download_version": manifest(ID, 0, &[0])});
        let pin = PinnedVersion::from_metadata(&empty).unwrap().unwrap();
        assert_eq!((pin.plaintext_size_bytes, pin.wire_size(0)), (0, Some(28)));
    }

    #[test]
    fn no_manifest_means_no_pin() {
        for meta in [json!({"id": "f"}), json!({"id": "f", "download_version": null})] {
            assert_eq!(PinnedVersion::from_metadata(&meta), Ok(None), "{meta}");
        }
    }

    #[test]
    fn a_manifest_no_real_version_could_have_is_refused() {
        let good = manifest(ID, 10, &[10, 10, 3]);
        let mutate = |f: &dyn Fn(&mut Value)| {
            let mut dv = good.clone();
            f(&mut dv);
            PinnedVersion::from_metadata(&json!({ "download_version": dv }))
        };
        let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
            ("no id", Box::new(|v: &mut Value| v["object_version_id"] = Value::Null)),
            ("id not a uuid", Box::new(|v: &mut Value| v["object_version_id"] = json!("v1"))),
            ("count off", Box::new(|v: &mut Value| v["chunk_count"] = json!(4))),
            ("zero chunks", Box::new(|v: &mut Value| {
                v["chunk_count"] = json!(0);
                v["chunks"] = json!([]);
            })),
            ("out of order", Box::new(|v: &mut Value| v["chunks"][1]["index"] = json!(2))),
            ("short middle", Box::new(|v: &mut Value| v["chunks"][1]["size_bytes"] = json!(37))),
            ("long last", Box::new(|v: &mut Value| v["chunks"][2]["size_bytes"] = json!(39))),
            ("below overhead", Box::new(|v: &mut Value| v["chunks"][2]["size_bytes"] = json!(27))),
            ("ciphertext total", Box::new(|v: &mut Value| v["ciphertext_size_bytes"] = json!(108))),
            ("plaintext total", Box::new(|v: &mut Value| v["plaintext_size_bytes"] = json!(24))),
            ("not an object", Box::new(|v: &mut Value| *v = json!("v1"))),
            // Totals that add up, chunks that do not: only the stride rule sees these, and a range read
            // maps offsets to chunks by the stride.
            ("uneven, totals add up", Box::new(|v: &mut Value| {
                v["chunks"][0]["size_bytes"] = json!(37);
                v["chunks"][1]["size_bytes"] = json!(39);
            })),
            ("last past the stride, totals add up", Box::new(|v: &mut Value| {
                v["chunks"][2]["size_bytes"] = json!(39);
                v["ciphertext_size_bytes"] = json!(115);
                v["plaintext_size_bytes"] = json!(31);
            })),
        ];
        for (label, f) in cases {
            assert!(mutate(&*f).is_err(), "{label} must be refused");
        }
        assert!(mutate(&|_| {}).is_ok(), "the unmutated manifest is fine");
    }
}
