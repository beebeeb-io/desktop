//! Desktop v2 (batch B08): the Office engine on this PC.
//!
//! The engine (LibreOffice compiled to WebAssembly, `beebeeb-io/office`; `soffice.wasm` about 164 MB,
//! `soffice.data` about 92 MB) is not in the installer. On the first Office open the window calls
//! `office_engine_prepare`: this module downloads every file of the bundle from our own release host
//! (the desktop updates' host family; no third party, no CDN), checks each one against the hash manifest
//! pinned in this file, caches it under the app's data dir and serves it to the window over the
//! `bboffice` scheme. The window never fetches the engine from the network itself.
//!
//! Errors are `"<code>: <detail>"`: `unpublished` (no bundle is pinned in this build), `host` (the host
//! did not answer or answered short), `hash` (a file did not match its pinned hash; nothing is kept),
//! `disk` (the cache could not be written).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};

#[cfg(test)]
mod tests;

/// The one place the engine's host is named. Files live at `<OFFICE_HOST>/<version>/<path>`.
pub(crate) const OFFICE_HOST: &str = "https://releases.beebeeb.io/office";
/// Development only (debug builds): a local directory holding `<version>/<path>` to read the bundle from
/// instead of the host. The pinned hashes are still checked.
pub(crate) const DEV_DIR_ENV: &str = "BB_OFFICE_ENGINE_DIR";
/// The scheme the window loads the cached engine from (`http://bboffice.localhost/` on Windows).
pub(crate) const SCHEME: &str = "bboffice";
/// The progress event: `{ loaded, total }` in bytes.
pub(crate) const PROGRESS_EVENT: &str = "office://progress";

pub(crate) struct PinnedAsset {
    pub(crate) path: &'static str,
    pub(crate) bytes: u64,
    /// Lowercase hex SHA-256 of the file.
    pub(crate) sha256: &'static str,
    pub(crate) content_type: &'static str,
}

pub(crate) struct PinnedBundle {
    pub(crate) version: &'static str,
    pub(crate) assets: &'static [PinnedAsset],
}

/// The bundle this build trusts. OPEN ITEM: publish the office bundle to `OFFICE_HOST` and pin its
/// version and hashes here. Until then no bundle is pinned and the editor says it can't be prepared.
pub(crate) static PINNED: Option<PinnedBundle> = None;

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EngineStatus {
    /// `ready` (every file cached and checked), `missing` (to download) or `unpublished`.
    pub(crate) state: &'static str,
    pub(crate) version: Option<&'static str>,
    pub(crate) total_bytes: u64,
}

#[derive(Serialize, Clone)]
struct Progress {
    loaded: u64,
    total: u64,
}

fn total(bundle: &PinnedBundle) -> u64 {
    bundle.assets.iter().map(|a| a.bytes).sum()
}

fn cache_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("office"))
        .map_err(|_| "disk: data dir".to_string())
}

/// A file is cached once it exists at its pinned size; `.ok` marks that its hash was checked when written.
pub(crate) fn is_cached(dir: &Path, asset: &PinnedAsset) -> bool {
    let file = dir.join(asset.path);
    let marker = dir.join(format!("{}.ok", asset.path));
    matches!(std::fs::metadata(&file), Ok(m) if m.len() == asset.bytes) && marker.exists()
}

pub(crate) fn status_of(bundle: Option<&'static PinnedBundle>, root: &Path) -> EngineStatus {
    let Some(bundle) = bundle else {
        return EngineStatus {
            state: "unpublished",
            version: None,
            total_bytes: 0,
        };
    };
    let dir = root.join(bundle.version);
    let ready = bundle.assets.iter().all(|a| is_cached(&dir, a));
    EngineStatus {
        state: if ready { "ready" } else { "missing" },
        version: Some(bundle.version),
        total_bytes: total(bundle),
    }
}

pub(crate) fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write `bytes` chunks to `<path>.part`, hash them, and keep the file only if size and hash match the pin.
pub(crate) struct Sink {
    part: PathBuf,
    file: std::fs::File,
    hasher: Sha256,
    written: u64,
}

impl Sink {
    pub(crate) fn create(dir: &Path, asset: &PinnedAsset) -> Result<Sink, String> {
        std::fs::create_dir_all(dir).map_err(|_| "disk: create".to_string())?;
        let part = dir.join(format!("{}.part", asset.path));
        let file = std::fs::File::create(&part).map_err(|_| "disk: create".to_string())?;
        Ok(Sink {
            part,
            file,
            hasher: Sha256::new(),
            written: 0,
        })
    }

    pub(crate) fn write(&mut self, chunk: &[u8]) -> Result<(), String> {
        self.hasher.update(chunk);
        self.written += chunk.len() as u64;
        self.file.write_all(chunk).map_err(|_| "disk: write".to_string())
    }

    pub(crate) fn finish(self, dir: &Path, asset: &PinnedAsset) -> Result<(), String> {
        let Sink {
            part,
            file,
            hasher,
            written,
        } = self;
        drop(file);
        if written != asset.bytes || hex(&hasher.finalize()) != asset.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(if written != asset.bytes {
                "host: short".to_string()
            } else {
                "hash: mismatch".to_string()
            });
        }
        std::fs::rename(&part, dir.join(asset.path)).map_err(|_| "disk: rename".to_string())?;
        std::fs::write(dir.join(format!("{}.ok", asset.path)), b"").map_err(|_| "disk: marker".to_string())
    }
}

#[tauri::command]
pub(crate) async fn office_engine_status(app: tauri::AppHandle) -> Result<EngineStatus, String> {
    Ok(status_of(PINNED.as_ref(), &cache_root(&app)?))
}

/// Download and check every missing file of the pinned bundle, emitting `office://progress`.
#[tauri::command]
pub(crate) async fn office_engine_prepare(app: tauri::AppHandle) -> Result<EngineStatus, String> {
    let Some(bundle) = PINNED.as_ref() else {
        return Err("unpublished: no bundle pinned".to_string());
    };
    let root = cache_root(&app)?;
    let dir = root.join(bundle.version);
    let total = total(bundle);
    let mut loaded = 0u64;
    let client = reqwest::Client::new();
    for asset in bundle.assets {
        if is_cached(&dir, asset) {
            loaded += asset.bytes;
            continue;
        }
        let mut sink = Sink::create(&dir, asset)?;
        if let Some(local) = dev_dir() {
            let bytes =
                std::fs::read(local.join(bundle.version).join(asset.path)).map_err(|_| "host: dev dir".to_string())?;
            sink.write(&bytes)?;
            loaded += bytes.len() as u64;
        } else {
            let url = format!("{OFFICE_HOST}/{}/{}", bundle.version, asset.path);
            let mut resp = client
                .get(url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|_| "host: unreachable".to_string())?;
            let mut last = 0u64;
            while let Some(chunk) = resp.chunk().await.map_err(|_| "host: interrupted".to_string())? {
                sink.write(&chunk)?;
                loaded += chunk.len() as u64;
                if loaded - last >= 1 << 20 {
                    last = loaded;
                    let _ = app.emit(PROGRESS_EVENT, Progress { loaded, total });
                }
            }
        }
        sink.finish(&dir, asset)?;
        let _ = app.emit(PROGRESS_EVENT, Progress { loaded, total });
    }
    Ok(status_of(PINNED.as_ref(), &root))
}

fn dev_dir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        std::env::var_os(DEV_DIR_ENV).map(PathBuf::from)
    } else {
        None
    }
}

/// Which cached file a `bboffice` request names: only a pinned asset of the pinned version, never a path.
pub(crate) fn resolve(bundle: &PinnedBundle, uri_path: &str) -> Option<&'static str> {
    let rest = uri_path
        .trim_start_matches('/')
        .strip_prefix(bundle.version)?
        .strip_prefix('/')?;
    bundle.assets.iter().find(|a| a.path == rest).map(|a| a.path)
}

fn content_type(bundle: &PinnedBundle, path: &str) -> &'static str {
    bundle
        .assets
        .iter()
        .find(|a| a.path == path)
        .map(|a| a.content_type)
        .unwrap_or("application/octet-stream")
}

/// The `bboffice` scheme: serves a cached, checked engine file, with the isolation headers the engine's
/// threads need. Anything else is a 404.
pub(crate) fn protocol<R: tauri::Runtime>(
    ctx: tauri::UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let root = ctx.app_handle().path().app_data_dir().map(|d| d.join("office")).ok();
    let path = request.uri().path().to_string();
    std::thread::spawn(move || {
        let not_found = || {
            tauri::http::Response::builder()
                .status(404)
                .body(Vec::new())
                .expect("static response")
        };
        let answer = (|| {
            let bundle = PINNED.as_ref()?;
            let file = resolve(bundle, &path)?;
            let dir = root?.join(bundle.version);
            let asset = bundle.assets.iter().find(|a| a.path == file)?;
            if !is_cached(&dir, asset) {
                return None;
            }
            let body = std::fs::read(dir.join(file)).ok()?;
            tauri::http::Response::builder()
                .header("Content-Type", content_type(bundle, file))
                .header("Cross-Origin-Opener-Policy", "same-origin")
                .header("Cross-Origin-Embedder-Policy", "require-corp")
                .header("Cross-Origin-Resource-Policy", "same-site")
                .body(body)
                .ok()
        })();
        responder.respond(answer.unwrap_or_else(not_found));
    });
}
