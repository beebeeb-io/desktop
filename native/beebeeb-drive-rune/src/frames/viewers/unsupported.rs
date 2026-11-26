//! The page for a file this app cannot show: a kind with no viewer, a player that refused, an office helper that
//! cannot run here. One calm page instead of an error: the file's icon and name, its size, that Beebeeb can't show this
//! kind of file here, where the file is (on this PC or in the cloud), and three ways out: "Open with <default app>",
//! "Open with another app…" and "Show in Finder" (File Explorer on Windows). A real network or decrypt failure is not
//! this page: it keeps its own error with Try again (`states::failed`, `states::stopped`).
//!
//! Other modules reach it through [`open_unsupported`].

use rune::El;

use super::source::{Body, NoPreview, OpenFile, Origin};
use super::{icons, Intent};
use crate::theme::tokens::Theme;
use crate::ui::button::{button, keys, ButtonState, Size, Variant};
use crate::State;

/// Why a file is shown as the unsupported page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedReason {
    /// No viewer for this kind of file.
    UnknownType,
    /// The player refused the recording or video.
    PlayerRefused,
    /// An office document, and the office helper cannot run here.
    OfficeUnavailable,
}

impl UnsupportedReason {
    /// The reason a [`NoPreview`] is drawn as this page (`None`: it has its own page).
    pub(super) fn of(why: NoPreview) -> Option<Self> {
        match why {
            NoPreview::NoViewer => Some(Self::UnknownType),
            NoPreview::PlayerRefused => Some(Self::PlayerRefused),
            NoPreview::OfficeUnavailable => Some(Self::OfficeUnavailable),
            _ => None,
        }
    }

    fn preview(self) -> NoPreview {
        match self {
            Self::UnknownType => NoPreview::NoViewer,
            Self::PlayerRefused => NoPreview::PlayerRefused,
            Self::OfficeUnavailable => NoPreview::OfficeUnavailable,
        }
    }
}

/// Shows `file_ref` (its id and name stay) as the unsupported page for `reason`: whatever it held is wiped first.
/// Another module (the office viewer, when its helper cannot run) calls this in place of an error.
#[allow(dead_code)] // the office viewer (task 64) calls it
pub fn open_unsupported(file_ref: &mut OpenFile, reason: UnsupportedReason) {
    file_ref.wipe();
    file_ref.body = Body::NotPreviewable(reason.preview());
}

/// The sentence under the name.
pub const SENTENCE: &str = "Beebeeb can\u{2019}t show this kind of file here.";

/// "Open with <app>" for the default app's name; "Open with default app" while it is not known.
pub fn open_label(default_app: Option<&str>) -> String {
    match default_app {
        Some(app) => format!("Open with {app}"),
        None => "Open with default app".to_string(),
    }
}

/// "Show in Finder" / "Show in File Explorer".
pub fn show_label() -> String {
    format!("Show in {}", beebeeb_engine::os_words::file_manager())
}

/// Where the file is, in the page's words.
fn local_state(origin: Origin) -> &'static str {
    match origin {
        Origin::ThisPc { .. } | Origin::ThisPcSyncOff => {
            concat!("Downloaded to ", beebeeb_engine::device_word!(), ".")
        }
        _ => "In the cloud, safe in Beebeeb.",
    }
}

fn ext_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, ext)| ext).filter(|ext| !ext.is_empty())
}

/// The page.
pub(super) fn page(file: &OpenFile, _reason: UnsupportedReason, copies: bool, origin: Origin, t: &Theme) -> El {
    let default_app =
        ext_of(&file.name).and_then(|ext| crate::engine_host::open_with_apps::default_app_name(ext, || {}));
    let body = format!("{} {} {}", file.facts, SENTENCE, local_state(origin));
    let press = |intent: Intent| {
        move |s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.intent(intent.clone());
            }
        }
    };
    let mut buttons = vec![
        button(
            keys("state-btn-0"),
            &open_label(default_app.as_deref()),
            None,
            Variant::Primary,
            Size::Md,
            ButtonState::Enabled,
            t,
        )
        .on_press(press(Intent::OpenInAnotherApp)),
        button(
            keys("state-btn-1"),
            "Open with another app\u{2026}",
            None,
            Variant::Secondary,
            Size::Md,
            ButtonState::Enabled,
            t,
        )
        .on_press(press(Intent::ChooseApp)),
        button(
            keys("state-btn-2"),
            &show_label(),
            None,
            Variant::Secondary,
            Size::Md,
            ButtonState::Enabled,
            t,
        )
        .on_press(press(Intent::ShowInFolder)),
    ];
    // "Save a copy…" only where there is somewhere to save to (never a button that only says "not built").
    if copies {
        buttons.push(
            button(
                keys("state-btn-3"),
                "Save a copy\u{2026}",
                None,
                Variant::Ghost,
                Size::Md,
                ButtonState::Enabled,
                t,
            )
            .on_press(press(Intent::SaveCopy)),
        );
    }
    super::states::empty_view(&icons::INFO, &file.name, body.trim(), buttons, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kinds_without_a_viewer_get_the_page_and_the_rest_keep_their_own() {
        for (why, reason) in [
            (NoPreview::NoViewer, UnsupportedReason::UnknownType),
            (NoPreview::PlayerRefused, UnsupportedReason::PlayerRefused),
            (NoPreview::OfficeUnavailable, UnsupportedReason::OfficeUnavailable),
        ] {
            assert_eq!(UnsupportedReason::of(why), Some(reason));
            assert_eq!(reason.preview(), why);
        }
        assert_eq!(UnsupportedReason::of(NoPreview::PdfLocked), None);
        assert_eq!(UnsupportedReason::of(NoPreview::Unreadable), None);
    }

    #[test]
    fn labels_and_local_state() {
        assert_eq!(open_label(Some("Preview")), "Open with Preview");
        assert_eq!(open_label(None), "Open with default app");
        assert!(local_state(Origin::ThisPc { unsent: false }).starts_with("Downloaded to "));
        assert!(local_state(Origin::Beebeeb).starts_with("In the cloud"));
        assert_eq!(ext_of("a.eps"), Some("eps"));
        assert_eq!(ext_of("noext"), None);
    }
}
