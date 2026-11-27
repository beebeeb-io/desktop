//! The page for a file this app cannot show: a kind with no viewer, a player that refused, an office helper that
//! cannot run here. One calm page instead of an error: the file's icon and name, its size, that Beebeeb can't show this
//! kind of file here, where the file is (on this PC or in the cloud), and three ways out: "Open with <default app>",
//! "Open with another app…" and "Show in Finder" (File Explorer on Windows). A real network or decrypt failure is not
//! this page: it keeps its own error with Try again (`states::failed`, `states::stopped`).
//!
//! The page says why it is shown ([`UnsupportedReason::sentence`]) and what became of the last Open with press on
//! the file ([`Note`]): that it is coming down to this Mac first, or why it did not open. A refusal is words on the
//! page, never a toast alone. Notes are fixed words, kept per file id for the run of the app; never a name.
//!
//! Other modules reach it through [`open_unsupported`], [`set_note`] and [`note_of`].

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use rune::El;

use super::source::{Body, NoPreview, OpenFile, Origin};
use super::{Intent, icons};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::button::{ButtonState, Size, Variant, button, keys};

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
    pub(crate) fn of(why: NoPreview) -> Option<Self> {
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

    /// Why the page is shown, as the sentence under the name.
    pub fn sentence(self) -> &'static str {
        match self {
            Self::UnknownType => SENTENCE,
            Self::PlayerRefused => PLAYER_REFUSED,
            Self::OfficeUnavailable => OFFICE_UNAVAILABLE,
        }
    }
}

/// What became of the last Open with press on a file (or of an open that failed before the page): the page says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// The file is coming down to this Mac before it opens.
    Downloading,
    /// It is here, and the app was asked to open it.
    Opening,
    /// Dm-83: the app (its name) was started with the file: it opened there, whatever the system said after.
    Opened(String),
    /// It did not open, and why, in words for a person.
    Failed(String),
}

impl Note {
    /// The words on the page.
    pub fn words(&self) -> String {
        match self {
            Note::Downloading => DOWNLOADING.to_string(),
            Note::Opening => OPENING.to_string(),
            Note::Opened(app) => opened_in(app),
            Note::Failed(why) => why.clone(),
        }
    }
}

/// What the page says while the file comes down before it opens.
pub const DOWNLOADING: &str = concat!(
    "Downloading it to ",
    beebeeb_engine::device_word!(),
    " first. It opens when it is here\u{2026}"
);
/// What the page says once the app was asked.
pub const OPENING: &str = "Opening it in the other app\u{2026}";

/// Dm-83: what the page says once the app was started with the file: "Opened in Adobe Illustrator."
pub fn opened_in(app: &str) -> String {
    format!("Opened in {app}.")
}

/// At most this many files' notes are held: the open file's, and the ones before it.
const NOTES: usize = 32;

/// The notes, newest last: `(file id, note)`. Fixed words only.
struct Notes(VecDeque<(String, Note)>);

impl Notes {
    fn set(&mut self, file_id: &str, note: Option<Note>) {
        self.0.retain(|(id, _)| id != file_id);
        if let Some(note) = note {
            if self.0.len() >= NOTES {
                self.0.pop_front();
            }
            self.0.push_back((file_id.to_string(), note));
        }
    }

    fn of(&self, file_id: &str) -> Option<Note> {
        self.0
            .iter()
            .find(|(id, _)| id == file_id)
            .map(|(_, note)| note.clone())
    }
}

static HELD: Mutex<Notes> = Mutex::new(Notes(VecDeque::new()));

/// Sets what the page of `file_id` says of its last press (`None`: nothing more to say).
pub fn set_note(file_id: &str, note: Option<Note>) {
    HELD.lock().unwrap_or_else(PoisonError::into_inner).set(file_id, note);
}

/// What the page of `file_id` says of its last press.
pub fn note_of(file_id: &str) -> Option<Note> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner).of(file_id)
}

/// Shows `file_ref` (its id and name stay) as the unsupported page for `reason`: whatever it held is wiped first.
/// Another module (the office viewer, when its helper cannot run) calls this in place of an error.
pub fn open_unsupported(file_ref: &mut OpenFile, reason: UnsupportedReason) {
    file_ref.wipe();
    file_ref.body = Body::NotPreviewable(reason.preview());
}

/// The sentence under the name of a kind with no viewer.
pub const SENTENCE: &str = "Beebeeb can\u{2019}t show this kind of file here.";
/// The sentence when the player refused the file itself.
pub const PLAYER_REFUSED: &str = concat!(
    beebeeb_engine::device_word_cap!(),
    " can\u{2019}t play this file, so Beebeeb can\u{2019}t show it here."
);
/// The sentence when the document's viewer cannot run here.
pub const OFFICE_UNAVAILABLE: &str = "Beebeeb can\u{2019}t show this document here.";

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

/// What the page says of a file whose bytes are on this Mac in Finder (dm-83).
pub const ON_THIS_DEVICE: &str = concat!("On ", beebeeb_engine::device_word!(), ".");
/// What the page says of a file whose bytes are only in the cloud.
pub const IN_THE_CLOUD: &str = "In the cloud, safe in Beebeeb.";

/// Where the file is, in the page's words. `here` (dm-83): Finder's own state of the file on a Mac, the one the rows'
/// marks show (`engine_host::macos_downloaded::finder_state`); `None` where there is none (not a Mac, not read yet).
fn local_state(origin: Origin, here: Option<bool>) -> &'static str {
    match (origin, here) {
        (_, Some(true)) => ON_THIS_DEVICE,
        (_, Some(false)) => IN_THE_CLOUD,
        (Origin::ThisPc { .. } | Origin::ThisPcSyncOff, None) => {
            concat!("Downloaded to ", beebeeb_engine::device_word!(), ".")
        }
        _ => IN_THE_CLOUD,
    }
}

/// Finder's state of the file `id` on a Mac (dm-83: the rows' marks), so the page says "On this Mac" for a file that
/// is there. `None` elsewhere, and while Finder has not said.
fn here_in_finder(id: &str) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        crate::engine_host::macos_downloaded::finder_state::here(id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        None
    }
}

fn ext_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, ext)| ext).filter(|ext| !ext.is_empty())
}

/// The page's words under the name: the file's facts, why it is not shown, where it is, and what became of the last
/// Open with press.
fn body_words(
    facts: &str,
    reason: UnsupportedReason,
    origin: Origin,
    here: Option<bool>,
    note: Option<&Note>,
) -> String {
    let mut body = format!("{facts} {} {}", reason.sentence(), local_state(origin, here));
    if let Some(note) = note {
        body.push(' ');
        body.push_str(&note.words());
    }
    body.trim().to_string()
}

/// A press of one of the page's ways out. A refusal the viewer would only say in a toast is said on the page.
fn press(intent: Intent) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        let Some(ui) = super::ui(s) else { return };
        let id = ui.file.id.clone();
        // The last press's words go: this press says its own.
        set_note(&id, None);
        ui.intent(intent.clone());
        if matches!(
            intent,
            Intent::OpenInAnotherApp | Intent::ChooseApp | Intent::ShowInFolder
        ) && let Some(why) = ui.toast.take()
        {
            tracing::warn!("unsupported page: a way out was refused");
            set_note(&id, Some(Note::Failed(why)));
        }
    }
}

/// The page.
pub(super) fn page(file: &OpenFile, reason: UnsupportedReason, copies: bool, origin: Origin, t: &Theme) -> El {
    let default_app =
        ext_of(&file.name).and_then(|ext| crate::engine_host::open_with_apps::default_app_name(ext, || {}));
    let note = note_of(&file.id);
    let body = body_words(&file.facts, reason, origin, here_in_finder(&file.id), note.as_ref());
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
    super::states::empty_view(&icons::INFO, &file.name, &body, buttons, t)
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
        assert!(local_state(Origin::ThisPc { unsent: false }, None).starts_with("Downloaded to "));
        assert!(local_state(Origin::Beebeeb, None).starts_with("In the cloud"));
        assert_eq!(ext_of("a.eps"), Some("eps"));
        assert_eq!(ext_of("noext"), None);
    }

    #[test]
    fn the_page_says_why_it_is_shown_and_what_became_of_the_last_press() {
        // Each reason has its own sentence.
        let sentences = [
            UnsupportedReason::UnknownType.sentence(),
            UnsupportedReason::PlayerRefused.sentence(),
            UnsupportedReason::OfficeUnavailable.sentence(),
        ];
        assert_eq!(sentences[0], SENTENCE);
        assert!(sentences[1].contains("can\u{2019}t play this file"), "{}", sentences[1]);
        assert!(sentences[0] != sentences[1] && sentences[1] != sentences[2] && sentences[0] != sentences[2]);

        let plain = body_words(
            "Video \u{b7} 366.9 MB",
            UnsupportedReason::PlayerRefused,
            Origin::Beebeeb,
            None,
            None,
        );
        assert!(
            plain.starts_with("Video \u{b7} 366.9 MB ") && plain.contains(PLAYER_REFUSED),
            "{plain}"
        );
        assert!(plain.ends_with("In the cloud, safe in Beebeeb."), "{plain}");
        // What the last press came to is the last thing the page says.
        for note in [
            Note::Downloading,
            Note::Opening,
            Note::Failed("The app didn\u{2019}t open this file.".into()),
        ] {
            let with = body_words("", UnsupportedReason::UnknownType, Origin::Beebeeb, None, Some(&note));
            assert!(with.starts_with(SENTENCE) && with.ends_with(&note.words()), "{with}");
        }
        assert!(Note::Downloading.words().starts_with("Downloading it to "));
    }

    /// Dm-83, RC14: the .eps page said "In the cloud, safe in Beebeeb" for a file `ls -lO` showed was on the Mac. The
    /// page says what the row's mark says: Finder's own state of the file.
    #[test]
    fn the_page_says_on_this_mac_for_a_file_finder_holds_and_the_cloud_for_one_it_does_not() {
        let eps = |here| {
            body_words(
                "EPS \u{b7} 2.1 MB",
                UnsupportedReason::UnknownType,
                Origin::Beebeeb,
                here,
                None,
            )
        };
        let local = eps(Some(true));
        assert!(local.ends_with(ON_THIS_DEVICE) && !local.contains("cloud"), "{local}");
        assert_eq!(ON_THIS_DEVICE, concat!("On ", beebeeb_engine::device_word!(), "."));
        assert!(eps(Some(false)).ends_with(IN_THE_CLOUD));
        // While Finder has not said (not read yet, not a Mac): where the viewer opened it from.
        assert!(eps(None).ends_with(IN_THE_CLOUD));
        assert_eq!(local_state(Origin::ThisPc { unsent: false }, Some(false)), IN_THE_CLOUD);
        // What the last press came to still follows it.
        let opened = Note::Opened("Adobe Illustrator 2026".into());
        let with = body_words(
            "",
            UnsupportedReason::UnknownType,
            Origin::Beebeeb,
            Some(true),
            Some(&opened),
        );
        assert!(
            with.ends_with("On this Mac. Opened in Adobe Illustrator 2026.") || !cfg!(target_os = "macos"),
            "{with}"
        );
        assert_eq!(opened.words(), "Opened in Adobe Illustrator 2026.");
    }

    #[test]
    fn a_note_is_its_files_alone_and_the_oldest_goes_when_there_are_too_many() {
        set_note("note-test-a", Some(Note::Downloading));
        set_note("note-test-b", Some(Note::Failed("why".into())));
        assert_eq!(note_of("note-test-a"), Some(Note::Downloading));
        assert_eq!(note_of("note-test-b"), Some(Note::Failed("why".into())));
        assert_eq!(note_of("note-test-c"), None);
        // The next press's word replaces the last; `None` takes it away.
        set_note("note-test-a", Some(Note::Opening));
        assert_eq!(note_of("note-test-a"), Some(Note::Opening));
        set_note("note-test-a", None);
        assert_eq!(note_of("note-test-a"), None);
        set_note("note-test-b", None);
        // Never more than a few are held, and the oldest goes first (a store of the test's own: the app's is shared
        // by every test that runs now).
        let mut notes = Notes(VecDeque::new());
        for n in 0..NOTES + 2 {
            notes.set(&format!("f-{n}"), Some(Note::Opening));
        }
        assert_eq!(notes.0.len(), NOTES);
        assert_eq!((notes.of("f-0"), notes.of("f-1")), (None, None));
        assert_eq!(notes.of("f-2"), Some(Note::Opening));
    }
}
