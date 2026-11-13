//! Share (DW-0407): a link to the selection, with who may do what, when it ends and an optional
//! password; or Invite someone… (the Shared section's DW-0514).
//!
//! Nothing is shared until Copy link is pressed: opening the dialog, changing a choice, Cancel
//! and Esc ask nothing of the source. Copy link asks once (`FilesSource::share_link`), with the
//! three choices; "Link copied" (DW-0107) is said when the source says it is on the clipboard.
//!
//! **Secrets.** The link is never drawn: the source puts it on the clipboard. The password is a
//! secure `TextEdit` (`Actions::share_password`): Rune draws bullets and keeps the characters
//! out of the element tree; it is wiped when the dialog closes, however it closes, and the
//! window is excluded from screen capture while the field shows ([`shows_secret`], checked in
//! `shell::host`).

use super::actions_data::{LinkAccess, LinkExpiry, ShareOptions};
use super::dialogs::{self, Dialog};
use super::source::Action;
use super::{Intent, Toast};
use crate::State;
use crate::source::{DriveItem, format_size};
use crate::theme::tokens::{Theme, radius, type_};
use crate::theme::{icons, icons_settings};
use crate::ui::settings_dialog::{self, BUTTON_MD, Weight};
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::{banner, button, field, kit, settings_controls as controls};
use rune::{Align, El, Side, Tx, col, el, row};

/// The key of the password's text input.
pub const PASSWORD_INPUT: &str = "share-password";

/// Which of the two menus is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Access,
    Expiry,
}

/// The open Share dialog: what is shared, and the choices so far.
#[derive(Clone, Debug)]
pub struct Share {
    pub items: Vec<DriveItem>,
    pub access: LinkAccess,
    pub expires: LinkExpiry,
    /// The Password switch. The password itself is `Actions::share_password`.
    pub password_on: bool,
    /// The password was asked for and none was typed: Copy link said so and made no link.
    pub password_missing: bool,
    pub menu: Option<Menu>,
}

/// Is a password field on screen? The window is then kept out of screen captures.
pub fn shows_secret(s: &State) -> bool {
    matches!(&s.actions.dialog, Some(Dialog::Share(share)) if share.password_on)
}

impl State {
    /// Share… (the file menu, the details panel, the preview's mark): the dialog for the
    /// selection, on the board's defaults (Can view, 30 days, no password).
    pub fn begin_share(&mut self) -> bool {
        let items = self.selected_items();
        // No dialog for nothing, and none for a protected item, whoever asks.
        if items.is_empty() || self.protects_selection() {
            return false;
        }
        self.close_menu();
        self.actions.share_password = rune::TextEdit::new("").secure();
        self.actions.dialog = Some(Dialog::Share(Share {
            items,
            access: LinkAccess::default(),
            expires: LinkExpiry::default(),
            password_on: false,
            password_missing: false,
            menu: None,
        }));
        true
    }

    /// Share… (the file menu, the details panel): the dialog; for a protected item the refusal,
    /// said under its own label and with no Try again (there is nothing to repeat).
    pub fn ask_share(&mut self, tx: &mut Tx) {
        if self.protects_selection() {
            self.failed(Action::Share, super::source::ActionError::Protected, tx);
        } else {
            self.intent(Intent::Frame("DW-0407"));
        }
    }

    fn share_mut(&mut self) -> Option<&mut Share> {
        match self.actions.dialog.as_mut() {
            Some(Dialog::Share(share)) => Some(share),
            _ => None,
        }
    }

    pub fn share_toggle_menu(&mut self, menu: Menu) {
        if let Some(share) = self.share_mut() {
            share.menu = (share.menu != Some(menu)).then_some(menu);
        }
    }

    pub fn share_choose(&mut self, menu: Menu, index: usize) {
        let Some(share) = self.share_mut() else { return };
        match menu {
            Menu::Access => {
                if let Some(&access) = LinkAccess::ALL.get(index) {
                    share.access = access;
                }
            }
            Menu::Expiry => {
                if let Some(&expires) = LinkExpiry::ALL.get(index) {
                    share.expires = expires;
                }
            }
        }
        share.menu = None;
    }

    /// The Password switch: on shows the field and gives it the keyboard; off wipes what was typed.
    pub fn share_toggle_password(&mut self, tx: &mut Tx) {
        let Some(share) = self.share_mut() else { return };
        share.password_on = !share.password_on;
        share.password_missing = false;
        let on = share.password_on;
        self.actions.share_password.clear();
        if on {
            // The field takes the keyboard when it appears (`autofocus`); this is for a field
            // that is there already.
            tx.focus(PASSWORD_INPUT, false);
        }
    }

    /// Copy link: make the link with the choices, once. With the Password switch on and nothing
    /// typed, no link is made: a link that should ask for a password never goes out without one.
    pub fn share_copy_link(&mut self, tx: &mut Tx) {
        let Some(Dialog::Share(share)) = self.actions.dialog.clone() else {
            return;
        };
        if self.needs_session(tx) {
            return;
        }
        let typed = self.actions.share_password.text.clone();
        if share.password_on && typed.is_empty() {
            if let Some(share) = self.share_mut() {
                share.password_missing = true;
            }
            tx.focus(PASSWORD_INPUT, true);
            return;
        }
        let options = ShareOptions {
            access: share.access,
            expires: share.expires,
            password: share.password_on.then_some(typed),
        };
        self.forget_undo();
        let answer = self.actions.source.share_link(&share.items, &options);
        drop(options);
        // The dialog closes either way; a failure says so, with Try again (DW-0109).
        self.close_dialog(tx);
        match answer {
            Ok(()) => {
                self.actions.retry_share = None;
                self.show_toast(Toast::LinkCopied, tx);
            }
            Err(why) => {
                // What Try again opens: the same items and choices, without the password.
                self.actions.retry_share = Some(Share {
                    password_missing: false,
                    menu: None,
                    ..share
                });
                self.failed(Action::ShareLink, why, tx);
            }
        }
    }

    /// Try again on "Couldn't make the link": the dialog opens again on the same items with the
    /// same choices. Nothing is asked of the source: the link is made by Copy link, as before,
    /// and when it should ask for a password, that password is typed again (it was wiped).
    pub fn share_again(&mut self, tx: &mut Tx) {
        let Some(share) = self.actions.retry_share.take() else {
            return;
        };
        self.close_menu();
        self.actions.share_password = rune::TextEdit::new("").secure();
        let password_on = share.password_on;
        self.actions.dialog = Some(Dialog::Share(share));
        if password_on {
            tx.focus(PASSWORD_INPUT, false);
        }
    }

    /// Invite someone…: the Shared section's invite dialog for this item's share (DW-0514).
    pub fn share_invite(&mut self, tx: &mut Tx) {
        let Some(Dialog::Share(share)) = self.actions.dialog.clone() else {
            return;
        };
        self.close_dialog(tx);
        self.intent(Intent::Frame("DW-0514"));
        let id = share
            .items
            .first()
            .and_then(|item| self.actions.source.share_of(item))
            .filter(|id| self.frames.shared.share(id).is_some());
        let Some(id) = id else {
            // There is no share to invite to (the engine makes one; nothing is wired here).
            self.failed(Action::Invite, super::source::ActionError::Unavailable, tx);
            return;
        };
        use crate::shell::app::{Event, Route, Section};
        crate::shell::host::dispatch(self, Event::Navigate(Route::section(Section::Shared)));
        self.frames.shared.start_invite(&id);
    }
}

/// Wipe the password (the dialog closed, or the switch went off).
pub fn wipe(s: &mut State) {
    s.actions.share_password.clear();
    s.actions.share_password.focused = false;
}

/// A dialog-size amber button with a glyph (`.kit-btn-md` with `icon`).
fn primary(index: usize, label: &str, icon: &'static rune::Icon, t: &Theme) -> El {
    let key = format!("btn-{index}");
    let fill = t.accent;
    row()
        .key(key.clone())
        .role("button")
        .label(label)
        .h(28.0)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(radius::MD)
        .fill(fill)
        .hover(fill.mix(rune::Color::BLACK, 0.05))
        .press(fill.mix(rune::Color::BLACK, 0.10))
        .focusable(true)
        .child(kit::glyph(format!("{key}-icon"), icon, 16.0, t.accent_text_on))
        .child(kit::label(format!("{key}-label"), label, BUTTON_MD, t.accent_text_on))
}

/// The file at the top of a dialog: its tile, its name, and a line under it.
/// Keys: `dlg-file`, `dlg-file-tile`, `dlg-file-icon`, `dlg-file-name`, `dlg-file-sub`.
pub fn file_head(item: &DriveItem, name: &str, sub: &str, mono: bool, tile: f32, t: &Theme) -> El {
    let sub_style = if mono { type_::KBD } else { type_::CAPTION };
    row()
        .key("dlg-file")
        .w_full()
        .center_y()
        .gap(12.0)
        .child(big_tile("dlg-file", item, tile, t))
        .child(
            col()
                .key("dlg-file-words")
                .grow(1.0)
                .min_w(0.0)
                .child(kit::label("dlg-file-name", name, type_::HEAD, t.text).truncate())
                .child(kit::label("dlg-file-sub", sub, sub_style, t.text_tertiary).truncate()),
        )
}

/// A file's tile at another size than the row's 22 pt (`FTile size`): the corner is 24 % of
/// the side, the glyph 62 %. Keys: `<key>-tile`, `<key>-icon`.
pub fn big_tile(key: &str, item: &DriveItem, size: f32, t: &Theme) -> El {
    let corner = (size * 0.24).round();
    let tile = el()
        .key(format!("{key}-tile"))
        .size(size, size)
        .radius(corner)
        .clip_rounded(corner)
        .center();
    if item.has_thumbnail {
        return crate::ui::file_list::thumbnail(tile);
    }
    tile.fill(t.tile).child(kit::glyph(
        format!("{key}-icon"),
        crate::ui::file_list::file_type_icon(item),
        (size * 0.62).round(),
        t.text_secondary,
    ))
}

fn menu_layer(share: &Share, t: &Theme) -> Option<El> {
    let menu = share.menu?;
    let (name, anchor, labels, chosen): (&str, &str, Vec<&str>, usize) = match menu {
        Menu::Access => (
            "Anyone with the link",
            "item-0-0-menu-at",
            LinkAccess::ALL.iter().map(|a| a.words()).collect(),
            LinkAccess::ALL.iter().position(|a| *a == share.access).unwrap_or(0),
        ),
        Menu::Expiry => (
            "Link expires",
            "item-0-1-menu-at",
            LinkExpiry::ALL.iter().map(|e| e.choice()).collect(),
            LinkExpiry::ALL.iter().position(|e| *e == share.expires).unwrap_or(0),
        ),
    };
    let items = labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            controls::menu_item(0, i, label, i == chosen, false, t)
                .on_press(move |s: &mut State| s.share_choose(menu, i))
        })
        .collect();
    Some(
        controls::menu(0, name, 130.0, items, t)
            .anchored(anchor, Side::Bottom, Align::Start, 0.0)
            .modal(true, |s: &mut State| {
                if let Some(share) = s.share_mut() {
                    share.menu = None;
                }
            }),
    )
}

pub fn view(s: &State, share: &Share, t: &Theme) -> El {
    let Some(first) = share.items.first() else {
        return el().key("dialog-share-empty");
    };
    // Several files share as one link to a read-only folder view: the head says how many.
    let (name, sub) = if share.items.len() == 1 {
        (first.name.clone(), first.size.map(format_size).unwrap_or_default())
    } else {
        let total: i64 = share.items.iter().filter_map(|i| i.size).sum();
        (format!("{} items", share.items.len()), format_size(total))
    };

    let pop = |menu: Menu, choice: &str| Trailing::Pop {
        choice: choice.to_string(),
        open: share.menu == Some(menu),
        // Under the pop-up button's left edge, a little below the row.
        menu_at: (252.0, 28.0),
    };
    // The keyboard arrives on the first choice (Rune gives a modal's first control the
    // keyboard; the password field takes it when it shows): never on the button that sends.
    // Enter and Space on arrival open this pop-up; a link is made only by going to Copy link
    // (`actions_tests::the_keyboard_arrives_on_a_choice_…` binds both).
    let rows = vec![
        lists::item(
            0,
            0,
            &Item::new(Some(&icons::LINK), "Anyone with the link")
                .trailing(pop(Menu::Access, share.access.words()))
                .role(Role::PopUp),
            t,
        )
        .on_press(|s: &mut State| s.share_toggle_menu(Menu::Access))
        .on_key(|s: &mut State, ev: &rune::KeyEvent, _: &mut Tx| {
            dialogs::plain_enter(ev) && {
                s.share_toggle_menu(Menu::Access);
                true
            }
        }),
        lists::item(
            0,
            1,
            &Item::new(Some(&icons_settings::CLOCK), "Link expires")
                .trailing(pop(Menu::Expiry, share.expires.words()))
                .role(Role::PopUp)
                .ruled(),
            t,
        )
        .on_press(|s: &mut State| s.share_toggle_menu(Menu::Expiry))
        .on_key(|s: &mut State, ev: &rune::KeyEvent, _: &mut Tx| {
            dialogs::plain_enter(ev) && {
                s.share_toggle_menu(Menu::Expiry);
                true
            }
        }),
        lists::item_with(
            0,
            2,
            &Item::new(Some(&icons::LOCK), "Password").ruled(),
            vec![banner::pressable(
                lists::switch("item-0-2", "Password", share.password_on, t)
                    .description("The link then asks for a password you choose."),
                State::share_toggle_password,
            )],
            t,
        ),
    ];
    let mut body = vec![
        file_head(first, &name, &sub, true, 36.0, t),
        lists::list(0, None, rows, None, t),
    ];
    if share.password_on {
        let message = if share.password_missing {
            "Type a password, or turn Password off."
        } else {
            "Whoever opens the link needs this password. Send it another way than the link."
        };
        body.push(field::bare_field(
            field::Bare {
                key: "field-0",
                name: "Password for the link",
                message: Some(message),
                error: share.password_missing,
            },
            &s.actions.share_password,
            PASSWORD_INPUT,
            |s: &mut State| &mut s.actions.share_password,
            |input| {
                input
                    .autofocus()
                    .on_submit(|s: &mut State, tx: &mut Tx| s.share_copy_link(tx))
            },
            t,
        ));
    }
    body.push(
        kit::label(
            "dlg-text-0",
            if share.items.len() == 1 {
                "The link carries the key to this file. Beebeeb never sees it."
            } else {
                "The link carries the key to these files. Beebeeb never sees it."
            },
            type_::CAPTION,
            t.text_tertiary,
        )
        .wrap(),
    );

    let foot = col()
        .child(
            primary(0, "Copy link", &icons::LINK, t)
                .w_full()
                .default_button()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.share_copy_link(tx)),
        )
        .child(
            // Enter on a focused button presses THAT button, never the default one (the Files
            // window has no web focus, so Rune would send Return to Copy link).
            banner::pressable(
                button::outline_button("btn-1", "Invite someone\u{2026}", Some(&icons::SHARED), t).w_full(),
                State::share_invite,
            ),
        )
        // `.flex-1` between the actions and the way out: one more gap.
        .child(el().key("dlg-foot-gap").h(0.0))
        .child(banner::pressable(
            settings_dialog::button(2, "Cancel", Weight::Quiet, t).w_full(),
            State::close_dialog,
        ));
    let dialog = dialogs::shell("Share", 440.0, "Share", None, body, foot, t);
    match menu_layer(share, t) {
        Some(menu) => dialog.child(menu),
        None => dialog,
    }
}
