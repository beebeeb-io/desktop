//! The lane's dialogs: what each says it is to assistive technology, where each opens from and
//! goes back to, what each control does by pointer and by keyboard, and what stays the engine's.

use super::Dialog;
use crate::frames::Intent;
use crate::frames::shared::scenes;
use crate::frames::shared::source::{Access, InviteResult, Notice};
use crate::frames::shared::tests::{click, focused, info, press, press_with, runtime, settle};
use crate::shell::app::Section;
use crate::{State, init};
use rune::{Key, KeyEvent, Mods, Runtime, Tx};

const LISBON: &str = "p-lisbon";
const JONAS: &str = "m-jonas";

fn dialog(rt: &Runtime<State>) -> Option<Dialog> {
    rt.state.frames.shared.dialog.clone()
}

fn people() -> Dialog {
    Dialog::People {
        share: LISBON.into(),
        key_changed: None,
    }
}

fn person() -> Dialog {
    Dialog::Person {
        share: LISBON.into(),
        person: JONAS.into(),
    }
}

/// Jonas's access to Lisbon trip, as the source has it.
fn jonas_access(rt: &Runtime<State>) -> Option<Access> {
    rt.state.frames.shared.member(LISBON, JONAS).and_then(|m| m.access)
}

fn role_and_name(rt: &Runtime<State>, key: &str) -> (&'static str, Option<String>) {
    let e = info(rt, key);
    (e.role, e.label)
}

fn named(role: &'static str, name: &str) -> (&'static str, Option<String>) {
    (role, Some(name.to_string()))
}

fn text(rt: &Runtime<State>, key: &str) -> Option<String> {
    info(rt, key).text
}

/// A key with no modifier.
fn plain(key: Key) -> KeyEvent {
    KeyEvent {
        key,
        mods: Mods::default(),
    }
}

#[test]
fn each_dialog_and_its_controls_are_named_as_the_board_names_them() {
    type Frame = (fn(&mut State), &'static str, &'static str, &'static str);
    let frames: [Frame; 9] = [
        (scenes::dw_0504, "dlg", "dialog", "New request"),
        (scenes::dw_0511, "dlg", "dialog", "People"),
        (scenes::dw_0512, "dlg", "dialog", "Jonas Meier"),
        (scenes::dw_0512b, "alert", "alertdialog", "Remove Jonas's access?"),
        (scenes::dw_0513, "dlg", "dialog", "Safety number"),
        (scenes::dw_0513b, "dlg", "dialog", "Sharing with Jonas is stopped"),
        (scenes::dw_0514, "dlg", "dialog", "Invite someone"),
        (scenes::dw_0515, "alert", "alertdialog", "Jonas's key has changed"),
        (scenes::dw_0749, "dlg", "dialog", "Empty Trash?"),
    ];
    for (scene, key, role, name) in frames {
        let rt = runtime(scene);
        assert_eq!(role_and_name(&rt, key), named(role, name), "{name}");
    }

    // New request: two fields named by their labels, the four settings as they start, the Title in focus.
    let rt = runtime(scenes::dw_0504);
    assert_eq!(info(&rt, super::TITLE_INPUT).label.as_deref(), Some("Title"));
    assert_eq!(info(&rt, super::NOTE_INPUT).label.as_deref(), Some("Note (optional)"));
    assert_eq!(
        role_and_name(&rt, "item-1-0"),
        named("combobox", "Files go to, All files")
    );
    assert_eq!(
        role_and_name(&rt, "item-1-3"),
        named("combobox", "Total size, No limit")
    );
    assert_eq!(role_and_name(&rt, "btn-create-link"), named("button", "Create link"));
    assert_eq!(focused(&rt).as_deref(), Some(super::TITLE_INPUT));

    // People: the owner is a row, everyone else opens their access.
    let rt = runtime(scenes::dw_0511);
    assert_eq!(
        role_and_name(&rt, "item-2-0"),
        named("listitem", "Anna Visser, You \u{00B7} Owner")
    );
    assert_eq!(role_and_name(&rt, "item-2-1"), named("link", "Jonas Meier, Can edit"));
    assert_eq!(
        role_and_name(&rt, "item-2-2"),
        named("link", "Mia de Vries, Can view \u{00B7} Invited, waiting")
    );
    assert_eq!(
        role_and_name(&rt, "btn-invite-someone"),
        named("button", "Invite someone\u{2026}")
    );

    // A person: the way back, the switch, the safety number's row.
    let rt = runtime(scenes::dw_0512);
    assert_eq!(role_and_name(&rt, "dlg-back"), named("button", "Back to People"));
    assert_eq!(role_and_name(&rt, "seg-1"), named("tablist", "Access"));
    assert_eq!(role_and_name(&rt, "seg-1-can-edit"), named("tab", "Can edit"));
    assert_eq!(
        role_and_name(&rt, "item-4-0"),
        named("link", "Safety number, Not compared")
    );

    // The confirmation opens on its way out: Enter never removes anyone by accident.
    let rt = runtime(scenes::dw_0512b);
    assert_eq!(focused(&rt).as_deref(), Some("btn-cancel"));

    // The safety number reads as one number.
    let rt = runtime(scenes::dw_0513);
    assert_eq!(
        role_and_name(&rt, "safety"),
        named(
            "group",
            "Safety number: 37195 04826 91730 62458 18094 75263 40917 86352 29641 05738 61294 83570"
        )
    );

    // Invite someone: the address field in focus, the note named by its words.
    let rt = runtime(scenes::dw_0514);
    assert_eq!(info(&rt, super::EMAIL_INPUT).label.as_deref(), Some("Email"));
    assert_eq!(focused(&rt).as_deref(), Some(super::EMAIL_INPUT));
    assert_eq!(info(&rt, "first-share").role, "note");

    // Empty Trash?: the password field is named without what is typed; the button is on once something is.
    let mut rt = runtime(scenes::dw_0749);
    assert_eq!(
        info(&rt, super::PASSWORD_INPUT).label.as_deref(),
        Some("Your account password")
    );
    assert!(
        !info(&rt, "btn-empty-trash-2").focusable,
        "Empty Trash is off: nothing is typed (the board's frame)"
    );
    rt.state
        .frames
        .shared
        .fields
        .password
        .replace_text("sample".to_string());
    settle(&mut rt);
    assert!(
        info(&rt, "btn-empty-trash-2").focusable,
        "Empty Trash is on: a password is typed"
    );
}

#[test]
fn people_opens_from_a_share_and_closes_back_to_it() {
    let mut rt = runtime(scenes::dw_0502);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(dialog(&rt), Some(people()));
    assert_eq!(text(&rt, "item-2-1-title").as_deref(), Some("Jonas Meier"));
    // Esc: closed, and the keyboard is back on the share.
    press(&mut rt, Key::Escape);
    assert_eq!(dialog(&rt), None);
    assert!(rt.find("dialog-people").is_none());
    assert_eq!(focused(&rt).as_deref(), Some("item-0-0"));
    // Done closes it too; nothing was asked of another frame.
    press(&mut rt, Key::Enter);
    click(&mut rt, "btn-done");
    assert_eq!(dialog(&rt), None);
    assert!(rt.state.frames.intents.is_empty());
}

#[test]
fn a_persons_access_changes_at_once_by_pointer_and_by_the_arrows() {
    // From People by the keyboard: Tab to Jonas, Enter.
    let mut rt = runtime(scenes::dw_0511);
    press(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some("item-2-1"));
    press(&mut rt, Key::Enter);
    assert_eq!(dialog(&rt), Some(person()));
    assert_eq!(text(&rt, "dlg-title").as_deref(), Some("Jonas Meier"));

    click(&mut rt, "seg-1-can-view");
    assert_eq!(jonas_access(&rt), Some(Access::View));
    // Right and Left move the switch, and focus with it.
    press(&mut rt, Key::Right);
    assert_eq!(jonas_access(&rt), Some(Access::Edit));
    assert_eq!(focused(&rt).as_deref(), Some("seg-1-can-edit"));
    press(&mut rt, Key::Left);
    assert_eq!(jonas_access(&rt), Some(Access::View));
    // Back: People says what changed.
    click(&mut rt, "dlg-back");
    assert_eq!(dialog(&rt), Some(people()));
    assert_eq!(text(&rt, "item-2-1-sub").as_deref(), Some("Can view"));
    // The owner has no access to change: their row opens nothing.
    rt.state.frames.shared.show_person(LISBON, "m-anna");
    assert_eq!(dialog(&rt), Some(people()));
}

#[test]
fn remove_access_asks_first_and_only_its_red_answer_removes() {
    let mut rt = runtime(scenes::dw_0512);
    click(&mut rt, "btn-remove-access");
    assert_eq!(
        dialog(&rt),
        Some(Dialog::RemoveAccess {
            share: LISBON.into(),
            person: JONAS.into(),
        })
    );
    // Enter on the way out: back to Jonas, who keeps his access.
    press(&mut rt, Key::Enter);
    assert_eq!(dialog(&rt), Some(person()));
    assert_eq!(jonas_access(&rt), Some(Access::Edit));
    // Esc is the way out too.
    click(&mut rt, "btn-remove-access");
    press(&mut rt, Key::Escape);
    assert_eq!(dialog(&rt), Some(person()));

    click(&mut rt, "btn-remove-access");
    click(&mut rt, "btn-remove-access");
    assert_eq!(dialog(&rt), Some(people()), "People shows who is left");
    assert_eq!(jonas_access(&rt), None);
    assert!(rt.state.frames.shared.member(LISBON, JONAS).is_none());
    assert_eq!(text(&rt, "item-2-1-title").as_deref(), Some("Mia de Vries"));
    // The share's row in By me names who is left.
    assert_eq!(rt.state.frames.shared.source.people_shares()[0].names, "Mia de Vries");
}

#[test]
fn an_invite_adds_the_address_as_invited_and_refuses_someone_already_there() {
    let mut rt = runtime(scenes::dw_0511);
    click(&mut rt, "btn-invite-someone");
    assert_eq!(
        dialog(&rt),
        Some(Dialog::Invite {
            share: LISBON.into(),
            access: Access::View,
        })
    );
    assert_eq!(focused(&rt).as_deref(), Some(super::EMAIL_INPUT));
    // Not an address yet: nothing is sent, the field keeps the keyboard.
    for part in ["lea", "@northwind"] {
        press(&mut rt, Key::Text(part.into()));
        press(&mut rt, Key::Enter);
        assert!(matches!(dialog(&rt), Some(Dialog::Invite { .. })), "sent at {part}");
        assert_eq!(focused(&rt).as_deref(), Some(super::EMAIL_INPUT));
    }
    press(&mut rt, Key::Text(".studio".into()));
    // A new invitation carries Beebeeb's default access only: Can edit is drawn where the board has it, and is no
    // choice (it is changed afterwards, on the person).
    click(&mut rt, "seg-1-can-edit");
    assert!(matches!(
        dialog(&rt),
        Some(Dialog::Invite {
            access: Access::View,
            ..
        })
    ));
    // Enter in the field sends it: nothing is drawn as sent before the answer, and no second press goes out.
    click(&mut rt, super::EMAIL_INPUT);
    press(&mut rt, Key::Enter);
    assert!(
        matches!(dialog(&rt), Some(Dialog::Invite { .. })),
        "open until the answer"
    );
    assert_eq!(text(&rt, "btn-send-invite-label").as_deref(), Some("Sending\u{2026}"));
    let people_now = rt.state.frames.shared.source.members(LISBON).len();
    click(&mut rt, "btn-send-invite");
    rt.state.frames.shared.fields.email = rune::TextEdit::new("max@northwind.studio");
    assert!(
        !rt.state.frames.shared.send_invite(),
        "a second press while one is on its way"
    );
    assert_eq!(
        rt.state.frames.shared.source.members(LISBON).len(),
        people_now,
        "sent once"
    );
    // The answer: People shows them as invited, with the access a new invitation gets.
    shared_poll(&mut rt);
    assert_eq!(dialog(&rt), Some(people()));
    assert_eq!(text(&rt, "item-2-3-title").as_deref(), Some("lea@northwind.studio"));
    assert_eq!(
        text(&rt, "item-2-3-sub").as_deref(),
        Some("Can view \u{00B7} Invited, waiting")
    );
    assert_eq!(
        rt.state.frames.shared.source.people_shares()[0].names,
        "Jonas Meier, Mia de Vries, lea@northwind.studio"
    );
    // The invited person is someone of their own: no id is shared, so taking one off a share
    // takes no one else.
    let ids: Vec<String> = rt
        .state
        .frames
        .shared
        .source
        .members(LISBON)
        .into_iter()
        .map(|m| m.id)
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");

    // Someone already on the share: nothing is sent, the dialog stays with what was typed.
    let mut rt = runtime(scenes::dw_0514);
    click(&mut rt, "btn-send-invite");
    assert!(matches!(dialog(&rt), Some(Dialog::Invite { .. })));
    assert_eq!(
        focused(&rt).as_deref(),
        Some(super::EMAIL_INPUT),
        "the field takes the keyboard back"
    );
    assert_eq!(rt.state.frames.shared.source.members(LISBON).len(), 3);
    assert_eq!(rt.state.frames.shared.fields.email.text, "jonas@northwind.studio");
    // Cancel: back to People, the field emptied.
    click(&mut rt, "btn-cancel");
    assert_eq!(dialog(&rt), Some(people()));
    assert_eq!(rt.state.frames.shared.fields.email.text, "");
}

#[test]
fn an_address_needs_a_name_an_at_and_a_domain_with_a_dot() {
    for good in ["lea@northwind.studio", "a.b@c.example.org"] {
        assert!(super::is_address(good), "{good}");
    }
    for bad in [
        "",
        "lea",
        "@northwind.studio",
        "lea@northwind",
        "lea@@northwind.studio",
        "lea@northwind..studio",
        "lea@.studio",
        "lea @northwind.studio",
    ] {
        assert!(!super::is_address(bad), "{bad}");
    }
}

#[test]
fn a_long_title_scrolls_and_the_caret_is_runes_where_only_rune_knows_its_place() {
    let mut rt = runtime(scenes::dw_0504);
    press(&mut rt, Key::Text("Signed".into()));
    assert!(
        rt.find("field-0-caret").is_some(),
        "the board's caret while the title fits"
    );
    press(
        &mut rt,
        Key::Text(" contracts, invoices and receipts for the whole of the year 2026, please".into()),
    );
    assert!(rt.find("field-0-caret").is_none(), "no drawn caret past the well");
    // Rune's caret stands at the end of what shows, inside the well.
    let well = info(&rt, "field-0").rect;
    let caret = info(&rt, super::TITLE_INPUT).caret.expect("the focused field's caret");
    assert!(
        caret[0] > well[0] + well[2] / 2.0,
        "at the end of the text: {caret:?} in {well:?}"
    );
    assert!(
        caret[0] + caret[2] <= well[0] + well[2],
        "inside the well: {caret:?} in {well:?}"
    );
}

/// The window's poll of Trash: takes in what the source answered (the dialog closes when a delete ended).
fn poll(rt: &mut Runtime<State>) {
    rt.update(|s, tx| {
        crate::frames::trash::poll(s, tx);
    });
    settle(rt);
}

#[test]
fn the_password_shows_to_no_reader_is_sent_once_and_wiped_and_empty_trash_waits_for_one() {
    let mut rt = runtime(scenes::dw_0749);
    assert_eq!(
        rt.state.frames.shared.fields.password.text, "",
        "the board's frame opens on an empty field"
    );
    rt.state
        .frames
        .shared
        .fields
        .password
        .replace_text("sample".to_string());
    let debug = format!("{:?}", rt.state.frames.shared.fields);
    assert!(!debug.contains("sample"), "the password is in the fields' Debug");
    // Nothing typed: the button is off and nothing is sent.
    rt.state.frames.shared.fields.password.clear();
    settle(&mut rt);
    assert!(
        !info(&rt, "btn-empty-trash-2").focusable,
        "Empty Trash is off without a password"
    );
    assert_eq!(rt.state.frames.trash.source.totals().items, 12);
    // A password typed: pressing sends it (the board's sample takes any), the field is wiped at once, the dialog
    // closes when the source says it ended, and the list is empty.
    rt.state
        .frames
        .shared
        .fields
        .password
        .replace_text("another".to_string());
    settle(&mut rt);
    click(&mut rt, "btn-empty-trash-2");
    assert_eq!(rt.state.frames.shared.fields.password.text, "");
    poll(&mut rt);
    assert_eq!(dialog(&rt), None);
    assert_eq!(rt.state.frames.trash.source.totals().items, 0);
}

#[test]
fn delete_permanently_names_its_item_and_deletes_only_that_one() {
    let mut rt = runtime(scenes::dw_0749);
    rt.state
        .frames
        .shared
        .open_dialog(Dialog::DeleteForever("t-contract-old".into()));
    settle(&mut rt);
    assert!(rt.find("dialog-delete-forever").is_some());
    assert!(
        text(&rt, "dlg-text-0").unwrap_or_default().contains("Contract old.pdf"),
        "the warning names the item"
    );
    rt.state
        .frames
        .shared
        .fields
        .password
        .replace_text("another".to_string());
    settle(&mut rt);
    click(&mut rt, "btn-delete-forever");
    poll(&mut rt);
    assert_eq!(dialog(&rt), None);
    assert_eq!(rt.state.frames.trash.source.totals().items, 11);
    assert!(
        rt.state
            .frames
            .trash
            .source
            .items()
            .iter()
            .all(|r| r.id != "t-contract-old")
    );
}

#[test]
fn what_needs_the_engine_is_asked_for_and_the_dialog_stays_as_it_is() {
    // The safety number: both answers are the engine's.
    let mut rt = runtime(scenes::dw_0513);
    let before = dialog(&rt);
    click(&mut rt, "btn-they-match");
    click(&mut rt, "btn-they-don-t-match");
    assert_eq!(
        rt.state.frames.intents,
        vec![
            Intent::Do("safety-number-matches", JONAS.into()),
            Intent::Do("safety-number-differs", JONAS.into()),
        ]
    );
    assert_eq!(dialog(&rt), before);

    // A person's safety number is shown by the security-reviewed wiring, never made up here.
    let mut rt = runtime(scenes::dw_0512);
    click(&mut rt, "item-4-0");
    assert_eq!(rt.state.frames.intents, vec![Intent::About("DW-0513", JONAS.into())]);
    assert_eq!(dialog(&rt), Some(person()));

    // A changed key: compare or share anyway are the engine's; Cancel leaves People.
    let mut rt = runtime(scenes::dw_0515);
    click(&mut rt, "btn-compare-safety-numbers");
    click(&mut rt, "btn-share-anyway");
    assert_eq!(
        rt.state.frames.intents,
        vec![
            Intent::About("DW-0513", JONAS.into()),
            Intent::Do("share-anyway", JONAS.into()),
        ]
    );
    click(&mut rt, "btn-cancel");
    assert_eq!(dialog(&rt), Some(people()));
    assert!(rt.find("alert").is_none() && rt.find("dlg").is_some());

    // Stopped: compare again with the same number, or remove them (asked first).
    let mut rt = runtime(scenes::dw_0513b);
    click(&mut rt, "btn-compare-again");
    assert!(matches!(dialog(&rt), Some(Dialog::SafetyNumber { .. })));
    let mut rt = runtime(scenes::dw_0513b);
    click(&mut rt, "btn-remove-jonas-s-access");
    assert!(matches!(dialog(&rt), Some(Dialog::RemoveAccess { .. })));

    // New request: no title, no request; the Title takes the keyboard. Then Enter asks for one.
    let mut rt = runtime(scenes::dw_0504);
    click(&mut rt, "btn-create-link");
    assert!(rt.state.frames.intents.is_empty());
    assert_eq!(focused(&rt).as_deref(), Some(super::TITLE_INPUT));
    press(&mut rt, Key::Text("Tax forms".into()));
    press(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.frames.intents,
        vec![Intent::Do("create-request", "Tax forms".into())]
    );
    assert_eq!(dialog(&rt), Some(Dialog::NewRequest));
}

#[test]
fn keys_stop_at_the_dialog_and_never_move_the_list_behind() {
    // Through the window: the keyboard on a row of the dialog.
    let mut rt = runtime(scenes::dw_0511);
    press(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some("item-2-1"));
    for key in [Key::Down, Key::Up, Key::End, Key::Home, Key::ContextMenu] {
        press(&mut rt, key);
    }
    press_with(
        &mut rt,
        Key::Function(10),
        Mods {
            shift: true,
            ..Mods::default()
        },
    );
    assert_eq!(rt.state.frames.shared.selected, Some(0));
    assert!(rt.state.frames.shared.menu.is_none());
    assert_eq!(dialog(&rt), Some(people()));

    // With focus nowhere (the keys the section gets first): Down, End and Enter do nothing to
    // the list; Tab is Rune's; Esc goes back the dialog's way.
    let mut s = init();
    scenes::dw_0511(&mut s);
    let mut tx = Tx::default();
    for key in [Key::Down, Key::End, Key::Enter, Key::ContextMenu] {
        assert!(crate::frames::shared::on_key(&mut s, &plain(key), &mut tx));
    }
    assert_eq!(s.frames.shared.selected, Some(0));
    assert!(s.frames.shared.menu.is_none());
    assert_eq!(s.frames.shared.dialog, Some(people()));
    assert!(!crate::frames::shared::on_key(&mut s, &plain(Key::Tab), &mut tx));
    assert!(crate::frames::shared::on_key(&mut s, &plain(Key::Escape), &mut tx));
    assert_eq!(s.frames.shared.dialog, None);
}

#[test]
fn a_dialog_shows_only_over_its_own_section_and_closes_once_left_behind() {
    let mut rt = runtime(scenes::dw_0749);
    assert!(rt.find("dialog-empty-trash").is_some());
    // Both sections answer (the scenes start the other one on the shell's "not built yet" frame).
    rt.state.frames.shared.source = Box::new(crate::frames::shared::source::BoardShared::new());
    crate::frames::go(&mut rt.state, Section::Shared);
    settle(&mut rt);
    assert!(rt.find("dialog-empty-trash").is_none(), "Trash's dialog over Shared");
    // Left behind: the first key in Shared closes it, and the password goes with it.
    press(&mut rt, Key::Down);
    assert_eq!(dialog(&rt), None);
    assert_eq!(rt.state.frames.shared.fields.password.text, "");
    crate::frames::go(&mut rt.state, Section::Trash);
    settle(&mut rt);
    assert!(rt.find("dialog-empty-trash").is_none(), "nothing waits in Trash");

    let mut rt = runtime(scenes::dw_0511);
    rt.state.frames.trash = crate::frames::trash::TrashUi::default();
    crate::frames::go(&mut rt.state, Section::Trash);
    settle(&mut rt);
    assert!(rt.find("row-0").is_some(), "Trash shows its items");
    assert!(rt.find("dialog-people").is_none(), "Shared's dialog over Trash");
}

#[test]
fn trashs_keys_stop_at_empty_trash_and_nothing_is_deleted_behind_it() {
    // Focus nowhere: the keys reach Trash's line in the shell's registry first.
    let mut s = init();
    scenes::dw_0749(&mut s);
    s.frames.trash.selected = Some(0);
    let group = crate::shell::registry::FrameGroup::Section(Section::Trash);
    let keys = crate::shell::registry::frame(group)
        .and_then(|f| f.on_key)
        .expect("Trash's keys");
    let mut tx = Tx::default();
    for key in [Key::Delete, Key::Down, Key::ContextMenu, Key::Enter] {
        assert!(keys(&mut s, &plain(key), &mut tx));
    }
    assert!(
        s.frames.intents.is_empty(),
        "nothing deleted behind the dialog: {:?}",
        s.frames.intents
    );
    assert_eq!(s.frames.trash.selected, Some(0));
    assert!(s.frames.trash.menu.is_none());
    // Esc closes it; then Trash has its keys again.
    assert!(keys(&mut s, &plain(Key::Escape), &mut tx));
    assert_eq!(s.frames.shared.dialog, None);
    assert!(keys(&mut s, &plain(Key::Down), &mut tx));
    assert_eq!(s.frames.trash.selected, Some(1));
}

#[test]
fn a_dialog_whose_share_went_closes_and_the_list_has_its_keys_again() {
    let mut rt = runtime(scenes::dw_0511);
    rt.state.frames.shared.source.stop_sharing(LISBON);
    settle(&mut rt);
    assert!(rt.find("dialog-people").is_none(), "nothing to draw");
    press(&mut rt, Key::Down);
    assert_eq!(dialog(&rt), None);
    assert_eq!(rt.state.frames.shared.selected, Some(1), "the list moves again");
}

/// The window's poll of Shared (`shell::host` calls `frames::shared::poll`), then a draw.
fn shared_poll(rt: &mut Runtime<State>) {
    rt.update(|s, tx| {
        crate::frames::shared::poll(s, tx);
    });
    settle(rt);
}

fn any_text(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| !e.exiting && e.text.as_deref() == Some(words))
}

#[test]
fn share_invite_closes_only_on_an_answer_that_sent_and_says_why_nothing_was_shared() {
    let mut rt = runtime(scenes::dw_0511);
    click(&mut rt, "btn-invite-someone");
    press(&mut rt, Key::Text("lea@northwind.studio".into()));
    click(&mut rt, "btn-send-invite");
    assert!(rt.state.frames.shared.fields.sending);

    // A changed key: open, its words, and Send invite back.
    rt.state.frames.shared.invite_answered(LISBON, InviteResult::KeyChanged);
    settle(&mut rt);
    assert!(matches!(dialog(&rt), Some(Dialog::Invite { .. })));
    assert_eq!(text(&rt, "invite-problem").as_deref(), Some(super::KEY_CHANGED));
    assert_eq!(text(&rt, "btn-send-invite-label").as_deref(), Some("Send invite"));

    // A refusal: open, with the source's words.
    rt.state.frames.shared.fields.sending = true;
    rt.state.frames.shared.invite_answered(
        LISBON,
        InviteResult::Refused("Can\u{2019}t reach Beebeeb. Nothing was shared."),
    );
    settle(&mut rt);
    assert!(matches!(dialog(&rt), Some(Dialog::Invite { .. })));
    assert_eq!(
        text(&rt, "invite-problem").as_deref(),
        Some("Can\u{2019}t reach Beebeeb. Nothing was shared.")
    );

    // An answer for another share is not this dialog's.
    rt.state.frames.shared.fields.sending = true;
    rt.state.frames.shared.invite_answered("p-invoices", InviteResult::Sent);
    assert!(matches!(dialog(&rt), Some(Dialog::Invite { .. })));

    // Sent, and the two other answers that reached Beebeeb: People.
    for answer in [
        InviteResult::Sent,
        InviteResult::HasAccess,
        InviteResult::WaitingForApproval,
    ] {
        rt.state.frames.shared.start_invite(LISBON);
        rt.state.frames.shared.fields.sending = true;
        rt.state.frames.shared.invite_answered(LISBON, answer);
        settle(&mut rt);
        assert_eq!(dialog(&rt), Some(people()), "{answer:?}");
    }

    // The dialog went before its answer came: a refusal still says so, over the lists; a send says nothing.
    rt.state.frames.shared.close_dialog();
    rt.state.frames.shared.notice = None;
    rt.state.frames.shared.invite_answered(LISBON, InviteResult::Sent);
    assert_eq!(rt.state.frames.shared.notice, None);
    rt.state.frames.shared.invite_answered(LISBON, InviteResult::KeyChanged);
    assert_eq!(rt.state.frames.shared.notice, Some(Notice::ChangeRefused));
}

#[test]
fn share_invite_for_an_item_shared_with_no_one_yet_is_titled_by_its_name() {
    let mut rt = runtime(scenes::dw_0511);
    rt.state.frames.shared.close_dialog();
    rt.state
        .frames
        .shared
        .start_invite_named("an-item-of-its-own", "Invented notes.txt");
    settle(&mut rt);
    assert!(rt.state.frames.shared.dialog_drawn());
    assert!(
        any_text(&rt, "Invented notes.txt"),
        "the item's name, as the title's line"
    );
    // No People to go back to: Esc and Cancel close it.
    press(&mut rt, Key::Escape);
    assert_eq!(dialog(&rt), None);
    rt.state
        .frames
        .shared
        .start_invite_named("an-item-of-its-own", "Invented notes.txt");
    settle(&mut rt);
    click(&mut rt, "btn-cancel");
    assert_eq!(dialog(&rt), None);
    // A share it lists: its own dialog, titled by the share.
    rt.state.frames.shared.start_invite_named(LISBON, "another name");
    settle(&mut rt);
    assert_eq!(rt.state.frames.shared.fields.unlisted, None);
    assert!(any_text(&rt, "Lisbon trip") && !any_text(&rt, "another name"));
}

#[test]
fn the_new_dialogs_open_draw_and_close_on_escape() {
    let new = [
        (Dialog::LinkExpiry { share_id: "s".into() }, "dialog-link-change"),
        (Dialog::LinkPassword { share_id: "s".into(), has_password: false }, "dialog-link-change"),
        (Dialog::ConfirmLeave { id: "i".into() }, "alert-confirm-leave"),
        (Dialog::ConfirmStop { id: "i".into() }, "alert-confirm-stop"),
        (Dialog::ConfirmClose { id: "i".into() }, "alert-confirm-close"),
    ];
    for (d, key) in new {
        let mut rt = runtime(scenes::dw_0511);
        rt.state.frames.shared.open_dialog(d.clone());
        settle(&mut rt);
        assert_eq!(d.section(), Section::Shared);
        assert!(rt.state.frames.shared.fields.link_password.text.is_empty());
        let _ = info(&rt, key);
        press(&mut rt, Key::Escape);
        settle(&mut rt);
        assert_eq!(dialog(&rt), None, "{d:?}");
    }
}

// ---- New request: the four menus and Create link (DW-0504) ----

mod new_request {
    use super::*;
    use crate::frames::shared::source::{
        BoardShared, Invitation, LinkShare, PeopleShare, Request, RequestForm, SharedSource,
    };
    use crate::frames::shared::parts::table::Row;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The board's source, recording every create it is asked for.
    struct Recording(BoardShared, Rc<RefCell<Vec<RequestForm>>>);

    impl SharedSource for Recording {
        fn invitations(&self) -> Vec<Invitation> {
            self.0.invitations()
        }
        fn with_me(&self) -> Vec<Row> {
            self.0.with_me()
        }
        fn people_shares(&self) -> Vec<PeopleShare> {
            self.0.people_shares()
        }
        fn links(&self) -> Vec<LinkShare> {
            self.0.links()
        }
        fn requests(&self) -> Vec<Request> {
            self.0.requests()
        }
        fn accept(&mut self, id: &str) -> bool {
            self.0.accept(id)
        }
        fn decline(&mut self, id: &str) -> bool {
            self.0.decline(id)
        }
        fn keep_offline(&mut self, id: &str) -> bool {
            self.0.keep_offline(id)
        }
        fn stop_sharing(&mut self, id: &str) -> bool {
            self.0.stop_sharing(id)
        }
        fn close_request(&mut self, id: &str) -> bool {
            self.0.close_request(id)
        }
        fn remove_request(&mut self, id: &str) -> bool {
            self.0.remove_request(id)
        }
        fn members(&self, share: &str) -> Vec<crate::frames::shared::source::Member> {
            self.0.members(share)
        }
        fn set_access(&mut self, share: &str, person: &str, access: Access) -> bool {
            self.0.set_access(share, person, access)
        }
        fn remove_access(&mut self, share: &str, person: &str) -> bool {
            self.0.remove_access(share, person)
        }
        fn create_request(&mut self, form: RequestForm) -> bool {
            self.1.borrow_mut().push(form);
            true
        }
        fn folders(&self) -> Vec<(String, String)> {
            vec![("folder-taxes".into(), "Taxes".into())]
        }
    }

    #[test]
    fn create_link_sends_one_create_with_the_chosen_options() {
        let mut rt = runtime(scenes::dw_0504);
        let sent = Rc::new(RefCell::new(Vec::new()));
        rt.state.frames.shared.source = Box::new(Recording(BoardShared::new(), sent.clone()));
        // Opening the dialog asks the source for the account's folders.
        let mut open_tx = rune::Tx::default();
        assert!(crate::frames::shared::actions_new::take(
            &mut rt.state,
            &mut open_tx,
            &Intent::Frame("DW-0504")
        ));
        let n = rt.state.frames.shared.lists_behind();
        click(&mut rt, &format!("item-{n}-0"));
        click(&mut rt, "menu-0-item-1");
        // The Expires menu opens under its row and its choice is taken; Most files too.
        click(&mut rt, &format!("item-{n}-1"));
        assert!(rt.find("menu-0").is_some());
        click(&mut rt, "menu-0-item-1");
        assert!(rt.find("menu-0").is_none());
        click(&mut rt, &format!("item-{n}-2"));
        click(&mut rt, "menu-0-item-3");
        click(&mut rt, &format!("item-{n}-3"));
        click(&mut rt, "menu-0-item-2");
        rt.state.frames.shared.fields.title.text = "Tax forms".into();
        click(&mut rt, "btn-create-link");
        assert_eq!(
            rt.state.frames.intents,
            vec![Intent::Do("create-request", "Tax forms".into())]
        );
        let mut tx = rune::Tx::default();
        let intents = std::mem::take(&mut rt.state.frames.intents);
        for intent in &intents {
            assert!(crate::frames::shared::actions_new::take(&mut rt.state, &mut tx, intent));
        }
        let sent = sent.borrow();
        assert_eq!(sent.len(), 1, "one create per press");
        assert_eq!(sent[0].title, "Tax forms");
        assert_eq!(sent[0].target_folder.as_deref(), Some("folder-taxes"));
        assert_eq!(sent[0].expires_in_hours, Some(24 * 7));
        assert_eq!(sent[0].max_files, Some(10));
        assert_eq!(sent[0].max_total_bytes, Some(1 << 30));
        assert_eq!(dialog(&rt), None, "the dialog closes once the source took it");
    }

    #[test]
    fn the_folder_menu_offers_the_accounts_folders_and_the_choice_is_the_target() {
        use super::super::request_menus::{Pick, Setting};
        let mut pick = Pick::default();
        assert_eq!(pick.words(Setting::Folder), vec!["All files".to_string()]);
        assert_eq!(pick.form("t", "").target_folder, None);
        pick.set_folders(vec![("f-1".into(), "Taxes".into()), ("f-2".into(), "Trips".into())]);
        assert_eq!(
            pick.words(Setting::Folder),
            vec!["All files".to_string(), "Taxes".to_string(), "Trips".to_string()]
        );
        // The default is still All files, with no target.
        assert_eq!(pick.chosen(Setting::Folder), 0);
        assert_eq!(pick.form("t", "").target_folder, None);
        pick.choose(Setting::Folder, 2);
        assert_eq!(pick.chosen(Setting::Folder), 2);
        assert_eq!(pick.form("t", "").target_folder.as_deref(), Some("f-2"));
        // A choice that is not in the menu changes nothing.
        pick.choose(Setting::Folder, 9);
        assert_eq!(pick.chosen(Setting::Folder), 2);
        pick.choose(Setting::Folder, 0);
        assert_eq!(pick.form("t", "").target_folder, None);
        // Folder names are file names: a debug print holds none.
        assert!(!format!("{pick:?}").contains("Taxes"));
    }

    #[test]
    fn a_new_request_starts_at_thirty_days_with_no_limits() {
        let form = super::super::request_menus::Pick::default().form(" A title ", "");
        assert_eq!(form.title, "A title");
        assert_eq!(form.expires_in_hours, Some(720));
        assert_eq!((form.max_files, form.max_total_bytes), (None, None));
    }
}

// ---- The link cards: Change expiry and Password (DW-0511) ----
//
// Shared has no "Copy link" card of its own: Copy link on a row is a press that asks the source (`actions_links`).
// The two cards a link row opens are Change expiry and Password; both are covered here.

mod link_cards {
    use super::*;
    use crate::frames::shared::parts::table::Row;
    use crate::frames::shared::source::{
        BoardShared, Invitation, LinkEdit, LinkShare, Member, PeopleShare, Request, SharedSource,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    /// What the cards sent: a password edit as `Some(Some(typed))` / `Some(None)` (remove), an expiry as `None`.
    type Sent = Rc<RefCell<Vec<(String, Option<Option<String>>)>>>;

    /// The board's source, recording every change of a link.
    struct Links(BoardShared, Sent);

    impl SharedSource for Links {
        fn invitations(&self) -> Vec<Invitation> {
            self.0.invitations()
        }
        fn with_me(&self) -> Vec<Row> {
            self.0.with_me()
        }
        fn people_shares(&self) -> Vec<PeopleShare> {
            self.0.people_shares()
        }
        fn links(&self) -> Vec<LinkShare> {
            self.0.links()
        }
        fn requests(&self) -> Vec<Request> {
            self.0.requests()
        }
        fn accept(&mut self, id: &str) -> bool {
            self.0.accept(id)
        }
        fn decline(&mut self, id: &str) -> bool {
            self.0.decline(id)
        }
        fn keep_offline(&mut self, id: &str) -> bool {
            self.0.keep_offline(id)
        }
        fn stop_sharing(&mut self, id: &str) -> bool {
            self.0.stop_sharing(id)
        }
        fn close_request(&mut self, id: &str) -> bool {
            self.0.close_request(id)
        }
        fn remove_request(&mut self, id: &str) -> bool {
            self.0.remove_request(id)
        }
        fn members(&self, share: &str) -> Vec<Member> {
            self.0.members(share)
        }
        fn set_access(&mut self, share: &str, person: &str, access: Access) -> bool {
            self.0.set_access(share, person, access)
        }
        fn remove_access(&mut self, share: &str, person: &str) -> bool {
            self.0.remove_access(share, person)
        }
        fn change_link(&mut self, share_id: &str, change: LinkEdit) -> bool {
            let seen = match change {
                LinkEdit::Password(typed) => Some(typed.map(|t| t.to_string())),
                LinkEdit::Expiry(_) => None,
            };
            self.1.borrow_mut().push((share_id.to_string(), seen));
            true
        }
    }

    fn open(dialog: Dialog) -> (Runtime<State>, Sent) {
        let mut rt = runtime(scenes::dw_0511);
        let sent = Sent::default();
        rt.state.frames.shared.source = Box::new(Links(BoardShared::new(), sent.clone()));
        rt.state.frames.shared.open_dialog(dialog);
        settle(&mut rt);
        (rt, sent)
    }

    fn password() -> Dialog {
        Dialog::LinkPassword {
            share_id: "link-1".into(),
            has_password: false,
        }
    }

    fn type_password(rt: &mut Runtime<State>, typed: &str) {
        let edit = &mut rt.state.frames.shared.fields.link_password;
        edit.text.clear();
        edit.text.push_str(typed);
        edit.anchor = edit.text.len();
        edit.focus = edit.text.len();
        settle(rt);
    }

    fn typed(rt: &Runtime<State>) -> String {
        rt.state.frames.shared.fields.link_password.text.clone()
    }

    #[test]
    fn the_password_card_opens_on_its_field_and_says_what_it_is() {
        let (rt, sent) = open(password());
        assert_eq!(dialog(&rt), Some(password()));
        assert_eq!(focused(&rt).as_deref(), Some(super::super::link_change::LINK_PASSWORD_INPUT));
        assert!(typed(&rt).is_empty());
        assert!(super::super::link_change::shows_secret(&rt.state));
        assert!(sent.borrow().is_empty(), "opening sends nothing");
    }

    #[test]
    fn enter_in_the_password_field_sends_what_was_typed_closes_the_card_and_wipes_the_field() {
        let (mut rt, sent) = open(password());
        type_password(&mut rt, "an invented password");
        press(&mut rt, Key::Enter);
        assert_eq!(
            *sent.borrow(),
            [("link-1".to_string(), Some(Some("an invented password".to_string())))]
        );
        assert_eq!(dialog(&rt), None);
        assert!(typed(&rt).is_empty(), "the field is wiped once it is sent");
        assert!(!super::super::link_change::shows_secret(&rt.state));
    }

    #[test]
    fn enter_with_nothing_typed_sends_nothing_and_keeps_the_card() {
        let (mut rt, sent) = open(password());
        type_password(&mut rt, "   ");
        press(&mut rt, Key::Enter);
        assert!(sent.borrow().is_empty());
        assert_eq!(dialog(&rt), Some(password()));
    }

    #[test]
    fn escape_closes_the_password_card_sends_nothing_and_wipes_what_was_typed() {
        let (mut rt, sent) = open(password());
        type_password(&mut rt, "never sent");
        press(&mut rt, Key::Escape);
        assert_eq!(dialog(&rt), None);
        assert!(sent.borrow().is_empty());
        assert!(typed(&rt).is_empty(), "typed words do not stay after Esc");
    }

    #[test]
    fn cancel_and_remove_password_close_the_card_and_wipe_it() {
        let (mut rt, sent) = open(password());
        type_password(&mut rt, "typed then cancelled");
        click(&mut rt, "btn-cancel");
        assert_eq!(dialog(&rt), None);
        assert!(typed(&rt).is_empty() && sent.borrow().is_empty());

        rt.state.frames.shared.open_dialog(Dialog::LinkPassword {
            share_id: "link-1".into(),
            has_password: true,
        });
        settle(&mut rt);
        type_password(&mut rt, "typed then removed");
        click(&mut rt, "btn-remove-password");
        assert_eq!(*sent.borrow(), [("link-1".to_string(), Some(None))]);
        assert_eq!(dialog(&rt), None);
        assert!(typed(&rt).is_empty(), "the password is gone with the card");
    }

    #[test]
    fn a_dialog_opened_over_a_typed_password_starts_with_an_empty_field() {
        let (mut rt, _) = open(password());
        type_password(&mut rt, "left behind");
        rt.state.frames.shared.open_dialog(Dialog::LinkExpiry { share_id: "link-1".into() });
        settle(&mut rt);
        assert!(typed(&rt).is_empty());
    }

    #[test]
    fn the_expiry_card_sends_the_choice_and_closes_and_escape_sends_nothing() {
        let link = BoardShared::new().links()[0].id.clone();
        let (mut rt, sent) = open(Dialog::LinkExpiry { share_id: link.clone() });
        let _ = info(&rt, "dialog-link-change");
        press(&mut rt, Key::Escape);
        assert_eq!(dialog(&rt), None);
        assert!(sent.borrow().is_empty());

        rt.state.frames.shared.open_dialog(Dialog::LinkExpiry { share_id: link.clone() });
        settle(&mut rt);
        click(&mut rt, "expiry-7-days");
        assert_eq!(dialog(&rt), None);
        assert_eq!(*sent.borrow(), [(link, None)]);
    }
}
