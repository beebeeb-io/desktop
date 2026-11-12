//! Tests of approving a sign-in (DW-A11 to DW-A17): what each control does, the code as a
//! secret, and what reaches the source. Input is scripted through Rune's `Runtime`.

use super::approve::{
    self, BOARD_REQUEST_CODE, CANCELLED_REQUEST_CODE, CODE_KEY, DROPPED_REQUEST_CODE, EXPIRED_REQUEST_CODE,
    HANDLED_REQUEST_CODE, Lookup, OFFLINE_REQUEST_CODE, UNUSUAL_REQUEST_CODE,
};
use super::source::{AccountSource, BoardAccount};
use super::{Dialog, Pane};
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

fn window(state: State) -> Runtime<State> {
    window_in(state, false)
}

fn window_in(state: State, dark: bool) -> Runtime<State> {
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    rt
}

/// The window on board frame `frame`.
fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    window(state)
}

/// The window on the empty code page (DW-A12), reached from Sign-in and security by its row.
fn on_code_page() -> Runtime<State> {
    let mut rt = runtime("DW-A52");
    click(&mut rt, "item-2-1");
    assert_eq!(rt.state.account.pane, Pane::ApproveCode, "the row opens the code page");
    rt
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn says(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn type_(rt: &mut Runtime<State>, text: &str) {
    key(rt, Key::Text(text.into()));
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn wire(rt: &Runtime<State>) -> Vec<String> {
    rt.state.account.source.as_deref().expect("the fixture").sent()
}

fn dialog(rt: &Runtime<State>) -> Option<Dialog> {
    rt.state.account.dialog.clone()
}

/// Type `code` (without its hyphen, as a person does) on the code page and press Continue.
fn review_of(code: &str) -> Runtime<State> {
    let mut rt = on_code_page();
    type_(&mut rt, &code.replace('-', "").to_lowercase());
    click(&mut rt, "btn-0");
    rt
}

#[test]
fn the_six_frames_are_scenes_with_their_headings_in_both_themes() {
    for (frame, key, heading) in [
        ("DW-A12", "set-title", "Approve a sign-in"),
        ("DW-A13", "dlg-title", "Allow Drive on Anna's iPhone?"),
        ("DW-A14", "dlg-title", "Allow Drive on Anna's iPhone?"),
        ("DW-A15", "dlg-title", "Approved"),
        ("DW-A16", "dlg-title", "Denied"),
        ("DW-A17", "dlg-title", "This request expired"),
    ] {
        for dark in [false, true] {
            let mut state = init();
            assert!(super::scene(frame, &mut state), "{frame} is a built frame");
            let rt = window_in(state, dark);
            assert_eq!(says(&rt, key), heading, "{frame}");
            assert_eq!(
                rt.state.shell.app.screen(),
                Screen::Settings(SettingsPane::Security),
                "{frame} is in the Security section"
            );
        }
    }
}

#[test]
fn typing_shapes_the_code_and_backspace_crosses_the_hyphen() {
    let mut rt = on_code_page();
    assert!(
        info(&rt, CODE_KEY).focused,
        "the field has the keyboard when the page opens"
    );
    type_(&mut rt, "k7qm");
    assert_eq!(
        rt.state.account.approve.code.text, "K7QM-",
        "capitals, and the hyphen after the fourth"
    );
    key(&mut rt, Key::Backspace);
    assert_eq!(
        rt.state.account.approve.code.text, "K7Q",
        "Backspace on the hyphen takes the character before it"
    );
    type_(&mut rt, "m4wtd99");
    assert_eq!(
        rt.state.account.approve.code.text, BOARD_REQUEST_CODE,
        "eight characters and no more"
    );
}

#[test]
fn continue_with_the_code_opens_the_review_and_a_wrong_code_says_one_honest_sentence() {
    let mut rt = on_code_page();
    type_(&mut rt, "zzzz9999");
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.account.pane,
        Pane::ApproveCode,
        "a code nothing waits for stays on the page"
    );
    assert_eq!(
        says(&rt, "field-0-msg"),
        "We could not find a waiting request with that code. Check it on the other device."
    );
    assert_eq!(info(&rt, "field-0-msg").role, "alert");
    // Typing again takes the message away.
    key(&mut rt, Key::Backspace);
    assert_eq!(says(&rt, "field-0-msg"), "Eight characters, shown as XXXX-XXXX.");

    let rt = review_of(BOARD_REQUEST_CODE);
    assert_eq!(
        dialog(&rt),
        Some(Dialog::ApproveReview),
        "Continue opens the review (DW-A13)"
    );
    assert_eq!(says(&rt, "item-1-0-value"), "Germany");
    assert_eq!(says(&rt, "item-1-1-value"), "203.0.113.47");
    assert_eq!(says(&rt, "item-2-0-value"), "Anna's iPhone");
    assert_eq!(
        says(&rt, "list-2-title"),
        "Reported by that device (it can be wrong)",
        "what the device says of itself is labelled as unverified"
    );
}

#[test]
fn approve_is_inert_until_the_typed_code_is_the_requests_code() {
    let mut state = init();
    state.account.source = Some(Box::new(BoardAccount::new()));
    let mut rt = window(state);
    rt.update(|s, _| assert!(approve::notification_clicked(s, "req-iphone"), "a request waits"));
    settle(&mut rt);
    assert_eq!(dialog(&rt), Some(Dialog::ApproveReview), "the toast opens the review");
    assert!(
        rt.state.account.approve.code.text.is_empty(),
        "the notification carries no code"
    );
    click(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert_eq!(
        dialog(&rt),
        Some(Dialog::ApproveReview),
        "Approve and Enter do nothing without the code"
    );
    assert!(
        !wire(&rt).iter().any(|l| l.contains("approve")),
        "and nothing was asked of the server"
    );
    // Another request's code is not this request's code.
    click(&mut rt, CODE_KEY);
    type_(&mut rt, &EXPIRED_REQUEST_CODE.replace('-', ""));
    assert!(!rt.state.account.approve.matches);
    for _ in 0..9 {
        key(&mut rt, Key::Backspace);
    }
    type_(&mut rt, &BOARD_REQUEST_CODE.replace('-', ""));
    assert!(
        rt.state.account.approve.matches,
        "the request's own code makes Approve live"
    );
    key(&mut rt, Key::Enter);
    assert_eq!(
        dialog(&rt),
        Some(Dialog::ApproveApproved),
        "Enter is Approve once the code matches"
    );
}

/// The rule holds in the handler too, not only in which button is live: without the request's
/// code nothing is asked, and a second call while Windows Hello is up asks nothing again.
#[test]
fn the_approve_handler_itself_refuses_without_the_code_and_while_it_waits() {
    let mut state = init();
    state.account.source = Some(Box::new(BoardAccount::new().holding_hello()));
    let mut rt = window(state);
    rt.update(|s, _| assert!(approve::notification_clicked(s, "req-iphone")));
    rt.update(|s, _| approve::approve(s));
    assert_eq!(dialog(&rt), Some(Dialog::ApproveReview), "no code, no approval");
    assert!(!wire(&rt).iter().any(|l| l.contains("/approve")));
    settle(&mut rt);
    click(&mut rt, CODE_KEY);
    type_(&mut rt, &BOARD_REQUEST_CODE.replace('-', ""));
    rt.update(|s, _| approve::approve(s));
    rt.update(|s, _| approve::approve(s));
    assert_eq!(dialog(&rt), Some(Dialog::ApproveHello));
    assert_eq!(
        wire(&rt).iter().filter(|l| l.contains("/approve")).count(),
        1,
        "asked once"
    );
}

#[test]
fn approving_waits_on_windows_hello_and_shares_nothing_until_it_answers() {
    let mut state = init();
    assert!(super::scene("DW-A13", &mut state));
    state.account.source = Some(Box::new(BoardAccount::new().holding_hello()));
    let mut rt = window(state);
    click(&mut rt, "btn-0");
    assert_eq!(
        dialog(&rt),
        Some(Dialog::ApproveHello),
        "Approve asks Windows Hello (DW-A14)"
    );
    assert_eq!(
        says(&rt, "banner-0-title"),
        "Approving shares your keys with that device"
    );
    rt.frame(1.0);
    assert_eq!(dialog(&rt), Some(Dialog::ApproveHello), "no answer, no result");
    // A second press while the prompt is up asks nothing again.
    click(&mut rt, "btn-0");
    assert_eq!(wire(&rt).iter().filter(|l| l.contains("/approve")).count(), 1);
    // Deny while it waits: the prompt goes, then the request ends.
    click(&mut rt, "btn-1");
    assert_eq!(dialog(&rt), Some(Dialog::ApproveDenied));
    let wire = wire(&rt);
    let at = |what: &str| {
        wire.iter()
            .position(|l| l.contains(what))
            .unwrap_or_else(|| panic!("no {what}"))
    };
    assert!(
        at("cancel approval") < at("/deny"),
        "the prompt is dismissed before the denial"
    );
}

#[test]
fn each_answer_of_windows_hello_leads_where_the_board_says() {
    for (code, end) in [
        (BOARD_REQUEST_CODE, Dialog::ApproveApproved),
        (EXPIRED_REQUEST_CODE, Dialog::ApproveExpired { handled: false }),
        (HANDLED_REQUEST_CODE, Dialog::ApproveExpired { handled: true }),
        // Cancelled: back to the review, nothing sent.
        (CANCELLED_REQUEST_CODE, Dialog::ApproveReview),
    ] {
        let mut rt = review_of(code);
        click(&mut rt, "btn-0");
        rt.frame(1.0);
        settle(&mut rt);
        assert_eq!(dialog(&rt), Some(end.clone()), "{code}");
        if end == Dialog::ApproveReview {
            assert_eq!(
                rt.state.account.approve.code.text, code,
                "the code is still there to try again"
            );
            assert!(!rt.state.account.approve.waiting);
        } else {
            assert!(
                rt.state.account.approve.code.text.is_empty(),
                "the code is wiped with the result"
            );
        }
    }
    let mut rt = review_of(HANDLED_REQUEST_CODE);
    click(&mut rt, "btn-0");
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(says(&rt, "dlg-title"), "Already handled");
}

#[test]
fn deny_and_escape_end_the_request_and_the_results_lead_on() {
    // Deny: DW-A16, then Check devices and sessions: DW-A55.
    let mut rt = review_of(BOARD_REQUEST_CODE);
    click(&mut rt, "btn-1");
    assert_eq!(dialog(&rt), Some(Dialog::ApproveDenied));
    assert_eq!(info(&rt, "banner-0").role, "alert", "the banner is an alert");
    click(&mut rt, "btn-0");
    assert_eq!((rt.state.account.pane, dialog(&rt)), (Pane::Sessions, None));

    // Esc on the review is Deny, while the code field holds the keyboard.
    let mut rt = review_of(BOARD_REQUEST_CODE);
    click(&mut rt, CODE_KEY);
    key(&mut rt, Key::Escape);
    assert_eq!(dialog(&rt), Some(Dialog::ApproveDenied), "Esc is Deny");
    assert!(wire(&rt).iter().any(|l| l == "POST /cli-auth/deny req-iphone"));

    // And wherever the keyboard is: the dialog is modal, so Esc is its own.
    let mut rt = review_of(BOARD_REQUEST_CODE);
    key(&mut rt, Key::Escape);
    assert_eq!(
        dialog(&rt),
        Some(Dialog::ApproveDenied),
        "Esc is Deny with the keyboard on a button"
    );
    assert!(
        !wire(&rt).iter().any(|l| l.contains("/approve")),
        "nothing was approved"
    );

    // Approved: See devices and sessions: DW-A55.
    let mut rt = runtime("DW-A15");
    assert_eq!(says(&rt, "item-1-0-pill-label"), "Signed in");
    click(&mut rt, "btn-0");
    assert_eq!((rt.state.account.pane, dialog(&rt)), (Pane::Sessions, None));

    // Expired: Done goes back to Account (DW-A51).
    let mut rt = runtime("DW-A17");
    click(&mut rt, "btn-0");
    assert_eq!((rt.state.account.pane, dialog(&rt)), (Pane::Account, None));
    assert!(rt.state.account.approve.request.is_none(), "and the flow is over");
}

/// The review and its results are modal: Tab stays in the dialog and the pane behind takes no press.
#[test]
fn the_review_is_modal_tab_stays_inside_and_the_pane_behind_takes_no_press() {
    let mut rt = review_of(BOARD_REQUEST_CODE);
    let mut stops = Vec::new();
    for _ in 0..12 {
        key(&mut rt, Key::Tab);
        if let Some(now) = rt
            .elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
        {
            if !stops.contains(&now) {
                stops.push(now);
            }
        }
    }
    stops.sort();
    assert_eq!(
        stops,
        vec![CODE_KEY.to_string(), "btn-0".into(), "btn-1".into(), "list-1".into()],
        "the code, Approve, Deny, and the facts (where the keyboard starts)"
    );
    // The row behind the dialog, where it shows beside it.
    let row = info(&rt, "item-0-0").rect;
    let dlg = info(&rt, "dlg").rect;
    let (x, y) = (dlg[0] + dlg[2] + 20.0, row[1] + row[3] / 2.0);
    assert!(x < row[0] + row[2], "the row reaches past the dialog");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(&mut rt);
    assert_eq!(
        (rt.state.account.pane, dialog(&rt)),
        (Pane::ApproveReview, Some(Dialog::ApproveReview))
    );
}

#[test]
fn offline_says_so_and_approves_nothing() {
    // The lookup cannot reach the server.
    let mut rt = on_code_page();
    type_(&mut rt, &OFFLINE_REQUEST_CODE.replace('-', ""));
    click(&mut rt, "btn-0");
    assert_eq!(rt.state.account.pane, Pane::ApproveCode);
    assert_eq!(
        says(&rt, "field-0-msg"),
        "Beebeeb can't be reached. Nothing was approved."
    );
    // The connection drops after Windows Hello: back on the review, with the same words.
    let mut rt = review_of(DROPPED_REQUEST_CODE);
    click(&mut rt, "btn-0");
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(dialog(&rt), Some(Dialog::ApproveReview));
    assert_eq!(
        says(&rt, "banner-0-text"),
        "Beebeeb can't be reached. Nothing was approved."
    );
    assert_eq!(
        rt.state.account.approve.code.text, DROPPED_REQUEST_CODE,
        "the code is kept to try again"
    );
}

#[test]
fn a_request_from_a_country_never_seen_is_marked_above_the_facts() {
    let rt = review_of(UNUSUAL_REQUEST_CODE);
    assert_eq!(says(&rt, "banner-0-title"), "Unusual place");
    assert!(says(&rt, "banner-0-text").contains("Brazil"));
    assert!(
        info(&rt, "banner-0").rect[1] < info(&rt, "list-1-body").rect[1],
        "above the facts"
    );
    let rt = review_of(BOARD_REQUEST_CODE);
    assert!(find(&rt, "banner-0").is_none(), "a usual place has no note");
}

/// The code is a secret the screen shows as typed: it is in no element text, no label, no
/// accessibility tree, no `Debug` and no line the fixture "sent", on DW-A12, DW-A13 and DW-A14.
#[test]
fn the_typed_code_is_in_no_tree_no_label_no_debug_and_no_request_line() {
    let check = |rt: &Runtime<State>, on: &str| {
        let tree: String = rt
            .elements()
            .iter()
            .map(|e| format!("{:?}|{:?}|{:?}|{}\n", e.key, e.label, e.text, e.path))
            .collect();
        let dumps = [
            ("element tree", tree),
            ("accessibility tree", rt.accessibility_json().to_string()),
            ("Debug", format!("{:?}", rt.state.account.approve.code)),
            ("wire", wire(rt).join("\n")),
        ];
        for (name, dump) in &dumps {
            for part in [BOARD_REQUEST_CODE, "K7QM", "4WTD"] {
                assert!(!dump.contains(part), "{on}: the code is in the {name}");
            }
        }
        assert_eq!(info(rt, CODE_KEY).text, None, "{on}: a secure field reports no text");
        assert!(dumps[1].1.contains("Code "), "{on}: the search finds the field's name");
    };
    let mut rt = on_code_page();
    type_(&mut rt, "k7qm4wtd");
    assert_eq!(
        rt.state.account.approve.code.text, BOARD_REQUEST_CODE,
        "the field took the keys"
    );
    check(&rt, "DW-A12");
    click(&mut rt, "btn-0");
    assert_eq!(dialog(&rt), Some(Dialog::ApproveReview));
    check(&rt, "DW-A13");
    rt.update(|s, _| s.account.source = Some(Box::new(BoardAccount::new().holding_hello())));
    click(&mut rt, "btn-0");
    assert_eq!(dialog(&rt), Some(Dialog::ApproveHello));
    check(&rt, "DW-A14");
}

/// Capture exclusion (task 2460): the screens that hold the code are excluded, the results are not.
#[test]
fn the_screens_that_hold_the_code_are_excluded_from_capture() {
    for (frame, secret) in [
        ("DW-A12", true),
        ("DW-A13", true),
        ("DW-A14", true),
        ("DW-A15", false),
        ("DW-A16", false),
        ("DW-A17", false),
        ("DW-A52", false),
    ] {
        let rt = runtime(frame);
        assert_eq!(rt.state.account.shows_secret(), secret, "{frame}");
    }
}

#[test]
fn leaving_the_section_ends_the_flow_and_wipes_the_code() {
    let mut rt = review_of(BOARD_REQUEST_CODE);
    rt.update(|s, _| s.account.source = Some(Box::new(BoardAccount::new().holding_hello())));
    click(&mut rt, "btn-0");
    assert!(rt.state.account.approve.waiting);
    rt.update(|s, _| super::go(s, "DW-0401"));
    settle(&mut rt);
    key(&mut rt, Key::Tab);
    assert!(
        rt.state.account.approve.code.text.is_empty(),
        "the code went with the page"
    );
    assert!(!rt.state.account.approve.waiting);
    assert!(
        wire(&rt).iter().any(|l| l == "cancel approval req-iphone"),
        "and the prompt was dismissed"
    );
}

/// DW-A11 is Windows' toast: its words are the app's. They are the board's, read from the golden.
#[test]
fn the_notifications_words_are_the_boards_and_carry_no_code() {
    let Lookup::Found(request) = BoardAccount::new().find_sign_in_request(BOARD_REQUEST_CODE) else {
        panic!("the board's request waits");
    };
    let (title, body) = approve::notification(&request);
    for theme in ["", "-dark"] {
        let golden = std::fs::read_to_string(format!("{}/golden/DW-A11{theme}.json", env!("CARGO_MANIFEST_DIR")))
            .expect("the golden of DW-A11");
        let golden: serde_json::Value = serde_json::from_str(&golden).expect("json");
        let words = |key: &str| golden["elements"][key]["text"].as_str().unwrap_or_default().to_string();
        assert_eq!(title, words("os-toast-title"));
        assert_eq!(body, words("os-toast-body"));
        assert_eq!(words("os-toast-app"), "Beebeeb");
    }
    assert!(
        !title.contains("K7QM") && !body.contains("K7QM"),
        "the push carries no code"
    );
}

/// Review P2-2: the code typed on DW-A12, then Enter, opens the review. A second Enter, a third
/// and Space do nothing: Approve does not hold the keyboard and is not the default, so Windows
/// Hello is not reached with the facts unread.
#[test]
fn enter_twice_from_the_code_page_does_not_reach_windows_hello() {
    let mut rt = on_code_page();
    type_(&mut rt, &BOARD_REQUEST_CODE.replace('-', "").to_lowercase());
    key(&mut rt, Key::Enter);
    assert_eq!(dialog(&rt), Some(Dialog::ApproveReview), "the first Enter is Continue");
    assert!(rt.state.account.approve.matches, "the code is this request's");
    let focused = rt
        .elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key);
    assert_eq!(focused.as_deref(), Some("list-1"), "the keyboard starts on the facts");
    for press in [Key::Enter, Key::Enter, Key::Text(" ".into())] {
        key(&mut rt, press);
        assert_eq!(dialog(&rt), Some(Dialog::ApproveReview), "still the review");
        assert!(!rt.state.account.approve.waiting, "Windows Hello was not asked");
    }
    assert_eq!(
        wire(&rt).iter().filter(|line| line.contains("/approve")).count(),
        0,
        "no approval was begun: {:?}",
        wire(&rt)
    );
    // Approve is still there for a deliberate press.
    click(&mut rt, "btn-0");
    assert_eq!(
        wire(&rt).iter().filter(|line| line.contains("/approve")).count(),
        1,
        "a press on Approve begins it"
    );
}

/// The board's sentence still holds where it is safe: "Enter is Approve only when the code
/// matches", once the code was typed on the review itself, under its facts.
#[test]
fn enter_approves_once_the_code_was_typed_on_the_review_itself() {
    let mut state = init();
    state.account.source = Some(Box::new(BoardAccount::new().holding_hello()));
    let mut rt = window(state);
    rt.update(|s, _| assert!(approve::notification_clicked(s, "req-iphone")));
    settle(&mut rt);
    key(&mut rt, Key::Enter);
    assert_eq!(
        dialog(&rt),
        Some(Dialog::ApproveReview),
        "no code yet: Enter does nothing"
    );
    type_(&mut rt, &BOARD_REQUEST_CODE.replace('-', ""));
    assert!(rt.state.account.approve.matches && rt.state.account.approve.typed_here);
    key(&mut rt, Key::Enter);
    assert_eq!(dialog(&rt), Some(Dialog::ApproveHello), "Enter is Approve now");
}

/// Review P2-4: an answer names its request. Windows Hello is up for one request, the person
/// leaves, and its answer is already on its way; then another request is approved. The late
/// answer is dropped: the second request ends as its own answer says.
#[test]
fn a_late_answer_for_another_request_is_dropped() {
    let mut rt = on_code_page();
    rt.update(|s, _| s.account.source = Some(Box::new(BoardAccount::new().late_answers())));
    type_(&mut rt, &BOARD_REQUEST_CODE.replace('-', ""));
    rt.update(|s, _| {
        // Approve and leave before the answer is read: the cancel cannot recall it.
        assert_eq!(approve::look_up_for_tests(s), Some("DW-A13"));
        crate::frames::account::go(s, "DW-A13");
        approve::press_approve(s);
        assert!(s.account.approve.waiting);
        crate::frames::account::go(s, "DW-A12");
        assert!(!s.account.approve.waiting && s.account.approve.request.is_none());
        // The second request, before any poll could read the first one's answer: Windows Hello
        // will be cancelled for it.
        super::form::type_in(&mut s.account.approve.code, CANCELLED_REQUEST_CODE);
        assert_eq!(approve::look_up_for_tests(s), Some("DW-A13"));
        crate::frames::account::go(s, "DW-A13");
        approve::press_approve(s);
        assert!(s.account.approve.waiting);
    });
    for _ in 0..4 {
        settle(&mut rt);
    }
    assert_eq!(
        dialog(&rt),
        Some(Dialog::ApproveReview),
        "the second request's own answer (cancelled): back on the review, not Approved"
    );
    assert!(!rt.state.account.approve.waiting);
}

/// Review P2-5: two requests wait. The toast that was clicked names its request: that one is
/// reviewed, and Deny ends that one.
#[test]
fn with_two_requests_waiting_the_clicked_one_is_reviewed_and_deny_ends_it() {
    let mut state = init();
    state.account.source = Some(Box::new(BoardAccount::new()));
    let mut rt = window(state);
    rt.update(|s, _| assert!(!approve::notification_clicked(s, "req-gone"), "no such request"));
    assert_eq!(dialog(&rt), None);
    rt.update(|s, _| assert!(approve::notification_clicked(s, "req-unusual")));
    settle(&mut rt);
    assert_eq!(says(&rt, "item-1-0-value"), "Brazil", "the request that was clicked");
    click(&mut rt, "btn-1");
    assert_eq!(dialog(&rt), Some(Dialog::ApproveDenied));
    assert_eq!(wire(&rt), vec!["POST /cli-auth/deny req-unusual".to_string()]);
    rt.update(|s, _| assert!(approve::notification_clicked(s, "req-iphone")));
    settle(&mut rt);
    assert_eq!(
        says(&rt, "item-1-0-value"),
        "Germany",
        "and the other one by its own toast"
    );
}
