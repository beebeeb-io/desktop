//! Preview in the window (DW-0408), and what it says of a file it cannot show (DW-0408b).
//!
//! Space, or Preview in the file menu, puts a card over the Files window: the file's first
//! page when the source has one (`FilesSource::preview`), else "No preview for this kind of
//! file" with the two ways on. It never leaves the Files window: OPEN is what goes to a
//! viewer or to the file's own app (`State::open_selection`).
//!
//! **Decrypted content.** The page is in the open panel's state and nowhere else: it is asked
//! of the source when the panel opens and dropped when it closes.

use super::actions_data::{Page, Previewed};
use super::dialogs::Dialog;
use super::source::{Action, ActionError};
use crate::source::{format_size, DriveItem};
use crate::theme::file_icons::icon_for;
use crate::theme::scale;
use crate::theme::tokens::{type_, Theme};
use crate::theme::{icons, icons_files};
use crate::ui::settings_dialog::DIALOG_TITLE;
use crate::ui::settings_shell::SUB;
use crate::ui::{button, kit};
use crate::State;
use rune::{col, el, row, Color, El, Key, KeyEvent, Span, TextStyle, Tx};

/// What the card offers. Files' panel: Share, Open in its app, Open in another app…, Make available offline. The
/// read-only panel of an item that is not in Files' list (`frames::item`: a file in Trash, one shared with the
/// person): none of them, only Close, and what it says after why there is no page is `tail`.
pub(crate) enum Mode {
    Files,
    ReadOnly {
        /// After why there is no page (" Restore it to open it."), or nothing.
        tail: &'static str,
        /// What Close (and Space on it) does.
        close: fn(&mut State, &mut Tx),
        /// The size in words, as the item's list shows it ("" when not known).
        size: String,
    },
}

/// The open preview: the file, and what there is to show of it.
#[derive(Clone, Debug)]
pub struct Panel {
    pub item: DriveItem,
    /// What there is to show of it: a page, or why not (DW-0408b).
    pub state: Previewed,
}

/// The card with a page: 640 x 700, 28 pt under the caption. Without one: 540 x 420, 120 pt under.
pub(crate) const PAGE_CARD: (f32, f32, f32) = (640.0, 700.0, 28.0);
pub(crate) const BLANK_CARD: (f32, f32, f32) = (540.0, 420.0, 120.0);
/// The board puts both cards 11 pt right of the window's middle.
const OFF_CENTRE: f32 = 11.0;

/// The page's own type: black on white in both themes, on the 17 pt line the board's page inherits.
const PAGE_TITLE: TextStyle = TextStyle::new(20.0, 650.0).lh(17.0);
const PAGE_BYLINE: TextStyle = TextStyle::new(11.0, 400.0).lh(17.0);
const PAGE_HEADING: TextStyle = TextStyle::new(13.5, 650.0).lh(17.0);
const PAGE_TEXT: TextStyle = TextStyle::new(12.0, 400.0).lh(19.0);

impl State {
    /// Preview of `item` (the selected file, or the file whose versions are listed).
    pub fn open_preview(&mut self, item: DriveItem) {
        if item.is_folder {
            return;
        }
        self.close_menu();
        // The read of an earlier file stops, and what it decrypted is wiped, before the next one starts.
        self.actions.source.cancel_preview();
        let now = self.actions.source.preview(&item);
        let state = self.actions.source.preview_later(&item).unwrap_or(match now {
            Some(page) => Previewed::Shown(page),
            None => Previewed::NoPreview,
        });
        self.actions.dialog = Some(Dialog::Preview(Box::new(Panel { item, state })));
    }

    /// Left and Right in the panel: the previous or next FILE of the list, previewed in turn.
    pub fn preview_step(&mut self, delta: isize) -> bool {
        let Some(Dialog::Preview(panel)) = &self.actions.dialog else {
            return false;
        };
        let items = &self.listing.items;
        let Some(mut at) = items.iter().position(|i| i.id == panel.item.id) else {
            return false;
        };
        loop {
            let Some(next) = at.checked_add_signed(delta).filter(|&i| i < items.len()) else {
                return false;
            };
            at = next;
            if !items[at].is_folder {
                break;
            }
        }
        let item = items[at].clone();
        self.select_only(at);
        self.open_preview(item);
        true
    }

    /// Open, from the panel: the panel closes and the file opens in the window's viewer (w29: nothing opens in
    /// another app without "Open in another app").
    pub fn open_from_preview(&mut self, tx: &mut Tx) {
        self.close_dialog(tx);
        self.open_selection(tx);
    }

    /// Open in another app…: Windows' app chooser; the file is decrypted for the app picked.
    pub fn open_with(&mut self, tx: &mut Tx) {
        let Some(Dialog::Preview(panel)) = self.actions.dialog.clone() else {
            return;
        };
        if self.needs_session(tx) {
            return;
        }
        self.close_dialog(tx);
        if let Err(why) = self.actions.source.open_with(&panel.item) {
            self.failed(Action::OpenWith, why, tx);
        }
    }

    /// Make available offline, from the panel: keeps the file on this PC (DW-0412) and closes.
    pub fn keep_from_preview(&mut self, tx: &mut Tx) {
        self.close_dialog(tx);
        if !self.selection_is_kept() {
            self.toggle_keep_offline(tx);
        }
    }

    /// Share, from the panel: the Share dialog (DW-0407) for the same file.
    pub fn share_from_preview(&mut self) {
        self.actions.source.cancel_preview();
        self.actions.dialog = None;
        self.begin_share();
    }
}

/// Every frame (`protected::poll`): while the panel waits for a source that answers later, ask where it stands and
/// keep a changed answer. `true`: the panel changed.
pub fn poll(s: &mut State) -> bool {
    let Some(Dialog::Preview(panel)) = &s.actions.dialog else {
        return false;
    };
    if panel.state != Previewed::Loading {
        return false;
    }
    let item = panel.item.clone();
    match s.actions.source.preview_later(&item) {
        Some(now) if now != Previewed::Loading => {
            if let Some(Dialog::Preview(panel)) = &mut s.actions.dialog {
                panel.state = now;
            }
            true
        }
        _ => false,
    }
}

/// The panel's keys: Space closes it (Esc is the dialog's own), Left and Right step through
/// the files. Every other key stops here, so the list behind never moves under the panel.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match &ev.key {
        Key::Text(space) if space == " " && tx.focused().is_none_or(|key| !key.starts_with("pv-")) => {
            s.close_dialog(tx);
            true
        }
        Key::Left => s.preview_step(-1),
        Key::Right => s.preview_step(1),
        _ => super::keys::on_key(s, ev, tx),
    }
}

/// The card's head: close, the file's name and where the preview is, Share, Open in its app (Files' panel only).
fn head(item: &DriveItem, state: &Previewed, mode: &Mode, t: &Theme) -> El {
    let mut title = col().key("pv-title").grow(1.0).min_w(0.0).px(scale::space::S4).child(
        kit::label("pv-name", item.name.clone(), type_::TITLE, t.text)
            .role("heading")
            .level(2)
            .truncate(),
    );
    if let Previewed::Shown(shown) = state {
        title = title.child(kit::label(
            "pv-place",
            shown.place.clone(),
            type_::TITLE_SUB,
            t.text_tertiary,
        ));
    }
    let close = crate::ui::toolbar::tool("pv-close", "Close preview", &icons_files::CLOSE, t);
    let close = match mode {
        Mode::Files => close.on_press_tx(|s: &mut State, tx: &mut Tx| s.close_dialog(tx)),
        Mode::ReadOnly { close: press, .. } => close.on_press_tx(*press),
    };
    let head = row()
        .key("pv-head")
        .w_full()
        .h(44.0)
        .px(scale::space::S12)
        .gap(scale::space::S8)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(close)
        .child(crate::ui::file_icon::file_icon(
            "preview-head-icon",
            icon_for(&item.name, false, item.mime.as_deref()),
            16.0,
            t,
        ))
        .child(title);
    // Nothing of a read-only panel shares or opens the file.
    if matches!(mode, Mode::ReadOnly { .. }) {
        return head;
    }
    let mut head = head.child(
        crate::ui::toolbar::tool("pv-share", "Share\u{2026}", &icons::SHARE, t)
            .on_press(|s: &mut State| s.share_from_preview()),
    );
    if matches!(state, Previewed::Shown(_)) {
        head = head.child(
            crate::ui::settings_controls::button(
                0,
                "Open in its app",
                None,
                crate::ui::settings_controls::Emphasis::Plain,
                t,
            )
            .key("pv-open")
            .on_press_tx(|s: &mut State, tx: &mut Tx| s.open_from_preview(tx)),
        );
    }
    head
}

/// The first page: black on white in both themes, as a page is.
fn page(p: &Page) -> El {
    let ink = rune::color::hex("#0d0d0d").unwrap_or(Color::BLACK);
    let mut sheet = col()
        .key("pv-page")
        .role("document")
        .label(p.title.clone())
        .w(440.0)
        .h_full()
        .padding(40.0)
        .gap(scale::space::S10)
        .radius(3.0)
        .fill(Color::WHITE)
        .box_shadow(kit::drop(4.0, 18.0, Color::BLACK.alpha(0.16)))
        .child(kit::label("pv-page-title", p.title.clone(), PAGE_TITLE, ink))
        .child(kit::label(
            "pv-page-byline",
            p.byline.clone(),
            PAGE_BYLINE,
            ink.alpha(0.6),
        ));
    for (i, (heading, text)) in p.sections.iter().enumerate() {
        sheet = sheet
            .child(kit::label(format!("pv-page-h-{i}"), heading.clone(), PAGE_HEADING, ink).mt(6.0))
            .child(
                kit::label(format!("pv-page-p-{i}"), text.clone(), PAGE_TEXT, ink)
                    .wrap()
                    .w_full(),
            );
    }
    sheet
}

/// What the panel says of a file it cannot show, and the two ways on (DW-0408b; none on a read-only panel).
fn blank(item: &DriveItem, state: &Previewed, mode: &Mode, t: &Theme) -> El {
    let size = match mode {
        Mode::Files => item.size.map(format_size).unwrap_or_default(),
        Mode::ReadOnly { size, .. } => size.clone(),
    };
    let (title, tail) = match state {
        Previewed::Loading => ("Getting the preview\u{2026}", ""),
        Previewed::NotYet => (
            "Preview isn't available for this kind of file in this version yet",
            " Open it to see it, or open it in another app.",
        ),
        Previewed::TooLarge => (
            "Too large to preview here",
            " Open it to see it, or open it in another app.",
        ),
        Previewed::Failed(ActionError::Offline) => (
            "Preview needs a connection",
            " Open it to see it, or open it in another app.",
        ),
        Previewed::Failed(_) => (
            "Couldn't show a preview",
            " Open it to see it, or open it in another app.",
        ),
        // No preview for this kind of file (DW-0408b).
        _ => (
            "No preview for this kind of file",
            concat!(
                " Open it in another app, or keep it on ",
                beebeeb_engine::device_word!(),
                " to open it offline."
            ),
        ),
    };
    // A read-only panel says its own words after why (none while it waits).
    let tail = match mode {
        Mode::ReadOnly { tail: own, .. } if !tail.is_empty() => own,
        _ => tail,
    };
    let read_only = matches!(mode, Mode::ReadOnly { .. });
    let words = if size.is_empty() && tail.is_empty() && read_only {
        item.name.clone()
    } else if size.is_empty() {
        format!("{}.{tail}", item.name)
    } else if tail.is_empty() {
        format!("{} \u{00B7} {size}", item.name)
    } else {
        format!("{} \u{00B7} {size}.{tail}", item.name)
    };
    // The size is a figure: JetBrains Mono inside the Inter line.
    let at = item.name.len() + " \u{00B7} ".len();
    let spans = if size.is_empty() {
        Vec::new()
    } else {
        vec![Span {
            start: at as u32,
            end: (at + size.len()) as u32,
            mono: true,
            ..Span::default()
        }]
    };
    let blank = col()
        .key("pv-blank")
        .grow(1.0)
        .w_full()
        .center()
        .gap(scale::space::S8)
        .px(40.0)
        .child(
            el().key("pv-blank-tile")
                .size(56.0, 56.0)
                .radius(16.0)
                .center()
                .fill(t.text.alpha(0.07))
                .child(crate::ui::file_icon::file_icon(
                    "pv-blank-icon",
                    icon_for(&item.name, false, item.mime.as_deref()),
                    28.0,
                    t,
                )),
        )
        .child(kit::label("pv-blank-title", title, DIALOG_TITLE, t.text).mt(8.0))
        .child(
            kit::label("pv-blank-text", words, SUB, t.text_tertiary)
                .spans(spans)
                .wrap()
                .align(rune::Align::Center)
                // `max-width: 36ch` of the 12.5 px line.
                .max_w(248.0),
        );
    if read_only {
        return blank;
    }
    let kept = item.pc_status.as_deref() == Some("available_offline");
    let mut keep = button::outline_button("pv-keep", "Make available offline", Some(&icons::AVAILABLE_OFFLINE), t);
    keep = if kept {
        // It is kept already: the button has nothing to do.
        keep.opacity(0.4).disabled(true)
    } else {
        keep.on_press_tx(|s: &mut State, tx: &mut Tx| s.keep_from_preview(tx))
    };
    blank.child(
        row()
            .key("pv-blank-actions")
            .gap(scale::space::S8)
            .mt(12.0)
            .child(
                button::outline_button("pv-open-with", "Open in another app\u{2026}", None, t)
                    .on_press_tx(|s: &mut State, tx: &mut Tx| s.open_with(tx)),
            )
            .child(keep),
    )
}

/// The card (`pv-card`), `size` (w, h), placed by its caller.
pub(crate) fn card(item: &DriveItem, state: &Previewed, mode: &Mode, size: (f32, f32), t: &Theme) -> El {
    let (w, h) = size;
    let body = match state {
        Previewed::Shown(shown) => row()
            .key("pv-body")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .padding(scale::space::S24)
            .justify_center()
            .fill(t.text.alpha(0.05))
            .clip_rounded(0.0)
            .child(page(&shown.page)),
        _ => blank(item, state, mode, t),
    };
    let card = col()
        .key("pv-card")
        .role("dialog")
        .label(format!("Preview of {}", item.name))
        .size(w, h)
        .radius(crate::theme::scale::radius::DIALOG)
        .fill(t.card)
        .box_shadow(kit::drop(30.0, 80.0, Color::BLACK.alpha(0.34)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key("pv-clip")
                .w_full()
                .h_full()
                .clip_rounded(12.0)
                .child(head(item, state, mode, t))
                .child(body),
        );
    match mode {
        Mode::Files => card,
        Mode::ReadOnly { .. } => card.description("Read only"),
    }
}

pub fn view(s: &State, panel: &Panel, t: &Theme) -> El {
    let shown = matches!(panel.state, Previewed::Shown(_));
    let (w, h, top) = if shown { PAGE_CARD } else { BLANK_CARD };
    let (win_w, win_h) = s.actions.window;
    // A window smaller than the board's: the card stays inside it.
    let area_h = (win_h - crate::TITLEBAR_H).max(0.0);
    let top = top.min((area_h - h).max(0.0) / 2.0);
    let h = h.min((area_h - top - 8.0).max(120.0));
    let left = ((win_w - w) / 2.0 + OFF_CENTRE).max(8.0);
    let card = card(&panel.item, &panel.state, &Mode::Files, (w, h), t).absolute(left, top);
    let scrim = el()
        .key("scrim")
        .pin(Some(0.0), Some(crate::TITLEBAR_H), Some(0.0), Some(0.0))
        .fill(crate::ui::settings_dialog::scrim_color(t))
        .child(card);
    el().key("dialog-preview")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(scrim)
        .on_key(on_key)
        .modal_tx(false, |s: &mut State, tx: &mut Tx| s.close_dialog(tx))
}
