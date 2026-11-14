//! Preview in the window (DW-0408), and what it says of a file it cannot show (DW-0408b).
//!
//! Space, or Preview in the file menu, puts a card over the Files window: the file's first
//! page when the source has one (`FilesSource::preview`), else "No preview for this kind of
//! file" with the two ways on. It never leaves the Files window: OPEN is what goes to a
//! viewer or to the file's own app (`State::open_selection`).
//!
//! **Decrypted content.** The page is in the open panel's state and nowhere else: it is asked
//! of the source when the panel opens and dropped when it closes.

use super::actions_data::{Page, PreviewOf};
use super::dialogs::Dialog;
use super::source::Action;
use crate::State;
use crate::source::{DriveItem, format_size};
use crate::theme::tokens::{Theme, type_};
use crate::theme::{icons, icons_files};
use crate::ui::settings_dialog::DIALOG_TITLE;
use crate::ui::settings_shell::SUB;
use crate::ui::{button, kit};
use rune::{Color, El, Key, KeyEvent, Span, TextStyle, Tx, col, el, row};

/// The open preview: the file, and what there is to show of it.
#[derive(Clone, Debug)]
pub struct Panel {
    pub item: DriveItem,
    /// `None`: no preview for this kind of file (DW-0408b).
    pub shown: Option<PreviewOf>,
}

/// The card with a page: 640 x 700, 28 pt under the caption. Without one: 540 x 420, 120 pt under.
const PAGE_CARD: (f32, f32, f32) = (640.0, 700.0, 28.0);
const BLANK_CARD: (f32, f32, f32) = (540.0, 420.0, 120.0);
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
        let shown = self.actions.source.preview(&item);
        self.actions.dialog = Some(Dialog::Preview(Box::new(Panel { item, shown })));
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

    /// Open in its app, from the panel: the panel closes and the file goes to its own app, never to the window's
    /// viewer (the button names where it opens).
    pub fn open_from_preview(&mut self, tx: &mut Tx) {
        self.close_dialog(tx);
        self.open_selection_with(tx, false);
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
        self.actions.dialog = None;
        self.begin_share();
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

/// `.mk-tbtn`: a 28 pt square with a glyph.
fn tool(key: &str, name: &str, icon: &'static rune::Icon, size: f32, t: &Theme) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .label(name)
        .size(28.0, 28.0)
        .center()
        .radius(7.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(kit::glyph(format!("{key}-icon"), icon, size, t.text_secondary))
}

/// The card's head: close, the file's name and where the preview is, Share, Open in its app.
fn head(panel: &Panel, t: &Theme) -> El {
    let mut title = col().key("pv-title").grow(1.0).min_w(0.0).px(4.0).child(
        kit::label("pv-name", panel.item.name.clone(), type_::TITLE, t.text)
            .role("heading")
            .level(2)
            .truncate(),
    );
    if let Some(shown) = &panel.shown {
        title = title.child(kit::label(
            "pv-place",
            shown.place.clone(),
            type_::TITLE_SUB,
            t.text_tertiary,
        ));
    }
    let mut head = row()
        .key("pv-head")
        .w_full()
        .h(44.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            tool("pv-close", "Close preview", &icons_files::CLOSE, 14.0, t)
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.close_dialog(tx)),
        )
        .child(title)
        .child(
            tool("pv-share", "Share\u{2026}", &icons::SHARE, 15.0, t).on_press(|s: &mut State| s.share_from_preview()),
        );
    if panel.shown.is_some() {
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
        .gap(10.0)
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

/// What the panel says of a file it cannot show, and the two ways on (DW-0408b).
fn blank(panel: &Panel, t: &Theme) -> El {
    let size = panel.item.size.map(format_size).unwrap_or_default();
    let words = if size.is_empty() {
        format!(
            "{}. Open it in another app, or keep it on this PC to open it offline.",
            panel.item.name
        )
    } else {
        format!(
            "{} \u{00B7} {size}. Open it in another app, or keep it on this PC to open it offline.",
            panel.item.name
        )
    };
    // The size is a figure: JetBrains Mono inside the Inter line.
    let at = panel.item.name.len() + " \u{00B7} ".len();
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
    let kept = panel.item.pc_status.as_deref() == Some("available_offline");
    let mut keep = button::outline_button("pv-keep", "Make available offline", Some(&icons::AVAILABLE_OFFLINE), t);
    keep = if kept {
        // It is kept already: the button has nothing to do.
        keep.opacity(0.4).disabled(true)
    } else {
        keep.on_press_tx(|s: &mut State, tx: &mut Tx| s.keep_from_preview(tx))
    };
    col()
        .key("pv-blank")
        .grow(1.0)
        .w_full()
        .center()
        .gap(8.0)
        .px(40.0)
        .child(
            el().key("pv-blank-tile")
                .size(56.0, 56.0)
                .radius(16.0)
                .center()
                .fill(t.text.alpha(0.07))
                .child(kit::glyph(
                    "pv-blank-icon",
                    &icons_files::DOC_BLANK,
                    28.0,
                    t.text_secondary,
                )),
        )
        .child(
            kit::label(
                "pv-blank-title",
                "No preview for this kind of file",
                DIALOG_TITLE,
                t.text,
            )
            .mt(8.0),
        )
        .child(
            kit::label("pv-blank-text", words, SUB, t.text_tertiary)
                .spans(spans)
                .wrap()
                .align(rune::Align::Center)
                // `max-width: 36ch` of the 12.5 px line.
                .max_w(248.0),
        )
        .child(
            row()
                .key("pv-blank-actions")
                .gap(8.0)
                .mt(12.0)
                .child(
                    button::outline_button("pv-open-with", "Open in another app\u{2026}", None, t)
                        .on_press_tx(|s: &mut State, tx: &mut Tx| s.open_with(tx)),
                )
                .child(keep),
        )
}

pub fn view(s: &State, panel: &Panel, t: &Theme) -> El {
    let (w, h, top) = if panel.shown.is_some() { PAGE_CARD } else { BLANK_CARD };
    let (win_w, win_h) = s.actions.window;
    // A window smaller than the board's: the card stays inside it.
    let area_h = (win_h - crate::TITLEBAR_H).max(0.0);
    let top = top.min((area_h - h).max(0.0) / 2.0);
    let h = h.min((area_h - top - 8.0).max(120.0));
    let left = ((win_w - w) / 2.0 + OFF_CENTRE).max(8.0);
    let body = match &panel.shown {
        Some(shown) => row()
            .key("pv-body")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .padding(24.0)
            .justify_center()
            .fill(t.text.alpha(0.05))
            .clip_rounded(0.0)
            .child(page(&shown.page)),
        None => blank(panel, t),
    };
    let card = col()
        .key("pv-card")
        .role("dialog")
        .label(format!("Preview of {}", panel.item.name))
        .absolute(left, top)
        .size(w, h)
        .radius(12.0)
        .fill(t.card)
        .box_shadow(kit::drop(30.0, 80.0, Color::BLACK.alpha(0.34)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key("pv-clip")
                .w_full()
                .h_full()
                .clip_rounded(12.0)
                .child(head(panel, t))
                .child(body),
        );
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
