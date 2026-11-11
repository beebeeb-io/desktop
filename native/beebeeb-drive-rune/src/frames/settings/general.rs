//! General (DW-0701), with a new version available (DW-0781), and the Theme menu open (DW-0787).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::settings_shell as shell;
use rune::El;

use super::source::{ThemeChoice, Update};
use super::{Intent, Menu, SettingsUi};

/// Where the Theme menu drops, in the window's body: over its pop-up button, the board's place.
const THEME_MENU_AT: (f32, f32) = (700.0, 86.0);

/// The pane, and the menu over it when one is open.
pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let g = ui.source.general();
    let menu_open = matches!(ui.menu, Some(Menu::Theme { .. }));

    let appearance = lists::list(
        0,
        Some("Appearance"),
        vec![
            lists::item(
                0,
                0,
                &Item::new(Some(&ic::THEME), "Theme")
                    .sub(g.theme.says())
                    .trailing(Trailing::Pop {
                        choice: g.theme.label().into(),
                        open: menu_open,
                    })
                    .role(Role::Button),
                t,
            )
            .on_press(|s: &mut State| s.settings.toggle_theme_menu()),
        ],
        None,
        t,
    );

    let help = lists::list(
        1,
        Some("Help"),
        vec![
            lists::item(1, 0, &Item::new(Some(&ic::INFO), "Show the guide again").opens(), t)
                .on_press(|s: &mut State| s.settings.go("DW-0380")),
            lists::item(
                1,
                1,
                &Item::new(Some(&ic::CHAT), "Help and report a problem").opens(),
                t,
            )
            .on_press(|s: &mut State| s.settings.go("DW-0782")),
            lists::item(
                1,
                2,
                &Item::new(Some(&ic::ACTIVITY), "Diagnostics")
                    .sub("App activity and a support bundle")
                    .opens(),
                t,
            )
            .on_press(|s: &mut State| s.settings.go("DW-0790")),
        ],
        None,
        t,
    );

    let privacy = lists::list(
        2,
        Some("Privacy and data"),
        vec![
            lists::item(2, 0, &Item::new(Some(&ic::SHIELD), "Privacy").opens(), t)
                .on_press(|s: &mut State| s.settings.ask(Intent::Open("privacy"))),
            lists::item(2, 1, &Item::new(Some(&ic::DOWNLOAD_TRAY), "Export my data").opens(), t)
                .on_press(|s: &mut State| s.settings.ask(Intent::Open("export-my-data"))),
            lists::item(
                2,
                2,
                &Item::new(Some(&ic::GLOBE), "Storage location")
                    .trailing(Trailing::Value(g.storage_location.clone()))
                    .opens(),
                t,
            )
            .on_press(|s: &mut State| s.settings.go("DW-0788")),
        ],
        None,
        t,
    );

    let version = match &g.update {
        Update::Current => format!("Version {}", g.version),
        Update::Available { .. } => format!("{} · update available", g.version),
    };
    let about = lists::list(
        3,
        Some("About"),
        vec![
            lists::item(
                3,
                0,
                &Item::new(Some(&ic::DOCUMENT), "Terms and privacy policy")
                    .trailing(Trailing::External)
                    .role(Role::Link),
                t,
            )
            .on_press(|s: &mut State| s.settings.ask(Intent::Web("terms-and-privacy-policy"))),
            lists::item(
                3,
                1,
                &Item::new(Some(&ic::LAPTOP), "About Beebeeb")
                    .trailing(Trailing::Value(version))
                    .opens(),
                t,
            )
            .on_press(|s: &mut State| s.settings.go("DW-0751")),
        ],
        None,
        t,
    );

    let mut children = vec![shell::heading("General", None, t), appearance];
    if let Update::Available { version } = &g.update {
        // The one primary action of this view: Amber.
        let action = controls::button(0, "Restart to update", None, Emphasis::Primary, t)
            .on_press(|s: &mut State| s.settings.ask(Intent::DownloadUpdate));
        children.push(controls::banner(
            0,
            &ic::DOWNLOAD_TRAY,
            &format!("Version {version} is available"),
            "Download it, then open it to install. Beebeeb never installs an update by itself.",
            Some(action),
            t,
        ));
    }
    children.extend([help, privacy, about]);

    let menu = match ui.menu {
        Some(Menu::Theme { hot }) => {
            let items = ThemeChoice::ALL
                .iter()
                .enumerate()
                .map(|(i, choice)| {
                    let choice = *choice;
                    controls::menu_item(0, i, choice.label(), choice == g.theme, i == hot, t)
                        .on_press(move |s: &mut State| s.settings.choose_theme(choice))
                })
                .collect();
            Some(
                controls::menu(0, "Theme", 130.0, items, t)
                    .absolute(THEME_MENU_AT.0, THEME_MENU_AT.1)
                    .modal(true, |s: &mut State| s.settings.menu = None),
            )
        }
        None => None,
    };
    (shell::pane(children, t), menu)
}
