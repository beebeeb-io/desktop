//! The account source's words: initials, how long ago, and the kind of client a session is. Moved out of
//! `account.rs` unchanged when that file passed the size limit.

use crate::frames::account::source::DeviceKind;

/// Up to two letters: the first of the first word and of the last.
pub(crate) fn initials_of(name: &str) -> String {
    let mut words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    let first = words.next().and_then(|w| w.chars().next());
    let last = words.last().and_then(|w| w.chars().next());
    first.into_iter().chain(last).flat_map(char::to_uppercase).collect()
}

/// "now", "12 min ago", "3 hours ago", "5 days ago"; "not recorded" when Beebeeb named no time.
pub(crate) fn ago(at: Option<i64>, now: i64) -> String {
    let Some(at) = at else {
        return "not recorded".into();
    };
    let seconds = (now - at).max(0);
    match seconds {
        0..=59 => "now".into(),
        60..=3_599 => format!("{} min ago", seconds / 60),
        3_600..=7_199 => "1 hour ago".into(),
        7_200..=86_399 => format!("{} hours ago", seconds / 3_600),
        86_400..=172_799 => "1 day ago".into(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// The icon and the words for a session's `device_kind`, as Beebeeb stores it (web, desktop, cli, ios, android,
/// app, api). Beebeeb names no product per session, so the row says the kind of client, never a product's name;
/// `ios` and `android` are a phone's app or its browser, so the words name the system only. A kind this list
/// does not know is "Other client": never a guess.
pub(super) fn kind_of(device_kind: &str) -> (DeviceKind, &'static str) {
    match device_kind {
        "web" => (DeviceKind::Browser, "Web browser"),
        "desktop" => (DeviceKind::Laptop, "Desktop app"),
        "cli" => (DeviceKind::CommandLine, "Command line"),
        "ios" => (DeviceKind::Phone, "iPhone or iPad"),
        "android" => (DeviceKind::Phone, "Android"),
        "app" => (DeviceKind::Laptop, "Beebeeb app"),
        _ => (DeviceKind::Browser, "Other client"),
    }
}
