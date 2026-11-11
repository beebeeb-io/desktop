//! Storage location (DW-0788): where new files are stored, inside the EU. A country chosen
//! (DW-0788b), a chosen country that cannot take files (DW-0788c), a list that could not
//! load (DW-0788d). Countries only: never a city, never a provider.

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::{detail_pane, settings_shell as shell};
use rune::El;

use super::SettingsUi;

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let mut children = vec![
        shell::crumb("General", t).on_press(|s: &mut State| s.settings.go("DW-0701")),
        shell::heading(
            "Storage location",
            Some("Where Beebeeb stores the files you add from now on. Every location is in the EU."),
            t,
        ),
    ];
    let Some(l) = ui.source.locations() else {
        let again = controls::button(0, "Try again", None, Emphasis::Plain, t)
            .on_press(|s: &mut State| s.settings.source.reload_locations());
        children.push(notices::problem(
            0,
            0,
            "Couldn't load storage locations",
            "Your files stay where they are. New files keep going to the location you chose before.",
            Some(again),
            t,
        ));
        return (shell::pane(children, t), None);
    };
    let chosen = l.chosen().clone();

    if l.unavailable {
        let rest = if l.only_here {
            "Uploads wait until it is."
        } else {
            "New files go wherever Beebeeb places them by default."
        };
        children.push(notices::note(
            0,
            Tone::Warn,
            &ic::WARN_CIRCLE,
            "Your chosen location isn't available right now.",
            rest,
            t,
        ));
    }

    let rows = l
        .choices
        .iter()
        .enumerate()
        .map(|(i, place)| {
            let icon = if place.default { &ic::GLOBE } else { &ic::SERVER };
            let mut it = Item::new(Some(icon), place.name.clone());
            if let Some(says) = &place.says {
                it = it.sub(says.clone());
            }
            if i > 0 {
                it = it.ruled();
            }
            if !place.live {
                // Not live: said so, and not a choice.
                return lists::item(0, i, &it.trailing(Trailing::Value("Coming soon".into())), t).disabled(true);
            }
            if i == l.chosen {
                it = it.trailing(Trailing::Check);
            }
            lists::item(0, i, &it.role(Role::Radio(i == l.chosen)), t)
                .on_press(move |s: &mut State| s.settings.choose_location(i))
        })
        .collect();
    children.push(lists::list(
        0,
        Some("Store new files in"),
        rows,
        Some("Existing files stay where they are. Only new files use the location you choose."),
        t,
    ));

    if !chosen.default {
        let title = format!("Only store in {}", chosen.name);
        let on = l.only_here;
        let switch = detail_pane::switch("item-1-0", &title, on, t)
            .on_press(move |s: &mut State| s.settings.source.set_only_here(!on));
        children.push(lists::list(
            1,
            Some(&format!("If {} can't take a new file", chosen.name)),
            vec![lists::item_with(
                1,
                0,
                &Item::new(Some(&ic::PADLOCK), title)
                    .sub("Off: it goes to another EU country. On: the upload stops and waits."),
                vec![switch],
                t,
            )],
            None,
            t,
        ));
    }

    let toast = ui.toast.map(|words| notices::toast(0, words, t));
    (shell::pane(children, t), toast)
}
