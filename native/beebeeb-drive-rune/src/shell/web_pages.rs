//! The addresses the window sends the person to, in ONE table. A frame asks by NAME; the shell holds the
//! address. A name with no address here is a control the frame hides (never a dead link). Every address
//! has its source in the repo it points into; one that could not be confirmed stays out.

use crate::engine_host::drive_settings::tools::{SUPPORT, percent};

/// Recovery replaces the password and re-wraps the keys (`repos/web`, route `/forgot-password`).
pub const RECOVERY_PAGE: &str = "https://app.beebeeb.io/forgot-password";
/// Creating an account on the web, where this build has no sign-up screens (`repos/web`, route `/signup`).
pub const SIGN_UP_PAGE: &str = "https://app.beebeeb.io/signup";
/// Sign-in's Terms (`repos/site`: `legalDocs.ts`, slug `terms`).
pub const TERMS_PAGE: &str = "https://beebeeb.io/terms";
/// Sign-in's Privacy (`repos/site/src/pages/privacy.astro`).
pub const PRIVACY_PAGE: &str = "https://beebeeb.io/privacy";
/// Windows Settings › Notifications.
pub const WINDOWS_NOTIFICATIONS: &str = "ms-settings:notifications";

/// Every name a frame may ask for, with its address. Names the frames use and this table lacks:
/// `about-audits` (the site has no audits section: `repos/site/src/pages/security.astro` holds only
/// responsible disclosure), so About does not draw that row.
pub const NAMES: &[(&str, &str)] = &[
    // `repos/site/src/pages/legal.astro`.
    ("terms-and-privacy-policy", "https://beebeeb.io/legal"),
    // The public core repository: a link, not data.
    ("encryption-core-on-github", "https://github.com/beebeeb-io/core"),
    ("beebeeb.io", "https://beebeeb.io"),
    // `repos/web/src/app.tsx`, route `/settings/billing`.
    ("plan", "https://app.beebeeb.io/settings/billing"),
    // `repos/web/src/app.tsx`, route `/settings/delete-account`.
    ("delete-account", "https://app.beebeeb.io/settings/delete-account"),
    // `repos/web/src/app.tsx`, route `/settings/privacy`.
    ("privacy", "https://app.beebeeb.io/settings/privacy"),
    // The web has no route of its own for the data export; it runs on the privacy page (`repos/web`,
    // `pages/settings/privacy`).
    ("export-my-data", "https://app.beebeeb.io/settings/privacy"),
    // `SUPPORT`; the subject is fixed here (see [`support_mailto`] for one with its own subject).
    ("support", "mailto:support@beebeeb.io?subject=Beebeeb%20support"),
];

/// The address of the named page, `None` when this build has no address for it.
pub fn url(name: &str) -> Option<&'static str> {
    NAMES.iter().find(|(n, _)| *n == name).map(|(_, address)| *address)
}

/// A draft to support with this subject (percent-encoded with the encoder `mailto:` drafts already use).
pub fn support_mailto(subject: &str) -> String {
    format!("mailto:{SUPPORT}?subject={}", percent(subject))
}

#[cfg(test)]
#[path = "web_pages_tests.rs"]
mod tests;
