//! The found-while-paused card (S2b, not on the board: D-W3-2): the changes made in the Beebeeb folder while sync was
//! paused, which the engine HOLDS and does not send, and what the person can ask of the engine for them: "Upload to my
//! Beebeeb", "Keep on this PC only", and, after a keep, "Show in File Explorer". Drawn from the kit as the banner
//! and the Problems bar are (`ui/banner.rs`, `history.rs`), with the 0.8 app's words.
//!
//! Counts only: the card never shows a name, a path or an id. Nothing here decides anything: every press is one ask of
//! the engine (`engine_host::found_while_paused`), which refuses another account's data and a session that moved, and
//! Upload and Keep carry only the ids the last review returned. A shipped build draws no sample card: the card exists
//! only once the engine's review answered with held changes ([`Card::visible`]).

use rune::{El, col, el, row};

use super::icons;
use crate::State;
use crate::ui::button;
use crate::theme::icons as base;
use crate::theme::tokens::{Theme, type_};
use crate::ui::{banner, kit};

/// What the engine's review says is held, by kind, with the ids Upload and Keep send back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    pub new_folders: usize,
    pub new_files: usize,
    /// Changes to items that exist: new content, a rename, a move.
    pub edits: usize,
    pub deletions: usize,
    pub in_shared_folders: usize,
    pub op_ids: Vec<String>,
    /// The service (OneDrive, Dropbox) that syncs the folder a keep moves the items into, if one does.
    pub synced_by: Option<String>,
}

/// What a keep did, in counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    pub moved_items: usize,
    pub moved_files: usize,
    pub not_moved: usize,
    pub stays_held: usize,
    pub synced_by: Option<String>,
}

/// One press of the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Upload,
    Keep,
    Show,
}

/// What the card holds: the last review, what the last keep did, the action in flight, and the engine's last refusal
/// (one of its fixed sentences).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Card {
    pub held: Option<Held>,
    pub kept: Option<Kept>,
    pub busy: Option<Action>,
    pub said: Option<String>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Held {
    pub fn total(&self) -> usize {
        self.new_folders + self.new_files + self.edits + self.deletions
    }

    /// "2 new folders, 3 new files, 1 change to a file, 1 deletion".
    pub fn kinds(&self) -> String {
        [
            (self.new_folders, "new folder", "new folders"),
            (self.new_files, "new file", "new files"),
            (self.edits, "change to a file", "changes to files"),
            (self.deletions, "deletion", "deletions"),
        ]
        .into_iter()
        .filter(|(n, ..)| *n > 0)
        .map(|(n, one, many)| plural(n, one, many))
        .collect::<Vec<_>>()
        .join(", ")
    }
}

fn synced_by_note(by: &str) -> String {
    format!("That folder is inside {by}, which may copy them to its own cloud.")
}

pub const TITLE: &str = "Found while sync was paused";
pub const UPLOAD: &str = "Upload to my Beebeeb";
pub const KEEP: &str = "Keep on this PC only";
pub const SHOW: &str = "Show in File Explorer";
pub const KEEP_WORDS: &str =
    "Keep on this PC only moves them into “Beebeeb - not uploaded”, next to your Beebeeb folder. Nothing is deleted.";

impl Card {
    /// Only what the engine's review held, or what the last keep did: never a sample.
    pub fn visible(&self) -> bool {
        self.held.as_ref().is_some_and(|h| !h.is_empty_review()) || self.kept.is_some() || self.said.is_some()
    }

    /// Upload and Keep are offered for held changes only, and one at a time.
    pub fn can_decide(&self) -> bool {
        self.busy.is_none() && self.held.as_ref().is_some_and(|h| !h.op_ids.is_empty())
    }

    pub fn can_show(&self) -> bool {
        self.busy.is_none() && self.kept.as_ref().is_some_and(|k| k.moved_items > 0)
    }

    /// The sentence about what was found.
    pub fn found_line(&self) -> Option<String> {
        let held = self.held.as_ref().filter(|h| h.total() > 0)?;
        let mut line = format!(
            "While sync was paused, these changes were made in your Beebeeb folder, and Beebeeb hasn’t sent them: {}.",
            held.kinds()
        );
        if held.in_shared_folders > 0 {
            line.push_str(&format!(" {} in shared folders.", held.in_shared_folders));
        }
        Some(line)
    }

    /// What the last keep did.
    pub fn kept_line(&self) -> Option<String> {
        let k = self.kept.as_ref()?;
        let mut parts = Vec::new();
        if k.moved_items > 0 {
            parts.push(format!(
                "Moved {} ({}) to “Beebeeb - not uploaded”.",
                plural(k.moved_items, "item", "items"),
                plural(k.moved_files, "file", "files")
            ));
        }
        if k.not_moved > 0 {
            parts.push(format!(
                "{} couldn’t be moved and stay where they are, not uploaded.",
                k.not_moved
            ));
        }
        if k.stays_held > 0 {
            parts.push(format!(
                "{} to items Beebeeb already has stay on hold.",
                plural(k.stays_held, "change", "changes")
            ));
        }
        if let Some(by) = &k.synced_by {
            parts.push(synced_by_note(by));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    /// The note before a keep about a service that syncs the folder they would move into.
    pub fn synced_note(&self) -> Option<String> {
        self.held.as_ref().filter(|h| h.total() > 0)?.synced_by.as_deref().map(synced_by_note)
    }
}

impl Held {
    fn is_empty_review(&self) -> bool {
        self.total() == 0 && self.op_ids.is_empty()
    }
}

/// The card, in the banner's look (`.mk-banner`, warn tone), or nothing. `press` is the window's one handler of a press
/// (`Ui` calls its source). Keys: `fwp-card`, `fwp-title`, `fwp-found`, `fwp-keep-words`, `fwp-note`, `fwp-kept`,
/// `fwp-said`, and the buttons `btn-fwp-upload`, `btn-fwp-keep`, `btn-fwp-show`.
pub fn view(card: &Card, press: fn(&mut State, Action), t: &Theme) -> Option<El> {
    if !card.visible() {
        return None;
    }
    let (wash, mark) = banner::tone(banner::Tone::Warn, t);
    let line = |key: &str, words: String, red: bool| {
        kit::label(
            key,
            words,
            type_::BODY,
            if red { icons::red_text(t) } else { t.text_secondary },
        )
        .w_full()
    };
    let mut words = col().key("fwp-words").grow(1.0).min_w(0.0).gap(4.0).child(kit::label(
        "fwp-title",
        TITLE,
        type_::HEAD,
        t.text,
    ));
    if let Some(found) = card.found_line() {
        words = words.child(line("fwp-found", found, false));
        words = words.child(line("fwp-keep-words", KEEP_WORDS.to_string(), false));
        if let Some(note) = card.synced_note() {
            words = words.child(line("fwp-note", note, false));
        }
    }
    if let Some(kept) = card.kept_line() {
        words = words.child(line("fwp-kept", kept, false));
    }
    if let Some(said) = &card.said {
        words = words.child(line("fwp-said", said.clone(), true));
    }

    let mut buttons = row().key("fwp-buttons").gap(8.0).center_y();
    let busy = |a: Action, idle: &'static str, doing: &'static str| if card.busy == Some(a) { doing } else { idle };
    if card.held.as_ref().is_some_and(|h| !h.op_ids.is_empty()) {
        let on = card.can_decide();
        buttons = buttons
            .child(button("btn-fwp-upload", busy(Action::Upload, UPLOAD, "Uploading…"), on, Action::Upload, press, t))
            .child(button("btn-fwp-keep", busy(Action::Keep, KEEP, "Moving…"), on, Action::Keep, press, t));
    }
    if card.kept.as_ref().is_some_and(|k| k.moved_items > 0) {
        buttons = buttons.child(button("btn-fwp-show", SHOW, card.can_show(), Action::Show, press, t));
    }

    Some(
        row()
            .key("fwp-card")
            .role("group")
            .label(TITLE)
            .w_full()
            .py(9.0)
            .pl(12.0)
            .pr(10.0)
            .gap(10.0)
            .items_center()
            .radius(10.0)
            .fill(wash)
            .child(kit::glyph("fwp-icon", &base::SYNC_HISTORY, 16.0, mark))
            .child(words)
            .child(buttons),
    )
}

/// An outline button that does nothing while it is off (an action is in flight, or there is nothing to decide).
fn button(key: &str, words: &str, on: bool, action: Action, press: fn(&mut State, Action), t: &Theme) -> El {
    let b = button::button(button::keys(key), words, None, button::Variant::Secondary, button::Size::Sm, button::ButtonState::Enabled, t);
    if on {
        b.on_press(move |s: &mut State| press(s, action))
    } else {
        el().key(format!("{key}-off")).opacity(0.5).child(b.focusable(false))
    }
}
