//! `update_window` items split out of `lib.rs`.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedReleaseVersion {
    pub(super) major: u64,
    pub(super) minor: u64,
    pub(super) patch: u64,
    pub(super) prerelease: Option<Vec<PrereleasePart>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PrereleasePart {
    Numeric(u64),
    Text(String),
}

impl Ord for PrereleasePart {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::Numeric(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Numeric(_)) => std::cmp::Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PrereleasePart {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ParsedReleaseVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => compare_prerelease_parts(a, b),
            })
    }
}

impl PartialOrd for ParsedReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub(super) fn compare_prerelease_parts(a: &[PrereleasePart], b: &[PrereleasePart]) -> std::cmp::Ordering {
    for (left, right) in a.iter().zip(b.iter()) {
        let ordering = left.cmp(right);
        if !ordering.is_eq() {
            return ordering;
        }
    }
    a.len().cmp(&b.len())
}

pub(super) fn parse_release_version(version: &str) -> Option<ParsedReleaseVersion> {
    let version_without_build = version.split_once('+').map(|(v, _)| v).unwrap_or(version);
    let (core, prerelease) = version_without_build
        .split_once('-')
        .map_or((version_without_build, None), |(core, pre)| (core, Some(pre)));

    let mut core_parts = core.split('.');
    let major = core_parts.next()?.parse::<u64>().ok()?;
    let minor = core_parts.next()?.parse::<u64>().ok()?;
    let patch = core_parts.next()?.parse::<u64>().ok()?;
    if core_parts.next().is_some() {
        return None;
    }

    let prerelease = match prerelease {
        Some(pre) if pre.is_empty() => return None,
        Some(pre) => {
            let mut parts = Vec::new();
            for part in pre.split('.') {
                if part.is_empty() {
                    return None;
                }
                if part.as_bytes().iter().all(u8::is_ascii_digit) {
                    parts.push(PrereleasePart::Numeric(part.parse::<u64>().ok()?));
                } else {
                    parts.push(PrereleasePart::Text(part.to_string()));
                }
            }
            Some(parts)
        }
        None => None,
    };

    Some(ParsedReleaseVersion {
        major,
        minor,
        patch,
        prerelease,
    })
}

pub(super) fn release_channel_from_version(version: &str) -> ReleaseChannel {
    let version_without_build = version.split_once('+').map(|(v, _)| v).unwrap_or(version);
    let Some((_, prerelease)) = version_without_build.split_once('-') else {
        return ReleaseChannel::Stable;
    };
    match prerelease.split('.').next() {
        Some("alpha") => ReleaseChannel::Alpha,
        Some("beta") => ReleaseChannel::Beta,
        _ => ReleaseChannel::Stable,
    }
}

pub(super) fn desktop_manifest_path_for_channel(channel: ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Stable => "desktop/latest.json",
        ReleaseChannel::Beta => "desktop/beta.json",
        ReleaseChannel::Alpha => "desktop/alpha.json",
    }
}

pub(super) fn update_manifest_url_for_channel(channel: ReleaseChannel) -> String {
    format!(
        "{UPDATE_MANIFEST_BASE_URL}/{}",
        desktop_manifest_path_for_channel(channel)
    )
}

pub(super) fn release_notes_url_for_version(version: &str) -> String {
    format!("{DESKTOP_RELEASE_REPO_URL}/{DESKTOP_RELEASE_TAG_PREFIX}{version}")
}

pub(super) fn compare_channel_release_to_current(
    current_version: &str,
    remote_version: &str,
) -> Option<std::cmp::Ordering> {
    Some(parse_release_version(remote_version)?.cmp(&parse_release_version(current_version)?))
}

pub(super) fn is_channel_downgrade(current_version: &str, remote_version: &str) -> bool {
    compare_channel_release_to_current(current_version, remote_version) == Some(std::cmp::Ordering::Less)
}

/// Return true only when the selected channel's manifest points to a version
/// newer than the installed app. Background checks deliberately hold the current
/// build until the selected channel catches up; the manual Settings path uses a
/// separate manifest check so it can report older channel builds as downgrades.
pub(super) fn should_offer_channel_update(current_version: &str, remote_version: &str) -> bool {
    match compare_channel_release_to_current(current_version, remote_version) {
        Some(ordering) => ordering == std::cmp::Ordering::Greater,
        _ => remote_version != current_version,
    }
}

pub(super) fn configured_release_channel() -> ReleaseChannel {
    match DesktopConfig::load() {
        Ok(cfg) => cfg.release_channel,
        Err(error) => {
            tracing::warn!(%error, "could not load desktop release channel; defaulting updater to stable");
            ReleaseChannel::Stable
        }
    }
}

pub(super) fn installed_release_channel_from_config(
    config: Result<DesktopConfig, String>,
    version: &str,
) -> ReleaseChannel {
    match config {
        Ok(cfg) => cfg
            .installed_release_channel
            .unwrap_or_else(|| release_channel_from_version(version)),
        Err(error) => {
            tracing::warn!(%error, "could not load installed release channel; deriving legacy channel from version");
            release_channel_from_version(version)
        }
    }
}

pub(super) fn installed_release_channel() -> ReleaseChannel {
    installed_release_channel_from_config(DesktopConfig::load(), real_app_version())
}

pub(super) fn record_installed_release_channel(channel: ReleaseChannel) -> Result<(), String> {
    DesktopConfig::update(|cfg| {
        cfg.installed_release_channel = Some(channel);
        (true, ())
    })
}

pub(super) fn updater_builder_for_release_channel(
    app: &tauri::AppHandle,
    channel: ReleaseChannel,
) -> Result<tauri_plugin_updater::UpdaterBuilder, String> {
    use tauri_plugin_updater::UpdaterExt;

    let endpoint = update_manifest_url_for_channel(channel);
    app.updater_builder()
        .endpoints(vec![
            endpoint
                .parse()
                .map_err(|e| format!("parse update manifest URL {endpoint}: {e}"))?,
        ])
        .map_err(|e| e.to_string())
}

pub(super) fn updater_for_release_channel(
    app: &tauri::AppHandle,
    channel: ReleaseChannel,
) -> Result<tauri_plugin_updater::Updater, String> {
    updater_builder_for_release_channel(app, channel)?
        .version_comparator(|_current, release| {
            // Compare the baked release version, not Tauri's `_current`.
            should_offer_channel_update(real_app_version(), &release.version.to_string())
        })
        .build()
        .map_err(|e| e.to_string())
}

pub(super) fn manifest_updater_for_release_channel(
    app: &tauri::AppHandle,
    channel: ReleaseChannel,
) -> Result<tauri_plugin_updater::Updater, String> {
    updater_builder_for_release_channel(app, channel)?
        // Manual checks report the selected channel's latest manifest, including an equal or older one.
        .version_comparator(|_current, _release| true)
        .build()
        .map_err(|e| e.to_string())
}

/// Baked release version, or the Cargo package version when no release was set.
pub(super) fn real_app_version() -> &'static str {
    option_env!("BEEBEEB_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct UpdateAvailablePayload {
    pub(super) version: String,
    pub(super) channel: String,
    pub(super) body: String,
    pub(super) release_notes_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum ManualUpdateCheckResult {
    UpToDate {
        current_version: String,
        channel: String,
    },
    UpdateAvailable {
        current_version: String,
        channel: String,
        version: String,
        body: String,
        release_notes_url: String,
    },
    DowngradeAvailable {
        current_version: String,
        current_channel: String,
        channel: String,
        version: String,
        body: String,
        release_notes_url: String,
    },
}

pub(super) fn manual_update_up_to_date_result(channel: ReleaseChannel) -> ManualUpdateCheckResult {
    ManualUpdateCheckResult::UpToDate {
        current_version: real_app_version().to_string(),
        channel: channel.as_str().to_string(),
    }
}

pub(super) fn manual_update_available_result(
    channel: ReleaseChannel,
    current_version: &str,
    version: String,
    body: Option<&str>,
) -> (ManualUpdateCheckResult, UpdateAvailablePayload) {
    let body = body.unwrap_or("").to_string();
    let release_notes_url = release_notes_url_for_version(&version);
    let channel = channel.as_str().to_string();
    let payload = UpdateAvailablePayload {
        version: version.clone(),
        channel: channel.clone(),
        body: body.clone(),
        release_notes_url: release_notes_url.clone(),
    };

    (
        ManualUpdateCheckResult::UpdateAvailable {
            current_version: current_version.to_string(),
            channel,
            version,
            body,
            release_notes_url,
        },
        payload,
    )
}

pub(super) fn manual_update_downgrade_result(
    channel: ReleaseChannel,
    current_channel: ReleaseChannel,
    current_version: &str,
    version: String,
    body: Option<&str>,
) -> ManualUpdateCheckResult {
    ManualUpdateCheckResult::DowngradeAvailable {
        current_version: current_version.to_string(),
        current_channel: current_channel.as_str().to_string(),
        channel: channel.as_str().to_string(),
        version: version.clone(),
        body: body.unwrap_or("").to_string(),
        release_notes_url: release_notes_url_for_version(&version),
    }
}

pub(super) fn manual_update_result_for_remote(
    channel: ReleaseChannel,
    current_channel: ReleaseChannel,
    current_version: &str,
    version: String,
    body: Option<&str>,
) -> (ManualUpdateCheckResult, Option<UpdateAvailablePayload>) {
    match compare_channel_release_to_current(current_version, &version) {
        Some(std::cmp::Ordering::Less) => (
            manual_update_downgrade_result(channel, current_channel, current_version, version, body),
            None,
        ),
        Some(std::cmp::Ordering::Equal) => (
            ManualUpdateCheckResult::UpToDate {
                current_version: current_version.to_string(),
                channel: channel.as_str().to_string(),
            },
            None,
        ),
        Some(std::cmp::Ordering::Greater) => {
            let (result, payload) = manual_update_available_result(channel, current_version, version, body);
            (result, Some(payload))
        }
        None if version == current_version => (
            ManualUpdateCheckResult::UpToDate {
                current_version: current_version.to_string(),
                channel: channel.as_str().to_string(),
            },
            None,
        ),
        None => {
            let (result, payload) = manual_update_available_result(channel, current_version, version, body);
            (result, Some(payload))
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub(super) struct AppVersion {
    pub version: &'static str,
    pub release: bool,
}

#[tauri::command]
pub(super) fn app_version() -> AppVersion {
    AppVersion {
        version: real_app_version(),
        release: option_env!("BEEBEEB_RELEASE_VERSION").is_some(),
    }
}

/// Check the selected channel now. An available update emits the background updater's banner event.
#[tauri::command]
pub(super) async fn check_for_updates_now(app: tauri::AppHandle) -> Result<ManualUpdateCheckResult, String> {
    let channel = configured_release_channel();
    let updater = manifest_updater_for_release_channel(&app, channel)
        .map_err(|e| format!("Could not prepare the {} update check: {e}", channel.as_str()))?;

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.to_string();
            let (result, payload) = manual_update_result_for_remote(
                channel,
                installed_release_channel(),
                real_app_version(),
                version.clone(),
                update.body.as_deref(),
            );

            match &result {
                ManualUpdateCheckResult::UpdateAvailable { .. } => {
                    tracing::info!(
                        version = %version,
                        channel = %channel.as_str(),
                        "manual update check found an update"
                    );
                }
                ManualUpdateCheckResult::DowngradeAvailable { .. } => {
                    tracing::info!(
                        current_version = %real_app_version(),
                        version = %version,
                        channel = %channel.as_str(),
                        "manual update check found a downgrade"
                    );
                }
                ManualUpdateCheckResult::UpToDate { .. } => {
                    tracing::info!(channel = %channel.as_str(), "manual update check found no update");
                }
            }

            if let Some(payload) = payload {
                popover_data::note_update(&payload.channel, Some(&payload.version));
                let _ = app.emit("update-available", payload);
            } else {
                popover_data::note_update(channel.as_str(), None);
            }

            Ok(result)
        }
        Ok(None) => {
            tracing::info!(channel = %channel.as_str(), "manual update check found no manifest release");
            popover_data::note_update(channel.as_str(), None);
            Ok(manual_update_up_to_date_result(channel))
        }
        Err(error) => Err(format!(
            "Could not reach the {} update manifest: {error}",
            channel.as_str()
        )),
    }
}

/// The code a refused in-place install leads with, so a caller can tell it from a failed one.
pub(super) const UPDATE_INSTALL_UNSUPPORTED_CODE: &str = "update_install_unsupported";

/// The code a refused second install leads with, so a caller can tell it from a failed one.
pub(super) const UPDATE_INSTALL_IN_PROGRESS_CODE: &str = "update_install_in_progress";
/// False when no window is installing. Both install commands take it with compare-and-swap.
static INSTALL_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
/// Holds the install slot for as long as it lives; every error path returns early, so `Drop` releases it.
pub(super) struct InstallClaim;
impl Drop for InstallClaim {
    fn drop(&mut self) {
        INSTALL_IN_PROGRESS.store(false, Ordering::Release);
    }
}
/// Claims the process-wide install slot. Refused before any download if another window already holds it.
pub(super) fn claim_install() -> Result<InstallClaim, String> {
    INSTALL_IN_PROGRESS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| InstallClaim)
        .map_err(|_| {
            format!("{UPDATE_INSTALL_IN_PROGRESS_CODE}: an update is already installing in another Beebeeb window.")
        })
}
#[cfg(test)]
#[path = "install_guard_tests.rs"]
mod install_guard_tests;

/// Task 2176: the App Sandbox denies replacing the running bundle, so on a Mac an in-place install can only fail, after
/// the download, with an error nobody can diagnose. A Mac is offered a download instead (two-surface spec §0 ruling 5),
/// and both install commands refuse here, first, as defence in depth behind the window's own rule.
pub(super) fn in_app_install_refusal(host: desktop_capabilities::HostOs) -> Result<(), String> {
    if host == desktop_capabilities::HostOs::Macos {
        return Err(format!(
            "{UPDATE_INSTALL_UNSUPPORTED_CODE}: Beebeeb on a Mac downloads updates and cannot install them in place."
        ));
    }
    Ok(())
}

/// Download and install the pending update, then relaunch.
///
/// Called by the frontend when the user clicks "Restart to update" in the
/// update-available banner (Windows and Linux; a Mac downloads, see `in_app_install_refusal`).
/// Returns an error string if installation fails.
#[tauri::command]
pub(super) async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    in_app_install_refusal(desktop_capabilities::host_os())?;
    let _claim = claim_install()?;
    let channel = configured_release_channel();
    let updater = updater_for_release_channel(&app, channel)?;
    let update = updater.check().await.map_err(|e| e.to_string())?;

    match update {
        Some(u) => {
            tracing::info!(version = %u.version, channel = %channel.as_str(), "downloading update");
            // Spec §8: the update's from and to versions go to the lifecycle log.
            let from_version = real_app_version().to_string();
            let to_version = u.version.to_string();
            let versions = (from_version.clone(), to_version.clone());

            u.download_and_install(
                |downloaded, total| {
                    // Emit progress so the frontend can show a progress bar.
                    // downloaded: usize, total: Option<u64>
                    let pct = total
                        .map(|t| if t > 0 { (downloaded as u64) * 100 / t } else { 0 })
                        .unwrap_or(0);
                    tracing::debug!("update download progress: {pct}%");
                },
                move || {
                    tracing::info!("update installed — relaunching");
                    lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateDownloaded {
                        from: versions.0,
                        to: versions.1,
                    });
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateInstalled {
                from: from_version,
                to: to_version,
            });

            record_installed_release_channel(channel).map_err(|e| format!("record installed release channel: {e}"))?;
            #[cfg(target_os = "macos")]
            note_updater_relaunch(&app);
            app.restart();
        }
        None => {
            tracing::info!("install_update called but no update available");
        }
    }

    Ok(())
}

/// Download and install the selected channel's currently advertised older
/// release. The version argument is checked against the manifest before any
/// install attempt so a stale Settings view cannot install a different build.
#[tauri::command]
pub(super) async fn install_channel_downgrade(app: tauri::AppHandle, version: String) -> Result<(), String> {
    in_app_install_refusal(desktop_capabilities::host_os())?;
    let _claim = claim_install()?;
    let channel = configured_release_channel();
    let updater = manifest_updater_for_release_channel(&app, channel)?;
    let update = updater.check().await.map_err(|e| e.to_string())?;

    let Some(u) = update else {
        return Err(format!(
            "The {} channel does not currently advertise a desktop release.",
            channel.as_str()
        ));
    };

    if u.version != version {
        return Err(format!(
            "The {} channel now points to version {}. Check for updates again before downgrading.",
            channel.as_str(),
            u.version
        ));
    }

    if !is_channel_downgrade(real_app_version(), &u.version) {
        return Err(format!(
            "Version {} is not older than the running version {}.",
            u.version,
            real_app_version()
        ));
    }

    tracing::info!(
        current_version = %real_app_version(),
        version = %u.version,
        channel = %channel.as_str(),
        "downloading downgrade"
    );
    // Spec §8: the downgrade's from and to versions go to the lifecycle log.
    let from_version = real_app_version().to_string();
    let to_version = u.version.to_string();
    let versions = (from_version.clone(), to_version.clone());

    u.download_and_install(
        |downloaded, total| {
            let pct = total
                .map(|t| if t > 0 { (downloaded as u64) * 100 / t } else { 0 })
                .unwrap_or(0);
            tracing::debug!("downgrade download progress: {pct}%");
        },
        move || {
            tracing::info!("downgrade installed — relaunching");
            lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateDownloaded {
                from: versions.0,
                to: versions.1,
            });
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateInstalled {
        from: from_version,
        to: to_version,
    });

    record_installed_release_channel(channel).map_err(|e| format!("record installed release channel: {e}"))?;
    #[cfg(target_os = "macos")]
    note_updater_relaunch(&app);
    app.restart()
}

/// Toggle "Start at login" autostart setting.
#[tauri::command]
pub(super) fn toggle_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    let manager = app.autolaunch();
    let currently_enabled = manager.is_enabled().map_err(|e| e.to_string())?;
    if currently_enabled {
        manager.disable().map_err(|e| e.to_string())?;
        tracing::info!("autostart disabled");
    } else {
        manager.enable().map_err(|e| e.to_string())?;
        tracing::info!("autostart enabled");
    }
    Ok(!currently_enabled)
}

/// Return the current autostart state (for the frontend settings page).
#[tauri::command]
pub(super) fn autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub(super) const WINDOWS_TRAY_FLYOUT_WIDTH: u32 = 364;
pub(super) const WINDOWS_TRAY_FLYOUT_HEIGHT: u32 = 592;
pub(super) const WINDOWS_TRAY_FLYOUT_GAP: i32 = 10;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn windows_tray_flyout_position(
    click_x: f64,
    click_y: f64,
    scale_factor: f64,
) -> tauri::PhysicalPosition<i32> {
    let scale_factor = valid_windows_tray_scale_factor(scale_factor);
    let physical_width = (WINDOWS_TRAY_FLYOUT_WIDTH as f64 * scale_factor).round() as i32;
    let physical_height = (WINDOWS_TRAY_FLYOUT_HEIGHT as f64 * scale_factor).round() as i32;
    let physical_gap = (WINDOWS_TRAY_FLYOUT_GAP as f64 * scale_factor).round() as i32;
    tauri::PhysicalPosition {
        x: (click_x.round() as i32).saturating_sub(physical_width).max(0),
        y: (click_y.round() as i32).saturating_sub(physical_height.saturating_add(physical_gap)),
    }
}

/// Where the flyout opens when it is not opened from the b (desktop v2: the main window's status line, DW-0401#10 →
/// DW-1201): the bottom-right corner of the work area (the display without its taskbar) of the display the window is
/// on, where Windows 11 opens its own Quick Settings. Kept inside the work area on a display smaller than the flyout.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(super) fn windows_flyout_position_in_work_area(
    work_x: i32,
    work_y: i32,
    work_width: u32,
    work_height: u32,
    scale_factor: f64,
) -> tauri::PhysicalPosition<i32> {
    let scale_factor = valid_windows_tray_scale_factor(scale_factor);
    let physical_width = (WINDOWS_TRAY_FLYOUT_WIDTH as f64 * scale_factor).round() as i32;
    let physical_height = (WINDOWS_TRAY_FLYOUT_HEIGHT as f64 * scale_factor).round() as i32;
    let right = work_x.saturating_add(i32::try_from(work_width).unwrap_or(i32::MAX));
    let bottom = work_y.saturating_add(i32::try_from(work_height).unwrap_or(i32::MAX));
    tauri::PhysicalPosition {
        x: right.saturating_sub(physical_width).max(work_x),
        y: bottom.saturating_sub(physical_height).max(work_y),
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn valid_windows_tray_scale_factor(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

#[cfg(target_os = "windows")]
pub(super) fn windows_tray_flyout_scale_factor(win: &tauri::WebviewWindow, click_x: f64, click_y: f64) -> f64 {
    let click_monitor_scale = win
        .monitor_from_point(click_x, click_y)
        .ok()
        .flatten()
        .map(|monitor| monitor.scale_factor())
        .filter(|scale_factor| scale_factor.is_finite() && *scale_factor > 0.0);

    click_monitor_scale
        .or_else(|| {
            win.scale_factor()
                .ok()
                .filter(|scale_factor| scale_factor.is_finite() && *scale_factor > 0.0)
        })
        .unwrap_or(1.0)
}

// ── macOS tray flyout anchoring (task 1384) ────────────────────────────────
//
// Retired with the menu-bar popover (task 1683 slice 3, review P3-7 of #160): the tray click opens the popover,
// placed by `surfaces::anchor`, and nothing hands this window an anchor any more. The statics and the `Popover`
// mode below remain until GL-0009 retires them. Linux keeps the original centered/decorated behaviour untouched.

pub(super) const MACOS_SETTINGS_WINDOW_WIDTH: f64 = 680.0;
pub(super) const MACOS_SETTINGS_WINDOW_HEIGHT: f64 = 540.0;

/// A one-shot handoff to `show_compact_app_window_with_nav`: the physical position the flyout should open at.
/// Nothing sets it now (review P3-7 of #160), so every open reads `None` until GL-0009 retires it; `take()`n
/// (consumed) the moment it's read, so an unrelated open falls back to centering instead of reusing a stale position.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) static MACOS_TRAY_FLYOUT_ANCHOR: LazyLock<Mutex<Option<tauri::PhysicalPosition<i32>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Debounces the focus-loss-hide handler against the spurious `Focused(false)`
/// some window managers deliver right as a borderless always-on-top window
/// is created/shown, before it has actually become key — only hide once a
/// real `Focused(true)` was observed since the last show.
#[cfg(target_os = "macos")]
pub(super) static MACOS_FLYOUT_GAINED_FOCUS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the shared macOS settings window is currently behaving as a
/// tray-anchored popover or as a regular window. Set every time
/// `show_compact_app_window_with_nav` shows the window (same one-shot-handoff
/// family as `MACOS_TRAY_FLYOUT_ANCHOR`/`MACOS_FLYOUT_GAINED_FOCUS`), and read
/// live by the focus-loss handler registered once at window-build time — the
/// window is reused for every subsequent open, so the handler cannot rely on
/// a mode captured in its closure at build time; it must consult this flag
/// each time focus changes (1384 review follow-up: only the tray-click path
/// should hide on focus loss and open chrome-less; "Open Beebeeb", the
/// Preferences/Settings menu items, and `show_main_app_window` must produce
/// the previous regular, decorated, centered, persistent window).
#[cfg(target_os = "macos")]
pub(super) static MACOS_FLYOUT_IS_POPOVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Which chrome/behavior the shared macOS settings window should adopt for
/// this open. Pure decision, extracted from `show_compact_app_window_with_nav`
/// so it's unit-testable without a live `AppHandle`/`WebviewWindow`: an
/// anchor is present only when one was handed off through
/// `MACOS_TRAY_FLYOUT_ANCHOR` (consumed via `take()`; nothing sets it since
/// the menu-bar popover, so today every open is `Regular`); every other caller
/// ("Open Beebeeb", Preferences/Settings menu items, a notification, a deep
/// link) passes `None` and gets the regular window treatment.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MacosWindowMode {
    /// Chrome-less, always-on-top, positioned under the status item, hides
    /// on focus loss — reads as a system popover.
    Popover,
    /// Decorated, centered, normal window-manager behavior, stays open when
    /// focus moves elsewhere.
    Regular,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn macos_window_mode_for_anchor(anchor: Option<tauri::PhysicalPosition<i32>>) -> MacosWindowMode {
    match anchor {
        Some(_) => MacosWindowMode::Popover,
        None => MacosWindowMode::Regular,
    }
}

// ── The Beebeeb window on a Mac (task 1683 slice 1) ──────────────────────────
//
// Two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §4, §7, §9: one window, label `main`, built on
// demand from `tauri.macos.conf.json` (`create: false`) and destroyed when closed (`surfaces::policy::close_policy`).
// It replaced the anchored Settings window (`macos-settings`), whose placement code retired with it. Every opener
// shows and focuses the one window and navigates it through one request: Rust stores the latest `MainNav`, emits
// `main:navigate`, and the window pulls it with `consume_main_navigation`. The window also pulls once as soon as it
// listens, so a window built for the request still gets it.

/// The event that tells the Beebeeb window a request is waiting (`src/mainWindowModel.ts` `MAIN_NAVIGATE_EVENT`).
pub(super) const MAIN_NAVIGATE_EVENT: &str = "main:navigate";

/// One request to the Beebeeb window, serialised as `src/mainWindowModel.ts`'s `MainNav` reads it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "to", rename_all = "lowercase")]
pub(super) enum MainNav {
    /// Home, and out of Settings (⌘1).
    Home,
    /// Settings on a section (`account`, `general`, `sync`, `about`); `None` opens the last section used in this
    /// window, else General (§7.8).
    Settings { section: Option<&'static str> },
    /// Out of Settings, to where it was entered from (⌘[, View › Back).
    Back,
    /// “Sign in again”, asked for by `open_reauth_window`: the window shows that card whatever the session says, as
    /// Linux's onboarding window does (`mode=reauth`), so the click always lands on it (review P3-1).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    Reauth,
    /// The popover found no local vault key: continue in the same Set up this Mac card.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    Setup,
}

impl MainNav {
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(super) const fn settings(section: Option<&'static str>) -> MainNav {
        MainNav::Settings { section }
    }
}

/// The latest request, kept until the window pulls it: one slot, so a burst of openers lands on the last one.
pub(super) static MAIN_NAV_PENDING: std::sync::Mutex<Option<MainNav>> = std::sync::Mutex::new(None);

pub(super) fn store_main_nav(slot: &std::sync::Mutex<Option<MainNav>>, nav: MainNav) {
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(nav);
}

pub(super) fn take_main_nav(slot: &std::sync::Mutex<Option<MainNav>>, label: &str) -> Option<MainNav> {
    if label != surfaces::policy::main_window_label(surfaces::policy::Platform::current()) {
        return None;
    }
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
}

/// The Beebeeb window pulls the request it was told about (or the one it was built for).
#[tauri::command]
pub(super) fn consume_main_navigation(window: tauri::WebviewWindow) -> Option<MainNav> {
    take_main_nav(&MAIN_NAV_PENDING, window.label())
}

/// Opens (or shows and focuses) the Beebeeb window and asks it to show `nav`.
#[cfg(target_os = "macos")]
pub(super) fn show_main_window(app: &tauri::AppHandle, nav: MainNav) {
    // macos_popover::hide deleted; the popover window no longer exists in v2-native.
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    store_main_nav(&MAIN_NAV_PENDING, nav);
    // §9.1: the Dock icon (and with it the app menu) comes first, so it is there when the window is.
    set_macos_dock_icon(app, true);
    if let Some(win) = app.get_webview_window(label) {
        unhide_app(app);
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.emit(MAIN_NAVIGATE_EVENT, ());
        return;
    }
    let Some(config) = app.config().app.windows.iter().find(|window| window.label == label) else {
        tracing::warn!("main window: no tauri.macos.conf.json entry named {label}; cannot open the Beebeeb window");
        refresh_macos_dock_icon(app, None);
        return;
    };
    match tauri::WebviewWindowBuilder::from_config(app, config).and_then(|builder| builder.build()) {
        Ok(win) => {
            place_main_window(app, &win, (config.width, config.height));
            unhide_app(app);
            let _ = win.show();
            let _ = win.set_focus();
        }
        Err(error) => {
            tracing::warn!(%error, "failed to create the Beebeeb window");
            refresh_macos_dock_icon(app, None);
        }
    }
}

/// Unhides the app (`AppHandle::show`, NSApp `unhide:`), which the tray's Hide and ⌘H hide. Not a window: a window
/// shown in a hidden app stays hidden, and tao's `set_focus` does nothing there, so every opener on a Mac calls this
/// before it shows its window (review round 2, P2-2).
#[cfg(target_os = "macos")]
pub(super) fn unhide_app(app: &tauri::AppHandle) {
    let _ = app.show();
}

/// `RunEvent::Reopen` (double-clicking Beebeeb.app, Spotlight, the Dock; spec §7.3): the window as it was, or a new one
/// on Home. With the window closed the app lives on in the menu bar, so this is the way back (review round 2, P2-1).
/// Which of the two is `surfaces::launch::reopen_action`.
#[cfg(target_os = "macos")]
pub(super) fn reopen_main_window(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    let window = app.get_webview_window(label);
    match (surfaces::launch::reopen_action(window.is_some()), window) {
        (surfaces::launch::ReopenAction::BringForward, Some(win)) => {
            set_macos_dock_icon(app, true);
            unhide_app(app);
            let _ = win.unminimize();
            let _ = win.show();
            let _ = win.set_focus();
        }
        _ => show_main_window(app, MainNav::Home),
    }
}

/// How this launch happened (two-surface spec §7.3, task 1683 slice 6): the login item's signal (the one seam task 1845
/// plugs into, `surfaces::launch::login_item_signal`) and the flag the updater left just before it restarted the app,
/// which this launch consumes. A Mac only: Windows and Linux are launched the way they always were.
#[cfg(target_os = "macos")]
pub(super) fn launch_source_now() -> surfaces::launch::LaunchSource {
    let now = u64::try_from(now_unix_seconds()).unwrap_or(0);
    let flag = state_paths::beebeeb_state_dir()
        .ok()
        .and_then(|dir| surfaces::launch::take_relaunch_flag(&dir, now));
    surfaces::launch::launch_source(surfaces::launch::login_item_signal(), flag, now)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn launch_source_now() -> surfaces::launch::LaunchSource {
    surfaces::launch::LaunchSource::Manual
}

/// Leaves the flag the next launch reads, just before the updater restarts the app: whether the Beebeeb window is open and
/// whether an account is signed in. A failure is logged and the restart goes on; the next launch then opens what a manual
/// one does.
#[cfg(target_os = "macos")]
pub(super) fn note_updater_relaunch(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    let window_open = app.get_webview_window(label).is_some();
    let state = app.state::<AppState>();
    let vault_unlocked = state
        .active_account()
        .ok()
        .and_then(|acct| acct.session.lock().ok().map(|guard| guard.is_some()))
        .unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
    let flag = surfaces::launch::RelaunchFlag {
        written_at: u64::try_from(now_unix_seconds()).unwrap_or(0),
        window_open,
        signed_in: vault_unlocked || auth_present,
    };
    let written = state_paths::beebeeb_state_dir()
        .and_then(|dir| surfaces::launch::write_relaunch_flag(&dir, &flag).map_err(|e| e.to_string()));
    if let Err(error) = written {
        tracing::warn!(%error, "could not leave the relaunch flag; the next launch opens as a manual one");
    }
}

/// View › Back (⌘[): only an open window has somewhere to go back from; it never opens one.
#[cfg(target_os = "macos")]
pub(super) fn back_in_main_window(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    if let Some(win) = app.get_webview_window(label) {
        store_main_nav(&MAIN_NAV_PENDING, MainNav::Back);
        let _ = win.emit(MAIN_NAVIGATE_EVENT, ());
    }
}

/// The displays in logical points (each one's physical frame over its own scale, which each `Display` keeps), and
/// the index of the primary one.
pub(super) fn macos_displays(win: &tauri::WebviewWindow) -> (Vec<surfaces::window_state::Display>, usize) {
    use surfaces::window_state::{Display, PRect};
    let monitors = win.available_monitors().unwrap_or_default();
    let displays = monitors
        .iter()
        .map(|m| {
            Display::from_physical(
                PRect {
                    x: m.position().x,
                    y: m.position().y,
                    w: m.size().width,
                    h: m.size().height,
                },
                PRect {
                    x: m.work_area().position.x,
                    y: m.work_area().position.y,
                    w: m.work_area().size.width,
                    h: m.work_area().size.height,
                },
                m.scale_factor(),
            )
        })
        .collect();
    let primary = win
        .primary_monitor()
        .ok()
        .flatten()
        .and_then(|p| {
            monitors
                .iter()
                .position(|m| m.name() == p.name() && m.position() == p.position())
        })
        .unwrap_or(0);
    (displays, primary)
}

/// The menu-bar icon's rect as tray-icon reports it (points × the icon's own scale).
#[cfg(target_os = "macos")]
pub(super) fn macos_status_item_rect(app: &tauri::AppHandle) -> Option<surfaces::window_state::PRect> {
    let rect = app.tray_by_id("tray")?.rect().ok().flatten()?;
    let position = rect.position.to_physical::<f64>(1.0);
    let size = rect.size.to_physical::<f64>(1.0);
    Some(surfaces::window_state::PRect {
        x: position.x.round() as i32,
        y: position.y.round() as i32,
        w: size.width.round().max(0.0) as u32,
        h: size.height.round().max(0.0) as u32,
    })
}

/// Where the Beebeeb window's frame is kept (logical points), in the app's config directory.
#[cfg(target_os = "macos")]
pub(super) fn main_frame_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(surfaces::window_state::MAIN_FRAME_FILENAME))
}

/// The kept frame, read back. A missing, empty, partial or unreadable file, or one that is not a usable frame, is no
/// frame: the window then opens at its default place.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn read_kept_frame(path: &Path) -> Option<surfaces::window_state::LRect> {
    let bytes = std::fs::read(path).ok()?;
    surfaces::window_state::parse_kept_frame(std::str::from_utf8(&bytes).ok()?)
}

/// Writes the kept frame atomically: a temporary file next to it, then a rename over it, so a crash mid-write leaves
/// the previous frame, never half a file.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn write_kept_frame(path: &Path, frame: &surfaces::window_state::LRect) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(frame).map_err(std::io::Error::other)?)?;
    std::fs::rename(&temporary, path)
}

/// Keeps the Beebeeb window's frame, when it closes and when the app quits while it is open (§7.2). A full-screen
/// or zoomed window keeps nothing, so the last ordinary frame stays (review item 8).
#[cfg(target_os = "macos")]
pub(super) fn save_main_window_frame(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    let (Ok(position), Ok(size), Ok(scale)) = (win.outer_position(), win.inner_size(), win.scale_factor()) else {
        return;
    };
    let fullscreen = win.is_fullscreen().unwrap_or(false);
    let zoomed = win.is_maximized().unwrap_or(false);
    let Some(frame) = surfaces::window_state::frame_to_keep(
        (position.x, position.y),
        (size.width, size.height),
        scale,
        fullscreen,
        zoomed,
    ) else {
        return;
    };
    let Some(path) = main_frame_path(app) else {
        return;
    };
    if let Err(error) = write_kept_frame(&path, &frame) {
        tracing::warn!(%error, "main window: its frame could not be kept");
    }
}

/// §7.2, planned in full before any of it is applied (review item 4): the kept frame when at least half of it is on a
/// display, else at its kept size (or `default_size`, the window's own config) centred on the menu-bar icon's display,
/// else on the primary one. Logical points throughout, so a mixed-DPI desk places it where it was (review item 5).
#[cfg(target_os = "macos")]
pub(super) fn place_main_window(app: &tauri::AppHandle, win: &tauri::WebviewWindow, default_size: (f64, f64)) {
    use surfaces::window_state as placement;
    let (displays, primary) = macos_displays(win);
    let icon = macos_status_item_rect(app).and_then(|rect| placement::icon_display(rect, &displays));
    let kept = main_frame_path(app).and_then(|path| read_kept_frame(&path));
    let Some(frame) = placement::plan_main_window(kept, &displays, icon, primary, default_size) else {
        tracing::warn!("main window: no display to place it on");
        return;
    };
    if kept.is_some() && kept != Some(frame) {
        tracing::info!("main window: less than half of its kept frame is on a display; centred again");
    }
    let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(frame.w, frame.h)));
    let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(frame.x, frame.y)));
}

/// The activation policy last applied: 0 unknown (the app starts Regular), 1 Dock icon, 2 none.
#[cfg(target_os = "macos")]
pub(super) static MACOS_DOCK_ICON: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// §9.1: Regular (a Dock icon and the app menu) or Accessory (neither). Applied only when it changes.
#[cfg(target_os = "macos")]
pub(super) fn set_macos_dock_icon(app: &tauri::AppHandle, wanted: bool) {
    let next = if wanted { 1 } else { 2 };
    if MACOS_DOCK_ICON.swap(next, Ordering::SeqCst) == next {
        return;
    }
    let policy = if wanted {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    };
    if let Err(error) = app.set_activation_policy(policy) {
        tracing::warn!(%error, wanted, "could not change the Dock icon");
    }
}

/// Hides a window whose close policy is Hide. The macOS popover's special hide path (macos_popover::hide) was deleted
/// with the web-view popover; the native app has its own flyout.
pub(super) fn hide_on_close(window: &tauri::Window) {
    let _ = window.hide();
}

/// The Dock icon for the windows that exist now, leaving out `gone` (a window being destroyed or hidden).
#[cfg(target_os = "macos")]
pub(super) fn refresh_macos_dock_icon(app: &tauri::AppHandle, gone: Option<&str>) {
    let windows: Vec<(String, bool)> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| Some(label.as_str()) != gone)
        .map(|(label, window)| {
            let shown = window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false);
            (label, shown)
        })
        .collect();
    let wanted = surfaces::policy::dock_icon_wanted(windows.iter().map(|(label, shown)| (label.as_str(), *shown)));
    set_macos_dock_icon(app, wanted);
}

/// What the app menu may offer around the Beebeeb window (two-surface spec §8.2, F15): View › Back only while
/// Settings shows in an open window; View › Home, Beebeeb › Settings… and Check for Updates… (which opens Settings ›
/// About) always, except while a sheet is open, so ⌘1, ⌘[, ⌘, and ⇧⌘U cannot unmount Settings › Account under its
/// sheet (review item 6 chose disabling over a second way to answer the check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MainMenuState {
    pub(super) settings: bool,
    pub(super) home: bool,
    pub(super) back: bool,
    pub(super) check_updates: bool,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn main_menu_state(main_open: bool, settings_showing: bool, sheet_open: bool) -> MainMenuState {
    let sheet = main_open && sheet_open;
    MainMenuState {
        settings: !sheet,
        home: !sheet,
        back: main_open && settings_showing && !sheet,
        check_updates: !sheet,
    }
}

#[cfg(target_os = "macos")]
pub(super) fn apply_macos_main_menu_state(app: &tauri::AppHandle, state: MainMenuState) {
    let Some(items) = app.try_state::<MacosMenuItems>() else {
        return;
    };
    let _ = items.settings.set_enabled(state.settings);
    let _ = items.check_updates.set_enabled(state.check_updates);
    let _ = items.view_home.set_enabled(state.home);
    let _ = items.view_back.set_enabled(state.back);
}

/// The Beebeeb window says whether Settings shows and whether a sheet is open; the menu follows (§8.2, F15).
#[tauri::command]
pub(super) fn main_window_state(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    settings_showing: bool,
    sheet_open: bool,
) {
    #[cfg(target_os = "macos")]
    if window.label() == surfaces::policy::main_window_label(surfaces::policy::Platform::current()) {
        apply_macos_main_menu_state(&app, main_menu_state(true, settings_showing, sheet_open));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, window, settings_showing, sheet_open);
}

/// A window's life on a Mac, beside the global close handler: the Beebeeb window keeps its frame when it closes and
/// resets the menu when it is gone, and every window that comes, goes or hides sets the Dock icon again (§9.1).
#[cfg(target_os = "macos")]
pub(super) fn macos_window_lifecycle(window: &tauri::Window, event: &tauri::WindowEvent) {
    let app = window.app_handle();
    let main = window.label() == surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    match event {
        tauri::WindowEvent::CloseRequested { .. } if main => save_main_window_frame(app),
        tauri::WindowEvent::Destroyed => {
            if main {
                apply_macos_main_menu_state(app, main_menu_state(false, false, false));
            }
            refresh_macos_dock_icon(app, Some(window.label()));
        }
        tauri::WindowEvent::Focused(true) => refresh_macos_dock_icon(app, None),
        _ => {}
    }
}

/// The window-state plugin, on Windows and Linux only, with today's default file and denylist. A Mac does not run
/// it (review item 5 on PR #134): it stores physical numbers without their scale, which cannot be placed back on a
/// mixed-DPI desk, so `place_main_window` and `save_main_window_frame` keep the Beebeeb window's frame in points.
#[cfg(not(target_os = "macos"))]
pub(super) fn window_state_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_window_state::Builder::default()
        .with_denylist(window_state_denylist_labels())
        .build()
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(super) fn window_state_denylist_labels() -> &'static [&'static str] {
    // `main-app` kept its own frame in logical points (`place_desktop_main_window`, now deleted), so the plugin left it alone.
    &["tray", "windows-onboarding", "main-app"]
}

// Tests relocated from desktop_main_window.rs live in flyout_geometry_tests.rs (the file-size ratchet).
#[cfg(test)]
#[path = "flyout_geometry_tests.rs"]
mod flyout_geometry_tests;
