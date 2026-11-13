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

use super::source::{AccountSource, Factor, FactorKind, PaneState, SecurityFacts, score};
use super::{Dialog, icons};

/// The score's line: larger than a list's head, smaller than the pane's title.
const SCORE_HEAD: TextStyle = TextStyle::new(20.0, 650.0).lh(24.0).tracking(-0.3);

/// What a part says when nobody read it.
pub(super) const NOT_KNOWN: &str = "Not known";
/// What a function says when this build has not bound it.
pub(super) const NOT_YET: &str = "Not available yet";

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

/// A row that opens `frame`. While nothing answers for that page it is the row alone, saying so: no press,
/// no chevron (never a row that leads to a page that is not there).
fn leads(list: usize, index: usize, it: Row, frame: &'static str, source: &dyn AccountSource, t: &Theme) -> El {
    if !answered(source, frame) {
        return set::item(list, index, &it.tail(Tail::Value(NOT_YET.into())), Vec::new(), t);
    }
    set::item(list, index, &it.link(true), Vec::new(), t).on_press(move |s: &mut State| super::go(s, frame))
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
    let ring = |fraction: f32, inside: String| {
        el().key("ring-box")
            .a11y_hidden()
            .size(56.0, 56.0)
            .center()
            .child(set::ring("ring", 56.0, 4.0, fraction, t.text, t).absolute(0.0, 0.0))
            .child(kit::label("ring-label", inside, set::RING_COUNT, t.text))
    };
    let words = |head: String, sub: String| {
        col()
            .key("score-words")
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
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
        .padding(18.0)
        .gap(12.0)
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, rune::Color::BLACK.alpha(0.04)));
    let line = row().key("score").w_full().center_y().gap(14.0);
    let Some(factors) = scored(facts) else {
        return card.child(line.child(ring(0.0, "?".into())).child(words(
            "Security score not known yet".into(),
            "Beebeeb can't say yet which steps are done. Nothing on this page is a guess.".into(),
        )));
    };
    let (ok, of) = score(factors);
    let says = match of - ok {
        0 => "Nothing to do. Checked each time you open this page.".to_string(),
        1 => "One thing left to do.".to_string(),
        n => format!("{n} things left to do."),
    };
    let mut card = card.child(
        line.child(ring(ok as f32 / of as f32, ok.to_string()))
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
            .gap(10.0)
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
                    &format!("btn-{i}"),
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
        } else {
            todo = todo.child(kit::label(
                format!("todo-{i}-value"),
                NOT_YET,
                set::SUB,
                t.text_tertiary,
            ));
        }
        card = card.child(todo);
    }
    card
}

/// The mark of a row whose state is good AND was read. Nothing else gets it.
fn checked(it: Row, good: bool) -> Row {
    if good { it.tail(Tail::Check) } else { it }
}

fn sign_in(facts: &SecurityFacts, source: &dyn AccountSource, t: &Theme) -> El {
    let password = Row::new(Lead::Icon(&icons::PADLOCK), "Password").sub(match &facts.password_changed {
        Some(when) => format!("Changed {when}"),
        None => "Last change not known".into(),
    });
    let password = checked(
        password,
        done(facts, FactorKind::StrongPassword) && done(facts, FactorKind::PasswordNotInBreach),
    );
    let two = Row::new(Lead::Icon(&icons::SHIELD), "Two-factor authentication").sub(
        match (facts.two_factor, facts.backup_codes) {
            (Some(true), Some((left, of))) => format!("On · {left} of {of} backup codes left"),
            (Some(true), None) => "On".into(),
            (Some(false), _) => "Off: your password alone signs you in".into(),
            (None, _) => NOT_KNOWN.into(),
        },
    );
    let two = checked(two, facts.two_factor == Some(true));
    let passkeys = Row::new(Lead::Icon(&icons::PASSKEY), "Passkeys").sub(match facts.passkeys {
        Some(0) => "None added".into(),
        Some(1) => "1 passkey".into(),
        Some(n) => format!("{n} passkeys"),
        None => NOT_KNOWN.into(),
    });
    let passkeys = checked(passkeys, facts.passkeys.is_some_and(|n| n > 0));
    lists::list(
        0,
        Some("Sign-in"),
        vec![
            leads(0, 0, password, "DW-A67", source, t),
            leads(0, 1, two, "DW-A71", source, t),
            leads(0, 2, passkeys, "DW-A73", source, t),
        ],
        None,
        t,
    )
}

/// App lock is Drive's own row (the board's `securityExtra` slot, DW-0716): its switch and delay are here,
/// not on a page behind it.
fn app_lock_row(facts: &SecurityFacts, t: &Theme) -> El {
    let it = Row::new(Lead::Icon(&icons::APP_LOCK), "Lock Beebeeb");
    let Some((on, after)) = &facts.app_lock else {
        return set::item(
            1,
            0,
            &it.sub("Windows Hello").tail(Tail::Value(NOT_YET.into())),
            Vec::new(),
            t,
        );
    };
    let on = *on;
    let mut controls = Vec::new();
    if on {
        controls.push(
            el().key("item-1-0-pop-button")
                .role("combobox")
                .label(format!("Locks again after, {after}"))
                .mr(8.0)
                .radius(6.0)
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
    controls.push(
        lists::switch("item-1-0", "Lock Beebeeb", on, t).on_press(move |s: &mut State| {
            if let Some(source) = s.account.source.as_mut() {
                source.set_app_lock(!on);
            }
        }),
    );
    set::item(
        1,
        0,
        &it.sub(if on {
            "Windows Hello · locks again after"
        } else {
            "Off: anyone using this PC can open Beebeeb"
        }),
        controls,
        t,
    )
}

fn devices(facts: &SecurityFacts, source: &dyn AccountSource, t: &Theme) -> El {
    let sessions = Row::new(Lead::Icon(&icons::DEVICE_PHONE), "Devices and sessions")
        .sub("Where your account is signed in")
        .tail(Tail::Value(match facts.sessions {
            Some(1) => "1 device".into(),
            Some(n) => format!("{n} devices"),
            None => NOT_KNOWN.into(),
        }));
    lists::list(
        1,
        Some("This PC and your devices"),
        vec![
            app_lock_row(facts, t),
            leads(1, 1, sessions, "DW-A55", source, t),
            leads(
                1,
                2,
                Row::new(Lead::Icon(&icons::SCAN), "Approve a sign-in").sub("Let another device sign in"),
                "DW-A12",
                source,
                t,
            ),
        ],
        None,
        t,
    )
}

/// The last sign-ins and security changes, then the way to all of them. Read as what happened, then when and
/// where: a country, never a city; never which files were opened.
fn activity(facts: &SecurityFacts, source: &dyn AccountSource, t: &Theme) -> El {
    let all = |index: usize, sub: &str| {
        leads(
            2,
            index,
            Row::new(Lead::Icon(&icons::ACTIVITY), "All sign-in activity").sub(sub),
            "DW-A77",
            source,
            t,
        )
    };
    let rows = match &facts.activity {
        None => vec![set::item(
            2,
            0,
            &Row::new(Lead::Icon(&icons::ACTIVITY), "Sign-in activity")
                .sub(NOT_KNOWN)
                .tail(Tail::Value(NOT_YET.into())),
            Vec::new(),
            t,
        )],
        Some(None) => vec![all(0, "Security logging is off: nothing is recorded")],
        Some(Some(events)) if events.is_empty() => vec![all(0, "Recording is on. Nothing recorded yet")],
        Some(Some(events)) => {
            let mut rows: Vec<El> = events
                .iter()
                .take(super::source::RECENT_EVENTS)
                .enumerate()
                .map(|(i, e)| {
                    set::item(
                        2,
                        i,
                        &Row::new(Lead::Icon(super::activity::event_icon(e.kind)), e.what.clone())
                            .sub_mono(e.when_where.clone())
                            .name(format!("{}, {}", e.what, e.when_where.replace(" · ", ", "))),
                        Vec::new(),
                        t,
                    )
                })
                .collect();
            let index = rows.len();
            rows.push(all(index, "Recording is on"));
            rows
        }
    };
    lists::list(2, Some("Recent sign-in activity"), rows, None, t)
}

/// The two destructive rows open a confirm over the page; neither touches other devices. A source that
/// cannot sign this app out gets the rows as text: nothing to press that would do nothing.
fn sign_out(facts: &SecurityFacts, t: &Theme) -> El {
    let able = facts.can_sign_out;
    let one = |index: usize, it: Row, dialog: Dialog| {
        if !able {
            return set::item(3, index, &it.tail(Tail::Value(NOT_YET.into())), Vec::new(), t);
        }
        set::item(3, index, &it.danger().link(false), Vec::new(), t)
            .on_press(move |s: &mut State| s.account.dialog = Some(dialog.clone()))
    };
    lists::list(
        3,
        Some("Sign out"),
        vec![
            one(
                0,
                Row::new(Lead::Icon(&icons::SIGN_OUT), "Sign out of Drive"),
                Dialog::SignOutApp,
            ),
            one(
                1,
                Row::new(Lead::Icon(&icons::SIGN_OUT), "Sign out of all Beebeeb apps").sub("On this PC"),
                Dialog::SignOutAllApps,
            ),
        ],
        Some("Signing out of Drive signs out Drive on this PC only. The other Beebeeb apps stay signed in."),
        t,
    )
}

fn delete(source: &dyn AccountSource, t: &Theme) -> El {
    let it = Row::new(Lead::Icon(&icons::TRASH), "Delete your Beebeeb account");
    // The red ink is for a row that deletes: one that leads nowhere yet is not drawn as a danger.
    let it = if answered(source, "DW-A58") { it.danger() } else { it };
    lists::list(
        4,
        Some("Delete"),
        vec![leads(4, 0, it, "DW-A58", source, t)],
        Some("Deletes every product's data after 72 hours to undo. After that it cannot be undone."),
        t,
    )
}

/// The page, and the confirm or the menu over it when one is open.
pub fn pane(source: &dyn AccountSource, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    let facts = source.security_facts();
    let over = match dialog {
        Some(Dialog::SignOutApp) if facts.can_sign_out => Some(sign_out_confirm(
            "Sign out of Drive?",
            "This signs out Drive on this PC only. Drive, VPN, Chat and Mail stay signed in. Your data stays encrypted on this PC; signing in again opens it.",
            "Sign out",
            false,
            t,
        )),
        Some(Dialog::SignOutAllApps) if facts.can_sign_out => Some(sign_out_confirm(
            "Sign out of all Beebeeb apps on this PC?",
            "Drive, Pass, VPN, Chat and Mail on this PC are signed out. Their data stays encrypted on this PC; signing in again opens it. Your other devices stay signed in.",
            "Sign out of all",
            true,
            t,
        )),
        Some(Dialog::LockDelay(hot)) => match &facts.app_lock {
            Some((true, after)) => Some(super::app_lock::menu(after, *hot, t)),
            _ => None,
        },
        _ => None,
    };
    let children = vec![
        set::heading("Sign-in and security", None, None, t),
        score_card(&facts, source, t),
        sign_in(&facts, source, t),
        devices(&facts, source, t),
        activity(&facts, source, t),
        sign_out(&facts, t),
        delete(source, t),
    ];
    (set::pane(children, t), over)
}

/// DW-A57 and DW-A57a: the scope sentence is read before the buttons; Enter is Cancel.
fn sign_out_confirm(title: &str, words: &str, action: &str, all: bool, t: &Theme) -> El {
    let foot = vec![
        (
            set::button("btn-1", action, None, Look::Danger, Size::Medium, false, t)
                .label(format!("{action}, destructive"))
                .on_press(move |s: &mut State| {
                    if let Some(source) = s.account.source.as_mut() {
                        if all {
                            source.sign_out_all_apps();
                        } else {
                            source.sign_out_this_app();
                        }
                    }
                    s.account.dialog = None;
                    // Sign in: with the Continue as card after one app (DW-A02), without it
                    // after all of them (DW-A03).
                    super::go(s, if all { "DW-A03" } else { "DW-A02" });
                }),
            false,
        ),
        (
            set::button("btn-2", "Cancel", None, Look::Plain, Size::Medium, false, t)
                .default_button()
                .on_press(|s: &mut State| s.account.dialog = None),
            true,
        ),
    ];
    set::dialog(title, 440.0, title, words, Vec::new(), foot, t).modal(false, |s: &mut State| s.account.dialog = None)
}
