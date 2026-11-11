//! `beebeeb://` links: untrusted input, parsed by one pure function against an allow-list.
//!
//! A link can only ask the window to SHOW a place: a sidebar section, a folder in it, or a
//! settings pane. It becomes an [`Intent`], which the route machine holds back while the
//! session gate is closed. There is no link that signs in, shares, deletes, pauses or changes a
//! setting, and none may be added without the window confirming the action first.
//!
//! Accepted, and nothing else:
//!
//! | link | shows |
//! |---|---|
//! | `beebeeb://open` | Files |
//! | `beebeeb://open/<section>` | that section (`files`, `shared`, `photos`, `backups`, `trash`, `sync-history`, `available-offline`) |
//! | `beebeeb://open/<section>?folder=<id>` | that folder; the id is 1 to 64 of `A-Z a-z 0-9 - _` |
//! | `beebeeb://settings` | Settings, General |
//! | `beebeeb://settings/<pane>` | that pane (`general`, `account`, `security`, `devices`, `notifications`, `storage`, `backups`, `sync`, `previews`) |
//!
//! The Tauri app (0.8.x) registers no URI scheme, so nothing depends on these yet; the scheme's
//! registry entries are [`scheme_registration`], for the installer to write.

use super::app::{Intent, Route, Section, SettingsPane};
use super::identity::AppIdentity;

/// Longer links are refused before anything looks at them.
pub const MAX_LEN: usize = 512;
pub const MAX_ID_LEN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeepLinkError {
    TooLong,
    /// Not `beebeeb://`.
    NotOurScheme,
    /// A character or part a link of ours never has (spaces, `%`, `#`, `@`, a port, non-ASCII).
    Malformed,
    /// A host, section or pane that is not on the list.
    UnknownTarget,
    /// A query that is not exactly one well-formed `folder=<id>`, or one where none belongs.
    BadParameter,
}

impl DeepLinkError {
    /// For the log. Never includes the link: it is someone else's text.
    pub fn reason(self) -> &'static str {
        match self {
            DeepLinkError::TooLong => "too long",
            DeepLinkError::NotOurScheme => "not a beebeeb link",
            DeepLinkError::Malformed => "malformed",
            DeepLinkError::UnknownTarget => "unknown target",
            DeepLinkError::BadParameter => "bad parameter",
        }
    }
}

/// Whether `id` may be handed to the engine as an item id.
pub fn is_item_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A link for THIS build's identity.
pub fn parse(link: &str) -> Result<Intent, DeepLinkError> {
    parse_for(link, AppIdentity::CURRENT)
}

pub fn parse_for(link: &str, identity: AppIdentity) -> Result<Intent, DeepLinkError> {
    if link.len() > MAX_LEN {
        return Err(DeepLinkError::TooLong);
    }
    let Some((scheme, rest)) = link.split_once("://") else {
        return Err(DeepLinkError::NotOurScheme);
    };
    if !scheme.eq_ignore_ascii_case(identity.url_scheme()) {
        return Err(DeepLinkError::NotOurScheme);
    }
    // Everything a link of ours can contain. No percent-encoding: nothing on the list needs
    // it, and decoding is where traversal and smuggling hide.
    if !rest
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/' | b'?' | b'='))
    {
        return Err(DeepLinkError::Malformed);
    }
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    let path = path.strip_suffix('/').unwrap_or(path);
    let mut parts = path.split('/');
    let host = parts.next().unwrap_or_default();
    let target = parts.next();
    if parts.next().is_some() || target == Some("") {
        return Err(DeepLinkError::Malformed);
    }
    match host {
        "open" => {
            let section = match target {
                None => Section::Files,
                Some(slug) => Section::from_slug(slug).ok_or(DeepLinkError::UnknownTarget)?,
            };
            let folder = match query {
                None => None,
                Some(query) => {
                    let id = query.strip_prefix("folder=").ok_or(DeepLinkError::BadParameter)?;
                    if !is_item_id(id) {
                        return Err(DeepLinkError::BadParameter);
                    }
                    Some(id.to_owned())
                }
            };
            Ok(Intent::Go(Route {
                section,
                folder,
                item: None,
            }))
        }
        "settings" => {
            if query.is_some() {
                return Err(DeepLinkError::BadParameter);
            }
            let pane = match target {
                None => SettingsPane::General,
                Some(slug) => SettingsPane::from_slug(slug).ok_or(DeepLinkError::UnknownTarget)?,
            };
            Ok(Intent::Settings(pane))
        }
        _ => Err(DeepLinkError::UnknownTarget),
    }
}

/// One value under `HKEY_CURRENT_USER`.
#[allow(dead_code)] // read by the installer lane; pinned by a test below
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryValue {
    pub key: String,
    /// Empty: the key's default value.
    pub name: &'static str,
    pub data: String,
}

/// What registers the scheme for this user on Windows, for the installer (and its removal on
/// uninstall: delete `Software\Classes\<scheme>`). **Not written in 0.9.0** (the scheme stays
/// unregistered); the shell never writes these itself.
///
/// The command is `"<exe>" --open-link "%1"`. Windows substitutes `%1` verbatim, so a link
/// that contains a quote can close the argument and append others; that is why the app takes a
/// link ONLY after `--open-link`, and honours no other flag on a command line that has it
/// (`instance::parse_args`).
#[allow(dead_code)] // for the installer lane; pinned by a test below
pub fn scheme_registration(identity: AppIdentity, exe: &str) -> Vec<RegistryValue> {
    let root = format!("Software\\Classes\\{}", identity.url_scheme());
    vec![
        RegistryValue {
            key: root.clone(),
            name: "",
            data: identity.url_scheme_label().into(),
        },
        RegistryValue {
            key: root.clone(),
            name: "URL Protocol",
            data: String::new(),
        },
        RegistryValue {
            key: format!("{root}\\shell\\open\\command"),
            name: "",
            data: format!(
                "{} {} \"%1\"",
                super::autostart::quote_arg_always(exe),
                super::instance::OPEN_LINK_FLAG
            ),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go(section: Section, folder: Option<&str>) -> Result<Intent, DeepLinkError> {
        Ok(Intent::Go(Route {
            section,
            folder: folder.map(str::to_owned),
            item: None,
        }))
    }

    #[test]
    fn the_allow_list() {
        assert_eq!(parse("beebeeb://open"), go(Section::Files, None));
        assert_eq!(parse("beebeeb://open/"), go(Section::Files, None));
        for section in Section::ALL {
            assert_eq!(parse(&format!("beebeeb://open/{}", section.slug())), go(section, None));
        }
        assert_eq!(parse("BeeBeeb://open/trash/"), go(Section::Trash, None));
        assert_eq!(
            parse("beebeeb://open/files?folder=0b6f3c1e-9a52-4d0e-8f3a-2c1d5e7a9b10"),
            go(Section::Files, Some("0b6f3c1e-9a52-4d0e-8f3a-2c1d5e7a9b10"))
        );
        assert_eq!(parse("beebeeb://settings"), Ok(Intent::Settings(SettingsPane::General)));
        for pane in SettingsPane::ALL {
            assert_eq!(
                parse(&format!("beebeeb://settings/{}", pane.slug())),
                Ok(Intent::Settings(pane))
            );
        }
    }

    #[test]
    fn other_schemes_and_non_links_are_not_ours() {
        for link in [
            "",
            "beebeeb",
            "beebeeb:open",
            "https://beebeeb.io/open",
            "file:///C:/x",
            "beebeebx://open",
            "C:\\x.txt",
        ] {
            assert_eq!(parse(link), Err(DeepLinkError::NotOurScheme), "{link:?}");
        }
    }

    #[test]
    fn unknown_targets_are_refused() {
        for link in [
            "beebeeb://",
            "beebeeb://delete",
            "beebeeb://sign-out",
            "beebeeb://open/Files",
            "beebeeb://open/everything",
            "beebeeb://settings/billing",
            "beebeeb://pause",
            "beebeeb://share/abc",
        ] {
            assert_eq!(parse(link), Err(DeepLinkError::UnknownTarget), "{link:?}");
        }
    }

    #[test]
    fn malformed_links_are_refused() {
        for link in [
            "beebeeb://open/files/extra",
            "beebeeb://open//",
            "beebeeb://open/../settings",
            "beebeeb://open/files#x",
            "beebeeb://user@open/files",
            "beebeeb://open:8080/files",
            "beebeeb://open/fi les",
            "beebeeb://open/files%2F..",
            "beebeeb://open/files\\..",
            "beebeeb://open/files\n",
            "beebeeb://open/files\0",
            "beebeeb://open/fïles",
            "beebeeb://open/files?folder=a&x=1",
            "beebeeb://open/files?folder=a;rm",
        ] {
            assert_eq!(parse(link), Err(DeepLinkError::Malformed), "{link:?}");
        }
    }

    #[test]
    fn parameters_are_exactly_one_folder_id_and_only_on_open() {
        let long_id = "a".repeat(MAX_ID_LEN + 1);
        for link in [
            "beebeeb://open/files?",
            "beebeeb://open/files?folder=",
            "beebeeb://open/files?item=abc",
            "beebeeb://open/files?folder=a?folder=b",
            "beebeeb://open/files?folder=a/b",
            "beebeeb://open/files?folder=a=b",
            &format!("beebeeb://open/files?folder={long_id}"),
            "beebeeb://settings?folder=abc",
            "beebeeb://settings/general?folder=abc",
        ] {
            assert_eq!(parse(link), Err(DeepLinkError::BadParameter), "{link:?}");
        }
        assert!(parse(&format!("beebeeb://open/files?folder={}", "a".repeat(MAX_ID_LEN))).is_ok());
    }

    #[test]
    fn oversized_links_are_refused_before_parsing() {
        let link = format!("beebeeb://open/files?folder={}", "a".repeat(MAX_LEN));
        assert_eq!(parse(&link), Err(DeepLinkError::TooLong));
        // Also when it is not ours at all: length is checked first.
        assert_eq!(parse(&"x".repeat(MAX_LEN + 1)), Err(DeepLinkError::TooLong));
        assert_eq!(parse(&"x".repeat(MAX_LEN)), Err(DeepLinkError::NotOurScheme));
    }

    #[test]
    fn no_link_yields_anything_but_navigation() {
        // The type says it: `Intent` has no variant that changes data. This pins the hosts.
        for host in ["open", "settings"] {
            assert!(parse(&format!("beebeeb://{host}")).is_ok());
        }
        for host in ["quit", "resume", "unlock", "sign-in", "autostart", "review"] {
            assert!(parse(&format!("beebeeb://{host}")).is_err(), "{host}");
        }
    }

    #[test]
    fn the_scheme_registration_passes_the_link_only_after_open_link() {
        let values = scheme_registration(AppIdentity::Real, "C:\\Program Files\\Beebeeb\\Beebeeb.exe");
        assert_eq!(values.len(), 3);
        assert_eq!(values[0].data, "URL:Beebeeb");
        assert_eq!(values[1].name, "URL Protocol");
        assert_eq!(values[2].key, "Software\\Classes\\beebeeb\\shell\\open\\command");
        assert_eq!(
            values[2].data,
            "\"C:\\Program Files\\Beebeeb\\Beebeeb.exe\" --open-link \"%1\""
        );
        // A path without a space is quoted too.
        let values = scheme_registration(AppIdentity::Test, "C:\\b\\Beebeeb.exe");
        assert_eq!(values[2].key, "Software\\Classes\\beebeeb-test\\shell\\open\\command");
        assert_eq!(values[2].data, "\"C:\\b\\Beebeeb.exe\" --open-link \"%1\"");
    }

    #[test]
    fn a_link_of_the_other_identity_is_not_ours() {
        assert!(parse_for("beebeeb://open", AppIdentity::Real).is_ok());
        assert_eq!(
            parse_for("beebeeb://open", AppIdentity::Test),
            Err(DeepLinkError::NotOurScheme)
        );
        assert!(parse_for("beebeeb-test://open", AppIdentity::Test).is_ok());
        assert_eq!(
            parse_for("beebeeb-test://open", AppIdentity::Real),
            Err(DeepLinkError::NotOurScheme)
        );
    }
}
