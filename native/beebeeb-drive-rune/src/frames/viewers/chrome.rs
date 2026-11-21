//! What every open file shares with the Files window (`.mk-win`): the caption, the sidebar, the
//! toolbar with the file's path where the folder's title was, and the status line.
//!
//! The toolbar's right half (the dimmed view switch, Sort, the details-panel toggle, New, the
//! search field) is `ui::toolbar`'s, drawn again here because that file builds it in one private
//! function (a request to the design-system lane in the PR body: make the halves public).

use rune::{Cx, El, col, el, row};

use super::source::OpenFile;
use super::{Ui, icons};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::{caption, sidebar, status_bar, toolbar};

/// The status line's dot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    /// Decrypted, saved, up to date.
    Green,
    /// Not done and not broken: decrypting, unsaved, no room.
    Warn,
    Red,
}

/// The status line of one frame: the dot, its words, and the words at the right.
pub struct Status {
    pub dot: Dot,
    pub text: String,
    pub details: String,
}

/// What the toolbar says about the file beyond its path.
#[derive(Default)]
pub struct Head {
    /// The line under the path, where it is not the file's own facts ("Changes not saved yet").
    pub facts: Option<String>,
    /// A dot after the name: the edit is not saved.
    pub unsaved: bool,
}

/// The file's name and the line under it, with its icon (and the unsaved dot): the shared title block.
fn title(file: &OpenFile, head: &Head, t: &Theme) -> El {
    let facts = head.facts.clone().unwrap_or_else(|| file.facts.clone());
    let icon = crate::theme::file_icons::icon_for(&file.name, false, None);
    let mut block = row()
        .key("viewer-title")
        .role("group")
        .label(format!("{}, {}, {}", file.path.join(" / "), file.name, facts))
        .min_w(0.0)
        .shrink()
        .center_y()
        .gap(6.0)
        .child(crate::ui::file_icon::file_icon("viewer-title-icon", icon, 16.0, t))
        .child(toolbar::title_block(&file.name, Some(&facts), t).shrink());
    if head.unsaved {
        block = block.child(
            el().key("crumb-dot")
                .role("image")
                .label("Not saved")
                .size(7.0, 7.0)
                .radius(3.5)
                .fill(t.text),
        );
    }
    block
}

/// The toolbar of an open file. `tools` are the file's own controls, drawn before the "more" button.
pub fn toolbar(_s: &State, ui: &Ui, head: &Head, tools: Vec<El>, t: &Theme) -> El {
    let more = toolbar::tool("more-btn", "More actions", &icons::MORE, t);
    // The file's own controls and "more" sit 6 apart (`.flex.items-center.gap-1.5`).
    let file_tools = row().key("file-tools").role("toolbar").label("File tools").focus_group().center_y().gap(6.0).children(tools).child(more);
    toolbar::file_bar(
        toolbar::Bar {
            name: ui.file.name.clone(),
            back: (true, toolbar::press(|s, _| super::close(s))),
            forward: (false, None),
            title: title(&ui.file, head, t),
            slot: Some(file_tools),
            slot_sep: false,
            view: toolbar::View::NotHere,
            on_view: None,
            sort: toolbar::Sort::NotHere,
            on_sort: None,
            details: if ui.show_details {
                toolbar::Details::Open
            } else {
                toolbar::Details::Closed
            },
            on_details: toolbar::press(|s, _| {
                if let Some(ui) = super::ui(s) {
                    ui.toggle_details();
                }
            }),
            inert: false,
            gate_closed: false,
            on_explorer: Box::new(|s, _| {
                let Some((id, name)) = super::ui(s).map(|ui| (ui.file.id.clone(), ui.file.name.clone())) else {
                    return;
                };
                let answer = super::super::show_in_folder::show(s, &id, &name);
                if let Some(ui) = super::ui(s) {
                    ui.toast = answer.err().map(str::to_string);
                }
            }),
            new: toolbar::New::Off,
            on_new: None,
            search: el(),
            trailing_inset: 12.0,
        },
        t,
    )
}

/// The status line (`.mk-foot`) with this frame's words.
pub fn status_bar(status: &Status, t: &Theme) -> El {
    let dot = match status.dot {
        Dot::Green => status_bar::Dot::Green,
        Dot::Warn => status_bar::Dot::Warn,
        Dot::Red => status_bar::Dot::Red,
    };
    status_bar::bar(Some(dot), Some(&status.text), &status.details, t)
}

/// What the status line says at the left while content read from the copy on this PC is on screen.
pub const FROM_THIS_PC: &str = "Opened from the copy on this PC";
/// The same, for a copy with a change that is not in Beebeeb yet: what is on screen is NOT what Beebeeb has.
pub const FROM_THIS_PC_UNSENT: &str = "Opened from the copy on this PC \u{00B7} its changes aren't in Beebeeb yet";
/// The same, for a copy read while sync is off: nothing says it is still the version Beebeeb has.
pub const FROM_THIS_PC_SYNC_OFF: &str =
    "Opened from the copy on this PC \u{00B7} sync is off, Beebeeb may have a newer version";
/// What the line says for the version Beebeeb has of a file whose copy on this PC could not be confirmed as that
/// version (older, or of a version not known: `LocalCopy::Behind`).
pub const FROM_BEEBEEB_UNCONFIRMED_HERE: &str =
    "The version in Beebeeb, decrypted on this PC \u{00B7} couldn't confirm the copy on this PC is this version";
/// What the line says for the version Beebeeb has of a file whose copy on this PC is not in sync with it.
pub const FROM_BEEBEEB_NOT_THIS_PCS: &str =
    "The version in Beebeeb, decrypted on this PC \u{00B7} the copy on this PC is not in sync with it";

/// `status`, saying where the content on screen came from when that is not plainly Beebeeb: content read from
/// the copy kept on this PC was not downloaded and decrypted just now, a copy with unsent changes is not the
/// version Beebeeb has, a copy read while sync is off may be older than Beebeeb's, and Beebeeb's version of a file
/// that is not in sync here may not be what is on this PC. The line never says a plain "Decrypted on this PC"
/// over any of those. Only while content is shown (a state has its own words). A frame's own warning (its dot)
/// is kept: the origin can add one, never take one away.
fn from_where(ui: &Ui, status: &Status) -> Status {
    use super::source::Body;
    let shows_content = matches!(
        ui.file.body,
        Body::Text(_) | Body::Markdown(_) | Body::Photo(_) | Body::Pdf(_)
    );
    origin_status(ui.origin(), shows_content && ui.editor.is_none(), status)
}

/// [`from_where`], from the origin alone. `shows_content`: content of the file is on screen (not a state, not
/// the editor).
pub(super) fn origin_status(origin: super::source::Origin, shows_content: bool, status: &Status) -> Status {
    use super::source::Origin;
    let said = match origin {
        _ if !shows_content => None,
        Origin::Beebeeb => None,
        Origin::ThisPc { unsent: false } => Some((FROM_THIS_PC, false)),
        Origin::ThisPc { unsent: true } => Some((FROM_THIS_PC_UNSENT, true)),
        Origin::ThisPcSyncOff => Some((FROM_THIS_PC_SYNC_OFF, true)),
        Origin::BeebeebNotThisPcs => Some((FROM_BEEBEEB_NOT_THIS_PCS, true)),
        Origin::BeebeebUnconfirmedHere => Some((FROM_BEEBEEB_UNCONFIRMED_HERE, true)),
    };
    match said {
        Some((text, warns)) => Status {
            dot: match status.dot {
                Dot::Green if warns => Dot::Warn,
                dot => dot,
            },
            text: text.into(),
            details: status.details.clone(),
        },
        None => Status {
            dot: status.dot,
            text: status.text.clone(),
            details: status.details.clone(),
        },
    }
}

/// The window with an open file in its content area: `body` between the toolbar and the status line.
pub fn window(
    s: &State,
    cx: &mut Cx<State>,
    ui: &Ui,
    head: Head,
    tools: Vec<El>,
    banner: Option<El>,
    body: El,
    status: &Status,
    t: &Theme,
) -> El {
    // Keyboard as Windows has it: Tab visits every control.
    cx.web_focus();
    let mut main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar(s, ui, &head, tools, t));
    if let Some(banner) = banner {
        main = main.child(banner);
    }
    let main = main
        .child({
            let mut content = row().key("viewer-row").grow(1.0).min_h(0.0).w_full().child(body);
            if ui.show_details {
                let reduced_quality =
                    super::details::is_reduced(ui.source.player().and_then(|h| h.state_of(&ui.file.id)).as_ref());
                content = content.child(super::details::panel_with(
                    &ui.file,
                    ui.knows_offline(),
                    reduced_quality,
                    t,
                ));
            }
            content
        })
        .child(status_bar(&from_where(ui, status), t));

    col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), t, cx))
        .child({
            // Under the caption: the window's body, and what sits over it.
            let mut under = el().key("under-caption").grow(1.0).min_h(0.0).w_full().child(
                row()
                    .key("body")
                    .w_full()
                    .h_full()
                    .child(sidebar::sidebar(s, t))
                    .child(main),
            );
            if let Some(words) = &ui.toast {
                under = under.child(super::dialogs::toast(words, t));
            }
            match &ui.dialog {
                Some(super::Dialog::Unsaved) => under = under.child(super::dialogs::unsaved(&ui.file.name, t)),
                Some(super::Dialog::Conflict(other)) => {
                    let mine = ui.editor.as_ref().map(super::Editor::change_words).unwrap_or_default();
                    under = under.child(super::dialogs::conflict(&ui.file.name, &mine, other, t));
                }
                None => {}
            }
            under
        })
}

#[cfg(test)]
#[path = "chrome_icon_tests.rs"]
mod icon_tests;
