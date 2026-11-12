//! Account (DW-A51): who is signed in, what became of the recovery phrase, which Beebeeb
//! products this account uses, and Drive's own part of the page (the plan).

use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::account_set::{self as set, Lead, Row, Tail};
use crate::ui::au::Tone;
use crate::ui::kit;
use crate::ui::settings_list as lists;
use rune::{El, col, row};

use super::icons;
use super::source::{AccountSource, Plan, ProductKind, ProductState};

/// The hue of the account's avatar tint (`.kit-avatar.hue-a`).
const AVATAR_HUE: f32 = 250.0;

pub fn pane(source: &dyn AccountSource, t: &Theme) -> El {
    let who = source.identity();
    let identity = lists::list(
        0,
        None,
        vec![
            set::item(
                0,
                0,
                &Row::new(
                    Lead::Avatar {
                        initials: who.initials.clone(),
                        hue: AVATAR_HUE,
                    },
                    who.name.clone(),
                )
                .name(format!("Name, {}", who.name))
                .link(true),
                Vec::new(),
                t,
            )
            .on_press(|s: &mut State| super::go(s, "DW-A64")),
            set::item(
                0,
                1,
                &Row::new(Lead::Icon(&icons::MAIL), who.email.clone())
                    .name(format!("Sign-in email, {}", who.email))
                    .link(true),
                Vec::new(),
                t,
            )
            .on_press(|s: &mut State| super::go(s, "DW-A65")),
        ],
        None,
        t,
    );

    // The phrase was shown once, at sign-up, and never again: this row only says so. It is
    // text, not a button: no reveal, no re-authentication, no regenerate.
    let recovery = lists::list(
        1,
        Some("Recovery"),
        vec![set::item(
            1,
            0,
            &Row::new(Lead::Tile(&icons::KEY, Tone::Neutral), "Recovery phrase")
                .sub("Shown once at sign-up")
                .name("Recovery phrase, shown once at sign-up"),
            Vec::new(),
            t,
        )],
        Some("We can't show it again. Keep your 12 words on paper or in a password manager."),
        t,
    );

    let products = source
        .products()
        .into_iter()
        .enumerate()
        .map(|(i, (kind, state))| {
            let (icon, tone): (&'static rune::Icon, Tone) = match kind {
                ProductKind::Drive => (&icons::FOLDER, Tone::Ink),
                ProductKind::Pass => (&icons::KEY, Tone::Neutral),
                ProductKind::Vpn => (&icons::GLOBE, Tone::Neutral),
                ProductKind::Chat => (&icons::CHAT, Tone::Neutral),
                ProductKind::Mail => (&icons::MAIL, Tone::Neutral),
            };
            // The state is text, not only colour.
            let tail = match state {
                ProductState::ThisApp => Tail::Pill {
                    label: state.label().into(),
                    tone: Tone::Neutral,
                    dot: false,
                },
                ProductState::SignedIn => Tail::Pill {
                    label: state.label().into(),
                    tone: Tone::Green,
                    dot: true,
                },
                ProductState::NotSignedIn => Tail::Value(state.label().into()),
            };
            set::item(
                2,
                i,
                &Row::new(Lead::Tile(icon, tone), kind.label()).tail(tail),
                Vec::new(),
                t,
            )
        })
        .collect();
    let uses = lists::list(
        2,
        Some("This account uses"),
        products,
        Some("State on this PC. One account and one phrase; each product keeps its own keys."),
        t,
    );

    let mut children = vec![set::heading("Account", None, None, t), identity, recovery, uses];
    if let Some(plan) = source.plan() {
        children.push(plan_card(&plan, t));
    }
    set::pane(children, t)
}

/// Drive's own part of the Account page: the plan, read only. Opens the plan pane (DW-0708).
fn plan_card(plan: &Plan, t: &Theme) -> El {
    row()
        .key("card-0")
        .role("link")
        .label(format!(
            "{}, {} {}. Manage on beebeeb.io",
            plan.title, plan.used, plan.rest
        ))
        .w_full()
        .center_y()
        .gap(12.0)
        .padding(12.0)
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::drop(1.0, 2.0, rune::Color::BLACK.alpha(0.04)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(|s: &mut State| super::go(s, "DW-0708"))
        .child(set::tile("card-0-tile", &icons::DATABASE, 34.0, 10.0, Tone::Neutral, t))
        .child(
            col()
                .key("card-0-words")
                .grow(1.0)
                .min_w(0.0)
                .child(kit::label("card-0-head", plan.title.clone(), type_::HEAD, t.text))
                .child(
                    row()
                        .key("card-0-line")
                        .gap(3.4)
                        .child(kit::label(
                            "card-0-mono",
                            plan.used.clone(),
                            set::SUB_MONO,
                            t.text_tertiary,
                        ))
                        .child(kit::label("card-0-sub", plan.rest.clone(), set::SUB, t.text_tertiary)),
                ),
        )
        .child(kit::label(
            "card-0-link",
            "Manage on beebeeb.io",
            set::SUB,
            t.text_tertiary,
        ))
        .child(kit::glyph("card-0-link-icon", &icons::EXTERNAL, 13.0, t.text))
}
