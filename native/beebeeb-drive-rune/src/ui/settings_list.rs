//! Settings lists (`.kit-list`): a small heading, a card of rows, a foot note.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | List | `list(index, title, rows, foot, t)` | `list-<L>-title`, `list-<L>-body`, `list-<L>-foot` |
//! | Row | `item(list, index, &Item, t)` | `item-<L>-<I>`, `-icon`, `-title`, `-sub`, and per trailing piece below |
//! | Value | `Trailing::Value` / `Mono` | `item-<L>-<I>-value` |
//! | Chevron (opens a pane) | `Trailing::Chevron` | `item-<L>-<I>-chev` |
//! | Leaves the app (a web page) | `Trailing::External` | `item-<L>-<I>-ext` |
//! | The chosen one of several | `Trailing::Check` | `item-<L>-<I>-check` |
//! | Pop-up button (a choice) | `Trailing::Pop` | `item-<L>-<I>-pop`, `-pop-label`, `-pop-caret`, `-menu-at` |
//! | Destructive row | `item_danger(list, index, icon, title, t)` | `item-<L>-<I>`, `-icon`, `-title` |
//! | Switch | `item_with(.., vec![switch("item-<L>-<I>", ..)], ..)` | `item-<L>-<I>-switch`, `-knob` |
//!
//! A row is 28 pt, 40 pt with a second line; the board rules a row off from the one
//! above only where the two are siblings in its markup (`Item::ruled`). What a row IS decides how it behaves: `Role::Link` and `Role::Radio` rows take
//! the press, focus and hover; a `Role::Plain` row is text, and a control inside it (a
//! switch, a button) is the thing that takes focus. A row with a pop-up button is ONE control,
//! `Role::PopUp`: the row takes the press and the focus, and the pop-up button inside it is
//! its picture (never a second Tab stop).

use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, radius, type_};
use rune::tok::Family;
use rune::{El, Icon, TextStyle, col, el, row};

use super::kit;
use super::settings_shell::SUB;

/// 10px/600 on a 13px line, tracked .01em: a list's heading (`.kit-list-title`).
pub const LIST_TITLE: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.1);
/// 13px/400 on a 17px line: a row's value (`.kit-item-value`).
pub const VALUE: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// 12.5px/400 mono: a machine's value in a row (`.kit-item-value.font-mono`).
pub const VALUE_MONO: TextStyle = TextStyle::new(12.5, 400.0).family(Family::Mono).lh(17.0);
/// 12.5px/400: a pop-up button's choice (`.mk-pop2`).
pub const POP: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
/// 12px/400 on a 16px line: a list's foot note (`.kit-list-foot`).
pub const FOOT: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);

/// What a row is to a person and to assistive technology.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Text; any control inside it takes the press.
    Plain,
    /// Shows another pane or page.
    Link,
    /// Opens a menu of choices (the row holds a `Trailing::Pop`): Space, Enter and the
    /// arrows Up and Down open it.
    PopUp,
    /// One of several choices; `true` for the chosen one.
    Radio(bool),
}

/// What sits at a row's trailing edge, in order.
#[derive(Clone, PartialEq, Debug)]
pub enum Trailing {
    Value(String),
    Mono(String),
    Chevron,
    External,
    Check,
    /// A pop-up button showing the current choice; `open` while its menu shows. `menu_at`
    /// is where the menu's top-left corner goes, from the row's own (the row marks the point
    /// with an element keyed `item-<L>-<I>-menu-at`, for `El::anchored`).
    Pop {
        choice: String,
        open: bool,
        menu_at: (f32, f32),
    },
}

/// One row of a settings list.
#[derive(Clone, Debug)]
pub struct Item {
    pub icon: Option<&'static Icon>,
    pub title: String,
    pub sub: Option<String>,
    pub trailing: Vec<Trailing>,
    pub role: Role,
    /// A hairline above the row, from 10 pt in (the board draws it between some rows only).
    pub ruled: bool,
}

impl Item {
    pub fn new(icon: Option<&'static Icon>, title: impl Into<String>) -> Self {
        Self {
            icon,
            title: title.into(),
            sub: None,
            trailing: Vec::new(),
            role: Role::Plain,
            ruled: false,
        }
    }
    pub fn sub(mut self, sub: impl Into<String>) -> Self {
        self.sub = Some(sub.into());
        self
    }
    pub fn trailing(mut self, t: Trailing) -> Self {
        self.trailing.push(t);
        self
    }
    /// A row that opens a pane: a chevron at the end, a link.
    pub fn opens(mut self) -> Self {
        self.trailing.push(Trailing::Chevron);
        self.role = Role::Link;
        self
    }
    /// A hairline above this row (`.kit-item + .kit-item::before`).
    pub fn ruled(mut self) -> Self {
        self.ruled = true;
        self
    }
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// The row in words: the title, then what it currently says.
    pub fn name(&self) -> String {
        let mut name = self.title.clone();
        for t in &self.trailing {
            match t {
                Trailing::Value(v) | Trailing::Mono(v) => name = format!("{name}, {v}"),
                Trailing::Pop { choice, .. } => name = format!("{name}, {choice}"),
                Trailing::Check => name = format!("{name}, chosen"),
                Trailing::External => name = format!("{name}, opens in the browser"),
                Trailing::Chevron => {}
            }
        }
        if let Some(sub) = &self.sub {
            name = format!("{name}. {sub}");
        }
        name
    }
}

/// A pop-up button (`.mk-pop2`): the current choice and a caret, raised a little off the
/// card. It is the picture of its row's control: the ROW takes the press and the focus
/// (`Role::PopUp`), so this has no role, no name and no Tab stop of its own.
/// Keys: `<key>-pop`, `<key>-pop-label`, `<key>-pop-caret`.
pub fn pop_button(key: &str, choice: &str, t: &Theme) -> El {
    row()
        .key(format!("{key}-pop"))
        .h(24.0)
        .pl(9.0)
        .pr(6.0)
        .gap(4.0)
        .center_y()
        .radius(radius::MD)
        .fill(t.card)
        .box_shadow(kit::drop(0.5, 1.5, t.shadow_raised.alpha(0.18)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(kit::label(format!("{key}-pop-label"), choice, POP, t.text))
        .child(kit::glyph(
            format!("{key}-pop-caret"),
            &ic::CARET_DOWN,
            12.0,
            t.text_tertiary,
        ))
}

/// A switch at a row's trailing edge: ink when on (never Amber: a switch is not the primary
/// action), the knob in the paper colour when on and white when off, as the board draws it.
/// (`detail_pane::switch` draws the knob white in both states.) Keys: `<key>-switch`, `<key>-knob`.
pub fn switch(key: &str, name: &str, on: bool, t: &Theme) -> El {
    row()
        .key(format!("{key}-switch"))
        .role("switch")
        .label(name)
        .checked(on)
        .size(36.0, 20.0)
        .radius(10.0)
        .fill(if on { t.text } else { t.paper3 })
        .border(1.0, if on { t.text } else { t.separator_strong })
        .border_layout()
        .center_y()
        .focusable(true)
        .child(
            el().key(format!("{key}-knob"))
                .size(14.0, 14.0)
                .radius(7.0)
                .fill(if on { t.paper } else { t.white })
                // `shadow-1`: the light theme's ink in both themes.
                .box_shadow(kit::drop(1.0, 3.0, rune::oklch(0.18, 0.01, 70.0).alpha(0.04)))
                .box_shadow(kit::drop(1.0, 2.0, rune::oklch(0.18, 0.01, 70.0).alpha(0.06)))
                .ml(if on { 18.0 } else { 2.0 }),
        )
}

/// One row (`.kit-item`). Keys: `item-<L>-<I>` and the pieces named in the module's table.
/// The row's own press is the caller's (`.on_press`), for `Role::Link` and `Role::Button`.
pub fn item(list: usize, index: usize, it: &Item, t: &Theme) -> El {
    item_with(list, index, it, Vec::new(), t)
}

/// A row with controls of the caller's at its trailing edge (a button), after the row's own.
pub fn item_with(list: usize, index: usize, it: &Item, extra: Vec<El>, t: &Theme) -> El {
    let key = format!("item-{list}-{index}");
    let mut r = row()
        .key(key.clone())
        .w_full()
        .min_h(if it.sub.is_some() { 40.0 } else { 28.0 })
        .px(10.0)
        .gap(8.0)
        .center_y();
    r = match it.role {
        Role::Plain => r.role("listitem").label(it.name()),
        Role::Link => r
            .role("link")
            .label(it.name())
            .hover(t.hover)
            .press(t.press)
            .focusable(true),
        Role::Radio(chosen) => r
            .role("radio")
            .label(it.name())
            .checked(chosen)
            .hover(t.hover)
            .press(t.press)
            .focusable(true),
        Role::PopUp => {
            let open = it
                .trailing
                .iter()
                .any(|piece| matches!(piece, Trailing::Pop { open: true, .. }));
            r.role("combobox")
                .label(it.name())
                .expanded(open)
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
        }
    };
    for piece in &it.trailing {
        if let Trailing::Pop { menu_at, .. } = piece {
            r = r.child(
                el().key(format!("{key}-menu-at"))
                    .pin(Some(menu_at.0), Some(menu_at.1), None, None)
                    .size(0.0, 0.0),
            );
        }
    }
    if it.ruled {
        r = r.child(
            el().key(format!("{key}-rule"))
                .pin(Some(10.0), Some(0.0), Some(0.0), None)
                .h(kit::HAIR)
                .fill(t.separator),
        );
    }
    if let Some(icon) = it.icon {
        r = r.child(el().key(format!("{key}-icon-box")).w(30.0).center().child(kit::glyph(
            format!("{key}-icon"),
            icon,
            18.0,
            t.text_secondary,
        )));
    }
    let mut words = col()
        .key(format!("{key}-text"))
        .grow(1.0)
        .min_w(0.0)
        .gap(1.0)
        .child(kit::label(format!("{key}-title"), it.title.clone(), type_::ITEM_TITLE, t.text).truncate());
    if let Some(sub) = &it.sub {
        words = words.child(kit::label(format!("{key}-sub"), sub.clone(), SUB, t.text_tertiary).truncate());
    }
    r = r.child(words);
    for piece in &it.trailing {
        r = r.child(match piece {
            Trailing::Value(v) => kit::label(format!("{key}-value"), v.clone(), VALUE, t.text_tertiary),
            Trailing::Mono(v) => kit::label(format!("{key}-value"), v.clone(), VALUE_MONO, t.text_tertiary),
            Trailing::Chevron => kit::glyph(format!("{key}-chev"), &ic::CHEV_RIGHT, 14.0, t.text_quaternary),
            Trailing::External => kit::glyph(format!("{key}-ext"), &ic::EXTERNAL, 14.0, t.text),
            Trailing::Check => kit::glyph(format!("{key}-check"), &ic::CHECK, 16.0, t.text),
            Trailing::Pop { choice, .. } => pop_button(&key, choice, t),
        });
    }
    for child in extra {
        r = r.child(child);
    }
    r
}

/// A list (`.kit-list`): the heading, the card with its rows, the foot note.
/// Keys: `list-<L>-title`, `list-<L>-body`, `list-<L>-foot`.
pub fn list(index: usize, title: Option<&str>, rows: Vec<El>, foot: Option<&str>, t: &Theme) -> El {
    let mut l = col().key(format!("list-{index}")).role("group").gap(6.0);
    if let Some(title) = title {
        l = l
            .label(title)
            .child(el().key(format!("list-{index}-title-box")).px(14.0).child(kit::label(
                format!("list-{index}-title"),
                title,
                LIST_TITLE,
                t.text_tertiary,
            )));
    }
    // The ring is outside the card, so the clip that rounds the rows' hover tint sits one level in.
    let mut rows_box = col()
        .key(format!("list-{index}-rows"))
        .w_full()
        .clip_rounded(radius::LG);
    for r in rows {
        rows_box = rows_box.child(r);
    }
    l = l.child(
        col()
            .key(format!("list-{index}-body"))
            .role("list")
            .w_full()
            .radius(radius::LG)
            .fill(t.card)
            .box_shadow(kit::ring(kit::HAIR, t.separator))
            .child(rows_box),
    );
    if let Some(foot) = foot {
        l = l.child(
            col()
                .key(format!("list-{index}-foot-box"))
                .w_full()
                .items_start()
                .px(16.0)
                .child(kit::label(format!("list-{index}-foot"), foot, FOOT, t.text_tertiary).wrap()),
        );
    }
    l
}

/// `--red-deep`: the ink of a destructive row's title.
pub fn danger_ink(t: &Theme) -> rune::Color {
    rune::color::hex(if t.dark { "#FF645F" } else { "#B00A1D" }).unwrap_or(t.red)
}

/// A destructive row (`.kit-item.is-danger`): its title in red ink, a button to assistive
/// technology. Keys: `item-<L>-<I>`, `-icon`, `-title`.
pub fn item_danger(list: usize, index: usize, icon: &'static Icon, title: &str, t: &Theme) -> El {
    let key = format!("item-{list}-{index}");
    row()
        .key(key.clone())
        .role("button")
        .label(title)
        .w_full()
        .min_h(28.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(el().key(format!("{key}-icon-box")).w(30.0).center().child(kit::glyph(
            format!("{key}-icon"),
            icon,
            18.0,
            t.text_secondary,
        )))
        .child(
            kit::label(format!("{key}-title"), title, type_::ITEM_TITLE, danger_ink(t))
                .grow(1.0)
                .truncate(),
        )
}
