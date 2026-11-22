//! Test support: Beebeeb's photo-preview routes over one [`FakeAccount`], as the public API answers them:
//! `GET/PUT /api/v1/preferences/photo_previews` (unset reads as `{enabled: false, format: "heic", px: 2560}`),
//! `GET /api/v1/files/previews/summary`, `GET /api/v1/files/media` (the account's own live files, each with
//! `has_preview`, `preview_format`, `preview_px`, `preview_bytes`, and its version: `updated_at` (RFC 3339) and
//! `size_bytes`), and `PUT /api/v1/files/{id}/thumbnail/preview?
//! format=…&px=…` (stores the body as it came; 413 `quota_exceeded` while [`PreviewRoutes::fill_storage`] is on; 404 for
//! a file that is not the account's). Every delete of a preview is answered and counted, so a test can say none came.
//!
//! [`PreviewRoutes::note`] is the one line `bb-fake-server` writes for a stored preview: the format and size it was
//! stored at, its length, whether it is sealed (never a JPEG's first bytes), and what it opens to with the fixture
//! account's own key (the iPhone app's recipe): a JPEG, with its width and height. Never its bytes, never the query.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::fake_account::FakeAccount;
use super::{Reply, Request};
use crate::photo_previews::{PX_CHOICES, PreviewSetting, seal};

/// One stored preview, as the server keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPreview {
    pub format: String,
    pub px: u32,
    pub body: Vec<u8>,
}

/// The preview routes of one account. See the module's doc.
pub struct PreviewRoutes {
    account: Arc<FakeAccount>,
    /// The stored preference value (`None`: never set).
    setting: Mutex<Option<Value>>,
    stored: Mutex<HashMap<String, StoredPreview>>,
    /// The media read lists these ids instead of the account's live files.
    media: Mutex<Option<Vec<String>>>,
    /// A listed file's version (`updated_at` in unix seconds, `size_bytes`) where a test set it; otherwise the
    /// account's own item's.
    versions: Mutex<HashMap<String, (i64, u64)>>,
    /// Every preview PUT is answered this server error (`0`: none).
    put_error: AtomicUsize,
    quota_full: AtomicBool,
    /// The next this many preview PUTs are answered `429`, with this `Retry-After` (seconds).
    rate_limited: AtomicUsize,
    rate_limit_wait: AtomicU64,
    preference_fails: AtomicBool,
    puts: AtomicUsize,
    deletes: AtomicUsize,
    preference_puts: Mutex<Vec<Value>>,
}

const PREVIEW_PUT_CAP: usize = 8 * 1024 * 1024;

impl PreviewRoutes {
    pub fn new(account: Arc<FakeAccount>) -> Self {
        Self {
            account,
            setting: Mutex::new(None),
            stored: Mutex::new(HashMap::new()),
            media: Mutex::new(None),
            versions: Mutex::new(HashMap::new()),
            put_error: AtomicUsize::new(0),
            quota_full: AtomicBool::new(false),
            rate_limited: AtomicUsize::new(0),
            rate_limit_wait: AtomicU64::new(0),
            preference_fails: AtomicBool::new(false),
            puts: AtomicUsize::new(0),
            deletes: AtomicUsize::new(0),
            preference_puts: Mutex::new(Vec::new()),
        }
    }

    /// Store the preference as a client's PUT would.
    pub fn set_setting(&self, setting: PreviewSetting) {
        *self.setting.lock().expect("setting") = Some(setting.to_value());
    }

    /// The preference as stored (`None`: never set).
    pub fn setting(&self) -> Option<Value> {
        self.setting.lock().expect("setting").clone()
    }

    /// Every preference value PUT so far, in order.
    pub fn preference_puts(&self) -> Vec<Value> {
        self.preference_puts.lock().expect("puts").clone()
    }

    /// The media read lists exactly these ids (the account's own media).
    pub fn list_media(&self, ids: &[&str]) {
        *self.media.lock().expect("media") = Some(ids.iter().map(|id| id.to_string()).collect());
    }

    /// The version the media read lists for `id`: when it last changed (unix seconds) and its size.
    pub fn set_version(&self, id: &str, updated_at: i64, size_bytes: u64) {
        self.versions
            .lock()
            .expect("versions")
            .insert(id.to_string(), (updated_at, size_bytes));
    }

    /// Answer every preview PUT with `status` (a server error), or no longer (`0`).
    pub fn fail_puts_with(&self, status: u16) {
        self.put_error.store(usize::from(status), Ordering::SeqCst);
    }

    /// A preview stored before the test (another device made it).
    pub fn store(&self, id: &str, format: &str, px: u32, body: Vec<u8>) {
        self.stored.lock().expect("stored").insert(
            id.to_string(),
            StoredPreview {
                format: format.to_string(),
                px,
                body,
            },
        );
    }

    pub fn stored(&self, id: &str) -> Option<StoredPreview> {
        self.stored.lock().expect("stored").get(id).cloned()
    }

    /// Every preview PUT that reached the routes (stored or refused).
    pub fn puts(&self) -> usize {
        self.puts.load(Ordering::SeqCst)
    }

    /// Every delete of a preview (one file's or all of them).
    pub fn deletes(&self) -> usize {
        self.deletes.load(Ordering::SeqCst)
    }

    /// Answer every preview PUT `413 quota_exceeded`, or no longer.
    pub fn fill_storage(&self, full: bool) {
        self.quota_full.store(full, Ordering::SeqCst);
    }

    /// Answer the next `puts` preview PUTs `429` with `Retry-After: <wait>` (counted as PUTs that reached the routes).
    pub fn rate_limit_next(&self, puts: usize, wait: u64) {
        self.rate_limit_wait.store(wait, Ordering::SeqCst);
        self.rate_limited.store(puts, Ordering::SeqCst);
    }

    /// Answer the preference read `500`, or no longer.
    pub fn fail_the_preference_read(&self, fail: bool) {
        self.preference_fails.store(fail, Ordering::SeqCst);
    }

    fn media_ids(&self) -> Vec<String> {
        self.media
            .lock()
            .expect("media")
            .clone()
            .unwrap_or_else(|| self.account.live_file_ids())
    }

    /// The answer to `request` when it is one of these routes; `None` for any other route.
    pub fn answer(&self, request: &Request) -> Option<Reply> {
        let route = request.route();
        let preview_id = route
            .strip_prefix("PUT /api/v1/files/")
            .or_else(|| route.strip_prefix("DELETE /api/v1/files/"))
            .and_then(|rest| rest.strip_suffix("/thumbnail/preview"));
        let ours = matches!(
            route.as_str(),
            "GET /api/v1/preferences/photo_previews"
                | "PUT /api/v1/preferences/photo_previews"
                | "GET /api/v1/files/previews/summary"
                | "GET /api/v1/files/media"
                | "DELETE /api/v1/files/previews"
        ) || preview_id.is_some();
        if !ours {
            return None;
        }
        if request.bearer() != Some(self.account.session_token.as_str()) {
            return Some(Reply::error(401, "unauthorized"));
        }
        Some(match route.as_str() {
            "GET /api/v1/preferences/photo_previews" => {
                if self.preference_fails.load(Ordering::SeqCst) {
                    return Some(Reply::error(500, "internal"));
                }
                let value = self.setting().unwrap_or_else(|| PreviewSetting::default().to_value());
                Reply::ok(json!({ "value": value }))
            }
            "PUT /api/v1/preferences/photo_previews" => {
                let value = request.json()["value"].clone();
                let valid = value["enabled"].is_boolean()
                    && matches!(value["format"].as_str(), Some("heic" | "jpeg"))
                    && value["px"].as_u64().is_some_and(|px| PX_CHOICES.contains(&(px as u32)));
                if !valid {
                    return Some(Reply::error(400, "invalid_photo_previews"));
                }
                self.preference_puts.lock().expect("puts").push(value.clone());
                *self.setting.lock().expect("setting") = Some(value);
                Reply::ok(json!({ "message": "preference saved" }))
            }
            "GET /api/v1/files/previews/summary" => {
                let stored = self.stored.lock().expect("stored");
                let mut by: Vec<((String, u32), i64)> = Vec::new();
                for preview in stored.values() {
                    match by
                        .iter_mut()
                        .find(|(at, _)| *at == (preview.format.clone(), preview.px))
                    {
                        Some((_, n)) => *n += 1,
                        None => by.push(((preview.format.clone(), preview.px), 1)),
                    }
                }
                by.sort();
                let total: usize = stored.values().map(|preview| preview.body.len()).sum();
                Reply::ok(json!({
                    "count": stored.len(),
                    "total_bytes": total,
                    "by_setting": by.iter().map(|((format, px), n)| json!({ "format": format, "px": px, "count": n })).collect::<Vec<_>>(),
                }))
            }
            "GET /api/v1/files/media" => {
                let stored = self.stored.lock().expect("stored");
                let files: Vec<Value> = self
                    .media_ids()
                    .into_iter()
                    .map(|id| {
                        let preview = stored.get(&id);
                        let version = self
                            .versions
                            .lock()
                            .expect("versions")
                            .get(&id)
                            .copied()
                            .or_else(|| self.account.item_version(&id));
                        let updated_at = version
                            .and_then(|(at, _)| chrono::DateTime::from_timestamp(at, 0))
                            .map(|at| at.to_rfc3339());
                        json!({
                            "id": id,
                            "is_folder": false,
                            "has_preview": preview.is_some(),
                            "preview_format": preview.map(|p| p.format.clone()),
                            "preview_px": preview.map(|p| p.px),
                            "preview_bytes": preview.map(|p| p.body.len()),
                            "updated_at": updated_at,
                            "size_bytes": version.map(|(_, size)| size),
                        })
                    })
                    .collect();
                Reply::ok(json!({ "files": files }))
            }
            "DELETE /api/v1/files/previews" => {
                self.deletes.fetch_add(1, Ordering::SeqCst);
                let removed = self.stored.lock().expect("stored").drain().count();
                Reply::ok(json!({ "removed": removed }))
            }
            _ if route.starts_with("DELETE ") => {
                self.deletes.fetch_add(1, Ordering::SeqCst);
                let id = preview_id.unwrap_or_default();
                self.stored.lock().expect("stored").remove(id);
                Reply::ok(json!({ "message": "preview removed" }))
            }
            _ => {
                self.puts.fetch_add(1, Ordering::SeqCst);
                let id = preview_id.unwrap_or_default();
                let put_error = self.put_error.load(Ordering::SeqCst);
                if put_error != 0 {
                    return Some(Reply::error(u16::try_from(put_error).unwrap_or(500), "internal"));
                }
                if self
                    .rate_limited
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok()
                {
                    return Some(
                        Reply::error(429, "rate_limited").retry_after(self.rate_limit_wait.load(Ordering::SeqCst)),
                    );
                }
                if self.quota_full.load(Ordering::SeqCst) {
                    return Some(Reply::json(
                        413,
                        json!({ "error": "quota_exceeded", "limit_bytes": 1, "used_bytes": 1, "is_trial_cap": false }),
                    ));
                }
                if request.body.len() > PREVIEW_PUT_CAP {
                    return Some(Reply::error(413, "payload_too_large"));
                }
                let format = request.query("format").unwrap_or_default();
                let px = request.query("px").and_then(|px| px.parse::<u32>().ok());
                let (Some(px), true) = (
                    px.filter(|px| PX_CHOICES.contains(px)),
                    matches!(format, "heic" | "jpeg"),
                ) else {
                    return Some(Reply::error(400, "invalid_preview_format"));
                };
                if !self.media_ids().iter().any(|own| own == id) {
                    return Some(Reply::error(404, "not_found"));
                }
                self.store(id, format, px, request.body.clone());
                Reply::ok(
                    json!({ "bytes": request.body.len(), "format": format, "px": px, "message": "preview stored" }),
                )
            }
        })
    }

    /// The line `bb-fake-server` writes after a preview PUT it stored (`None` for any other request): the format, the
    /// size, the length, whether it is sealed, and what it opens to with the account's key. No id, no bytes.
    pub fn note(&self, request: &Request, status: u16) -> Option<String> {
        let id = request
            .route()
            .strip_prefix("PUT /api/v1/files/")?
            .strip_suffix("/thumbnail/preview")?
            .to_string();
        if status != 200 {
            return Some(format!("preview refused: {status}"));
        }
        let stored = self.stored(&id)?;
        let sealed = !stored.body.starts_with(&[0xFF, 0xD8]);
        let opens = match seal::open_preview(&self.account.master_key, &id, &stored.body) {
            Ok(plain) => match image::load_from_memory_with_format(&plain, image::ImageFormat::Jpeg) {
                Ok(picture) => format!("a JPEG {}x{}", picture.width(), picture.height()),
                Err(_) => "bytes that are not a JPEG".to_string(),
            },
            Err(_) => "nothing (it does not open with the account's key)".to_string(),
        };
        Some(format!(
            "preview stored: format {}, px {}, {} bytes, sealed {}, opens to {opens}",
            stored.format,
            stored.px,
            stored.body.len(),
            if sealed { "yes" } else { "NO" }
        ))
    }
}
