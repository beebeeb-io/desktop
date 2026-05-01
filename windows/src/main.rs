mod api;
mod config;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use beebeeb_sync::watcher::{FileChange, FileWatcher};
use clap::Parser;
use tracing::{debug, error, info, warn};

use crate::api::ApiClient;
use crate::config::{load_config, Config};

/// Beebeeb desktop sync daemon for Windows.
///
/// Runs as a background process managed by the WinUI shell.
/// Watches the local vault folder, encrypts new/changed files,
/// and uploads them to the Beebeeb server.
#[derive(Parser, Debug)]
#[command(name = "beebeeb-desktop-windows", version)]
struct Args {
    /// Start minimized to the system tray (no window).
    #[arg(long, default_value_t = false)]
    start_minimized: bool,

    /// Path to the local vault folder to sync.
    /// Defaults to %USERPROFILE%\Beebeeb.
    #[arg(long, env = "BEEBEEB_SYNC_PATH")]
    sync_path: Option<String>,

    /// Server endpoint override (for development).
    #[arg(long, env = "BEEBEEB_SERVER_URL")]
    server_url: Option<String>,

    /// Log level filter (e.g. "info", "debug", "beebeeb_sync=trace").
    #[arg(long, default_value = "info", env = "BEEBEEB_LOG")]
    log_level: String,
}

fn default_sync_path() -> String {
    std::env::var("USERPROFILE")
        .map(|home| format!(r"{}\Beebeeb", home))
        .unwrap_or_else(|_| {
            // Fallback for Linux/macOS dev environments.
            dirs::home_dir()
                .map(|h| format!("{}/Beebeeb", h.display()))
                .unwrap_or_else(|| "Beebeeb".to_string())
        })
}

/// 1 MiB chunk size (matches beebeeb_types::CHUNK_SIZE).
const CHUNK_SIZE: usize = 1024 * 1024;

/// Debounce window for batching rapid file changes.
const DEBOUNCE_MS: u64 = 500;

/// How often to poll the server for remote changes (v1: log only).
const POLL_INTERVAL: Duration = Duration::from_secs(30);

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// Load the master key from config, returning a beebeeb_core MasterKey.
fn load_master_key(config: &Config) -> Result<beebeeb_core::kdf::MasterKey, String> {
    let mk_b64 = config
        .master_key
        .as_deref()
        .ok_or("No master key found. Run `bb login` first.")?;
    let mk_bytes = b64()
        .decode(mk_b64)
        .map_err(|e| format!("invalid master key in config: {e}"))?;
    if mk_bytes.len() != 32 {
        return Err(format!(
            "master key must be 32 bytes, got {}",
            mk_bytes.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&mk_bytes);
    Ok(beebeeb_core::kdf::MasterKey::from_bytes(arr))
}

/// Guess MIME type from file extension.
fn guess_mime_type(filename: &str) -> Option<String> {
    let ext = filename.rsplit('.').next()?.to_lowercase();
    let mime = match ext.as_str() {
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "md" => "text/markdown",
        "rs" => "text/x-rust",
        "toml" => "application/toml",
        _ => "application/octet-stream",
    };
    Some(mime.to_string())
}

/// Encrypt and upload a single file to the server.
async fn upload_file(
    api: &ApiClient,
    config: &Config,
    file_path: &Path,
    sync_root: &Path,
) -> Result<(), String> {
    let rel_path = file_path
        .strip_prefix(sync_root)
        .unwrap_or(file_path);

    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let file_bytes =
        std::fs::read(file_path).map_err(|e| format!("failed to read {}: {e}", rel_path.display()))?;

    // Load master key and derive a per-file key.
    let master_key = load_master_key(config)?;
    let file_id = uuid::Uuid::new_v4();
    let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes());

    // Encrypt the filename.
    let name_blob = beebeeb_core::encrypt::encrypt_metadata(&file_key, &file_name)
        .map_err(|e| format!("failed to encrypt filename: {e}"))?;
    let name_encrypted = serde_json::to_string(&name_blob)
        .map_err(|e| format!("failed to serialize name blob: {e}"))?;

    // Chunk and encrypt the file data.
    let total_chunks = if file_bytes.is_empty() { 1 } else { file_bytes.len().div_ceil(CHUNK_SIZE) };

    let mut encrypted_chunks: Vec<(u32, Vec<u8>)> = Vec::with_capacity(total_chunks);
    let mut total_encrypted_size: i64 = 0;

    if file_bytes.is_empty() {
        let blob = beebeeb_core::encrypt::encrypt_chunk(&file_key, &[])
            .map_err(|e| format!("failed to encrypt empty chunk: {e}"))?;
        let serialized = serde_json::to_vec(&blob)
            .map_err(|e| format!("failed to serialize chunk: {e}"))?;
        total_encrypted_size += serialized.len() as i64;
        encrypted_chunks.push((0, serialized));
    } else {
        for (i, chunk) in file_bytes.chunks(CHUNK_SIZE).enumerate() {
            let blob = beebeeb_core::encrypt::encrypt_chunk(&file_key, chunk)
                .map_err(|e| format!("failed to encrypt chunk {i}: {e}"))?;
            let serialized = serde_json::to_vec(&blob)
                .map_err(|e| format!("failed to serialize chunk {i}: {e}"))?;
            total_encrypted_size += serialized.len() as i64;
            encrypted_chunks.push((i as u32, serialized));
        }
    }

    // Build metadata JSON.
    let metadata = serde_json::json!({
        "name_encrypted": name_encrypted,
        "parent_id": serde_json::Value::Null,
        "mime_type": guess_mime_type(&file_name),
        "size_bytes": total_encrypted_size,
    });
    let metadata_json = serde_json::to_string(&metadata)
        .map_err(|e| format!("failed to serialize metadata: {e}"))?;

    // Upload.
    let result = api
        .upload_encrypted(&metadata_json, &encrypted_chunks)
        .await?;

    let server_id = result
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    info!(
        file = %rel_path.display(),
        chunks = encrypted_chunks.len(),
        id = server_id,
        "uploaded"
    );

    Ok(())
}

/// Process a batch of changed file paths: encrypt and upload each one.
async fn sync_batch(
    api: &ApiClient,
    config: &Config,
    paths: &[PathBuf],
    sync_root: &Path,
) {
    let count = paths.len();
    info!(count, "syncing batch");

    for path in paths {
        // Skip files that vanished between event and processing.
        if !path.is_file() {
            continue;
        }

        match upload_file(api, config, path, sync_root).await {
            Ok(()) => {}
            Err(e) => {
                let rel = path.strip_prefix(sync_root).unwrap_or(path);
                error!(file = %rel.display(), error = %e, "upload failed");
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&args.log_level)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Load persistent config.
    let config = load_config();

    // CLI flag overrides config for API URL.
    let api_url = args
        .server_url
        .unwrap_or_else(|| config.api_url.clone());

    // Resolve sync path: CLI arg > config > default.
    let sync_path = args
        .sync_path
        .or_else(|| config.sync_path.clone())
        .unwrap_or_else(default_sync_path);

    info!(
        sync_path = %sync_path,
        api_url = %api_url,
        start_minimized = args.start_minimized,
        "Beebeeb sync daemon starting"
    );

    // Check for session token.
    if config.session_token.is_none() {
        error!("No session token found. Run `bb login` first to authenticate.");
        std::process::exit(1);
    }

    // Check for master key.
    if config.master_key.is_none() {
        error!("No master key found. Run `bb login` first to derive encryption keys.");
        std::process::exit(1);
    }

    // Build API client.
    let api = ApiClient::new(api_url, config.session_token.clone());

    // Verify the session is still valid.
    match api.verify_session().await {
        Ok(me) => {
            let email = me
                .get("email")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            info!(email, "session verified");
        }
        Err(e) => {
            error!(error = %e, "session invalid. Run `bb login` to re-authenticate.");
            std::process::exit(1);
        }
    }

    // Ensure the sync folder exists.
    let vault_path = PathBuf::from(&sync_path);
    if !vault_path.exists() {
        std::fs::create_dir_all(&vault_path)?;
        info!(path = %sync_path, "created vault folder");
    }

    // Canonicalize so all path comparisons are consistent.
    let vault_path = std::fs::canonicalize(&vault_path)?;

    // Start the file watcher from beebeeb-sync.
    let watcher = FileWatcher::start(&vault_path)
        .map_err(|e| anyhow::anyhow!("failed to start file watcher: {e}"))?;

    info!(path = %vault_path.display(), "file watcher started");

    if !args.start_minimized {
        info!("Ready. The WinUI shell will connect over the named pipe.");
    }

    // Shutdown signal.
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    // Main loop: drain file events with debouncing + periodic server poll.
    let mut pending: HashSet<PathBuf> = HashSet::new();
    let mut last_event: Option<Instant> = None;
    let mut last_poll = Instant::now();
    let tick = Duration::from_millis(50);

    loop {
        // Check for shutdown.
        if (&mut shutdown).as_mut().poll(&mut std::task::Context::from_waker(
            futures_task_noop_waker(),
        )).is_ready()
        {
            break;
        }

        // Drain file-system events from the watcher (non-blocking).
        loop {
            match watcher.try_recv() {
                Some(change) => {
                    let paths = match &change {
                        FileChange::Created(p) | FileChange::Modified(p) => {
                            if p.is_file() && !is_ignored_file(p) {
                                vec![p.clone()]
                            } else {
                                vec![]
                            }
                        }
                        FileChange::Deleted(p) => {
                            debug!(path = %p.display(), "file deleted (server-side delete not yet implemented)");
                            vec![]
                        }
                        FileChange::Renamed { from, to } => {
                            debug!(from = %from.display(), to = %to.display(), "file renamed");
                            if to.is_file() && !is_ignored_file(to) {
                                vec![to.clone()]
                            } else {
                                vec![]
                            }
                        }
                    };
                    for p in paths {
                        pending.insert(p);
                    }
                    if !pending.is_empty() {
                        last_event = Some(Instant::now());
                    }
                }
                None => break,
            }
        }

        // If debounce window has passed, sync the batch.
        if !pending.is_empty() {
            if let Some(last) = last_event {
                if last.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
                    let batch: Vec<PathBuf> = pending.drain().collect();
                    last_event = None;
                    sync_batch(&api, &config, &batch, &vault_path).await;
                }
            }
        }

        // Periodic server poll (v1: list files and log count).
        if last_poll.elapsed() >= POLL_INTERVAL {
            last_poll = Instant::now();
            match api.list_files(None).await {
                Ok(files) => {
                    let count = files
                        .as_array()
                        .map(|a| a.len())
                        .unwrap_or(0);
                    debug!(remote_files = count, "server poll complete");
                }
                Err(e) => {
                    warn!(error = %e, "server poll failed");
                }
            }
        }

        tokio::time::sleep(tick).await;
    }

    warn!("Received shutdown signal, stopping sync daemon...");
    info!("Beebeeb sync daemon stopped cleanly.");
    Ok(())
}

/// Simple check for files we should never try to upload.
fn is_ignored_file(path: &Path) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return true,
    };

    // Hidden files.
    if name.starts_with('.') {
        return true;
    }

    // Temp files and OS junk.
    matches!(
        name,
        "Thumbs.db" | "desktop.ini" | ".DS_Store"
    ) || name.ends_with('~')
        || name.ends_with(".tmp")
        || name.ends_with(".swp")
        || name.starts_with("~$")
}

/// Create a noop waker for polling the shutdown future.
fn futures_task_noop_waker() -> &'static std::task::Waker {
    use std::task::{RawWaker, RawWakerVTable, Waker};

    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );

    // SAFETY: The waker is trivial (all no-ops) and has 'static lifetime.
    static WAKER: std::sync::OnceLock<Waker> = std::sync::OnceLock::new();
    WAKER.get_or_init(|| unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) })
}
