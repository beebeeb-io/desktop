//! What Mail Backup draws inside Backups: its page (on DW-0740, off DW-0740b, the first archive
//! DW-1405, a refused password DW-1406) and its dialogs (DW-1401 to DW-1404, DW-0743, DW-0743b).
//!
//! Keys: the settings lists, buttons, tiles, pill, banner and dialog keep the keys every
//! settings pane has; this file adds `mail-title`, `mail-sub`, `mail-note`, `mail-count`,
//! `mail-left` and the fields `field-<N>` (`-label`, `-value`, `-caret`, `-msg`).

use rune::tok::Family;
use rune::{Color, El, Icon, Span, TextEdit, TextStyle, col, el, rich, row, text_input};

use super::mail::{Connection, Field, Mail, MailFolder, MailState};
use super::source::BackupsSource;
use super::view::thousands;
use super::{BackupsUi, Dialog, Page};
use crate::State;
use crate::theme::icons as drive_icons;
use crate::theme::icons_files as file_icons;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::backups_kit::{self as bk, Md, TileTone};
use crate::ui::settings_card as cards;
use crate::ui::settings_controls::{self as controls, BANNER_TEXT, BANNER_TITLE, Emphasis};
use crate::ui::settings_dialog::{self as dialogs, DIALOG_TITLE, Weight};
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::settings_notice::{self as notice, PILL, Tone};
use crate::ui::settings_shell::{self as shell, SUB};
use crate::ui::{au, kit, settings_meter};

/// A folder that is not ticked: a point where the tick would be (`M12 11.9v.2`).
static NOT_TICKED: Icon = Icon {
    name: "not-ticked",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M12 11.9v.2"></path></g>"#,
};

/// 11px/500 on 15px: a field's label (`.kit-field-outer`).
const FIELD_LABEL: TextStyle = TextStyle::new(11.0, 500.0).lh(15.0);
/// 13px/400 on 17px: what a field holds.
const FIELD_VALUE: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// The same, hidden: bullets 1.56px apart (`.is-secure`).
const FIELD_SECURE: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0).tracking(1.56);
/// 11.5px mono on 17px: a server's name, a port.
const FIELD_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(17.0);
/// 12px/400 on 16px: the words under a field.
const FIELD_MSG: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);

/// Words with a machine's value inside them (`mono`: byte ranges), noted for the gate as one text.
fn words_rich(key: &str, words: &str, mono: &[std::ops::Range<usize>], style: TextStyle, ink: Color) -> El {
    kit::note_text(key, key, words, style, ink);
    let spans = mono.iter().map(|r| Span::new(r.start, r.end).mono()).collect();
    rich(words, spans).key(key.to_string()).font(style).color(ink)
}

/// "On" on green (`.kit-pill.tone-green`). Keys: `pill-0`, `-dot`, `-label`.
fn pill_on(t: &Theme) -> El {
    row()
        .key("pill-0")
        .h(20.0)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(10.0)
        .fill(t.green.alpha(0.14))
        .child(settings_meter::disc("pill-0-dot", 6.0, t.green))
        .child(kit::label("pill-0-label", "On", PILL, au::green_text(t)))
}

/// The page's head: the mail tile, what Mail Backup is doing, and the state's one control.
fn head(title: &str, sub: &str, trailing: Option<El>, t: &Theme) -> El {
    let mut h = row()
        .key("mail-head")
        .w_full()
        .gap(12.0)
        .center_y()
        .child(notice::tile_sized(0, Tone::Neutral, &ic::MAIL, 40.0, t))
        .child(
            col()
                .key("mail-head-words")
                .grow(1.0)
                .min_w(0.0)
                .child(
                    kit::label("mail-title", title, DIALOG_TITLE, t.text)
                        .role("heading")
                        .level(1)
                        .w_full(),
                )
                .child(kit::label("mail-sub", sub, SUB, t.text_tertiary).w_full()),
        );
    if let Some(trailing) = trailing {
        h = h.child(trailing);
    }
    h
}

/// An inline notice in red (`.kit-banner.tone-red`): what failed and what it did not touch,
/// with a machine's value in the words, and at most one action.
/// Keys: `banner-<N>`, `-icon`, `-title`, `-text`.
fn banner_red(
    index: usize,
    title: &str,
    words: &str,
    mono: &[std::ops::Range<usize>],
    action: Option<El>,
    t: &Theme,
) -> El {
    let key = format!("banner-{index}");
    let mut b = row()
        .key(key.clone())
        .role("alert")
        .label(format!("{title}. {words}"))
        .w_full()
        .items_start()
        .gap(10.0)
        .py(11.0)
        .px(13.0)
        .radius(13.0)
        .fill(t.red.alpha(0.12))
        .child(el().key(format!("{key}-icon-box")).pt(1.0).child(kit::glyph(
            format!("{key}-icon"),
            &ic::WARN_CIRCLE,
            16.0,
            t.red,
        )))
        .child(
            col()
                .key(format!("{key}-words"))
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(kit::label(format!("{key}-title"), title, BANNER_TITLE, t.text).w_full())
                .child(words_rich(&format!("{key}-text"), words, mono, BANNER_TEXT, t.text_secondary).wrap()),
        );
    if let Some(action) = action {
        b = b.child(action);
    }
    b
}

fn account_rows(list: usize, mail: &Mail, t: &Theme) -> El {
    let mut rows: Vec<El> = mail
        .accounts
        .iter()
        .enumerate()
        .map(|(i, account)| {
            let folders = if account.folders == 1 {
                "1 folder".to_string()
            } else {
                format!("{} folders", thousands(account.folders))
            };
            let mut item = Item::new(Some(&ic::MAIL), account.address.clone())
                .sub(format!("IMAP \u{00B7} {} \u{00B7} {folders}", account.server));
            if let Some(last) = &account.last {
                item = item.trailing(Trailing::Mono(last.clone()));
            }
            if i > 0 {
                item = item.ruled();
            }
            // The chevron is the board's; the board gives the row no frame to open, so it is
            // drawn and takes no press.
            lists::item(list, i, &item.trailing(Trailing::Chevron), t)
        })
        .collect();
    rows.push(
        lists::item(
            list,
            mail.accounts.len(),
            &Item::new(Some(&drive_icons::PLUS), "Add a mail account\u{2026}").role(Role::Link),
            t,
        )
        .on_press(|s: &mut State| s.backups.mail_add_account()),
    );
    lists::list(list, Some("Mail accounts"), rows, None, t)
}

fn open_folder_row(list: usize, index: usize, t: &Theme) -> El {
    lists::item(list, index, &Item::new(Some(&ic::FOLDER), "Open its folder").opens(), t)
        .on_press(|s: &mut State| s.backups.mail_open_folder())
}

/// The Mail Backup page. `over`: a dialog is drawn over it (the page then has no primary
/// action of its own: the dialog's is the one Amber button). Returns the children and how
/// many lists and buttons they hold.
pub fn page(mail: &Mail, over: bool, t: &Theme) -> (Vec<El>, usize, usize) {
    let crumb = shell::crumb("Backups", t, |s: &mut State| s.backups.show(Page::Section));
    let mut children = Vec::new();
    let mut buttons = 0;
    let mut lists_drawn = 0;
    if let Some(failure) = &mail.failure {
        let words = format!(
            "The app password was changed or revoked. Nothing was deleted; the last backup is from today at {}.",
            failure.last_at
        );
        let at = words.len() - 1 - failure.last_at.len();
        children.push(banner_red(
            0,
            &format!("Couldn't back up {}", failure.address),
            &words,
            &[at..at + failure.last_at.len()],
            Some(
                controls::button(buttons, "Update password", None, Emphasis::Plain, t)
                    .on_press(|s: &mut State| s.backups.mail_update_password()),
            ),
            t,
        ));
        buttons += 1;
    }
    children.push(crumb);
    match &mail.state {
        MailState::On => {
            children.push(head(
                "Mail Backup is on",
                &format!("Runs on this PC ({})", mail.pc),
                Some(pill_on(t)),
                t,
            ));
            let mut facts = Vec::new();
            if let Some(used) = &mail.used {
                facts.push(Item::new(None, "Used").trailing(Trailing::Mono(used.clone())));
            }
            if let Some(last) = &mail.last_backup {
                facts.push(Item::new(None, "Last backup").trailing(Trailing::Mono(last.clone())));
            }
            if let Some(next) = &mail.next_backup {
                facts.push(Item::new(None, "Next backup").trailing(Trailing::Value(next.clone())));
            }
            let mut rows: Vec<El> = facts
                .into_iter()
                .enumerate()
                .map(|(i, item)| lists::item(0, i, &if i > 0 { item.ruled() } else { item }, t))
                .collect();
            rows.push(open_folder_row(0, rows.len(), t));
            children.push(lists::list(0, None, rows, None, t));
            children.push(account_rows(1, mail, t));
            children.push(lists::list(
                2,
                Some("Turn off"),
                vec![
                    lists::item_danger(2, 0, &ic::POWER, "Turn off Mail Backup", t)
                        .on_press(|s: &mut State| s.backups.mail_ask_turn_off()),
                ],
                Some(
                    "Turning it off offers to move its files to Trash. If you keep them, the folder stays read-only until you turn it on again.",
                ),
                t,
            ));
            lists_drawn = 3;
        }
        MailState::Off => {
            let turn_on = if over {
                bk::button_md(buttons, "Turn on\u{2026}", None, Md::Plain, t)
            } else {
                dialogs::button(buttons, "Turn on\u{2026}", Weight::Primary, t)
            }
            .on_press(|s: &mut State| s.backups.mail_ask_turn_on());
            buttons += 1;
            children.push(head(
                "Mail Backup is off",
                "It runs on a PC: this one, once you turn it on.",
                Some(turn_on),
                t,
            ));
            if let Some(used) = &mail.used {
                let mut rows = vec![lists::item(
                    0,
                    0,
                    &Item::new(Some(&ic::FOLDER), "Mail Backup")
                        .sub("Read-only until you turn it on again")
                        .trailing(Trailing::Mono(used.clone())),
                    t,
                )];
                if let Some(last) = &mail.last_backup {
                    rows.push(lists::item(
                        0,
                        1,
                        &Item::new(None, "Last backup")
                            .trailing(Trailing::Mono(last.clone()))
                            .ruled(),
                        t,
                    ));
                }
                rows.push(open_folder_row(0, rows.len(), t));
                children.push(lists::list(0, Some("Kept folder"), rows, None, t));
                lists_drawn = 1;
            }
            children.push(
                kit::label(
                    "mail-note",
                    "Mail Backup copies your mail accounts into a protected folder inside Backups, encrypted on this PC first. It reads your mail over IMAP; it never sends mail and never changes your mailbox.",
                    SUB,
                    t.text_tertiary,
                )
                .wrap()
                .w_full(),
            );
        }
        MailState::First {
            done,
            total,
            minutes_left,
            paused,
        } => {
            let now_paused = *paused;
            let pause = controls::button(
                buttons,
                if now_paused { "Resume" } else { "Pause" },
                (!now_paused).then_some(&file_icons::PAUSE),
                Emphasis::Plain,
                t,
            )
            .on_press(move |s: &mut State| s.backups.mail_pause(!now_paused));
            buttons += 1;
            children.push(head(
                if now_paused {
                    "Mail backup is paused"
                } else {
                    "Backing up your mail"
                },
                "The first backup takes longest. You can use your PC as normal.",
                Some(pause),
                t,
            ));
            let count = format!("{} of {}", thousands(*done), thousands(*total));
            let words = format!("{count} messages");
            // Whole per cents, as the board draws 6,912 of 17,480 at 40 %.
            let fraction = if *total == 0 {
                0.0
            } else {
                (f64::from(*done) * 100.0 / f64::from(*total)).round() as f32 / 100.0
            };
            let left = format!("about {minutes_left} min left");
            // The line as one text too: the gate's key for a card's small print (`card-<N>-sub`).
            kit::note_text(
                "card-0-sub",
                "card-0-sub",
                &format!("{words}{left}"),
                SUB,
                t.text_tertiary,
            );
            children.push(
                cards::column(0, 12.0, 8.0, t)
                    .child(
                        row()
                            .key("card-0-sub")
                            .w_full()
                            .justify_between()
                            .child(words_rich(
                                "mail-count",
                                &words,
                                &[0..count.len()],
                                SUB,
                                t.text_tertiary,
                            ))
                            .child(kit::label("mail-left", left, SUB, t.text_tertiary)),
                    )
                    .child(bk::bar(
                        "card-0-bar",
                        fraction,
                        &format!("Backing up your mail, {count} messages"),
                        t,
                    )),
            );
            children.push(account_rows(0, mail, t));
            lists_drawn = 1;
        }
    }
    (children, lists_drawn, buttons)
}

/// One labelled field of "Add a mail account". The focus look is the model's (`edit.focused`),
/// as the account forms draw it; a secure field shows bullets and its characters are in no tree.
#[allow(clippy::too_many_arguments)]
fn field(
    index: usize,
    label: &str,
    which: Field,
    edit: &TextEdit,
    mono: bool,
    error: bool,
    message: Option<&str>,
    t: &Theme,
) -> El {
    let key = format!("field-{index}");
    let hidden = edit.is_secure() && !edit.is_revealed();
    let style = if hidden {
        FIELD_SECURE
    } else if mono {
        FIELD_MONO
    } else {
        FIELD_VALUE
    };
    let shown = au::gate_text(edit);
    if let Some(shown) = shown.as_deref().filter(|shown| !shown.is_empty()) {
        kit::note_text(
            &format!("{key}-value"),
            &format!("{key}-value-box"),
            shown,
            style,
            t.text,
        );
    }
    let focused = edit.focused;
    // How wide the text may be inside its well (the dialog's column, less the well's padding and
    // line). Text wider than that scrolls in the input, where only Rune knows the caret's place:
    // then Rune's own caret shows and the drawn one does not.
    let inner_w = match which {
        Field::Email | Field::Password => 394.0,
        Field::Server => 292.0,
        Field::Port => 72.0,
    };
    let width = shown.as_deref().map_or(0.0, |shown| rune::text::advance(shown, style));
    let scrolls = width + 2.5 > inner_w;
    let mut input = text_input(edit, style)
        .key(format!("{key}-input"))
        .label(label)
        .single_line()
        .color(t.text)
        .caret(if scrolls { t.accent_deep } else { Color::TRANSPARENT }, t.selection)
        .invalid(error)
        .on_edit(move |s: &mut State| s.backups.mail.edit(which))
        // Esc and Enter in a field are the dialog's: close it, or connect.
        .on_input_key(|s: &mut State, ev: &rune::KeyEvent, tx: &mut rune::Tx| match ev.key {
            rune::Key::Escape => super::on_key(s, ev, tx),
            rune::Key::Enter => {
                s.backups.mail_connect();
                true
            }
            _ => false,
        })
        .grow(1.0)
        .min_w(0.0);
    if let Some(message) = message {
        input = input.description(message);
    }
    let mut well = row()
        .key(key.clone())
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .border_layout();
    well = if error {
        well.fill(t.field)
            .border(1.0, t.red)
            .box_shadow(kit::ring(3.5, t.red.alpha(0.16)))
    } else if focused {
        well.fill(t.card)
            .border(1.0, t.accent_deep)
            .box_shadow(kit::ring(3.0, t.accent.alpha(0.30)))
    } else {
        well.fill(t.field)
            .border(1.0, Color::TRANSPARENT)
            .box_shadow(kit::ring_inset(t.separator))
    };
    // The typed text's own box, for the gate: as wide as the text Rune draws in the input.
    let mut inner = row()
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center_y()
        .clip_rounded(0.0)
        .child(input)
        .child(
            el().key(format!("{key}-value-box"))
                .a11y_hidden()
                .absolute(0.0, 0.0)
                .w(width.min(inner_w))
                .h_full(),
        );
    if focused
        && !scrolls
        && let Some(shown) = shown.as_deref()
    {
        // The caret as the board draws it (`.kit-caret`: 1.5 x 17, 1 pt after the character it
        // follows). Rune's own caret is 2 pt wide and touches the text, so it is left unseen.
        let before = edit.text[..edit.focus.min(edit.text.len())].chars().count();
        let end = shown.char_indices().nth(before).map_or(shown.len(), |(at, _)| at);
        inner = inner.child(
            el().key(format!("{key}-caret"))
                .a11y_hidden()
                // On a whole device pixel at 200 %, where the board's browser paints it.
                .absolute(
                    ((rune::text::advance(&shown[..end], style) + 1.0) * 2.0).round() / 2.0,
                    4.0,
                )
                .size(1.5, 17.0)
                .radius(1.0)
                .fill(t.accent_deep),
        );
    }
    well = well.child(inner);
    let mut wrap = col()
        .key(format!("{key}-wrap"))
        .w_full()
        .gap(6.0)
        .child(
            kit::label(format!("{key}-label"), label, FIELD_LABEL, t.text_secondary)
                .a11y_hidden()
                .mx(2.0),
        )
        .child(well);
    if let Some(message) = message {
        wrap = wrap.child(
            kit::label(
                format!("{key}-msg"),
                message,
                FIELD_MSG,
                if error { au::red_text(t) } else { t.text_tertiary },
            )
            .role(if error { "alert" } else { "note" })
            .wrap()
            .mx(4.0),
        );
    }
    wrap
}

/// DW-1401: what Mail Backup does, before anything is asked.
fn turn_on(list: usize, first_button: usize, t: &Theme) -> El {
    let facts: [(&'static Icon, &str, &str); 4] = [
        (
            &ic::MAIL,
            "Copies your mail into Beebeeb",
            "Every message and attachment, encrypted on this PC first",
        ),
        (
            &ic::PADLOCK,
            "A protected folder",
            "Backups \u{203A} Mail Backup: open and download, never edit",
        ),
        (
            &ic::LAPTOP,
            "Runs on this PC",
            "While it's on and awake; it catches up after sleep",
        ),
        (
            &ic::SHIELD,
            "Read only",
            "It never sends mail and never changes your mailbox",
        ),
    ];
    let rows = facts
        .iter()
        .enumerate()
        .map(|(i, (icon, title, sub))| {
            let item = Item::new(Some(icon), *title).sub(*sub);
            lists::item(list, i, &if i > 0 { item.ruled() } else { item }, t)
        })
        .collect();
    let body = vec![
        lists::list(list, None, rows, None, t),
        dialogs::caption(
            0,
            "It signs in to your mail server with an app password. Beebeeb can't read your mail or the password.",
            t,
        ),
    ];
    let foot = vec![
        dialogs::button(first_button, "Add a mail account", Weight::Primary, t)
            .default_button()
            .on_press(|s: &mut State| s.backups.mail_add_account()),
        dialogs::button(first_button + 1, "Cancel", Weight::Quiet, t)
            .on_press(|s: &mut State| s.backups.close_dialog()),
    ];
    dialogs::dialog("Turn on Mail Backup", 460.0, "Turn on Mail Backup", None, body, foot, t)
        .modal(false, |s: &mut State| s.backups.close_dialog())
}

/// DW-1402, DW-1403: the account step, and the same step after the server refused.
fn account(ui: &BackupsUi, connection: &Connection, banners: usize, first_button: usize, t: &Theme) -> El {
    let typed = &ui.mail;
    let refused = matches!(connection, Connection::Refused { .. });
    let checking = *connection == Connection::Checking;
    let mut body = Vec::new();
    if let Connection::Refused { server } = connection {
        body.push(banner_red(
            banners,
            &format!("Couldn't sign in to {server}"),
            "The server said the password isn't right. Nothing was saved. Check the app password, then try again.",
            &[],
            None,
            t,
        ));
    }
    body.push(field(0, "Email", Field::Email, &typed.email, false, false, None, t));
    body.push(
        row()
            .key("mail-server-row")
            .w_full()
            .gap(12.0)
            .child(el().key("mail-server-cell").grow(1.0).min_w(0.0).child(field(
                1,
                "IMAP server",
                Field::Server,
                &typed.server,
                true,
                false,
                None,
                t,
            )))
            .child(el().key("mail-port-cell").w(90.0).child(field(
                2,
                "Port",
                Field::Port,
                &typed.port,
                true,
                false,
                None,
                t,
            ))),
    );
    body.push(field(
        3,
        "App password",
        Field::Password,
        &typed.password,
        false,
        refused,
        Some(if refused {
            "That password wasn't accepted."
        } else {
            "Make one in your mail provider's security settings. Your normal password also works if the provider allows it."
        }),
        t,
    ));
    if !refused {
        // The refusal's notice takes this line's place (DW-1403).
        body.push(dialogs::caption(
            0,
            "Connected over TLS. The password stays in Windows Credential Manager on this PC.",
            t,
        ));
    }
    let ready = typed.complete() && !checking;
    let mut connect = dialogs::button(
        first_button,
        if checking {
            "Connecting\u{2026}"
        } else if refused {
            "Try again"
        } else {
            "Connect"
        },
        Weight::Primary,
        t,
    );
    connect = if ready {
        connect.default_button().on_press(|s: &mut State| {
            s.backups.mail_connect();
        })
    } else {
        connect.opacity(0.5).disabled(true)
    };
    let foot = vec![
        connect,
        dialogs::button(first_button + 1, "Cancel", Weight::Quiet, t)
            .on_press(|s: &mut State| s.backups.close_dialog()),
    ];
    dialogs::dialog("Add a mail account", 460.0, "Add a mail account", None, body, foot, t)
        .modal(false, |s: &mut State| s.backups.close_dialog())
}

/// Whether a folder shows ticked: the person's ticks once the answer was taken
/// ([`BackupsUi::mail_answered`]), the mailbox's suggestion while it was not.
fn ticked(ui: &BackupsUi, folder: &MailFolder) -> bool {
    if ui.dialog == Some(Dialog::MailFolders) {
        ui.mail.picked.contains(&folder.name)
    } else {
        folder.suggested
    }
}

/// DW-1404: which folders of the mailbox to archive.
#[allow(clippy::too_many_arguments)]
fn folders(
    ui: &BackupsUi,
    address: &str,
    all: &[MailFolder],
    size: &str,
    list: usize,
    first_button: usize,
    t: &Theme,
) -> El {
    let rows = all
        .iter()
        .enumerate()
        .map(|(i, folder)| {
            let on = ticked(ui, folder);
            let name = folder.name.clone();
            let messages = if folder.messages == 1 {
                "1 message".to_string()
            } else {
                format!("{} messages", thousands(folder.messages))
            };
            let item = Item::new(Some(if on { &ic::CHECK } else { &NOT_TICKED }), folder.name.clone())
                .trailing(Trailing::Mono(messages.clone()));
            lists::item(list, i, &if i > 0 { item.ruled() } else { item }, t)
                .role("checkbox")
                .label(format!("{}, {messages}", folder.name))
                .checked(on)
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(move |s: &mut State| s.backups.mail_toggle_folder(&name))
        })
        .collect();
    let any = all.iter().any(|folder| ticked(ui, folder));
    let body = vec![
        lists::list(list, None, rows, None, t),
        dialogs::caption(
            0,
            &format!("About {size} to back up. New mail in these folders is added every 15 minutes."),
            t,
        ),
    ];
    let mut start = dialogs::button(first_button, "Start backup", Weight::Primary, t);
    start = if any {
        start.default_button().on_press(|s: &mut State| {
            s.backups.mail_start();
        })
    } else {
        start.opacity(0.5).disabled(true)
    };
    let foot = vec![
        start,
        dialogs::button(first_button + 1, "Back", Weight::Quiet, t).on_press(|s: &mut State| s.backups.mail_back()),
    ];
    dialogs::dialog("Choose folders", 440.0, "Choose folders", Some(address), body, foot, t)
        .modal(false, |s: &mut State| s.backups.close_dialog())
}

/// DW-0743: turning off a service is a risky action, a code first.
fn turn_off(ui: &BackupsUi, first_button: usize, t: &Theme) -> El {
    let ready = ui.code_len() == bk::CODE_LEN;
    let body = vec![
        bk::para(
            0,
            "Your mail stops being backed up. Next, you choose whether to move its files to Trash. If you keep them, the folder stays read-only until you turn it on again.",
            &[],
            t,
        ),
        bk::code_boxes("Authenticator code", ui.code_digits(), t),
    ];
    let mut off = bk::button_md(first_button, "Turn off", None, Md::Danger(ready), t);
    if ready {
        off = off.on_press(|s: &mut State| {
            s.backups.mail_turn_off();
        });
    }
    let foot = vec![
        off,
        bk::button_md(first_button + 1, "Cancel", None, Md::Plain, t)
            .on_press(|s: &mut State| s.backups.close_dialog()),
    ];
    dialogs::dialog(
        "Turn off Mail Backup?",
        420.0,
        "Turn off Mail Backup?",
        None,
        body,
        foot,
        t,
    )
    .modal(false, |s: &mut State| s.backups.close_dialog())
}

/// DW-0743b: keep the archive, or move it to Trash.
fn keep_files(used: &str, first_button: usize, t: &Theme) -> El {
    let foot = vec![
        dialogs::button(first_button, "Keep them", Weight::Primary, t)
            .default_button()
            .on_press(|s: &mut State| {
                s.backups.mail_keep_files(true);
            }),
        bk::button_md(first_button + 1, "Move to Trash", None, Md::QuietRed, t).on_press(|s: &mut State| {
            s.backups.mail_keep_files(false);
        }),
    ];
    dialogs::alert(
        "Keep Mail Backup's files?",
        380.0,
        bk::tile(1, TileTone::Neutral, &ic::MAIL, 36.0, t),
        "Keep Mail Backup's files?",
        &format!(
            "{used} of archived mail. Keep them read-only in Backups, or move them to Trash, where they stay 30 days."
        ),
        foot,
        t,
    )
    // Keeping is the answer that changes nothing: it is what Esc means.
    .modal(false, |s: &mut State| {
        s.backups.mail_keep_files(true);
    })
}

/// The dialog over the Mail Backup page, if one of Mail Backup's is open.
pub fn overlay(
    ui: &BackupsUi,
    source: &dyn BackupsSource,
    mail: &Mail,
    lists_drawn: usize,
    buttons: usize,
    t: &Theme,
) -> Option<El> {
    match ui.dialog.as_ref()? {
        Dialog::MailTurnOn => Some(turn_on(lists_drawn, buttons, t)),
        // The step follows the source's answer, not the section's memory of it: "accepted"
        // draws Choose folders at once, anything else the account step.
        step @ (Dialog::MailAccount | Dialog::MailFolders) => match source.mail_connection() {
            Connection::Folders {
                address,
                folders: all,
                size,
            } => Some(folders(ui, &address, &all, &size, lists_drawn, buttons, t)),
            connection if *step == Dialog::MailAccount => Some(account(
                ui,
                &connection,
                usize::from(mail.failure.is_some()),
                buttons,
                t,
            )),
            _ => None,
        },
        Dialog::MailTurnOff => Some(turn_off(ui, buttons, t)),
        Dialog::MailKeepFiles => mail.used.as_deref().map(|used| keep_files(used, buttons, t)),
        Dialog::StopFolder(_) | Dialog::DeleteDevice(_) => None,
    }
}
