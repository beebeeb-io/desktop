//! RENDER-ONLY PROTOTYPE (not for merge): three ways Settings > Security could read as one overview.
//! Fixture data only; rows link to the existing frames, nothing new is bound.
//!
//! - `A`: DW-A52 in place: the score on the pane, each row says its state.
//! - `B`: a separate overview in front of DW-A52: score, a grid of status cards, recent activity.
//! - `C`: one page instead of DW-A52 and DW-A84: score with what is left to do, a card per subject.

use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::account_set::{self as set, Lead, Look, Row, Size, Tail};
use crate::ui::au::Tone;
use crate::ui::kit;
use crate::ui::settings_list as lists;
use rune::{El, Icon, TextStyle, col, el, row};

use super::icons;
use super::source::{AccountSource, Event, Factor, score};

const BIG: TextStyle = TextStyle::new(20.0, 650.0).lh(24.0).tracking(-0.3);

fn card(key: &str, t: &Theme) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .padding(14.0)
        .gap(10.0)
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, rune::Color::BLACK.alpha(0.04)))
}

fn pill(on: bool, yes: &str, no: &str) -> Tail {
    Tail::Pill {
        label: if on { yes.into() } else { no.into() },
        tone: if on { Tone::Green } else { Tone::Neutral },
        dot: true,
    }
}

fn says(factors: &[Factor]) -> String {
    let (done, of) = score(factors);
    match of - done {
        0 => "Nothing to do. Checked each time you open this page.".into(),
        1 => "One thing left to do.".into(),
        n => format!("{n} things left to do."),
    }
}

/// The score: a ring with the count in it, the count in words, one sentence.
fn hero(factors: &[Factor], d: f32, big: bool, trailing: Option<El>, t: &Theme) -> El {
    let (done, of) = score(factors);
    let mut line = row()
        .key("score")
        .w_full()
        .center_y()
        .gap(14.0)
        .child(
            el().key("ring-box")
                .size(d, d)
                .center()
                .child(set::ring("ring", d, 4.0, done as f32 / of.max(1) as f32, t.text, t).absolute(0.0, 0.0))
                .child(kit::label("ring-label", done.to_string(), set::RING_COUNT, t.text)),
        )
        .child(
            col()
                .key("score-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(kit::label(
                    "ring-head",
                    format!("{done} of {of} security steps done"),
                    if big { BIG } else { type_::HEAD },
                    t.text,
                ))
                .child(kit::label("ring-sub", says(factors), set::SUB, t.text_secondary)),
        );
    if let Some(trailing) = trailing {
        line = line.child(trailing);
    }
    line
}

fn go_button(key: &str, label: &str, frame: &'static str, t: &Theme) -> El {
    set::button(key, label, None, Look::Plain, Size::Small, false, t).on_press(move |s: &mut State| super::go(s, frame))
}

fn opens(list: usize, index: usize, it: Row, frame: &'static str, t: &Theme) -> El {
    set::item(list, index, &it.link(true), Vec::new(), t).on_press(move |s: &mut State| super::go(s, frame))
}

fn recent(source: &dyn AccountSource) -> Option<Vec<Event>> {
    source
        .activity()
        .map(|periods| periods.into_iter().flat_map(|(_, e)| e).take(3).collect())
}

fn passkeys_say(n: usize) -> String {
    match n {
        0 => "None added".into(),
        1 => "1 passkey".into(),
        n => format!("{n} passkeys"),
    }
}

fn danger(list: usize, t: &Theme) -> El {
    lists::list(
        list,
        Some("Delete"),
        vec![opens(
            list,
            0,
            Row::new(Lead::Icon(&icons::TRASH), "Delete your Beebeeb account").danger(),
            "DW-A58",
            t,
        )],
        Some("Deletes every product's data after 72 hours to undo. After that it cannot be undone."),
        t,
    )
}

fn sign_out(list: usize, t: &Theme) -> El {
    lists::list(
        list,
        Some("Sign out"),
        vec![
            set::item(
                list,
                0,
                &Row::new(Lead::Icon(&icons::SIGN_OUT), "Sign out of Drive")
                    .danger()
                    .link(false),
                Vec::new(),
                t,
            ),
            set::item(
                list,
                1,
                &Row::new(Lead::Icon(&icons::SIGN_OUT), "Sign out of all Beebeeb apps")
                    .sub("On this PC")
                    .danger()
                    .link(false),
                Vec::new(),
                t,
            ),
        ],
        None,
        t,
    )
}

/// A: DW-A52 in place. Same lists, same order; the score is on the pane and every row says its state.
fn in_place(source: &dyn AccountSource, t: &Theme) -> El {
    let sec = source.security();
    let two = source.two_factor();
    let factors = source.factors();
    let last = recent(source).and_then(|e| e.into_iter().next());
    let score_card = card("card-0", t).child(hero(
        &factors,
        44.0,
        false,
        Some(go_button("btn-0", "Open checkup", "DW-A84", t)),
        t,
    ));
    let app_lock = lists::list(
        0,
        Some("This PC"),
        vec![set::item(
            0,
            0,
            &Row::new(Lead::Icon(&icons::APP_LOCK), "Lock Beebeeb").sub(if sec.app_lock {
                "Windows Hello · locks again after"
            } else {
                "Off: anyone using this PC can open Beebeeb"
            }),
            vec![
                lists::pop_button("item-0-0", &sec.lock_after, t).mr(8.0),
                lists::switch("item-0-0", "Lock Beebeeb", sec.app_lock, t),
            ],
            t,
        )],
        None,
        t,
    );
    let sign_in = lists::list(
        1,
        Some("Sign-in"),
        vec![
            opens(
                1,
                0,
                Row::new(Lead::Icon(&icons::PADLOCK), "Password").sub(format!("Changed {}", sec.password_changed)),
                "DW-A67",
                t,
            ),
            opens(
                1,
                1,
                Row::new(Lead::Icon(&icons::SHIELD), "Two-factor authentication")
                    .sub(if sec.two_factor {
                        format!(
                            "Authenticator app · {} of {} backup codes left",
                            two.codes_left, two.codes_total
                        )
                    } else {
                        "A code from your phone when you sign in".into()
                    })
                    .tail(pill(sec.two_factor, "On", "Off")),
                "DW-A71",
                t,
            ),
            opens(
                1,
                2,
                Row::new(Lead::Icon(&icons::PASSKEY), "Passkeys")
                    .sub("Sign in without typing your password")
                    .tail(Tail::Value(passkeys_say(sec.passkeys))),
                "DW-A73",
                t,
            ),
            opens(
                1,
                3,
                Row::new(Lead::Icon(&icons::ACTIVITY), "Sign-in activity")
                    .sub(match &last {
                        Some(e) => format!("Last: {}", e.what),
                        None => "Nothing is recorded".into(),
                    })
                    .tail(pill(sec.sign_in_activity, "Recording", "Off")),
                "DW-A77",
                t,
            ),
        ],
        None,
        t,
    );
    let devices = lists::list(
        2,
        Some("Devices"),
        vec![
            opens(
                2,
                0,
                Row::new(Lead::Icon(&icons::DEVICE_PHONE), "Devices and sessions")
                    .sub("Where your account is signed in")
                    .tail(Tail::Value(format!("{} devices", sec.sessions))),
                "DW-A55",
                t,
            ),
            opens(
                2,
                1,
                Row::new(Lead::Icon(&icons::SCAN), "Approve a sign-in").sub("Let another device sign in"),
                "DW-A12",
                t,
            ),
        ],
        None,
        t,
    );
    set::pane(
        vec![
            set::heading("Sign-in and security", None, None, t),
            score_card,
            app_lock,
            sign_in,
            devices,
            sign_out(3, t),
            danger(4, t),
        ],
        t,
    )
}

/// One status card of B's grid.
fn status_card(
    key: &str,
    icon: &'static Icon,
    title: &str,
    state: &str,
    detail: &str,
    good: Option<bool>,
    frame: &'static str,
    t: &Theme,
) -> El {
    let mut head = row()
        .key(format!("{key}-head"))
        .w_full()
        .center_y()
        .gap(8.0)
        .child(set::tile(&format!("{key}-tile"), icon, 28.0, 8.0, Tone::Neutral, t))
        .child(
            kit::label(format!("{key}-title"), title, type_::HEAD, t.text)
                .grow(1.0)
                .truncate(),
        );
    if let Some(good) = good {
        head = head.child(set::pill(
            &format!("{key}-pill"),
            state,
            if good { Tone::Green } else { Tone::Neutral },
            true,
            t,
        ));
    } else {
        head = head.child(kit::label(format!("{key}-value"), state, set::SUB, t.text_tertiary));
    }
    card(key, t)
        .w(253.0)
        .gap(8.0)
        .role("link")
        .label(format!("{title}, {state}, {detail}"))
        .focusable(true)
        .hover(t.hover)
        .on_press(move |s: &mut State| super::go(s, frame))
        .child(head)
        .child(kit::label(format!("{key}-sub"), detail, set::SUB, t.text_tertiary).truncate())
}

fn activity_list(list: usize, source: &dyn AccountSource, t: &Theme) -> El {
    let rows: Vec<El> = match recent(source) {
        Some(events) => events
            .iter()
            .enumerate()
            .map(|(i, e)| {
                set::item(
                    list,
                    i,
                    &Row::new(Lead::Icon(&icons::DEVICE_LAPTOP), e.what.clone()).sub_mono(e.when_where.clone()),
                    Vec::new(),
                    t,
                )
            })
            .chain(std::iter::once(opens(
                list,
                3,
                Row::new(Lead::Icon(&icons::ACTIVITY), "All sign-in activity"),
                "DW-A77",
                t,
            )))
            .collect(),
        None => vec![opens(
            list,
            0,
            Row::new(Lead::Icon(&icons::ACTIVITY), "Security logging is off").sub("Nothing is recorded"),
            "DW-A77",
            t,
        )],
    };
    lists::list(list, Some("Recent sign-in activity"), rows, None, t)
}

/// B: a separate overview. DW-A52 stays as it is, one press further ("All security settings").
fn separate(source: &dyn AccountSource, t: &Theme) -> El {
    let sec = source.security();
    let two = source.two_factor();
    let factors = source.factors();
    let pair = |a: El, b: El, key: &str| row().key(key.to_string()).w_full().gap(14.0).child(a).child(b);
    let grid = col()
        .key("grid")
        .w_full()
        .gap(14.0)
        .child(pair(
            status_card(
                "st-0",
                &icons::PADLOCK,
                "Password",
                "Change",
                &format!("Changed {}", sec.password_changed),
                None,
                "DW-A67",
                t,
            ),
            status_card(
                "st-1",
                &icons::SHIELD,
                "Two-factor",
                if sec.two_factor { "On" } else { "Off" },
                &format!("{} of {} backup codes left", two.codes_left, two.codes_total),
                Some(sec.two_factor),
                "DW-A71",
                t,
            ),
            "grid-0",
        ))
        .child(pair(
            status_card(
                "st-2",
                &icons::PASSKEY,
                "Passkeys",
                &passkeys_say(sec.passkeys),
                "Sign in without your password",
                None,
                "DW-A73",
                t,
            ),
            status_card(
                "st-3",
                &icons::APP_LOCK,
                "This PC",
                if sec.app_lock { "Locked" } else { "Not locked" },
                &format!("Windows Hello · after {}", sec.lock_after),
                Some(sec.app_lock),
                "DW-A52",
                t,
            ),
            "grid-1",
        ))
        .child(pair(
            status_card(
                "st-4",
                &icons::DEVICE_PHONE,
                "Devices",
                &format!("{} signed in", sec.sessions),
                "See and end every session",
                None,
                "DW-A55",
                t,
            ),
            status_card(
                "st-5",
                &icons::ACTIVITY,
                "Activity log",
                if sec.sign_in_activity { "Recording" } else { "Off" },
                "Sign-ins and security changes",
                Some(sec.sign_in_activity),
                "DW-A77",
                t,
            ),
            "grid-2",
        ));
    set::pane(
        vec![
            set::heading("Security", None, None, t),
            card("card-0", t).padding(18.0).child(hero(
                &factors,
                56.0,
                true,
                Some(go_button("btn-0", "Review", "DW-A84", t)),
                t,
            )),
            grid,
            activity_list(0, source, t),
            lists::list(
                1,
                None,
                vec![
                    opens(
                        1,
                        0,
                        Row::new(Lead::Icon(&icons::SHIELD), "All sign-in and security settings")
                            .sub("App lock, approve a sign-in, sign out"),
                        "DW-A52",
                        t,
                    ),
                    opens(
                        1,
                        1,
                        Row::new(Lead::Icon(&icons::TRASH), "Delete your Beebeeb account").danger(),
                        "DW-A58",
                        t,
                    ),
                ],
                None,
                t,
            ),
        ],
        t,
    )
}

/// C: one page instead of DW-A52 and DW-A84. The score card holds what is left to do, with its button;
/// the lists follow with their state; recent activity is on the page.
fn merged(source: &dyn AccountSource, t: &Theme) -> El {
    let sec = source.security();
    let two = source.two_factor();
    let factors = source.factors();
    let mut top = card("card-0", t)
        .padding(18.0)
        .gap(12.0)
        .child(hero(&factors, 56.0, true, None, t));
    for (i, f) in factors.iter().filter(|f| !f.done).enumerate() {
        let frame = f.kind.fix_frame();
        top = top.child(
            row()
                .key(format!("todo-{i}"))
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
                    &icons::PASSKEY,
                    28.0,
                    8.0,
                    Tone::Neutral,
                    t,
                ))
                .child(
                    col()
                        .key(format!("todo-{i}-text"))
                        .grow(1.0)
                        .gap(1.0)
                        .child(kit::label(
                            format!("todo-{i}-title"),
                            f.says.clone(),
                            type_::ITEM_TITLE,
                            t.text,
                        ))
                        .child(kit::label(
                            format!("todo-{i}-sub"),
                            "Sign in with Windows Hello or your phone instead of typing your password.",
                            set::SUB,
                            t.text_tertiary,
                        )),
                )
                .child(
                    set::button(
                        &format!("btn-{i}"),
                        f.kind.fix_label(),
                        None,
                        Look::Primary,
                        Size::Small,
                        false,
                        t,
                    )
                    .on_press(move |s: &mut State| super::go(s, frame)),
                ),
        );
    }
    let sign_in = lists::list(
        0,
        Some("Sign-in"),
        vec![
            opens(
                0,
                0,
                Row::new(Lead::Icon(&icons::PADLOCK), "Password")
                    .sub(format!(
                        "Changed {} · strong, not in a known breach",
                        sec.password_changed
                    ))
                    .tail(Tail::Check),
                "DW-A67",
                t,
            ),
            opens(
                0,
                1,
                Row::new(Lead::Icon(&icons::SHIELD), "Two-factor authentication")
                    .sub(format!(
                        "Authenticator app · {} of {} backup codes left",
                        two.codes_left, two.codes_total
                    ))
                    .tail(if sec.two_factor {
                        Tail::Check
                    } else {
                        pill(false, "On", "Off")
                    }),
                "DW-A71",
                t,
            ),
            opens(
                0,
                2,
                Row::new(Lead::Icon(&icons::PASSKEY), "Passkeys")
                    .sub("Sign in without typing your password")
                    .tail(if sec.passkeys > 0 {
                        Tail::Check
                    } else {
                        Tail::Value("None added".into())
                    }),
                "DW-A73",
                t,
            ),
        ],
        None,
        t,
    );
    let this_pc = lists::list(
        1,
        Some("This PC and your devices"),
        vec![
            set::item(
                1,
                0,
                &Row::new(Lead::Icon(&icons::APP_LOCK), "Lock Beebeeb").sub("Windows Hello · locks again after"),
                vec![
                    lists::pop_button("item-1-0", &sec.lock_after, t).mr(8.0),
                    lists::switch("item-1-0", "Lock Beebeeb", sec.app_lock, t),
                ],
                t,
            ),
            opens(
                1,
                1,
                Row::new(Lead::Icon(&icons::DEVICE_PHONE), "Devices and sessions")
                    .sub("Where your account is signed in")
                    .tail(Tail::Value(format!("{} devices", sec.sessions))),
                "DW-A55",
                t,
            ),
            opens(
                1,
                2,
                Row::new(Lead::Icon(&icons::SCAN), "Approve a sign-in").sub("Let another device sign in"),
                "DW-A12",
                t,
            ),
        ],
        None,
        t,
    );
    set::pane(
        vec![
            set::heading("Sign-in and security", None, None, t),
            top,
            sign_in,
            this_pc,
            activity_list(2, source, t),
            sign_out(3, t),
            danger(4, t),
        ],
        t,
    )
}

pub fn pane(source: &dyn AccountSource, variant: u8, t: &Theme) -> El {
    match variant {
        0 => in_place(source, t),
        1 => separate(source, t),
        _ => merged(source, t),
    }
}
