//! The lane's dialogs: what each says it is to assistive technology, where each opens from and
//! goes back to, what each control does by pointer and by keyboard, and what stays the engine's.

use super::Dialog;
use crate::frames::Intent;
use crate::frames::shared::scenes;
use crate::frames::shared::source::Access;
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

#[test]
fn each_dialog_and_its_controls_are_named_as_the_board_names_them() {
    let frames: [(fn(&mut State), &str, &str, &str); 9] = [
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
        named("listitem", "Files go to, Documents")
    );
    assert_eq!(
        role_and_name(&rt, "item-1-3"),
        named("listitem", "Total size, No limit")
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

    // Empty Trash?: ONE control for the code, named without its digits; the button is off.
    let rt = runtime(scenes::dw_0749);
    assert_eq!(
        role_and_name(&rt, "secure-code"),
        named("textbox", "Authenticator code, 6 digits, 2 entered")
    );
    assert_eq!(focused(&rt).as_deref(), Some("secure-code"));
    assert!(!info(&rt, "btn-empty-trash-2").focusable, "Empty Trash is off");
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
    press(&mut rt, Key::Text("lea@northwind.studio".into()));
    click(&mut rt, "seg-1-can-edit");
    assert!(matches!(
        dialog(&rt),
        Some(Dialog::Invite {
            access: Access::Edit,
            ..
        })
    ));
    // Enter in the field sends it.
    click(&mut rt, super::EMAIL_INPUT);
    press(&mut rt, Key::Enter);
    assert_eq!(dialog(&rt), Some(people()));
    assert_eq!(text(&rt, "item-2-3-title").as_deref(), Some("lea@northwind.studio"));
    assert_eq!(
        text(&rt, "item-2-3-sub").as_deref(),
        Some("Can edit \u{00B7} Invited, waiting")
    );
    assert_eq!(
        rt.state.frames.shared.source.people_shares()[0].names,
        "Jonas Meier, Mia de Vries, lea@northwind.studio"
    );

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
fn the_code_takes_six_digits_at_most_shows_none_to_any_reader_and_is_wiped_on_close() {
    let mut rt = runtime(scenes::dw_0749);
    // Digits only, six at most; Backspace takes the last.
    press(&mut rt, Key::Text("x7".into()));
    press(&mut rt, Key::Text("2345".into()));
    assert_eq!(rt.state.frames.shared.code_len(), 6);
    assert_eq!(rt.state.frames.shared.fields.code.text, "517234");
    press(&mut rt, Key::Backspace);
    assert_eq!(rt.state.frames.shared.code_len(), 5);
    assert_eq!(
        info(&rt, "secure-code").label.as_deref(),
        Some("Authenticator code, 6 digits, 5 entered")
    );
    // No reader sees a digit: each is secure text, the control's name counts them only.
    for i in 0..5 {
        assert_eq!(info(&rt, &format!("code-{i}-digit")).text, None, "digit {i}");
    }
    let debug = format!("{:?}", rt.state.frames.shared.fields);
    assert!(!debug.contains("51723"), "the code is in the fields' Debug");
    // Six digits still do not empty the Trash (the engine has no call for it yet).
    press(&mut rt, Key::Text("9".into()));
    click(&mut rt, "btn-empty-trash-2");
    assert!(rt.state.frames.intents.is_empty());
    assert_eq!(rt.state.frames.trash.source.totals().items, 12);
    // Cancel or Esc: closed, and the code is gone.
    press(&mut rt, Key::Escape);
    assert_eq!(dialog(&rt), None);
    assert!(rt.find("dialog-empty-trash").is_none());
    assert_eq!(rt.state.frames.shared.fields.code.text, "");
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
    let plain = |key: Key| KeyEvent {
        key,
        mods: Mods::default(),
    };
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
fn a_dialog_shows_only_over_its_own_section() {
    let mut rt = runtime(scenes::dw_0749);
    assert!(rt.find("dialog-empty-trash").is_some());
    // Both sections answer (the scenes start the other one on the shell's "not built yet" frame).
    rt.state.frames.shared.source = Box::new(crate::frames::shared::source::BoardShared::new());
    crate::frames::go(&mut rt.state, Section::Shared);
    settle(&mut rt);
    assert!(rt.find("dialog-empty-trash").is_none(), "Trash's dialog over Shared");
    crate::frames::go(&mut rt.state, Section::Trash);
    settle(&mut rt);
    assert!(rt.find("dialog-empty-trash").is_some());

    let mut rt = runtime(scenes::dw_0511);
    rt.state.frames.trash = crate::frames::trash::TrashUi::default();
    crate::frames::go(&mut rt.state, Section::Trash);
    settle(&mut rt);
    assert!(rt.find("row-0").is_some(), "Trash shows its items");
    assert!(rt.find("dialog-people").is_none(), "Shared's dialog over Trash");
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
