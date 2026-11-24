//! Slice B5 (plan40 batch 1): Beebeeb's notices as Windows toasts (DW-0750), honouring the alert switches.
//!
//! The shell calls [`start`] once at the app's start, before the engine opens (`shell::open_engine`), and [`show`]
//! for every notice the window drains from the engine (`engine_host::LiveEngine::drain`).
//!
//! The rules are plain data and compile into the tests: which switch of DW-0750 a notice belongs to
//! ([`Kind`], [`wanted`]), the words ([`words_of`]), the toast's XML ([`toast_xml`]) and the app's Windows identity
//! ([`aumid`]). Only [`start`], [`show`] and what they call touch Windows.
//!
//! What a toast says: the board's words for its switch and nothing a person named. No file name, no path, no
//! e-mail, no account id: a toast shows on the lock screen and in the notification centre, where anyone near the PC
//! reads it. A notice's own text ([`Notice::BackupRefusal`] carries a path) is never put in a toast.
//!
//! The switches are read from this PC's settings file at the moment a notice arrives, so a switch changed in
//! Settings acts on the next toast and the file is the one the Notifications pane writes.

use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::host::Notice;

use crate::shell::identity::AppIdentity;

/// The alert a notice belongs to: one row (or group) of DW-0750.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// "New sign-ins and security changes": Always on, no switch.
    // Nothing raises one through a `Notice` yet; the account's own security alerts come from Beebeeb.
    #[allow(dead_code)]
    Security,
    /// DW-A11: another device asks to sign in and the request reached this PC. Security, so always on.
    SignInRequest,
    /// "Sync finished" (Sync on this PC).
    SyncFinished,
    /// "Files need a decision" (Sync on this PC).
    NeedsDecision,
    /// "Space on this PC" (Sync on this PC).
    LocalSpace,
    /// "Backup needs attention" (Backups on this PC).
    // Nothing raises one through a `Notice` yet: a refused move of Backups is DW-1182 ([`Kind::ServiceFolder`]).
    #[allow(dead_code)]
    BackupAttention,
    /// DW-1182: File Explorer was refused a move, rename or delete, of a service folder or of Backups (a refused
    /// Backups move, [`Notice::BackupRefusal`], is DW-1182 too). No row of its own: the person did it a moment
    /// ago, so only "Allow notifications" decides.
    ServiceFolder,
}

/// This PC's notification switches ([`DesktopConfig`]'s `notify_*`), the ones a toast honours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Switches {
    pub allow: bool,
    pub sync_finished: bool,
    pub needs_decision: bool,
    pub local_space: bool,
    pub backup_attention: bool,
}

impl Switches {
    pub(crate) fn from_config(config: &DesktopConfig) -> Self {
        Self {
            allow: config.notify_allow,
            sync_finished: config.notify_sync_complete,
            needs_decision: config.notify_needs_decision,
            local_space: config.notify_local_space,
            backup_attention: config.notify_backup_attention,
        }
    }
}

/// The alert a notice is.
pub(crate) fn kind_of(notice: &Notice) -> Kind {
    match notice {
        Notice::SyncComplete => Kind::SyncFinished,
        Notice::QuotaWarning { .. } => Kind::LocalSpace,
        Notice::Conflict { .. } => Kind::NeedsDecision,
        Notice::BackupRefusal { .. } => Kind::ServiceFolder,
        Notice::ServiceFolderRefused => Kind::ServiceFolder,
    }
}

/// Whether a toast of this kind is shown: Security always (the pane says these cannot be turned off, so not even
/// "Allow notifications" silences them); every other kind needs "Allow notifications" and its own switch.
pub(crate) fn wanted(kind: Kind, switches: &Switches) -> bool {
    match kind {
        Kind::Security | Kind::SignInRequest => true,
        _ if !switches.allow => false,
        Kind::SyncFinished => switches.sync_finished,
        Kind::NeedsDecision => switches.needs_decision,
        Kind::LocalSpace => switches.local_space,
        Kind::BackupAttention => switches.backup_attention,
        Kind::ServiceFolder => true,
    }
}

/// A toast's two lines.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Words {
    pub title: &'static str,
    pub body: &'static str,
}

/// The words of a kind: the board's (DW-0750's row titles and sentences; DW-1182 and DW-0425 for the folder).
/// Fixed text only, so nothing a person named can reach a toast.
pub(crate) fn words(kind: Kind) -> Words {
    match kind {
        Kind::Security => Words {
            title: "Security change on your account",
            body: "Open Beebeeb to see what changed.",
        },
        Kind::SignInRequest => Words {
            title: "Sign-in request",
            body: "Another device is asking to sign in. Open Beebeeb to approve or deny.",
        },
        Kind::SyncFinished => Words {
            title: "Sync finished",
            body: "Everything on this PC has synced.",
        },
        Kind::NeedsDecision => Words {
            title: "Files need a decision",
            body: "A file changed in two places.",
        },
        Kind::LocalSpace => Words {
            title: "Space on this PC",
            body: "Downloaded files are near their limit.",
        },
        Kind::BackupAttention => Words {
            title: "Backup needs attention",
            body: "A folder Beebeeb backs up was kept in place.",
        },
        Kind::ServiceFolder => Words {
            title: "Beebeeb manages this folder",
            body: "Backups and the folders Beebeeb services keep can't be moved, renamed or deleted. You can open and download what's inside.",
        },
    }
}

/// The words of a notice.
#[allow(dead_code)] // no caller yet
pub(crate) fn words_of(notice: &Notice) -> Words {
    words(kind_of(notice))
}

/// The toast's XML. The text is escaped, so no word can end the element it is in.
pub(crate) fn toast_xml(words: &Words) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        escape(words.title),
        escape(words.body),
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// The AppUserModelID of an identity: the real app and the test build never share one, so a toast of the test build
/// carries the test build's name and its switches in Windows Settings stay apart from the real app's.
pub(crate) fn aumid(identity: AppIdentity) -> String {
    identity.bundle_identifier().to_string()
}

/// Show the toast of a kind, whatever the switches say. Errors are the caller's to ignore: no toast is no harm.
fn show_kind(identity: AppIdentity, kind: Kind) -> Result<(), String> {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;

    let xml = toast_xml(&words(kind));
    let document = XmlDocument::new().map_err(|e| e.to_string())?;
    document.LoadXml(&HSTRING::from(xml)).map_err(|e| e.to_string())?;
    let toast = ToastNotification::CreateToastNotification(&document).map_err(|e| e.to_string())?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid(identity)))
        .map_err(|e| e.to_string())?;
    notifier.Show(&toast).map_err(|e| e.to_string())
}

/// The identity `start` was handed by the shell; `show` reads it. Unset: no toast.
static KEPT_IDENTITY: std::sync::OnceLock<AppIdentity> = std::sync::OnceLock::new();

/// What Windows needs before this app can show a toast: its identity. Registers the AppUserModelID of an
/// unpackaged app (display name and icon, under HKCU) and sets it as this process's. A failure is logged and ends
/// nothing: the toasts do not show, the app works.
pub(crate) fn start(identity: AppIdentity) {
    // `show` is handed no identity (its caller keeps `show(notice)`), so the one `start` was given is kept here.
    let _ = KEPT_IDENTITY.set(identity);
    if register(identity).is_err() {
        tracing::warn!("toast identity not set");
    }
}

fn register(identity: AppIdentity) -> Result<(), String> {
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::core::HSTRING;

    let key = format!("Software\\Classes\\AppUserModelId\\{}", aumid(identity));
    set_string(&key, "DisplayName", identity.product_name())?;
    // The icon is the app's own file beside the program, when it is there; without it Windows shows the program's.
    if let Some(icon) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("beebeeb.ico")))
        .filter(|path| path.is_file())
    {
        set_string(&key, "IconUri", &icon.to_string_lossy())?;
    }
    // SAFETY: a plain call with a NUL-terminated string that lives for the call.
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(aumid(identity))) }.map_err(|e| e.to_string())
}

/// One string value under `HKEY_CURRENT_USER`, the key made when missing.
fn set_string(key: &str, name: &str, value: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};

    let wide = |text: &str| text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (key_w, name_w, value_w) = (wide(key), wide(name), wide(value));
    // SAFETY: all three are NUL-terminated and live for the call; the size counts the terminator in bytes.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key_w.as_ptr(),
            name_w.as_ptr(),
            REG_SZ,
            value_w.as_ptr().cast(),
            (value_w.len() * 2) as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("registry write failed: Win32 error {status}"))
    }
}

/// One notice of the engine, as the window drains it: a toast when its switch is on (Security: always). The
/// switches are read now; a settings file that cannot be read means the defaults.
pub(crate) fn show(notice: &Notice) {
    let Some(identity) = KEPT_IDENTITY.get().copied() else {
        tracing::warn!("toast not shown");
        return;
    };
    let switches =
        Switches::from_config(&DesktopConfig::load_for(crate::shell::identity::engine_identity()).unwrap_or_default());
    let kind = kind_of(notice);
    if !wanted(kind, &switches) {
        return;
    }
    if show_kind(identity, kind).is_err() {
        tracing::warn!("toast not shown");
    }
}

/// DW-A11: a sign-in request arrived. Always shown (security); the app opens on click and the Approve page
/// holds the request.
pub(crate) fn show_sign_in_request() {
    let Some(identity) = KEPT_IDENTITY.get().copied() else {
        return;
    };
    if show_kind(identity, Kind::SignInRequest).is_err() {
        tracing::warn!("toast not shown");
    }
}

/// DW-1182: "Beebeeb manages this folder", for the callers that raise it directly.
#[allow(dead_code)]
pub(crate) fn show_service_folder_refused() {
    show(&Notice::ServiceFolderRefused);
}

#[cfg(test)]
#[path = "windows_toast_tests.rs"]
mod tests;
