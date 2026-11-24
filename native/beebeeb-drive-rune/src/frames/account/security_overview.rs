//! Sign-in and security, ONE page (a design of this app's, no board frame; it stands in place of DW-A52 and the
//! security checkup DW-A84 / DW-A84a): the score with what is left to do, then every subject as a row that
//! says its state and opens its own page, the last sign-ins, signing out, and deleting the account.
//!
//! **Honest states.** Everything drawn comes from [`SecurityFacts`], where each part is known or not. A part
//! nobody read says "Not known"; a function this build has not bound says "Not available yet" and is not a
//! link, a switch or a button. No check mark and no score is ever drawn from a value that was not read: with
//! no factors there is no ring fill, no count and no "Nothing to do".
//!
//! Its two sign-out confirms are the board's (DW-A57, DW-A57a), and App lock's delay menu is DW-0716.

use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::account_set::{self as set, Lead, Look, Row, Size, Tail};
use crate::ui::au::Tone;
use crate::ui::kit;
use crate::ui::settings_list as lists;
use rune::{El, TextStyle, col, el, row};

use super::source::{AccountSource, Count, Factor, FactorKind, PaneState, SecurityFacts, SignOutAnswer, score};
use super::{Dialog, icons};

/// The score's line: larger than a list's head, smaller than the pane's title.
const SCORE_HEAD: TextStyle = TextStyle::new(20.0, 650.0).lh(24.0).tracking(-0.3);

/// What a part says when nobody read it.
pub(super) const NOT_KNOWN: &str = "Not known";
/// What a part read from Beebeeb says when Beebeeb could not be reached to read it (`AccountSource::offline`).
pub(super) const OFFLINE: &str = "Offline";

/// The words for a part read from Beebeeb that nobody read: offline, or not known.
fn unread(source: &dyn AccountSource) -> &'static str {
    if source.offline() { OFFLINE } else { NOT_KNOWN }
}

fn factor_icon(kind: FactorKind) -> &'static rune::Icon {
    match kind {
        FactorKind::EmailVerified => &icons::MAIL,
        FactorKind::TwoFactor => &icons::SHIELD,
        FactorKind::StrongPassword => &icons::PADLOCK,
        FactorKind::RecoveryPhraseSaved => &icons::KEY,
        FactorKind::Passkeys => &icons::PASSKEY,
        FactorKind::PasswordNotInBreach => &icons::EYE_OFF,
        FactorKind::Sessions => &icons::DEVICE_PHONE,
    }
}

/// Why a factor that is left to do matters, in one sentence.
fn why(kind: FactorKind) -> &'static str {
    match kind {
        FactorKind::EmailVerified => "Shows the address is yours, so sign-in mail reaches you.",
        FactorKind::TwoFactor => "A code from your phone at sign-in, so a stolen password is not enough.",
        FactorKind::StrongPassword => "A longer password is harder to guess.",
        FactorKind::RecoveryPhraseSaved => "Your 12 words get you back in when you forget your password.",
        FactorKind::Passkeys => "Sign in with Windows Hello or your phone instead of typing your password.",
        FactorKind::PasswordNotInBreach => "This password is in a known leak. Change it.",
        FactorKind::Sessions => "See which devices are signed in, and end the ones you don't use.",
    }
}

/// Whether something answers for the page `frame` shows: only then is a row a link or a button drawn.
fn answered(source: &dyn AccountSource, frame: &str) -> bool {
    super::local(frame).is_none_or(|(pane, _)| source.pane_state(pane) != PaneState::Unwired)
}

/// What a row says of its own state: nothing, a state that was read, or a read state with something at the
/// trailing edge (a value, the mark of a thing done).
enum State_ {
    None,
    Read,
    Marked(Tail),
}

/// A row that opens `frame`. While nothing answers for that page the row is not drawn: a row either works or
/// is not there (never a row that leads to a page that is not there).
fn leads(
    list: usize,
    index: usize,
    it: Row,
    state: State_,
    frame: &'static str,
    source: &dyn AccountSource,
    t: &Theme,
) -> Option<El> {
    if !answered(source, frame) {
        return None;
    }
    let it = match state {
        State_::Marked(mark) => it.tail(mark),
        State_::Read | State_::None => it,
    };
    Some(set::item(list, index, &it.link(true), Vec::new(), t).on_press(move |s: &mut State| super::go(s, frame)))
}

/// A list of the rows that are drawn; none drawn, no list. `roving`: every row is the control (it opens a
/// page or a confirm), so the list is one Tab stop with the arrows and type-to-jump (`settings_list::roving_list`);
/// a list with a switch or a text row in it stays a plain list.
fn list_of(
    list: usize,
    head: Option<&str>,
    rows: Vec<Option<El>>,
    foot: Option<&str>,
    roving: bool,
    t: &Theme,
) -> Option<El> {
    let rows: Vec<El> = rows.into_iter().flatten().collect();
    let build = if roving { lists::roving_list } else { lists::list };
    (!rows.is_empty()).then(|| build(list, head, rows, foot, t))
}

/// The factors, when there is a score to say: known and not empty.
fn scored(facts: &SecurityFacts) -> Option<&[Factor]> {
    facts.factors.as_deref().filter(|factors| !factors.is_empty())
}

fn done(facts: &SecurityFacts, kind: FactorKind) -> bool {
    scored(facts).is_some_and(|factors| factors.iter().any(|f| f.kind == kind && f.done))
}

/// The score card: the ring and the count, then what is left to do, each with the button that fixes it.
/// With no factors: no fill, no count, and a sentence that says there is no score.
fn score_card(facts: &SecurityFacts, source: &dyn AccountSource, t: &Theme) -> El {
    // `ink`: the fill's colour. With no score the whole ring is drawn in the track's own colour: nothing of it
    // reads as done, and no end of a fill shows.
    let ring = |fraction: f32, inside: String, ink: rune::Color| {
        el().key("ring-box")
            .a11y_hidden()
            .size(56.0, 56.0)
            .center()
            .child(set::ring("ring", 56.0, 4.0, fraction, ink, t).absolute(0.0, 0.0))
            .child(kit::label("ring-label", inside, set::RING_COUNT, t.text))
    };
    let words = |head: String, sub: String| {
        col()
            .key("score-words")
            .grow(1.0)
            .min_w(0.0)
            .gap(crate::theme::scale::space::S2)
            .child(
                kit::label("ring-head", head, SCORE_HEAD, t.text)
                    .role("heading")
                    .level(2),
            )
            .child(kit::label("ring-sub", sub, set::SUB, t.text_secondary).wrap().w_full())
    };
    let card = col()
        .key("card-0")
        .role("group")
        .label("Security score")
        .w_full()
        .padding(crate::theme::scale::space::S18)
        .gap(crate::theme::scale::space::S12)
        .radius(crate::theme::scale::radius::PANEL)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, rune::Color::BLACK.alpha(0.04)));
    let line = row().key("score").w_full().center_y().gap(crate::theme::scale::space::S14);
    let Some(factors) = scored(facts) else {
        return card.child(line.child(ring(1.0, "?".into(), t.separator)).child(words(
            "Security score not known yet".into(),
            "Beebeeb can't say yet which steps are done. Nothing on this page is a guess.".into(),
        )));
    };
    let (ok, of) = score(factors);
    let says = match of - ok {
        0 => "Nothing to do.".to_string(),
        1 => "One thing left to do.".to_string(),
        n => format!("{n} things left to do."),
    };
    let mut card = card.child(
        line.child(ring(ok as f32 / of as f32, ok.to_string(), t.text))
            .child(words(format!("{ok} of {of} security steps done"), says)),
    );
    for (i, f) in factors.iter().filter(|f| !f.done).enumerate() {
        let frame = f.kind.fix_frame();
        let mut todo = row()
            .key(format!("todo-{i}"))
            .role("listitem")
            .label(format!("To do: {}. {}", f.says, why(f.kind)))
            .w_full()
            .center_y()
            .gap(crate::theme::scale::space::S10)
            .pt(12.0)
            .child(
                el().key(format!("todo-{i}-rule"))
                    .pin(Some(0.0), Some(0.0), Some(0.0), None)
                    .h(kit::HAIR)
                    .fill(t.separator),
            )
            .child(set::tile(
                &format!("todo-{i}-tile"),
                factor_icon(f.kind),
                28.0,
                8.0,
                Tone::Neutral,
                t,
            ))
            .child(
                col()
                    .key(format!("todo-{i}-text"))
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(1.0)
                    .child(kit::label(format!("todo-{i}-title"), f.says.clone(), type_::ITEM_TITLE, t.text).truncate())
                    .child(
                        kit::label(format!("todo-{i}-sub"), why(f.kind), set::SUB, t.text_tertiary)
                            .wrap()
                            .w_full(),
                    ),
            );
        // The button only where something answers for the page that fixes it. The first is the page's one
        // primary action; the others are plain.
        if answered(source, frame) {
            let look = if i == 0 { Look::Primary } else { Look::Plain };
            todo = todo.child(
                set::button(
                    &format!("todo-{i}-btn"),
                    f.kind.fix_label(),
                    None,
                    look,
                    Size::Small,
                    false,
                    t,
                )
                .label(format!("{}, {}", f.kind.fix_label(), f.says))
                .on_press(move |s: &mut State| super::go(s, frame)),
            );
        }
        card = card.child(todo);
    }
    card
}

/// The state of a row whose value was read (`known`), with the mark when it is good AND was read. Nothing
/// else gets the mark.
fn checked(known: bool, good: bool) -> State_ {
    match (known, good) {
        (_, true) => State_::Marked(Tail::Check),
        (true, false) => State_::Read,
        (false, false) => State_::None,
    }
}

fn sign_in(facts: &SecurityFacts, source: &dyn AccountSource, t: &Theme) -> Option<El> {
    let password = Row::new(Lead::Icon(&icons::PADLOCK), "Password").sub(match &facts.password_changed {
        Some(when) => format!("Changed {when}"),
        None => "Last change not known".into(),
    });
    let password_ok = checked(
        facts.password_changed.is_some(),
        done(facts, FactorKind::StrongPassword) && done(facts, FactorKind::PasswordNotInBreach),
    );
    let two = Row::new(Lead::Icon(&icons::SHIELD), "Two-factor authentication").sub(
        match (facts.two_factor, facts.backup_codes) {
            (Some(true), Some((left, of))) => format!("On · {left} of {of} backup codes left"),
            (Some(true), None) => "On".into(),
            (Some(false), _) => "Off: your password alone signs you in".into(),
            (None, _) => unread(source).into(),
        },
    );
    let two_ok = checked(facts.two_factor.is_some(), facts.two_factor == Some(true));
    let passkeys = Row::new(Lead::Icon(&icons::PASSKEY), "Passkeys").sub(match facts.passkeys {
        Some(0) => "None added".into(),
        Some(1) => "1 passkey".into(),
        Some(n) => format!("{n} passkeys"),
        None => NOT_KNOWN.into(),
    });
    let passkeys_ok = checked(facts.passkeys.is_some(), facts.passkeys.is_some_and(|n| n > 0));
    list_of(
        0,
        Some("Sign-in"),
        vec![
            leads(0, 0, password, password_ok, "DW-A67", source, t),
            leads(0, 1, two, two_ok, "DW-A71", source, t),
            leads(0, 2, passkeys, passkeys_ok, "DW-A73", source, t),
        ],
        None,
        true,
        t,
    )
}

/// App lock is Drive's own row (the board's `securityExtra` slot, DW-0716): its switch and delay are here,
/// not on a page behind it. A source that gives no delay (an empty one: nothing acts on it in this version)
/// gets no delay control and no "locks again after".
fn app_lock_row(facts: &SecurityFacts, t: &Theme) -> Option<El> {
    let it = Row::new(Lead::Icon(&icons::APP_LOCK), "Lock Beebeeb");
    // No facts: nothing acts on it in this build, so the row is not drawn.
    let (on, after) = facts.app_lock.as_ref()?;
    let on = *on;
    let delay = on && !after.is_empty();
    let mut controls = Vec::new();
    if delay {
        controls.push(
            el().key("item-1-0-pop-button")
                .role("combobox")
                .label(format!("Locks again after, {after}"))
                .mr(8.0)
                .radius(crate::theme::tokens::radius::MD)
                .focusable(true)
                .on_press(|s: &mut State| super::go(s, "DW-0716"))
                .child(lists::pop_button("item-1-0", after, t)),
        );
        // Where the delay menu's corner goes (DW-0716): 3 pt under the pop-up button, 4 pt inside the
        // card's trailing edge.
        controls.push(
            el().key(super::app_lock::ANCHOR)
                .pin(None, Some(35.0), Some(134.0), None)
                .size(0.0, 0.0),
        );
    }
    // Windows Hello did not confirm turning it off or a longer delay: the account password can (D-2).
    if facts.app_lock_password.is_some() {
        controls.push(super::app_lock_password::link(t));
    }
    controls.push(
        crate::ui::switch::switch_sized(&format!("{}-switch", "item-1-0"), "", on, true, crate::ui::button::Size::Md, t).label("Lock Beebeeb").on_press(move |s: &mut State| {
            if let Some(source) = s.account.source.as_mut() {
                source.set_app_lock(!on);
            }
        }),
    );
    Some(set::item(
        1,
        0,
        &it.sub(if delay {
            "Windows Hello · locks again after"
        } else if on {
            "Windows Hello"
        } else {
            "Off: anyone using this PC can open Beebeeb"
        }),
        controls,
        t,
    ))
}

/// "Hide Beebeeb from screen capture and remote desktop": a per-PC switch, off by default (`shell::capture_setting`).
/// Off, the windows show in screenshots, screen shares and remote desktop; on, every window is left out of them.
fn capture_row(on: bool, t: &Theme) -> El {
    let it = Row::new(Lead::Icon(&icons::EYE_OFF), "Hide Beebeeb from screen capture and remote desktop").sub(
        if on {
            "On: screenshots, recordings and remote desktop show nothing of Beebeeb"
        } else {
            "Off: Beebeeb shows in screenshots, screen shares and remote desktop"
        },
    );
    set::item(
        1,
        3,
        &it,
        vec![
            crate::ui::switch::switch_sized(&format!("{}-switch", "item-1-3"), "", on, true, crate::ui::button::Size::Md, t).label("Hide Beebeeb from screen capture and remote desktop").on_press(
                move |s: &mut State| s.shell.set_hide_from_capture(!on),
            ),
        ],
        t,
    )
}

/// Approving a sign-in from another device. A source that has it opens it; one that has not draws no row.
fn approve_row(source: &dyn AccountSource, t: &Theme) -> Option<El> {
    let it = Row::new(Lead::Icon(&icons::SCAN), "Approve a sign-in");
    leads(
        1,
        2,
        it.sub("Let another device sign in"),
        State_::None,
        "DW-A12",
        source,
        t,
    )
}

fn devices(facts: &SecurityFacts, source: &dyn AccountSource, hide_capture: bool, t: &Theme) -> Option<El> {
    // "N signed in": true of devices and of sessions alike (a source lists one or the other).
    let sessions =
        Row::new(Lead::Icon(&icons::DEVICE_PHONE), "Devices and sessions").sub("Where your account is signed in");
    let signed_in = Tail::Value(match facts.sessions {
        Count::Known(n) => format!("{n} signed in"),
        Count::Reading => "Loading".into(),
        Count::NotKnown => unread(source).into(),
    });
    list_of(
        1,
        Some("This PC and your devices"),
        vec![
            app_lock_row(facts, t),
            leads(1, 1, sessions, State_::Marked(signed_in), "DW-A55", source, t),
            approve_row(source, t),
            Some(capture_row(hide_capture, t)),
        ],
        None,
        false,
        t,
    )
}

/// The last sign-ins and security changes, then the way to all of them. Read as what happened, then when and
/// where: a country, never a city; never which files were opened.
fn activity(facts: &SecurityFacts, source: &dyn AccountSource, t: &Theme) -> Option<El> {
    let all = |index: usize, sub: &str| {
        leads(
            2,
            index,
            Row::new(Lead::Icon(&icons::ACTIVITY), "All sign-in activity").sub(sub),
            State_::None,
            "DW-A77",
            source,
            t,
        )
    };
    let rows = match &facts.activity {
        // Not read: the way to the page stays, when the page answers.
        None => vec![all(0, unread(source))],
        Some(None) => vec![all(0, "Security logging is off: nothing is recorded")],
        Some(Some(events)) if events.is_empty() => vec![all(0, "Recording is on. Nothing recorded yet")],
        Some(Some(events)) => {
            let mut rows: Vec<Option<El>> = events
                .iter()
                .take(super::source::RECENT_EVENTS)
                .enumerate()
                .map(|(i, e)| {
                    Some(set::item(
                        2,
                        i,
                        &Row::new(Lead::Icon(super::activity::event_icon(e.kind)), e.what.clone())
                            .sub_mono(e.when_where.clone())
                            .name(format!("{}, {}", e.what, e.when_where.replace(" · ", ", "))),
                        Vec::new(),
                        t,
                    ))
                })
                .collect();
            let index = rows.len();
            rows.push(all(index, "Recording is on"));
            rows
        }
    };
    list_of(2, Some("Recent sign-in activity"), rows, None, false, t)
}

/// The two destructive rows open a confirm over the page; neither touches other devices. A row is drawn only
/// for what the source can do: one that cannot sign this app out draws none, and "all apps" is drawn only when
/// the source can sign every Beebeeb app on this PC out.
fn sign_out(facts: &SecurityFacts, t: &Theme) -> Option<El> {
    if !facts.can_sign_out {
        return None;
    }
    let one = |index: usize, it: Row, dialog: Dialog| {
        Some(set::item(3, index, &it.danger().link(false), Vec::new(), t).on_press(move |s: &mut State| {
            s.account.dialog = Some(dialog.clone());
            arrive(&mut s.account);
        }))
    };
    let all = facts.can_sign_out_all;
    let rows = vec![
        one(
            0,
            Row::new(Lead::Icon(&icons::SIGN_OUT), "Sign out of Drive"),
            Dialog::SignOutApp,
        ),
        all.then(|| {
            one(
                1,
                Row::new(Lead::Icon(&icons::SIGN_OUT), "Sign out of all Beebeeb apps").sub("On this PC"),
                Dialog::SignOutAllApps,
            )
        })
        .flatten(),
    ];
    list_of(
        3,
        Some("Sign out"),
        rows,
        Some(if all {
            "Signing out of Drive signs out Drive on this PC only. The other Beebeeb apps stay signed in."
        } else {
            "Signing out of Drive signs out Drive on this PC only."
        }),
        true,
        t,
    )
}

fn delete(source: &dyn AccountSource, t: &Theme) -> Option<El> {
    // A source that has no deletion flow draws no row.
    if !answered(source, "DW-A58") {
        return None;
    }
    let it = Row::new(Lead::Icon(&icons::TRASH), "Delete your Beebeeb account").danger();
    // A deletion this session already knows of opens where it stands: the link sent, or the schedule and its undo.
    let frame = match source.deletion() {
        super::source::Deletion::Scheduled { .. } => "DW-A82",
        super::source::Deletion::LinkSent => "DW-A81",
        super::source::Deletion::None => "DW-A58",
    };
    list_of(
        4,
        Some("Delete"),
        vec![leads(4, 0, it, State_::None, frame, source, t)],
        // How long it can be undone is Beebeeb's: the date is shown once it is scheduled.
        Some("After you confirm by email you can still undo it for a time. After that it cannot be undone."),
        true,
        t,
    )
}

/// A confirm was just asked for over the page (`go`): one this source cannot act on does not open at all, so
/// nothing that is not drawn holds the keyboard.
pub fn arrive(ui: &mut super::AccountUi) {
    // A confirm that opens starts clean: what an earlier one said (a refusal) is not this one's.
    if ui.signing_out.waiting.is_none() {
        ui.signing_out = SigningOut::default();
    }
    let facts = ui.source.as_deref().map(|s| s.security_facts());
    let may = |all: bool| {
        facts
            .as_ref()
            .is_some_and(|f| if all { f.can_sign_out_all } else { f.can_sign_out })
    };
    if (matches!(ui.dialog, Some(Dialog::SignOutApp)) && !may(false))
        || (matches!(ui.dialog, Some(Dialog::SignOutAllApps)) && !may(true))
    {
        ui.dialog = None;
    }
}

/// The page, and the confirm or the menu over it when one is open. `signing`: where a sign-out asked from the
/// confirm stands.
pub fn pane(
    source: &dyn AccountSource,
    dialog: Option<&Dialog>,
    signing: &SigningOut,
    unsaved_name: &str,
    hide_capture: bool,
    t: &Theme,
) -> (El, Option<El>) {
    let facts = source.security_facts();
    let over = match dialog {
        Some(Dialog::SignOutApp) if facts.can_sign_out => Some(sign_out_confirm(
            "Sign out of Drive?",
            "This signs out Drive on this PC only. Drive, VPN, Chat and Mail stay signed in. Your data stays encrypted on this PC; signing in again opens it.",
            "Sign out",
            false,
            signing,
            unsaved_name,
            t,
        )),
        Some(Dialog::SignOutAllApps) if facts.can_sign_out_all => Some(sign_out_confirm(
            "Sign out of all Beebeeb apps on this PC?",
            "Drive, Pass, VPN, Chat and Mail on this PC are signed out. Their data stays encrypted on this PC; signing in again opens it. Your other devices stay signed in.",
            "Sign out of all",
            true,
            signing,
            unsaved_name,
            t,
        )),
        Some(Dialog::LockDelay(hot)) => match &facts.app_lock {
            Some((true, after)) if !after.is_empty() => Some(super::app_lock::menu(after, *hot, t)),
            _ => None,
        },
        _ => None,
    };
    let mut children = vec![
        set::heading("Sign-in and security", None, None, t),
        score_card(&facts, source, t),
    ];
    children.extend(
        [
            sign_in(&facts, source, t),
            devices(&facts, source, hide_capture, t),
            activity(&facts, source, t),
            sign_out(&facts, t),
            delete(source, t),
        ]
        .into_iter()
        .flatten(),
    );
    (set::pane(children, t), over)
}

/// Where a sign-out asked from the confirm stands. Nothing of a sign-out is drawn before its answer: the
/// window leaves for the sign-in screen only on `SignOutAnswer::Done`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SigningOut {
    /// Sent, not answered yet: all apps (`true`) or this one. The confirm waits and takes no press.
    pub waiting: Option<bool>,
    /// It was refused: the confirm says why, and that nothing was signed out.
    pub refused: Option<&'static str>,
    /// An open editor holds unsaved changes: the confirm asks Save, Discard and sign out, or Cancel.
    pub unsaved: bool,
    /// Save was pressed: the confirm waits for the editor's save, then signs out. A save that did not land says so.
    pub saving: bool,
    /// The question is for "Sign out of all" (not this app only).
    pub all: bool,
}

/// What the confirm says when the save it waited for did not land.
pub(super) const NOT_SAVED: &str = "The changes were not saved. Nothing was signed out.";

/// What the confirm says while its sign-out is out.
pub(super) const SIGNING_OUT: &str = "Signing out. Nothing is signed out until Beebeeb has answered.";

/// Take in how a sign-out ended: done leaves for the sign-in screen (with the Continue as card after one app,
/// DW-A02; without it after all of them, DW-A03); refused stays on the confirm and says why.
pub(super) fn settle_sign_out(s: &mut State, all: bool, answer: SignOutAnswer) {
    match answer {
        SignOutAnswer::Done => {
            s.account.signing_out = SigningOut::default();
            s.account.dialog = None;
            super::go(s, if all { "DW-A03" } else { "DW-A02" });
        }
        SignOutAnswer::Refused(why) => {
            s.account.signing_out = SigningOut {
                refused: Some(why),
                ..SigningOut::default()
            };
        }
        SignOutAnswer::Unsaved => {
            // The editor's name, read where the open document is (nothing else says which one holds the changes).
            s.account.unsaved_name = crate::frames::viewers::unsaved_guard::name(s);
            s.account.signing_out = SigningOut {
                unsaved: true,
                all,
                ..SigningOut::default()
            };
        }
        SignOutAnswer::Pending => s.account.signing_out.waiting = Some(all),
    }
}

/// The sign-out met an open editor with changes no save has landed: the shared question (`unsaved_guard`) with
/// Save (saves, then signs out), Discard and sign out (the only way they are dropped) and Cancel (Enter).
fn unsaved_confirm(title: &str, name: &str, saving: bool, t: &Theme) -> El {
    use crate::frames::viewers::unsaved_guard as guard;
    guard::question(
        title,
        &guard::words(name, saving),
        saving,
        "Discard and sign out",
        |s: &mut State, _tx: &mut rune::Tx| {
            // Save is the editor's own save; the sign-out follows when it landed (`follow_save`).
            let all = s.account.signing_out.all;
            if guard::save_open_editor(s) {
                s.account.signing_out = SigningOut {
                    saving: true,
                    all,
                    ..SigningOut::default()
                };
            } else {
                s.account.signing_out = SigningOut {
                    refused: Some(NOT_SAVED),
                    ..SigningOut::default()
                };
            }
        },
        |s: &mut State, _tx: &mut rune::Tx| {
            let all = s.account.signing_out.all;
            let Some(source) = s.account.source.as_mut() else {
                return;
            };
            let answer = if all {
                source.sign_out_all_apps()
            } else {
                source.sign_out_discarding_unsaved()
            };
            settle_sign_out(s, all, answer);
        },
        |s: &mut State, _tx: &mut rune::Tx| {
            s.account.signing_out = SigningOut::default();
            s.account.dialog = None;
        },
        t,
    )
}

/// While Save waits for the editor: when its save landed the sign-out goes on (the engine asks again, and
/// refuses if anything is still unsaved); a save that did not land says so and signs out nothing. `true`: changed.
pub(super) fn follow_save(s: &mut State, tx: &mut rune::Tx) -> bool {
    if !s.account.signing_out.saving {
        return false;
    }
    let state = s.viewer.ui.as_ref().and_then(|ui| ui.office.edit.as_ref().map(|e| (e.saving, e.unsaved())));
    match state {
        Some((true, _)) => {
            tx.wake_after(std::time::Duration::from_millis(100));
            false
        }
        Some((false, true)) => {
            s.account.signing_out = SigningOut {
                refused: Some(NOT_SAVED),
                ..SigningOut::default()
            };
            true
        }
        // Saved (or the editor is gone): the engine decides whether anything is still unsaved.
        _ => {
            let all = s.account.signing_out.all;
            s.account.signing_out = SigningOut::default();
            let Some(source) = s.account.source.as_mut() else { return true };
            let answer = if all { source.sign_out_all_apps() } else { source.sign_out_this_app() };
            settle_sign_out(s, all, answer);
            true
        }
    }
}

/// DW-A57 and DW-A57a: the scope sentence is read before the buttons; Enter is Cancel. While the sign-out is
/// out the confirm says so and neither button takes a press; it closes only with the answer.
fn sign_out_confirm(
    title: &str,
    words: &str,
    action: &str,
    all: bool,
    signing: &SigningOut,
    unsaved_name: &str,
    t: &Theme,
) -> El {
    if signing.unsaved || signing.saving {
        return unsaved_confirm(title, unsaved_name, signing.saving, t);
    }
    let waiting = signing.waiting.is_some();
    let words = match (waiting, signing.refused) {
        (true, _) => SIGNING_OUT,
        (false, Some(why)) => why,
        (false, None) => words,
    };
    let mut act = set::button("btn-1", action, None, Look::Danger, Size::Medium, waiting, t)
        .label(format!("{action}, destructive"));
    let mut cancel = set::button("btn-2", "Cancel", None, Look::Plain, Size::Medium, waiting, t).default_button();
    if !waiting {
        act = act.on_press(move |s: &mut State| {
            // "Sign out of all" ends this app's editors too: the same question as for this app's sign-out.
            if all && crate::frames::viewers::unsaved_guard::holds(s) {
                settle_sign_out(s, true, SignOutAnswer::Unsaved);
                return;
            }
            let Some(source) = s.account.source.as_mut() else {
                return;
            };
            let answer = if all {
                source.sign_out_all_apps()
            } else {
                source.sign_out_this_app()
            };
            settle_sign_out(s, all, answer);
        });
        cancel = cancel.on_press(|s: &mut State| s.account.dialog = None);
    }
    let foot = vec![(act, false), (cancel, true)];
    set::dialog(title, 440.0, title, words, Vec::new(), foot, t).modal(false, |s: &mut State| {
        if s.account.signing_out.waiting.is_none() {
            s.account.dialog = None;
        }
    })
}

/// dc-005: what a sign-out keeps and removes, with counts. Sealed staged copies stay encrypted on this PC and open
/// again after sign-in; unsaved Office recovery copies are removed. `waiting`: uploads not sent yet.
#[cfg_attr(not(test), allow(dead_code))]
pub fn sign_out_counts_words(waiting: u64, sealed: u64, office_recovery: u64) -> String {
    let plural = |n: u64, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let mut out = Vec::new();
    if waiting > 0 {
        out.push(format!(
            "{} not sent yet stay on this PC and send after you sign in again.",
            plural(waiting, "change is", "changes are")
        ));
    }
    if sealed > 0 {
        out.push(format!(
            "{} stay encrypted on this PC and open again after you sign in.",
            plural(sealed, "saved copy", "saved copies")
        ));
    }
    if office_recovery > 0 {
        out.push(format!(
            "{} of unsaved Office work will be removed.",
            plural(office_recovery, "recovery copy", "recovery copies")
        ));
    }
    out.join(" ")
}

#[cfg(test)]
mod sign_out_counts_words_tests {
    use super::sign_out_counts_words;

    #[test]
    fn says_each_count_and_what_is_removed() {
        assert_eq!(sign_out_counts_words(0, 0, 0), "");
        let w = sign_out_counts_words(1, 2, 1);
        assert!(w.contains("1 change is not sent yet"), "{w}");
        assert!(w.contains("2 saved copies stay encrypted"), "{w}");
        assert!(w.contains("1 recovery copy of unsaved Office work will be removed"), "{w}");
    }
}
