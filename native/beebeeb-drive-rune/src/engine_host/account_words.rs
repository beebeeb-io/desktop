//! The account source's words: initials, how long ago, and the kind of client a session is. Moved out of
//! `account.rs` unchanged when that file passed the size limit.

use beebeeb_engine::account_dto::Subscription;
use chrono::{DateTime, Local};

use crate::frames::account::source::{DeviceKind, Plan, PlanDetails};
// ONE size formatter: the one behind the app's storage figure.
use crate::frames::global::source::size_words;

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

/// The plan as Account and the plan dialog draw it. `None`: Beebeeb named no size.
///
/// How the period ends is said from `status`, in Beebeeb's own words for it (`active`, `trialing`, `cancelling`,
/// `paused`, `past_due`, `cancelled`), and only for the ones this function knows: a plan that renews, a plan
/// that was cancelled and runs to the end of its period, a trial (Beebeeb sets the period's end to the trial's
/// end), a paused plan. Any other status, or no day: "used" and nothing more. Never "renews" for a plan that
/// does not.
pub(super) fn plan_of(plan: &Subscription) -> Option<(Plan, Option<PlanDetails>)> {
    if plan.quota_bytes <= 0 {
        return None;
    }
    let size_of = |bytes: i64| size_words(bytes.max(0) as u64);
    let (size, used) = (size_of(plan.quota_bytes), size_of(plan.used_bytes));
    let day = plan
        .current_period_end
        .as_deref()
        .and_then(|end| DateTime::parse_from_rfc3339(end).ok())
        .map(|end| end.with_timezone(&Local).format("%-d %b %Y").to_string());
    // The card's words, and the dialog's row (its label and the day).
    let (rest, dated) = match (plan.status.as_str(), day) {
        ("active", Some(day)) => (format!("used \u{b7} renews {day}"), Some(("Renews", day))),
        ("cancelling", Some(day)) => (format!("used \u{b7} cancels on {day}"), Some(("Cancels on", day))),
        ("trialing", Some(day)) => (format!("used \u{b7} trial ends {day}"), Some(("Trial ends", day))),
        ("paused", _) => ("used \u{b7} paused".into(), None),
        _ => ("used".into(), None),
    };
    let billing = match plan.billing_cycle.to_ascii_lowercase().as_str() {
        "yearly" | "annual" | "year" => Some("Yearly"),
        "monthly" | "month" => Some("Monthly"),
        _ => None,
    };
    let card = Plan {
        title: format!("Drive plan \u{b7} {size}"),
        used: used.clone(),
        rest,
    };
    // The dialog draws a day and a billing period: without both it is not opened (the card then leads to the
    // plan page on the website, as the trait says).
    let details = dated.zip(billing).map(|((day_label, renews), billing)| PlanDetails {
        size,
        used,
        share: (plan.used_bytes.max(0) as f32 / plan.quota_bytes as f32).clamp(0.0, 1.0),
        day_label,
        renews,
        billing: billing.into(),
    });
    Some((card, details))
}
