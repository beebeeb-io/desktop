//! Drawing the dialogs (`super::Dialog`), as the board draws them: the Windows dialog of each
//! Mac sheet (`WinDialog`, the main action first) and of each Mac alert (`WinAlert`).

use super::kit::{self, Field, Foot};
use super::{Dialog, EMAIL_INPUT, NOTE_INPUT, PASSWORD_INPUT, SafetyNumber, TITLE_INPUT, ask, close, dismiss};
use crate::State;
use crate::frames::Intent;
use crate::frames::shared::parts::controls::{ButtonKind, ButtonSize, button_keyed, button_md};
use crate::frames::shared::source::{Access, Member, PeopleShare};
use crate::theme::icons;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use crate::ui::settings_dialog::DIALOG_TITLE;
use crate::ui::settings_list::{self, Item, Role, Trailing};
use crate::ui::settings_notice::{Tone, tile_sized};
use crate::ui::{dialog, kit as ui_kit};
use rune::{El, Icon, Span, Tx, col, el, rich, row};

/// The two accesses, as the switches show them.
const ACCESS: [Access; 2] = [Access::View, Access::Edit];

/// The widths of the two dialogs with fields (`.wn-dlg`, the board's `width`).
const NEW_REQUEST_W: f32 = 460.0;
const INVITE_W: f32 = 440.0;

/// The room a field's text has in a dialog `card` wide: less the body's padding (24 a side),
/// the well's (8 a side) and its line (1 a side).
fn text_w(card: f32) -> f32 {
    card - 2.0 * (24.0 + 8.0 + 1.0)
}

/// The switch's tabs, `access` chosen.
fn access_tabs(access: Access) -> Vec<(&'static str, bool)> {
    ACCESS.iter().map(|a| (a.words(), *a == access)).collect()
}

/// Buttons at the foot's trailing edge (`.wn-dlg-foot`: `justify-content: flex-end`).
fn trailing(buttons: Vec<El>) -> El {
    row().center_y().justify_end().children(buttons)
}

/// Buttons from the foot's leading edge (the board's `.flex-1` after them, or a `mr-auto`
/// before the last).
fn leading(buttons: Vec<El>) -> El {
    row().center_y().children(buttons).child(spacer())
}

/// The room that pushes what follows it to the foot's far end.
fn spacer() -> El {
    el().key("dlg-foot-gap").grow(1.0)
}

/// The line under a dialog's title, or its paragraph (`.t-sub`).
fn sub(key: &str, words: &str, t: &Theme) -> El {
    ui_kit::label(
        key.to_string(),
        words,
        crate::frames::shared::parts::style::SUB,
        t.text_tertiary,
    )
    .wrap()
}

/// A dialog's small print (`.t-cap`).
fn cap(words: &str, t: &Theme) -> El {
    ui_kit::label("dlg-text-0", words, type_::CAPTION, t.text_tertiary).wrap()
}

/// The standard card: the title, the line under it, the body and the foot.
fn standard(name: &str, width: f32, title: &str, line: Option<&str>, body: Vec<El>, foot: El, t: &Theme) -> El {
    dialog::card(name, width, title, line.map(dialog::Sub::Wrap), body, foot, t)
}

/// "Lisbon trip · 212 items": the share's name, and how much it holds when that is known.
fn share_line(share: &PeopleShare) -> String {
    match &share.contents {
        Some(contents) => format!("{} \u{00B7} {contents}", share.name),
        None => share.name.clone(),
    }
}

/// New request (DW-0504): its title and note, where the files go, when it expires and how much
/// it takes. The four settings show what a request starts with; their menus are not built
/// (the board names no choices).
fn new_request(s: &State, t: &Theme) -> El {
    let ui = &s.frames.shared;
    let n = ui.lists_behind();
    let title = kit::field(
        &Field {
            key: "field-0",
            label: "Title",
            placeholder: "e.g. Send me your signed contract",
            input_key: TITLE_INPUT,
            inner_w: text_w(NEW_REQUEST_W),
        },
        &ui.fields.title,
        |s: &mut State| &mut s.frames.shared.fields.title,
        |input| input.on_submit(|s: &mut State, tx: &mut Tx| create_link(s, tx)),
        t,
    );
    let note = kit::field(
        &Field {
            key: "field-1",
            label: "Note (optional)",
            placeholder: "Shown on the upload page",
            input_key: NOTE_INPUT,
            inner_w: text_w(NEW_REQUEST_W),
        },
        &ui.fields.note,
        |s: &mut State| &mut s.frames.shared.fields.note,
        |input| input.on_submit(|s: &mut State, tx: &mut Tx| create_link(s, tx)),
        t,
    );
    let setting = |m: usize, icon: &'static Icon, words: &str, choice: &str| {
        let mut item = Item::new(Some(icon), words)
            .trailing(Trailing::Pop {
                choice: choice.to_string(),
                open: false,
                menu_at: (0.0, 0.0),
            })
            .role(Role::Plain);
        if m > 0 {
            item = item.ruled();
        }
        settings_list::item(n, m, &item, t)
    };
    let settings = kit::rows(
        n,
        "Request settings",
        vec![
            setting(0, &ic::FOLDER, "Files go to", "Documents"),
            setting(1, &ic::CLOCK, "Expires", "In 30 days"),
            setting(2, &crate::frames::shared::parts::icons::FILE, "Most files", "No limit"),
            setting(3, &ic::DATABASE, "Total size", "No limit"),
        ],
    );
    let body = vec![
        title,
        note,
        kit::list(n, "Request settings", None, settings, None, t),
        cap(
            "Anyone with the link can upload. They can't see other people's files or anything else in this folder.",
            t,
        ),
    ];
    let foot = trailing(vec![
        button_md("Create link", ButtonKind::Amber, Some(&icons::LINK), t)
            .on_press_tx(|s: &mut State, tx: &mut Tx| create_link(s, tx)),
        button_md("Cancel", ButtonKind::Ghost, None, t).on_press(close),
    ]);
    standard("New request", NEW_REQUEST_W, "New request", None, body, foot, t)
}

/// Create link: a request needs a title (the field takes the keyboard while it has none).
/// Making it is the engine's (frames-open §5, create-file-request): recorded, the dialog stays.
fn create_link(s: &mut State, tx: &mut Tx) {
    let title = s.frames.shared.fields.title.text.trim().to_string();
    if title.is_empty() {
        tx.focus(TITLE_INPUT, true);
        return;
    }
    ask(s, Intent::Do("create-request", title));
}

/// People with access (DW-0511): everyone who can open what is shared here, the owner first.
fn people(s: &State, share: &PeopleShare, t: &Theme) -> El {
    let ui = &s.frames.shared;
    let n = ui.lists_behind();
    let members = ui.source.members(&share.id);
    let items = members
        .iter()
        .enumerate()
        .map(|(m, member)| {
            let link = member.access.is_some();
            let row = kit::person(n, m, member, link, t);
            if link {
                let (share, person) = (share.id.clone(), member.id.clone());
                row.on_press(move |s: &mut State| s.frames.shared.show_person(&share, &person))
            } else {
                row
            }
        })
        .collect();
    let body = vec![
        kit::list(n, "People", None, kit::rows(n, "People", items), None, t),
        cap("Everyone listed can open what's shared here.", t),
    ];
    let invite = share.id.clone();
    let foot = leading(vec![
        button_md("Done", ButtonKind::Outline, None, t)
            .default_button()
            .on_press(close),
        button_md("Invite someone\u{2026}", ButtonKind::Outline, Some(&icons::PLUS), t)
            .on_press(move |s: &mut State| s.frames.shared.start_invite(&invite)),
    ]);
    standard("People", 440.0, "People", Some(&share_line(share)), body, foot, t)
}

/// A person's access (DW-0512): their email, Can view or Can edit, and their safety number.
fn person(s: &State, share: &PeopleShare, member: &Member, t: &Theme) -> El {
    let ui = &s.frames.shared;
    let n = ui.lists_behind();
    let (share_id, person_id) = (share.id.clone(), member.id.clone());
    let back = {
        let share = share_id.clone();
        el().key("dlg-back")
            .role("button")
            .label("Back to People")
            .size(28.0, 28.0)
            .px(6.0)
            .center()
            .radius(7.0)
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .on_press(move |s: &mut State| {
                s.frames.shared.show_people(&share);
            })
            .child(ui_kit::glyph("dlg-back-icon", &ic::BACK, 15.0, t.text_secondary))
    };
    let head = row()
        .key("dlg-head-row")
        .center_y()
        .gap(8.0)
        .child(back)
        .child(kit::title(&member.name, t));
    let email = settings_list::item(
        n,
        0,
        &Item::new(None, "Email").trailing(Trailing::Value(member.email.clone())),
        t,
    );
    let access = member.access.unwrap_or(Access::View);
    let tabs = access_tabs(access);
    let switch = kit::seg(1, "Access", &tabs, t, |i, tab| {
        let (share, person) = (share_id.clone(), person_id.clone());
        tab.on_press(move |s: &mut State| {
            s.frames.shared.set_access(&share, &person, ACCESS[i]);
        })
    });
    let compared = if member.compared { "Compared" } else { "Not compared" };
    let safety = {
        let person = person_id.clone();
        settings_list::item(
            n + 2,
            0,
            &Item::new(Some(&ic::SHIELD), "Safety number")
                .trailing(Trailing::Value(compared.to_string()))
                .opens(),
            t,
        )
        // Comparing shows crypto material the engine gives: the security-reviewed wiring opens it.
        .on_press(move |s: &mut State| ask(s, Intent::About("DW-0513", person.clone())))
    };
    let body = vec![
        kit::list(n, "Email", None, kit::rows(n, "Email", vec![email]), None, t),
        kit::list(
            n + 1,
            "Access",
            Some("Access"),
            el().key("access-box").w_full().px(12.0).py(8.0).child(switch),
            None,
            t,
        ),
        kit::list(
            n + 2,
            "Identity",
            Some("Identity"),
            kit::rows(n + 2, "Identity", vec![safety]),
            Some(&format!(
                "Optional. Comparing proves your files go to the real {}.",
                member.first_name()
            )),
            t,
        ),
    ];
    let foot = {
        let (share, person) = (share_id.clone(), person_id.clone());
        row()
            .center_y()
            .child(
                button_md("Remove access", ButtonKind::GhostDanger, None, t).on_press(move |s: &mut State| {
                    s.frames.shared.open_dialog(Dialog::RemoveAccess {
                        share: share.clone(),
                        person: person.clone(),
                    })
                }),
            )
            .child(spacer())
            .child(
                button_md("Done", ButtonKind::Outline, None, t)
                    .default_button()
                    .on_press(close),
            )
    };
    kit::card(&member.name, 440.0, Some(head), body, foot, t)
}

/// "the shared folder" or "the shared file".
fn shared_thing(share: &PeopleShare) -> &'static str {
    match share.kind {
        crate::frames::shared::parts::table::Kind::Folder => "folder",
        _ => "file",
    }
}

/// Remove access (DW-0512b): asked once, with what it does and does not do.
fn remove_access(share: &PeopleShare, member: &Member, t: &Theme) -> El {
    let first = member.first_name();
    let p = member.pronoun;
    let title = format!("Remove {first}'s access?");
    let words = format!(
        "{first} can't open anything new in {}, and the shared {} goes from {} list. Copies {} already saved stay with {}.",
        share.name,
        shared_thing(share),
        p.possessive(),
        p.subject(),
        p.object()
    );
    let (share_id, person_id) = (share.id.clone(), member.id.clone());
    let buttons = vec![
        button_md("Remove access", ButtonKind::Danger, None, t).on_press(move |s: &mut State| {
            s.frames.shared.remove_access(&share_id, &person_id);
        }),
        // The way out takes the keyboard first: Enter never removes anyone by accident.
        button_md("Cancel", ButtonKind::Ghost, None, t)
            .autofocus()
            .on_press_tx(dismiss),
    ];
    kit::alert(
        &title,
        380.0,
        tile_sized(0, Tone::Red, &ic::PEOPLE, 36.0, t),
        &title,
        &words,
        Foot::Even(buttons),
        false,
        super::on_key,
        dismiss,
        t,
    )
}

/// The safety number (DW-0513): compared in person or on a call. What they match or not means
/// is the engine's to act on: recorded, the dialog stays.
fn safety_number(member: &Member, number: &SafetyNumber, t: &Theme) -> El {
    let body = vec![
        sub(
            "dlg-text-0",
            &format!(
                "Compare this with {}, in person or on a call. It should be exactly the same on both devices.",
                member.first_name()
            ),
            t,
        ),
        kit::groups(&kit::GROUPS_LARGE, &number.0, t),
    ];
    let (differ, matched) = (member.id.clone(), member.id.clone());
    let foot = row()
        .center_y()
        .child(
            button_md("They don't match", ButtonKind::Outline, None, t)
                .on_press(move |s: &mut State| ask(s, Intent::Do("safety-number-differs", differ.clone()))),
        )
        .child(spacer())
        .child(
            button_md("They match", ButtonKind::Amber, None, t)
                .on_press(move |s: &mut State| ask(s, Intent::Do("safety-number-matches", matched.clone()))),
        );
    standard("Safety number", 440.0, "Safety number", None, body, foot, t)
}

/// The numbers did not match (DW-0513b): what stops, what stays, the groups to compare again.
fn stopped(s: &State, share: &PeopleShare, member: &Member, number: &SafetyNumber, t: &Theme) -> El {
    let n = s.frames.shared.lists_behind();
    let first = member.first_name();
    let heading = row()
        .key("dlg-alert-head")
        .w_full()
        .pt(8.0)
        .gap(12.0)
        .items_start()
        .child(tile_sized(0, Tone::Red, &ic::SHIELD, 40.0, t))
        .child(
            col()
                .key("dlg-alert-words")
                .child(
                    ui_kit::label(
                        "dlg-title",
                        format!("Sharing with {first} is stopped"),
                        DIALOG_TITLE,
                        t.text,
                    )
                    .role("heading")
                    .level(2),
                )
                .child(sub("dlg-sub", "The safety numbers didn't match.", t).mt(2.0)),
        );
    let item = |m: usize, icon: &'static Icon, title: &str, line: String| {
        let mut it = Item::new(Some(icon), title).sub(line);
        if m > 0 {
            it = it.ruled();
        }
        settings_list::item(n, m, &it, t)
    };
    let what = kit::rows(
        n,
        "What changes",
        vec![
            item(
                0,
                &ic::CLOSE_X,
                "Stops",
                format!("New files, invites and links to {first}"),
            ),
            item(1, &ic::CHECK, "Stays", format!("What {first} already has")),
        ],
    );
    let (again, remove) = (
        (share.id.clone(), member.id.clone(), number.clone()),
        (share.id.clone(), member.id.clone()),
    );
    let body = vec![
        heading,
        kit::list(n, "What changes", None, what, None, t),
        kit::groups(&kit::GROUPS_SMALL, &number.0, t),
    ];
    let foot = leading(vec![
        button_md("Compare again", ButtonKind::Amber, None, t)
            .default_button()
            .on_press(move |s: &mut State| {
                let (share, person, number) = again.clone();
                s.frames
                    .shared
                    .open_dialog(Dialog::SafetyNumber { share, person, number });
            }),
        button_md(&format!("Remove {first}'s access"), ButtonKind::GhostDanger, None, t).on_press(
            move |s: &mut State| {
                let (share, person) = remove.clone();
                s.frames.shared.open_dialog(Dialog::RemoveAccess { share, person });
            },
        ),
    ]);
    kit::card(&format!("Sharing with {first} is stopped"), 440.0, None, body, foot, t)
}

/// Invite someone (DW-0514): their email, what they may do, and the first share's trust step.
fn invite(s: &State, share: &PeopleShare, access: Access, t: &Theme) -> El {
    let ui = &s.frames.shared;
    let email = kit::field(
        &Field {
            key: "field-0",
            label: "Email",
            placeholder: "",
            input_key: EMAIL_INPUT,
            inner_w: text_w(INVITE_W),
        },
        &ui.fields.email,
        |s: &mut State| &mut s.frames.shared.fields.email,
        |input| input.on_submit(send_invite),
        t,
    );
    let tabs = access_tabs(access);
    let switch = kit::seg(1, "Access", &tabs, t, |i, tab| {
        tab.on_press(move |s: &mut State| set_invite_access(s, ACCESS[i]))
    });
    let body = vec![
        email,
        col()
            .key("access-wrap")
            .w_full()
            .gap(6.0)
            .child(kit::outer_label("access-label", "Access", t))
            .child(switch),
        kit::note(
            "first-share",
            &ic::SHIELD,
            "The first time you share with someone, Beebeeb supplies their key, so you're trusting Beebeeb for that step. You can compare safety numbers afterwards.",
            t,
        ),
    ];
    let back = share.id.clone();
    let foot = trailing(vec![
        button_md("Send invite", ButtonKind::Amber, None, t).on_press_tx(send_invite),
        button_md("Cancel", ButtonKind::Ghost, None, t).on_press(move |s: &mut State| {
            s.frames.shared.show_people(&back);
        }),
    ]);
    standard(
        "Invite someone",
        INVITE_W,
        "Invite someone",
        Some(&share.name),
        body,
        foot,
        t,
    )
}

/// Send invite: sent, and People shows them as invited. Nothing typed, or they are on the share
/// already: nothing is sent, and the address field takes the keyboard to type or correct it.
fn send_invite(s: &mut State, tx: &mut Tx) {
    if !s.frames.shared.send_invite() {
        tx.focus(EMAIL_INPUT, true);
    }
}

fn set_invite_access(s: &mut State, to: Access) {
    if let Some(Dialog::Invite { access, .. }) = s.frames.shared.dialog.as_mut() {
        *access = to;
    }
}

/// Left and Right on a switch: the other access (a person's: at once; an invite's: its own).
pub(super) fn step_access(s: &mut State, right: bool, tx: &mut Tx) {
    let to = if right { Access::Edit } else { Access::View };
    match s.frames.shared.dialog.clone() {
        Some(Dialog::Person { share, person }) => {
            s.frames.shared.set_access(&share, &person, to);
        }
        Some(Dialog::Invite { .. }) => set_invite_access(s, to),
        _ => return,
    }
    tx.focus(kit::tab_key(1, to.words()), true);
}

/// A changed key (DW-0515), over People: nothing new goes to them until the person chooses.
/// Both choices are the engine's (it holds the keys): recorded, the alert stays.
fn key_changed(member: &Member, t: &Theme) -> El {
    let first = member.first_name();
    let title = format!("{first}'s key has changed");
    let words = format!(
        "{first} may have set up a new account, or someone may be trying to read what you share. Nothing new goes to {first} until you choose."
    );
    let (compare, anyway) = (member.id.clone(), member.id.clone());
    let buttons = vec![
        button_md("Compare safety numbers", ButtonKind::Amber, None, t)
            .default_button()
            .on_press(move |s: &mut State| ask(s, Intent::About("DW-0513", compare.clone()))),
        button_md("Share anyway", ButtonKind::Outline, None, t)
            .on_press(move |s: &mut State| ask(s, Intent::Do("share-anyway", anyway.clone()))),
        button_md("Cancel", ButtonKind::Ghost, None, t).on_press_tx(dismiss),
    ];
    kit::alert(
        &title,
        420.0,
        tile_sized(0, Tone::Red, &ic::SHIELD, 36.0, t),
        &title,
        &words,
        Foot::Stack(buttons),
        true,
        super::on_key,
        dismiss,
        t,
    )
}

/// Empty Trash? (DW-0749) and Delete permanently? (DW-0748's warning, for one item): what goes for good, and the
/// account password that proves it is the person. The button is off until a password is typed, and while the delete
/// is on its way. Nothing is deleted, or said to be, before the engine answered.
fn purge(s: &State, target: Option<&str>, t: &Theme) -> Option<El> {
    let source = &s.frames.trash.source;
    let (title, words, button, key) = match target {
        None => {
            let totals = source.totals();
            let count = match totals.items {
                1 => "1 item".to_string(),
                n => format!("{n} items"),
            };
            let lead = format!("{count} and ");
            let they = if totals.items == 1 { "It" } else { "They" };
            let words = format!(
                "{lead}{} will be permanently deleted. {they} can't be restored by you or by Beebeeb, on any device.",
                totals.size
            );
            let paragraph = rich(
                words,
                vec![Span::new(lead.len(), lead.len() + totals.size.len()).mono()],
            );
            ("Empty Trash?", paragraph, "Empty Trash", "btn-empty-trash-2")
        }
        Some(id) => {
            let row = source.items().into_iter().find(|row| row.id == id)?;
            let words = format!(
                "\u{201C}{}\u{201D} will be permanently deleted. It can't be restored by you or by Beebeeb, on any device.",
                row.name
            );
            (
                "Delete permanently?",
                rich(words, Vec::new()),
                "Delete permanently",
                "btn-delete-forever",
            )
        }
    };
    let paragraph = words.key("dlg-text-0").font(kit::BODY).color(t.text_secondary).w_full();
    let ui = &s.frames.shared;
    let busy = source.purging();
    let password = kit::field(
        &Field {
            key: "field-0",
            label: "Your account password",
            placeholder: "",
            input_key: PASSWORD_INPUT,
            inner_w: text_w(PURGE_W),
        },
        &ui.fields.password,
        |s: &mut State| &mut s.frames.shared.fields.password,
        |input| input.on_submit(submit_purge),
        t,
    );
    let mut body = vec![paragraph, password];
    if let Some(problem) = ui.fields.problem {
        body.push(
            ui_kit::label("purge-problem", problem, kit::NOTE, t.red)
                .role("alert")
                .label(problem)
                .w_full(),
        );
    }
    let ready = !ui.fields.password.text.is_empty() && !busy;
    let confirm = if ready {
        button_keyed(key.into(), button, ButtonKind::Danger, ButtonSize::Medium, None, t).on_press_tx(submit_purge)
    } else {
        let shown = if busy { "Deleting\u{2026}" } else { button };
        button_keyed(key.into(), shown, ButtonKind::DangerOff, ButtonSize::Medium, None, t)
    };
    let foot = trailing(vec![
        confirm,
        button_md("Cancel", ButtonKind::Outline, None, t).on_press(close),
    ]);
    Some(standard(title, PURGE_W, title, None, body, foot, t))
}

/// The dialog's width.
const PURGE_W: f32 = 420.0;

/// Send the delete: the password goes to the source (and out of the field at once); a field with nothing typed takes
/// the keyboard instead. The dialog stays until the source's answer closes it (Trash's poll).
fn submit_purge(s: &mut State, tx: &mut Tx) {
    let target = match &s.frames.shared.dialog {
        Some(Dialog::EmptyTrash) => None,
        Some(Dialog::DeleteForever(id)) => Some(id.clone()),
        _ => return,
    };
    let ui = &mut s.frames.shared;
    if ui.fields.password.text.is_empty() || s.frames.trash.source.purging() {
        tx.focus(PASSWORD_INPUT, true);
        return;
    }
    ui.fields.problem = None;
    let started = s.frames.trash.source.purge(target.as_deref(), &ui.fields.password.text);
    ui.fields.password.clear();
    if !started {
        // Nothing was started (the item is gone, the session moved on): the dialog goes, and Trash says what it has.
        ui.close_dialog();
    }
}

/// The open dialog's layers over the section that shows (none over another section): one, or
/// People and the alert over it (DW-0515).
pub fn layers(s: &State, t: &Theme) -> Vec<El> {
    let ui = &s.frames.shared;
    let Some(dialog) = ui.dialog.as_ref() else {
        return Vec::new();
    };
    if super::section_shown(s) != Some(dialog.section()) {
        return Vec::new();
    }
    let modal = |name: &str, card: El| kit::layer(name, card, super::on_key, dismiss, false, t);
    let share_and = |share: &str, person: &str| Some((ui.share(share)?, ui.member(share, person)?));
    match dialog {
        Dialog::NewRequest => vec![modal("new-request", new_request(s, t))],
        Dialog::EmptyTrash => purge(s, None, t)
            .map(|card| modal("empty-trash", card))
            .into_iter()
            .collect(),
        Dialog::DeleteForever(id) => purge(s, Some(id), t)
            .map(|card| modal("delete-forever", card))
            .into_iter()
            .collect(),
        Dialog::People { share, key_changed } => {
            let Some(share) = ui.share(share) else {
                return Vec::new();
            };
            let changed = key_changed.as_deref().and_then(|p| ui.member(&share.id, p));
            let people = kit::layer(
                "people",
                people(s, &share, t),
                super::on_key,
                dismiss,
                changed.is_some(),
                t,
            );
            let mut out = vec![people];
            if let Some(member) = changed {
                out.push(self::key_changed(&member, t));
            }
            out
        }
        Dialog::Person { share, person } => share_and(share, person)
            .map(|(share, member)| vec![modal("person", self::person(s, &share, &member, t))])
            .unwrap_or_default(),
        Dialog::RemoveAccess { share, person } => share_and(share, person)
            .map(|(share, member)| vec![remove_access(&share, &member, t)])
            .unwrap_or_default(),
        Dialog::SafetyNumber { share, person, number } => share_and(share, person)
            .map(|(_, member)| vec![modal("safety-number", safety_number(&member, number, t))])
            .unwrap_or_default(),
        Dialog::Stopped { share, person, number } => share_and(share, person)
            .map(|(share, member)| vec![modal("stopped", stopped(s, &share, &member, number, t))])
            .unwrap_or_default(),
        Dialog::Invite { share, access } => ui
            .share(share)
            .map(|share| vec![modal("invite", invite(s, &share, *access, t))])
            .unwrap_or_default(),
    }
}
