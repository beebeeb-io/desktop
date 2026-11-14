//! What the account panes draw, made from what Beebeeb answered: a session's row, the name and the plan of
//! Account, and the words for a plan's period, a time and a kind of client. Pure: nothing is asked here.
//!
//! Must not: print what it holds (no type here has `Debug`: an email, a name, a device's name).

use beebeeb_engine::account_dto::{AccountSession, Subscription};
use beebeeb_engine::app::account_api::AccountBasics;
use chrono::{DateTime, Local, Utc};

// ONE size formatter: the one behind the app's storage figure.
use crate::frames::account::source::{DeviceKind, Identity, Plan, PlanDetails};
use crate::frames::global::source::size_words;

/// One session as Beebeeb lists it. No `Debug`: it holds a device's name.
#[derive(Clone, PartialEq)]
pub(super) struct Row {
    pub(super) id: String,
    pub(super) kind: DeviceKind,
    /// The kind of client, in the row's words (`kind_of`).
    pub(super) client: &'static str,
    pub(super) name: String,
    /// Unix seconds of the last activity, when Beebeeb named one that reads.
    pub(super) at: Option<i64>,
    pub(super) country: String,
    pub(super) current: bool,
}

/// Where a part of Account (the name, the plan) comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    /// This read answered it.
    Read,
    /// This read did not; an earlier read of the same session and account did, and its value stands.
    Before,
    /// Never read: the email stands in for the name, no plan card is drawn.
    Missing,
}

/// What Account draws. No `Debug`: it holds the email and the name.
#[derive(Clone)]
pub(super) struct Overview {
    pub(super) identity: Identity,
    pub(super) plan: Option<(Plan, Option<PlanDetails>)>,
    /// Two-factor, as the profile marks it. `None`: Beebeeb did not say.
    pub(super) two_factor: Option<bool>,
    pub(super) name_from: Part,
    pub(super) plan_from: Part,
}

impl Overview {
    /// Every part answered in this read. `false`: what is drawn stands, but it is not fresh (the next opening
    /// reads again), and the pane says so.
    pub(super) fn whole(&self) -> bool {
        self.name_from == Part::Read && self.plan_from == Part::Read
    }

    /// This read, over the one that was drawn for the same session and account: a part this read could not
    /// get keeps the value `before` had (never the email in place of a name that was drawn, never a card that
    /// disappears).
    pub(super) fn over(mut self, before: &Overview) -> Self {
        if self.name_from == Part::Missing && before.name_from != Part::Missing {
            self.identity.name = before.identity.name.clone();
            self.identity.initials = before.identity.initials.clone();
            self.name_from = Part::Before;
        }
        if self.plan_from == Part::Missing && before.plan_from != Part::Missing {
            self.plan = before.plan.clone();
            self.plan_from = Part::Before;
        }
        self
    }
}

/// Up to two letters: the first of the first word and of the last.
pub(super) fn initials_of(name: &str) -> String {
    let mut words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    let first = words.next().and_then(|w| w.chars().next());
    let last = words.last().and_then(|w| w.chars().next());
    first.into_iter().chain(last).flat_map(char::to_uppercase).collect()
}

/// "now", "12 min ago", "3 hours ago", "5 days ago"; "not recorded" when Beebeeb named no time.
pub(super) fn ago(at: Option<i64>, now: i64) -> String {
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

pub(super) fn row_of(session: AccountSession) -> Row {
    let country = session
        .country_code
        .map(|code| code.trim().to_ascii_uppercase())
        // Two letters: a country, never anything longer (a city can never reach the pane).
        .filter(|code| code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()))
        .unwrap_or_default();
    let (kind, client) = kind_of(&session.device_kind);
    Row {
        id: session.id,
        kind,
        client,
        name: session.device_name,
        at: session
            .last_active_at
            .as_deref()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp()),
        country,
        current: session.is_current,
    }
}

/// The plan as Account and the plan dialog draw it. `None`: Beebeeb named no size.
///
/// How the period ends is said from `status`, in Beebeeb's own words for it (`active`, `trialing`, `cancelling`,
/// `paused`, `past_due`, `cancelled`), and only for the ones this function knows: a plan that renews, a plan
/// that was cancelled and runs to the end of its period, a trial (Beebeeb sets the period's end to the trial's
/// end), a paused plan. Any other status, or no day: "used" and nothing more.
///
/// Never "renews" for a plan that does not: Beebeeb also sends `active` with a period end for a plan nothing is
/// charged for (`Subscription::is_mock`), so "renews" needs Beebeeb to say the plan is not one of those. When it
/// does not say (the field is missing), less is said. And no day that has passed is drawn as one to come, for
/// any status.
pub(super) fn plan_of(plan: &Subscription, now: DateTime<Utc>) -> Option<(Plan, Option<PlanDetails>)> {
    if plan.quota_bytes <= 0 {
        return None;
    }
    let size_of = |bytes: i64| size_words(bytes.max(0) as u64);
    let (size, used) = (size_of(plan.quota_bytes), size_of(plan.used_bytes));
    let day = plan
        .current_period_end
        .as_deref()
        .and_then(|end| DateTime::parse_from_rfc3339(end).ok())
        .filter(|end| *end > now)
        .map(|end| end.with_timezone(&Local).format("%-d %b %Y").to_string());
    let charged_again = plan.is_mock == Some(false);
    // The card's words, and the dialog's row (its label and the day).
    let (rest, dated) = match (plan.status.as_str(), day) {
        ("active", Some(day)) if charged_again => (format!("used \u{b7} renews {day}"), Some(("Renews", day))),
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

/// What a sign-out of this app answers here: it is not bound, and the page draws no control for it.
pub(super) const NOT_BOUND: &str = "Signing out is not available here yet. Nothing was signed out.";

/// What Account draws, from the one read. The profile answered (else the read failed). A name that could not be
/// read is "no name": the email stands in. A plan that could not be read hides the card (never a guessed plan).
/// (A 401 on either is the whole call's refusal, in the engine.)
pub(super) fn overview_of(basics: AccountBasics, now: DateTime<Utc>) -> Overview {
    let from = |read: bool| if read { Part::Read } else { Part::Missing };
    let (name_from, plan_from) = (from(basics.display_name.is_ok()), from(basics.subscription.is_ok()));
    let two_factor = basics.profile.totp_enabled;
    let email = basics.profile.email;
    let name = basics.display_name.ok().flatten();
    // The email standing in for the name gives the initials of its part before the `@`: never a letter of
    // the domain. A name that was set is taken whole.
    let initials = initials_of(
        name.as_deref()
            .unwrap_or_else(|| email.split('@').next().unwrap_or_default()),
    );
    Overview {
        identity: Identity {
            initials,
            name: name.unwrap_or_else(|| email.clone()),
            email,
        },
        plan: basics.subscription.ok().and_then(|plan| plan_of(&plan, now)),
        two_factor,
        name_from,
        plan_from,
    }
}
