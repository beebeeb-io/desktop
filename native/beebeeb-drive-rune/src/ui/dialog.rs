//! The one dialog (`.wn-dlg`, `.mk-sheet`, a panel) and its scrim (`.mk-scrim`), for every window.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Any dialog: surface, head, body, foot, Enter | `dialog(Dialog { .. }, t)` | `key`; `p-clip`, `p-head`, `p-title`, `p-sub`, `p-words`, `p-body`, `p-foot` (`p` = `prefix`) |
//! | The scrim, modal: Esc dismisses, Tab stays inside, focus back on the opener | `modal(top, card, dismiss, t)` | `scrim` |
//! | The same, focus back on a named opener (one that went away) | `modal_from(top, card, opener, dismiss, t)` | `scrim` |
//! | The same, focus to a fallback pane or list when the opener is gone too | `modal_back(top, card, opener, fallback, dismiss, t)` | `scrim` |
//! | The card: title, line under it, body, foot (delegate) | `card(name, width, title, sub, body, foot, t)` | `dlg`, `dlg-clip`, `dlg-head`, `dlg-title`, `dlg-sub`, `dlg-body`, `dlg-foot` |
//! | The scrim with the card in its middle | `scrim(top, card, t)` | `scrim` |
//!
//! The radius, shadow, ring, paddings, the foot's band and its hairline are written here once.
//! Callers write `Dialog { head: .., foot: .., ..Dialog::new("dlg", name, 380.0) }`.

use crate::theme::tokens::Theme;
use rune::el;
use rune::{Color, El, col, row};

use super::kit;
use super::settings_dialog::{DIALOG_TITLE, scrim_color};
use super::settings_shell::SUB;

/// The card's corner (`.wn-dlg`), also the clip that rounds the foot's corners.
pub const RADIUS: f32 = 8.0;
/// The panel's corner (the licence panel, uploads).
pub const PANEL_RADIUS: f32 = 12.0;
/// The sheet's corners (`.mk-sheet`): it hangs from the caption, its lower corners rounded.
pub const SHEET_CORNERS: [f32; 4] = [0.0, 0.0, 14.0, 14.0];

/// What the line under the title does when it is longer than the card.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sub<'a> {
    /// A sentence: it goes on to the next line.
    Wrap(&'a str),
    /// A name (a file's, a folder's): one line, cut with an ellipsis.
    Truncate(&'a str),
}

/// The dialog's surface.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Surface {
    /// `.wn-dlg`: radius 8, the drop and the ring, the head, body and foot paddings.
    #[default]
    Card,
    /// `.mk-sheet`: corners `[0, 0, 14, 14]`, the drop at 0.30. No padding.
    Sheet,
    /// A panel: radius 12, the card's drop. No padding.
    Panel,
}

/// The dialog's head.
pub enum Head<'a> {
    /// The title (heading level 2) and the line under it.
    Title { title: &'a str, sub: Option<Sub<'a>> },
    /// An alert's head: the toned tile, then the question and what it means, both wrapping.
    Alert { tile: El, title: &'a str, sub: &'a str },
    /// The caller's content in the title's place and paddings (a back button beside the title).
    Custom(El),
    /// A head placed as given: no key, no padding (the viewers' own heads).
    Raw(El),
    /// No head: the body comes first.
    None,
}

/// The dialog's foot: its buttons, the primary first, the way out last.
pub enum Foot {
    /// As given, at the trailing edge.
    Row(Vec<El>),
    /// Side by side across the foot, each as wide as the rest (`.is-even`).
    Even(Vec<El>),
    /// One under the other, each as wide as the foot (`.is-stack`).
    Stack(Vec<El>),
    /// The caller's own layout (`row()` or `col()` with its children); the card gives the band.
    Custom(El),
    /// No foot.
    None,
}

/// One dialog. `Dialog::new` gives the defaults.
pub struct Dialog<'a> {
    /// The surface's key.
    pub key: &'a str,
    /// The parts' key prefix: `p-clip`, `p-head`, ... ("dlg" for most, "sc" under "sc-dlg").
    pub prefix: &'a str,
    /// What assistive tech calls the dialog.
    pub name: &'a str,
    pub width: f32,
    pub surface: Surface,
    /// Role `alertdialog` instead of `dialog`.
    pub alert: bool,
    pub head: Head<'a>,
    /// Left out (no `p-body`) when empty.
    pub body: Vec<El>,
    /// The body's top padding on a Card in place of 10 (the account kit's body-first dialogs: 22).
    pub body_top: Option<f32>,
    pub foot: Foot,
    /// The foot button (Row, Even, Stack) Enter presses. Never a destructive answer.
    pub enter: Option<usize>,
    /// A fixed height for the surface. A Panel without one sizes to its content; with one its
    /// clip grows to fill it.
    pub height: Option<f32>,
}

impl<'a> Dialog<'a> {
    /// A Card keyed `key` (and its parts `key-…`), not an alert, no head, no body, no foot, no Enter.
    pub fn new(key: &'a str, name: &'a str, width: f32) -> Self {
        Dialog {
            key,
            prefix: key,
            name,
            width,
            surface: Surface::Card,
            alert: false,
            head: Head::None,
            body: Vec::new(),
            body_top: None,
            foot: Foot::None,
            enter: None,
            height: None,
        }
    }
}

/// The dialog alone (no scrim).
pub fn dialog(d: Dialog, t: &Theme) -> El {
    build(d, false, t)
}

/// The scrim with `card` in its middle, modal: the window behind takes no press, Esc calls
/// `dismiss`, Tab stays inside, rune's autofocus picks the first field or Tab stop.
pub fn modal<S: 'static>(top: f32, card: El, dismiss: impl Fn(&mut S) + 'static, t: &Theme) -> El {
    scrim(top, card, t).modal(false, dismiss)
}

/// [`modal`] that hands focus to the control keyed `opener` as it closes (Esc, a button), for
/// an opener that is gone by then: a menu's row, a toolbar overflow item. An opener that stays
/// gets focus back from [`modal`] alone.
#[cfg(test)]
pub fn modal_from<S: 'static>(
    top: f32,
    card: El,
    opener: &str,
    dismiss: impl Fn(&mut S) + 'static,
    t: &Theme,
) -> El {
    modal(top, card, dismiss, t).focus_return(opener)
}

/// [`modal_from`] for an opener that may be gone at close (the row was trashed, the device
/// revoked): focus goes to `opener` when it is still there, else into `fallback` (a pane's or a
/// list's key: its selected item, else its first Tab stop), else nowhere.
///
/// Two return layers: Rune walks the removed nodes innermost first and skips one whose key is
/// gone. The opener's layer is a zero-size leaf inside the scrim (its scope is the scrim), the
/// fallback's is the scrim itself.
pub fn modal_back<S: 'static>(
    top: f32,
    card: El,
    opener: &str,
    fallback: &str,
    dismiss: impl Fn(&mut S) + 'static,
    t: &Theme,
) -> El {
    let back = el().size(0.0, 0.0).pin(Some(0.0), Some(0.0), None, None).focus_return(opener);
    scrim(top, card, t).child(back).modal(false, dismiss).focus_return(fallback)
}

/// A dialog's card. `foot`: the buttons in the caller's layout, the primary first, the way out
/// last, as the board orders them.
pub fn card(name: &str, width: f32, title: &str, sub: Option<Sub>, body: Vec<El>, foot: El, t: &Theme) -> El {
    build(
        Dialog {
            head: Head::Title { title, sub },
            body,
            foot: Foot::Custom(foot),
            ..Dialog::new("dlg", name, width)
        },
        true,
        t,
    )
}

/// The scrim that takes every press outside the card, the card in its middle. `top`: how far
/// under its parent's top it starts (a window's caption bar stays free, so the window can still
/// be moved and closed).
pub fn scrim(top: f32, card: El, t: &Theme) -> El {
    col()
        .key("scrim")
        .pin(Some(0.0), Some(top), Some(0.0), Some(0.0))
        .center()
        .fill(scrim_color(t))
        .child(card)
}

fn title_label(key: String, title: &str, t: &Theme) -> El {
    kit::label(key, title, DIALOG_TITLE, t.text)
        .role("heading")
        .level(2)
        .w_full()
}

/// `keep_body`: an empty body still gets its `p-body` box (the `card(..)` delegate, as before).
fn build(d: Dialog, keep_body: bool, t: &Theme) -> El {
    let p = d.prefix;
    let card = d.surface == Surface::Card;
    let k = |part: &str| format!("{p}-{part}");

    let head = match d.head {
        Head::Title { title, sub } => {
            let mut h = col().key(k("head")).gap(3.0).child(title_label(k("title"), title, t));
            match sub {
                Some(Sub::Wrap(words)) => h = h.child(kit::label(k("sub"), words, SUB, t.text_tertiary).wrap()),
                Some(Sub::Truncate(words)) => h = h.child(kit::label(k("sub"), words, SUB, t.text_tertiary).truncate()),
                None => {}
            }
            Some(if card { h.pt(20.0).px(24.0).pb(4.0) } else { h })
        }
        Head::Custom(content) => {
            let h = col().key(k("head")).gap(3.0).child(content);
            Some(if card { h.pt(20.0).px(24.0).pb(4.0) } else { h })
        }
        Head::Alert { tile, title, sub } => {
            let words = col()
                .key(k("words"))
                .grow(1.0)
                .min_w(0.0)
                .gap(6.0)
                .child(title_label(k("title"), title, t).wrap())
                .child(kit::label(k("sub"), sub, SUB, t.text_tertiary).wrap().w_full());
            let h = row()
                .key(k("head"))
                .w_full()
                .gap(14.0)
                .items_start()
                .child(tile)
                .child(words);
            Some(if card { h.pt(20.0).px(24.0).pb(16.0) } else { h })
        }
        Head::Raw(el) => Some(el),
        Head::None => None,
    };

    let body = if d.body.is_empty() && !keep_body {
        None
    } else {
        let mut b = col().key(k("body")).min_h(0.0);
        if card {
            b = b.pt(d.body_top.unwrap_or(10.0)).px(24.0).pb(18.0).gap(12.0);
        }
        for child in d.body {
            b = b.child(child);
        }
        Some(b)
    };

    let pad = if card { 48.0 } else { 0.0 };
    let enter = d.enter;
    let mark = |i: usize, b: El| if enter == Some(i) { b.default_button() } else { b };
    let foot = match d.foot {
        Foot::Row(buttons) => {
            let mut f = row().center_y().justify_end();
            for (i, b) in buttons.into_iter().enumerate() {
                f = f.child(mark(i, b));
            }
            Some(f)
        }
        Foot::Even(buttons) => {
            let n = buttons.len().max(1) as f32;
            let each = (d.width - pad - 8.0 * (n - 1.0)) / n;
            let mut f = row().center_y().justify_end();
            for (i, b) in buttons.into_iter().enumerate() {
                f = f.child(mark(i, b.w(each)));
            }
            Some(f)
        }
        Foot::Stack(buttons) => {
            let mut f = col().w_full();
            for (i, b) in buttons.into_iter().enumerate() {
                f = f.child(mark(i, b.w_full()));
            }
            Some(f)
        }
        Foot::Custom(f) => Some(f),
        Foot::None => None,
    }
    .map(|f| {
        let f = f.key(k("foot")).gap(8.0);
        if card {
            f.py(16.0)
                .px(24.0)
                .fill(t.surface)
                .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
                .border_layout()
        } else {
            f
        }
    });

    // The shadow and the hairline ring are outside the surface; the clip that rounds the
    // foot's corners would cut them, so it sits one level in.
    let mut clip = col().key(k("clip")).w_full().shrink().min_h(0.0);
    clip = match d.surface {
        Surface::Card => clip.clip_rounded(RADIUS),
        Surface::Panel => {
            let c = clip.clip_rounded(PANEL_RADIUS);
            if d.height.is_some() { c.grow(1.0) } else { c }
        }
        Surface::Sheet => clip.grow(1.0).clip_corners(SHEET_CORNERS),
    };
    for part in [head, body, foot].into_iter().flatten() {
        clip = clip.child(part);
    }

    let surface = col()
        .key(d.key.to_string())
        .role(if d.alert { "alertdialog" } else { "dialog" })
        .label(d.name)
        .w(d.width)
        .fill(t.card);
    let surface = match d.height {
        Some(h) => surface.h(h),
        None => surface,
    };
    let surface = match d.surface {
        // The foot's hairline makes a dialog's height end in a half point, so centring puts
        // its top on a quarter point. The board draws it there, edges half covered; so does this.
        Surface::Card => {
            surface
                .pixel_snap(false)
                .radius(RADIUS)
                .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        }
        Surface::Panel => surface
            .radius(PANEL_RADIUS)
            .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28))),
        Surface::Sheet => surface
            .corners(SHEET_CORNERS)
            .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.3))),
    };
    surface.box_shadow(kit::ring(kit::HAIR, t.separator)).child(clip)
}

#[cfg(test)]
#[path = "dialog_tests.rs"]
mod tests;
