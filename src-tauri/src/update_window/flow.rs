//! How this build reads an update feed and hands an accepted update to the Tauri updater (tasks 2449 and 2451).
//!
//! Owns: reading `desktop/v2/<channel>.json` and its `.sig` first on Windows; the channel's replay mark (the newest
//! signed time accepted, kept beside the config); the one check every offer and install goes through
//! ([`checked_update`]), which gives each feed the one key of its line; quieting the engine before a native
//! installer runs (`preflight`), then recording the channel and the hand-off note.
//! Must not: decide a rule itself (those are `trust`'s), or fall back to the unsigned feed for anything but "the v2
//! manifest does not exist" (404). A v2 manifest that exists and fails any check is a hard error.
//! Tests: `flow_tests`.

use super::trust::{self, Direction, Feed, Host, Line, SignedManifest, TrustedKeys};
use super::*;

/// The code an error leads with when the signed feed could not be read (anything but "not there").
pub(crate) const UPDATE_MANIFEST_UNAVAILABLE_CODE: &str = "update_manifest_unavailable";
/// The largest manifest or signature this build reads.
const MAX_FEED_BYTES: usize = 1024 * 1024;
/// The note a native build reads once to learn it replaced this app in place.
pub(crate) const UPDATE_HANDOFF_FILE: &str = "update-handoff.json";
/// Where the replay marks live, beside the config: `{"<channel>": <signed time of the newest accepted manifest>}`.
pub(crate) const UPDATE_SEEN_FILE: &str = "update-v2-seen.json";

/// This PC's clock in unix seconds.
pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// One writer of the marks file at a time in this process (the background check and a manual one may overlap).
static MARKS_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn read_marks(dir: &Path) -> std::collections::BTreeMap<String, u64> {
    std::fs::read(dir.join(UPDATE_SEEN_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The channel's replay mark, or `None` when none was written. A file that cannot be read counts as no mark: the
/// fixed not-before bound and the future bound still hold, and a mark that blocked every update for good after one
/// damaged write would be worse.
pub(crate) fn replay_mark(dir: &Path, channel: ReleaseChannel) -> Option<u64> {
    read_marks(dir).get(channel.as_str()).copied()
}

/// Stores `accepted` as the channel's replay mark unless the stored one is larger (a clock set back never lowers
/// it). Written through a temp file and a rename.
pub(crate) fn record_replay_mark(dir: &Path, channel: ReleaseChannel, accepted: u64) -> std::io::Result<()> {
    let _one_writer = MARKS_WRITE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut marks = read_marks(dir);
    let stored = marks.get(channel.as_str()).copied();
    let next = trust::next_replay_mark(stored, accepted);
    if stored == Some(next) {
        return Ok(());
    }
    marks.insert(channel.as_str().to_string(), next);
    let temp = dir.join(format!("{UPDATE_SEEN_FILE}.tmp"));
    std::fs::write(&temp, serde_json::to_vec(&marks)?)?;
    std::fs::rename(&temp, dir.join(UPDATE_SEEN_FILE))
}

/// The folder the config lives in: where the replay marks and the hand-off note go.
pub(crate) fn state_dir() -> Option<PathBuf> {
    DesktopConfig::path()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

pub(crate) fn v2_manifest_url(base: &str, channel: ReleaseChannel) -> String {
    let file = trust::v2_manifest_file(channel.as_str()).expect("every release channel has a v2 manifest file");
    format!("{base}/desktop/v2/{file}")
}

/// What the signed feed's address answered.
pub(crate) enum V2Feed {
    /// 404 for the manifest: no native release exists on this channel yet.
    Absent,
    Present {
        manifest: Vec<u8>,
        signature: String,
    },
}

/// One GET: no redirect, 30 s, at most [`MAX_FEED_BYTES`]. The error is a status word, never the client's text.
async fn get_small(client: &reqwest::Client, url: &str) -> Result<(u16, Vec<u8>), String> {
    let unavailable = |why: &str| format!("{UPDATE_MANIFEST_UNAVAILABLE_CODE}: the signed update feed {why}.");
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| unavailable("could not be reached"))?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable("could not be read"))? {
        if body.len() + chunk.len() > MAX_FEED_BYTES {
            return Err(unavailable("is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, body))
}

/// Reads the v2 manifest and its detached signature from `base`. Only a 404 for the manifest is [`V2Feed::Absent`];
/// every other answer that is not two 200s is an error: a manifest without its signature is not "absent".
pub(crate) async fn fetch_v2_feed(base: &str, channel: ReleaseChannel) -> Result<V2Feed, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| format!("{UPDATE_MANIFEST_UNAVAILABLE_CODE}: the update client could not start."))?;
    let url = v2_manifest_url(base, channel);
    let (status, manifest) = get_small(&client, &url).await?;
    if status == 404 {
        return Ok(V2Feed::Absent);
    }
    if status != 200 {
        return Err(format!(
            "{UPDATE_MANIFEST_UNAVAILABLE_CODE}: the signed update feed answered {status}."
        ));
    }
    let (status, signature) = get_small(&client, &format!("{url}.sig")).await?;
    let signature = String::from_utf8(signature).ok().filter(|_| status == 200);
    match signature {
        Some(signature) => Ok(V2Feed::Present { manifest, signature }),
        None => Err(trust::Refusal::ManifestSignature.message()),
    }
}

/// The feed this install reads: the signed manifest when it exists (Windows only), else today's unsigned one.
/// `None` is the unsigned feed. A manifest that passed becomes the channel's replay mark in `marks_dir`; a refused
/// one (signed in the future included) is never recorded.
pub(crate) async fn signed_manifest_for(
    windows: bool,
    base: &str,
    keys: &TrustedKeys,
    channel: ReleaseChannel,
    now: u64,
    marks_dir: Option<&Path>,
) -> Result<Option<SignedManifest>, String> {
    if !windows {
        return Ok(None);
    }
    match fetch_v2_feed(base, channel).await? {
        V2Feed::Absent => Ok(None),
        V2Feed::Present { manifest, signature } => {
            let seen = marks_dir.and_then(|dir| replay_mark(dir, channel));
            let signed = trust::verify_v2_manifest(keys, channel.as_str(), &manifest, &signature, now, seen).map_err(
                |refusal| {
                    tracing::warn!(code = refusal.code(), channel = %channel.as_str(), "the signed update manifest was refused");
                    refusal.message()
                },
            )?;
            if let Some(dir) = marks_dir
                && let Err(error) = record_replay_mark(dir, channel, signed.signed_at)
            {
                tracing::warn!(kind = ?error.kind(), "could not store the update replay mark");
            }
            Ok(Some(signed))
        }
    }
}

/// The keys this build trusts: the one in `tauri.conf.json` (legacy line) and `updater-v2.pub` (native line).
pub(crate) fn trusted_keys(app: &tauri::AppHandle) -> Result<TrustedKeys, String> {
    let baked = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(|pubkey| pubkey.as_str())
        .unwrap_or_default();
    TrustedKeys::new(baked, trust::NEXT_PUBKEY_FILE)
}

/// This PC's `<os>-<arch>` in the names the Tauri updater gives a manifest's platform keys (`windows-x86_64`). The
/// updater's own `Update::target` holds the OS alone, so it cannot tell one architecture's entry from another's.
pub(crate) fn updater_target(os: &str, arch: &str) -> String {
    let os = if os == "macos" { "darwin" } else { os };
    let arch = match arch {
        "x86" => "i686",
        "arm" => "armv7",
        other => other,
    };
    format!("{os}-{arch}")
}

/// Strictly newer, by this app's own version order; `None` when either version cannot be read.
pub(crate) fn strictly_newer(installed: &str, offered: &str) -> Option<bool> {
    compare_channel_release_to_current(installed, offered).map(|ordering| ordering == std::cmp::Ordering::Greater)
}

/// The update the selected channel offers this install, accepted by `trust::accept`, on an updater that verifies
/// its bytes with the one key of the feed's line: the v2 key for the signed manifest, the baked key for the legacy
/// feed. `Ok(None)`: nothing to offer. Every offer and every install goes through here.
pub(crate) async fn checked_update(
    app: &tauri::AppHandle,
    channel: ReleaseChannel,
    direction: Direction,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    use tauri_plugin_updater::UpdaterExt;

    let keys = trusted_keys(app)?;
    let marks_dir = state_dir();
    let signed = signed_manifest_for(
        cfg!(target_os = "windows"),
        UPDATE_MANIFEST_BASE_URL,
        &keys,
        channel,
        unix_now(),
        marks_dir.as_deref(),
    )
    .await?;
    let (line, endpoint) = match &signed {
        Some(_) => (Line::Native, v2_manifest_url(UPDATE_MANIFEST_BASE_URL, channel)),
        None => (Line::Legacy, update_manifest_url_for_channel(channel)),
    };
    // The key the Tauri updater verifies the downloaded bytes with is fixed by the line, before anything is read.
    let line_key = keys
        .for_line(line)
        .ok_or_else(|| trust::Refusal::ManifestSignature.message())?;
    let url = endpoint
        .parse()
        .map_err(|e| format!("parse update manifest URL {endpoint}: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        // The decision is `trust::accept`'s, against `real_app_version()`, not Tauri's own comparison.
        .version_comparator(|_current, _release| true)
        .pubkey(line_key.tauri_pubkey.clone())
        .build()
        .map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    if direction == Direction::NewerOnly && strictly_newer(real_app_version(), &update.version) != Some(true) {
        return Ok(None);
    }
    let target = updater_target(std::env::consts::OS, std::env::consts::ARCH);
    let host = Host {
        macos: cfg!(target_os = "macos"),
        target: &target,
    };
    let feed = signed.as_ref().map_or(Feed::Unsigned, Feed::Signed);
    let key = trust::accept(
        &keys,
        &feed,
        host,
        direction,
        real_app_version(),
        &update.version,
        update.download_url.as_str(),
        &update.signature,
        strictly_newer,
    )
    .map_err(|refusal| {
        tracing::warn!(code = refusal.code(), version = %update.version, channel = %channel.as_str(), "an update was refused");
        refusal.message()
    })?;
    // `accept` names the key of the feed's line or refuses; anything else is a bug, and nothing is installed.
    if key.tauri_pubkey != line_key.tauri_pubkey {
        return Err(trust::Refusal::KeyScope.message());
    }
    Ok(Some(update))
}

/// Runs `install` with the record written first. On Windows a successful install exits the process inside
/// `install`, so anything written after it is never written; a failed install takes the record back.
pub(crate) fn record_then_install<E>(
    record: impl FnOnce() -> Result<(), String>,
    take_back: impl FnOnce(),
    install: impl FnOnce() -> Result<(), E>,
) -> Result<(), String>
where
    E: std::fmt::Display,
{
    record()?;
    install().map_err(|error| {
        take_back();
        error.to_string()
    })
}

/// Writes `<dir>/update-handoff.json` (`{"from","to","channel","at"}`) through a temp file and a rename, so a
/// reader never sees half a note. The next version reads it once and deletes it.
pub(crate) fn write_update_handoff(
    dir: &Path,
    from: &str,
    to: &str,
    channel: ReleaseChannel,
    at: u64,
) -> std::io::Result<()> {
    let note = serde_json::json!({ "from": from, "to": to, "channel": channel.as_str(), "at": at });
    let temp = dir.join(format!("{UPDATE_HANDOFF_FILE}.tmp"));
    std::fs::write(&temp, serde_json::to_vec(&note)?)?;
    std::fs::rename(&temp, dir.join(UPDATE_HANDOFF_FILE))
}

/// The code an error leads with when the installer did not take over after sync was stopped for it.
pub(crate) const UPDATE_INSTALL_FAILED_SYNC_STOPPED_CODE: &str = "update_install_failed_sync_stopped";

/// Downloads the accepted update (the updater verifies the bytes with the key `checked_update` chose). For a
/// native release (0.9.0 or later) it then makes sync quiet (`preflight::quiet_engine`: uploads in flight finish,
/// the engine stops, the Cloud Files provider disconnects) and keeps the engine slot until the installer has taken
/// over. Then, for every update: the channel that served it is recorded (and, for a native release, the hand-off
/// note is written), and the install runs. On Windows a started installer ends this process: what it does after
/// that (a cancel, a failure) can no longer take the record back here. Returns only when the install did not replace this process (Linux), or failed.
pub(crate) async fn download_record_install(
    app: &tauri::AppHandle,
    update: tauri_plugin_updater::Update,
    channel: ReleaseChannel,
) -> Result<(), String> {
    let from = real_app_version().to_string();
    let to = update.version.clone();
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateDownloaded {
        from: from.clone(),
        to: to.clone(),
    });
    // Signed out: no account, no engine, nothing to make quiet.
    let acct = app.state::<AppState>().active_account().ok();
    let board = app
        .try_state::<crate::popover_data::PopoverRuntime>()
        .map(|runtime| runtime.transfers.clone());
    let quiet = match acct.as_deref() {
        Some(acct) if trust::is_native_line(&to) => {
            Some(preflight::quiet_engine(acct, board.as_deref(), preflight::UPLOAD_WAIT).await?)
        }
        _ => None,
    };
    let previous = DesktopConfig::load().ok().and_then(|cfg| cfg.installed_release_channel);
    let handoff_dir = state_dir();
    let at = unix_now();
    let installed = record_then_install(
        || {
            record_installed_release_channel(channel).map_err(|e| format!("record installed release channel: {e}"))?;
            // Only a native build reads (and deletes) the note; a 0.8.x update would leave a stale one behind.
            if trust::is_native_line(&to)
                && let Some(dir) = handoff_dir.as_deref()
                && let Err(error) = write_update_handoff(dir, &from, &to, channel, at)
            {
                tracing::warn!(kind = ?error.kind(), "could not write the update hand-off note");
            }
            lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateInstalled {
                from: from.clone(),
                to: to.clone(),
            });
            Ok(())
        },
        || {
            let _ = DesktopConfig::update(|cfg| {
                cfg.installed_release_channel = previous;
                (true, ())
            });
            if let Some(dir) = handoff_dir.as_deref() {
                let _ = std::fs::remove_file(dir.join(UPDATE_HANDOFF_FILE));
            }
        },
        || update.install(bytes),
    );
    match (installed, quiet) {
        (Ok(()), _) => Ok(()),
        (Err(error), None) => Err(error),
        (Err(error), Some(quiet)) => {
            let stopped = quiet.stopped_an_engine;
            drop(quiet);
            tracing::error!(%error, stopped, "the native installer did not take over after the pre-flight");
            Err(if stopped {
                format!(
                    "{UPDATE_INSTALL_FAILED_SYNC_STOPPED_CODE}: the update could not be installed and sync is stopped. Restart Beebeeb to sync again."
                )
            } else {
                error
            })
        }
    }
}
