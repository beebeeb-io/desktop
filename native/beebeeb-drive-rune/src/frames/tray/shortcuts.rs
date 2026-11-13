//! "Commands and keyboard shortcuts" (DW-1232, DW-1233, DW-1234): a Windows app has no menus
//! along the top of the screen, so this dialog lists every command of the other desktop's app
//! menus with its key, or with where it lives. The shell opens it on Ctrl+/ and draws the
//! scrim and the frame behind (`registry::DIALOGS`); this file is the card.
//!
//! The board draws three contents of the one dialog and no control to go between them:
//! Beebeeb, File, Edit (DW-1232); View, Go, Window, Help (DW-1233); and the editor's lists
//! while a document is open (DW-1234). Page Down / Page Up (and the arrow keys) turn between
//! the first two; a window with a document open shows the editor's.
//!
//! Every measurement is the board's computed CSS (`.wn-dlg`, `.wn-cmds`, `.wn-cmd`).

use rune::{Align, Color, Cx, El, Key, KeyEvent, ScrollSpec, TextStyle, Tx, col, el, row};

use super::commands::{APP, EDITOR, WINDOW};
use crate::State;
use crate::frames::shared::parts::controls::{ButtonKind, ButtonSize, button_keyed};
use crate::shell::app::{Dialog, Event};
use crate::shell::registry::{self, DialogFrame, FeedbackPolicy, FrameGroup};
use crate::theme::tokens::{self, Theme};
use crate::ui::kit;

/// One of the other desktop's menus: its name and its commands, each with its key or its home.
pub struct List {
    pub name: &'static str,
    pub commands: &'static [(&'static str, &'static str)],
}

/// Which of the window's two contents shows (a document open has one content, the editor's).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Page {
    /// Beebeeb, File, Edit (DW-1232).
    #[default]
    App,
    /// View, Go, Window, Help (DW-1233).
    Window,
}

/// What the dialog keeps on `State`.
#[derive(Default)]
pub struct Shortcuts {
    pub page: Page,
}

pub const TITLE: &str = "Commands and keyboard shortcuts";
pub const SUB: &str = "Ctrl+/ or the account menu. Each command lives in the window, a right-click menu or Settings; this list shows where.";
/// `.wn-dlg`: "width: 1120px", never nearer than 24 px to the window's edges.
pub const WIDTH: f32 = 1120.0;
const MARGIN: f32 = 24.0;
pub const CLOSE_KEY: &str = "btn-close";
pub const BODY_KEY: &str = "sc-body";

mod type_sc {
    use super::TextStyle;
    /// `.t-title` in a dialog's head.
    pub const TITLE: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.204);
    /// `.t-sub` in a dialog's head.
    pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
    /// `.wn-cmdhead`: a menu's name, in capitals.
    pub const HEAD: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0).tracking(0.66);
    /// `.wn-cmd`: a command.
    pub const NAME: TextStyle = TextStyle::new(12.5, 400.0).lh(16.0);
    /// `.wn-cmd .h`: its key, or where it lives.
    pub const HOME: TextStyle = TextStyle::new(11.5, 400.0).lh(16.0);
}

/// The registry's line (`shell::registry::DIALOGS`). The dialog shows no file name and no
/// secret; comment mode stays off until the lane has the proving test of `feedback/README.md`.
pub const DIALOG: DialogFrame = DialogFrame {
    group: FrameGroup::ShortcutsDialog,
    view: card,
    on_key: Some(on_key),
    feedback: FeedbackPolicy::Denied("its lane has not opted in yet"),
};

/// The lists the dialog shows now: the editor's while a document is open, else the page.
pub fn lists(s: &State) -> &'static [List] {
    if registry::group_for(&s.shell.app) == FrameGroup::Viewer {
        return EDITOR;
    }
    match s.shortcuts.page {
        Page::App => APP,
        Page::Window => WINDOW,
    }
}

/// Opens the dialog on its first page, as Ctrl+/ does.
pub fn open(s: &mut State) {
    s.shortcuts.page = Page::App;
    crate::shell::host::dispatch(s, Event::OpenDialog(Dialog::Shortcuts));
}

/// Closes it (Close; Esc is the shell's).
pub fn close(s: &mut State) {
    if s.shell.app.dialog() == Some(&Dialog::Shortcuts) {
        crate::shell::host::dispatch(s, Event::Escape);
    }
}

/// A closed dialog forgets its page: the next opening starts on the first (`host::dispatch`
/// calls this after every event, as it does for settings).
pub fn sync(s: &mut State) {
    if s.shell.app.dialog() != Some(&Dialog::Shortcuts) {
        s.shortcuts.page = Page::App;
    }
}

/// The dialog's keys: Page Down, Page Up and the arrow keys turn the page. Esc is the shell's.
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let m = ev.mods;
    if m.cmd || m.ctrl || m.alt || m.shift || m.logo {
        return false;
    }
    let to = match ev.key {
        Key::PageDown | Key::Right | Key::Down => Page::Window,
        Key::PageUp | Key::Left | Key::Up => Page::App,
        _ => return false,
    };
    if registry::group_for(&s.shell.app) == FrameGroup::Viewer || s.shortcuts.page == to {
        return false;
    }
    s.shortcuts.page = to;
    true
}

/// The card, in the place the board gives it: centred in the window under the caption, 24 px
/// clear of every edge; in a window too low for it the lists scroll and the card never grows
/// past the window (`max-height: calc(100% - 48px)`).
pub fn card(_dialog: &Dialog, s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    let lists = lists(s);
    let names: Vec<&str> = lists.iter().map(|list| list.name).collect();
    let card = col()
        .key("sc-dlg")
        .role("dialog")
        .label(format!("{TITLE}: {}", names.join(", ")))
        // The foot's hairline makes the height end in a half point, so centring puts the top
        // on a quarter point, where the board draws it.
        .pixel_snap(false)
        .w_full()
        .max_w(WIDTH)
        .shrink()
        .min_h(0.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key("sc-clip")
                .w_full()
                .shrink()
                .min_h(0.0)
                .clip_rounded(8.0)
                .child(head(&t))
                .child(body(lists, &t))
                .child(foot(&t)),
        );
    // A column, so the card gives up height (its main axis) in a window too low for it.
    col()
        .key("sc-place")
        .w_full()
        .h_full()
        .pt(crate::TITLEBAR_H + MARGIN)
        .pb(MARGIN)
        .px(MARGIN)
        .center()
        .child(card)
}

/// `.wn-dlg-head`: "padding: 20px 24px 4px", "gap: 3px".
fn head(t: &Theme) -> El {
    col()
        .key("sc-head")
        .w_full()
        .pt(20.0)
        .px(24.0)
        .pb(4.0)
        .gap(3.0)
        .child(kit::label("sc-title", TITLE, type_sc::TITLE, t.text).role("heading"))
        .child(kit::label("sc-sub", SUB, type_sc::SUB, t.text_tertiary))
}

/// `.wn-dlg-body` ("padding: 10px 24px 18px") around `.wn-cmds`: one column per menu, 22 px apart.
fn body(lists: &'static [List], t: &Theme) -> El {
    let mut grid = row().key("sc-lists").w_full().gap(22.0).items_start();
    for (c, list) in lists.iter().enumerate() {
        grid = grid.child(column(c, list, t));
    }
    col()
        .key(BODY_KEY)
        .w_full()
        .shrink()
        .min_h(0.0)
        .pt(10.0)
        .px(24.0)
        .pb(18.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .child(grid)
}

fn column(c: usize, list: &List, t: &Theme) -> El {
    let mut column = col()
        .key(format!("sc-{c}"))
        .role("list")
        .label(list.name)
        .grow(1.0)
        .min_w(0.0)
        // `.wn-cmdhead`: "padding: 2px 0 6px", in capitals ("text-transform: uppercase").
        .child(
            kit::label(
                format!("sc-{c}-head"),
                list.name.to_uppercase(),
                type_sc::HEAD,
                t.text_tertiary,
            )
            .role("heading")
            .w_full()
            .pt(2.0)
            .pb(6.0),
        );
    for (r, (name, home)) in list.commands.iter().enumerate() {
        column = column.child(command(c, r, name, home, t));
    }
    column
}

/// `.wn-cmd`: the command at the start, its key or its home at the end, on one baseline
/// ("align-items: baseline": the smaller words sit half a point lower), 12 px apart, both
/// giving way and wrapping when the column is too narrow; a hairline under it.
fn command(c: usize, r: usize, name: &str, home: &str, t: &Theme) -> El {
    let key = format!("sc-{c}-{r}");
    col()
        .key(key.clone())
        .role("listitem")
        // A row with no home (Free up space) is read by its name alone.
        .label(if home.is_empty() {
            name.to_string()
        } else {
            format!("{name}: {home}")
        })
        .w_full()
        .child(
            row()
                .key(format!("{key}-words"))
                .w_full()
                .py(3.0)
                .gap(12.0)
                .justify_between()
                .items_start()
                .child(
                    kit::label(format!("{key}-name"), name, type_sc::NAME, t.text)
                        .wrap()
                        .shrink(),
                )
                .child(if home.is_empty() {
                    // No home (Free up space): the board's empty box, nothing wide or tall, on the baseline.
                    kit::label(format!("{key}-home"), home, type_sc::HOME, t.text_secondary)
                        .size(0.0, 0.0)
                        .mt(12.5)
                } else {
                    kit::label(format!("{key}-home"), home, type_sc::HOME, t.text_secondary)
                        .wrap()
                        .shrink()
                        .align(Align::End)
                        // The line box is 16.5 tall: the smaller words hang half a point lower.
                        .mt(0.5)
                }),
        )
        // `border-bottom: .5px solid var(--kit-hair)`.
        .child(el().key(format!("{key}-line")).w_full().h(kit::HAIR).fill(t.separator))
}

/// `.wn-dlg-foot`: the ground, a hairline over it, "padding: 16px 24px", Close at the end.
fn foot(t: &Theme) -> El {
    row()
        .key("sc-foot")
        .w_full()
        .px(24.0)
        .py(16.0)
        .justify_end()
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(
            button_keyed(
                CLOSE_KEY.into(),
                "Close",
                ButtonKind::Outline,
                ButtonSize::Medium,
                None,
                t,
            )
            .focus_ring(2.0, 2.0, t.accent_deep)
            .on_press(close),
        )
}
