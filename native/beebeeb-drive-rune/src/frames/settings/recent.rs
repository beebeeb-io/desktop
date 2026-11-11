//! Recent notifications (DW-0791): the account's recent alerts inside the app, for when a
//! notification was missed or dismissed; and nothing to show (DW-0791b).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use crate::ui::settings_list::{self as lists, Item, Role};
use crate::ui::settings_notice as notices;
use crate::ui::{kit, settings_shell as shell};
use rune::tok::Family;
use rune::{El, row};

use super::SettingsUi;
use super::source::AlertKind;

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let days = ui.source.recent_alerts();
    let mut children = vec![
        shell::crumb("Notifications", t).on_press(|s: &mut State| super::go(s, "DW-0750")),
        shell::heading("Recent notifications", Some("Recent alerts from your account."), t),
    ];
    if days.is_empty() {
        children.push(notices::empty(
            0,
            0,
            &ic::INBOX,
            "No notifications",
            "When something needs your attention, a new device sign-in, a share or a storage warning, it shows up here.",
            t,
        ));
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
                let leads_to = a.leads_to;
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
    (shell::pane(children, t), None)
}
