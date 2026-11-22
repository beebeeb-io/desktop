//! Recent notifications (DW-0791): the account's recent alerts inside the app, for when a
//! notification was missed or dismissed; and nothing to show (DW-0791b).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use crate::ui::settings_list::{self as lists, Item, Role};
use crate::ui::settings_notice as notices;
use crate::ui::view_state::{ViewState, try_again, view_state};
use crate::ui::{kit, settings_shell as shell};
use rune::tok::Family;
use rune::{El, row};

use super::SettingsUi;
use super::source::{AlertKind, ReadState};

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let days = ui.source.recent_alerts();
    let mut children = vec![
        shell::crumb("Notifications", t, |s: &mut State| super::go(s, "DW-0750")),
        shell::heading("Recent notifications", Some("Recent alerts from your account."), t),
    ];
    // Not read: an empty list would be a statement about the account that nobody checked.
    match ui.source.recent_state() {
        ReadState::Ready => {}
        ReadState::Loading => {
            let state = ViewState::Loading {
                words: "Loading your notifications\u{2026}",
            };
            children.push(view_state(state, t));
            return (shell::pane(children, t), super::notifications::said_over(ui, None, t));
        }
        ReadState::Failed => {
            let retry = try_again("recent-failed", t).on_press(super::notifications::ask_again);
            let state = ViewState::Failed {
                key: "recent-failed",
                title: "Couldn't load your notifications",
                // Only what holds in every way this is reached (a mark may have been taken for some of them).
                why: "There may be alerts that aren't shown here.",
                retry: Some(retry),
            };
            children.push(view_state(state, t));
            return (shell::pane(children, t), super::notifications::said_over(ui, None, t));
        }
    }
    if days.is_empty() {
        let state = ViewState::Empty {
            key: "recent-empty",
            icon: &ic::INBOX,
            title: "No notifications",
            body: "When something needs your attention, a new device sign-in, a share or a storage warning, it shows up here.",
            action: None,
        };
        children.push(view_state(state, t));
        return done(ui, children, t);
    }
    let mut pills = 0;
    for (l, (day, alerts)) in days.iter().enumerate() {
        let rows = alerts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let icon = match a.kind {
                    AlertKind::SignIn => &ic::LAPTOP,
                    AlertKind::Share => &ic::PEOPLE,
                    AlertKind::Request => &ic::FOLDER,
                    AlertKind::Security => &ic::SHIELD,
                };
                let mut trailing = row().key(format!("item-{l}-{i}-when")).center_y().gap(8.0);
                let mut name = a.title.clone();
                if a.new {
                    trailing = trailing.child(crate::ui::pill::pill(&format!("pill-{}", pills), "New", crate::ui::pill::Tone::Neutral, true, t));
                    pills += 1;
                    name = format!("New. {name}");
                }
                trailing = trailing.child(kit::label(
                    format!("item-{l}-{i}-time"),
                    a.time.clone(),
                    type_::CAPTION.family(Family::Mono),
                    t.text_tertiary,
                ));
                let leads_to = super::frame_of(a.leads_to);
                lists::item_with(
                    l,
                    i,
                    &Item::new(Some(icon), a.title.clone())
                        .sub(a.says.clone())
                        .role(Role::Link),
                    vec![trailing],
                    t,
                )
                .label(format!("{name}. {} {day}, {}", a.says, a.time))
                .on_press(move |s: &mut State| super::go(s, leads_to))
            })
            .collect();
        children.push(lists::roving_list(l, Some(day), rows, None, t));
    }
    done(ui, children, t)
}

/// Read, but a later change could not be confirmed: the list stays, and over the pane (so no row moves under
/// the pointer when this comes or goes, and it is in view wherever the list is scrolled) the page says it may be
/// out of date, with "Try again". The pane's column and what is said over it.
fn done(ui: &SettingsUi, mut children: Vec<El>, t: &Theme) -> (El, Option<El>) {
    let standing = ui
        .source
        .notifications_unconfirmed()
        .then_some(super::notifications::UNCONFIRMED);
    if standing.is_some() {
        children.push(notices::toast_room(1));
    }
    (
        shell::pane(children, t),
        super::notifications::said_over(ui, standing, t),
    )
}
