//! Recent notifications (DW-0791): the account's recent alerts inside the app, for when a
//! notification was missed or dismissed; and nothing to show (DW-0791b).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use crate::ui::settings_list::{self as lists, Item, Role};
use crate::ui::settings_notice::{self as notices, Tone};
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
            children.push(notices::note(
                0,
                Tone::Neutral,
                &ic::INBOX,
                "Loading your notifications\u{2026}",
                "Asking Beebeeb for the list.",
                t,
            ));
            return (shell::pane(children, t), None);
        }
        ReadState::Failed => {
            let again = super::notifications::try_again(0, t);
            children.push(notices::problem(
                0,
                0,
                "Couldn't load your notifications",
                "Nothing was changed. There may be alerts that aren't shown here.",
                Some(again),
                t,
            ));
            return (shell::pane(children, t), None);
        }
    }
    if days.is_empty() {
        children.push(notices::empty(
            0,
            0,
            &ic::INBOX,
            "No notifications",
            "When something needs your attention, a new device sign-in, a share or a storage warning, it shows up here.",
            t,
        ));
        children.extend(unconfirmed(ui, t));
        return (shell::pane(children, t), None);
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
                    trailing = trailing.child(notices::pill(pills, "New", t));
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
        children.push(lists::list(l, Some(day), rows, None, t));
    }
    children.extend(unconfirmed(ui, t));
    (shell::pane(children, t), None)
}

/// Read, but a later change could not be confirmed: the list stays, and below it (so no row moves under the
/// pointer when this comes or goes) the page says it may be out of date, with "Try again".
fn unconfirmed(ui: &SettingsUi, t: &Theme) -> Option<El> {
    let (strong, words) = super::notifications::UNCONFIRMED;
    ui.source.notifications_unconfirmed().then(|| {
        notices::note_with(
            0,
            Tone::Warn,
            &ic::WARN_CIRCLE,
            strong,
            words,
            Some(super::notifications::try_again(0, t)),
            t,
        )
    })
}
