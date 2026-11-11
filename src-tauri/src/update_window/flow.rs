//! How this build reads an update feed and hands an accepted update to the Tauri updater (tasks 2449 and 2451).
//!
//! Owns: reading `desktop/v2/<channel>.json` and its `.sig` first on Windows; the one check every offer and install
//! goes through ([`checked_update`]); recording the channel and the hand-off note before the installer runs.
//! Must not: decide a rule itself (those are `trust`'s), or fall back to the unsigned feed for anything but "the v2
//! manifest does not exist" (404). A v2 manifest that exists and fails any check is a hard error.
//! Tests: `flow_tests`.

use super::trust::{self, Direction, Feed, Host, SignedManifest, TrustedKeys};
use super::*;

/// The code an error leads with when the signed feed could not be read (anything but "not there").
pub(crate) const UPDATE_MANIFEST_UNAVAILABLE_CODE: &str = "update_manifest_unavailable";
/// The largest manifest or signature this build reads.
const MAX_FEED_BYTES: usize = 1024 * 1024;
/// The note a native build reads once to learn it replaced this app in place.
pub(crate) const UPDATE_HANDOFF_FILE: &str = "update-handoff.json";

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
/// `None` is the unsigned feed.
pub(crate) async fn signed_manifest_for(
    windows: bool,
    base: &str,
    keys: &TrustedKeys,
    channel: ReleaseChannel,
) -> Result<Option<SignedManifest>, String> {
    if !windows {
        return Ok(None);
    }
    match fetch_v2_feed(base, channel).await? {
        V2Feed::Absent => Ok(None),
        V2Feed::Present { manifest, signature } => {
            trust::verify_v2_manifest(keys, channel.as_str(), &manifest, &signature)
                .map(Some)
                .map_err(|refusal| {
                    tracing::warn!(code = refusal.code(), channel = %channel.as_str(), "the signed update manifest was refused");
                    refusal.message()
                })
        }
    }
}

/// The keys this build trusts: the one in `tauri.conf.json` and the next one (`updater-v2.pub`).
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

/// Strictly newer, by this app's own version order; `None` when either version cannot be read.
pub(crate) fn strictly_newer(installed: &str, offered: &str) -> Option<bool> {
    compare_channel_release_to_current(installed, offered).map(|ordering| ordering == std::cmp::Ordering::Greater)
}

/// The update the selected channel offers this install, accepted by `trust::accept`, on an updater that verifies
/// its bytes with the trusted key that signed it. `Ok(None)`: nothing to offer. Every offer and every install
/// goes through here.
pub(crate) async fn checked_update(
    app: &tauri::AppHandle,
    channel: ReleaseChannel,
    direction: Direction,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    use tauri_plugin_updater::UpdaterExt;

    let keys = trusted_keys(app)?;
    let signed = signed_manifest_for(cfg!(target_os = "windows"), UPDATE_MANIFEST_BASE_URL, &keys, channel).await?;
    let endpoint = match &signed {
        Some(_) => v2_manifest_url(UPDATE_MANIFEST_BASE_URL, channel),
        None => update_manifest_url_for_channel(channel),
    };
    let updater_with = |pubkey: Option<&str>| -> Result<tauri_plugin_updater::Updater, String> {
        let url = endpoint
            .parse()
            .map_err(|e| format!("parse update manifest URL {endpoint}: {e}"))?;
        let mut builder = app
            .updater_builder()
            .endpoints(vec![url])
            .map_err(|e| e.to_string())?
            // The decision is `trust::accept`'s, against `real_app_version()`, not Tauri's own comparison.
            .version_comparator(|_current, _release| true);
        if let Some(pubkey) = pubkey {
            builder = builder.pubkey(pubkey);
        }
        builder.build().map_err(|e| e.to_string())
    };
    let Some(update) = updater_with(None)?.check().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    if direction == Direction::NewerOnly && strictly_newer(real_app_version(), &update.version) != Some(true) {
        return Ok(None);
    }
    let host = Host {
        macos: cfg!(target_os = "macos"),
        windows: cfg!(target_os = "windows"),
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
    if key.tauri_pubkey == keys.baked().tauri_pubkey {
        return Ok(Some(update));
    }
    // Signed with the next key: the same update again, on an updater that verifies with that key.
    let again = updater_with(Some(&key.tauri_pubkey))?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    match again {
        Some(again) if same_update(&update, &again) => Ok(Some(again)),
        _ => Err(trust::Refusal::NotInManifest.message()),
    }
}

fn same_update(a: &tauri_plugin_updater::Update, b: &tauri_plugin_updater::Update) -> bool {
    a.version == b.version && a.download_url == b.download_url && a.signature == b.signature
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

/// Downloads the accepted update (the updater verifies the bytes with the key `checked_update` chose), records
/// the channel that served it and the hand-off note, then installs. Returns only when the install did not replace
/// this process (Linux), or failed.
pub(crate) async fn download_record_install(
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
    let previous = DesktopConfig::load().ok().and_then(|cfg| cfg.installed_release_channel);
    let handoff_dir = DesktopConfig::path()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    record_then_install(
        || {
            record_installed_release_channel(channel).map_err(|e| format!("record installed release channel: {e}"))?;
            if let Some(dir) = handoff_dir.as_deref()
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
    )
}
