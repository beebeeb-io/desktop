//! General (DW-0701), with a new version available (DW-0781), and the Theme menu open (DW-0787).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::settings_shell as shell;
use rune::El;

use super::source::Update;
use super::{Dialog, Intent, MenuKind, SettingsUi};
use crate::ui::settings_dialog::{self as dialogs, Weight};

/// Where the Theme menu drops, from its row's top-left corner: over its pop-up button, the
/// board's place (DW-0787: the menu at 700, 118 over the row at 254, 115).
const THEME_MENU_AT: (f32, f32) = (446.0, 3.0);

/// The pane, and the menu over it when one is open.
pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let g = ui.source.general();
    let menu_open = ui.menu.is_some_and(|m| m.kind == MenuKind::Theme);

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
                        menu_at: THEME_MENU_AT,
                    })
                    .role(Role::PopUp),
                t,
            )
            .on_press(|s: &mut State| s.settings.toggle_menu(MenuKind::Theme)),
        ],
        None,
        t,
    );

    let help = lists::list(
        1,
        Some("Help"),
        vec![
            lists::item(1, 0, &Item::new(Some(&ic::INFO), "Show the guide again").opens(), t)
                .on_press(|s: &mut State| super::go(s, "DW-0380")),
            lists::item(
                1,
                1,
                &Item::new(Some(&ic::CHAT), "Help and report a problem").opens(),
                t,
            )
            .on_press(|s: &mut State| super::go(s, "DW-0782")),
            lists::item(
                1,
                2,
                &Item::new(Some(&ic::ACTIVITY), "Diagnostics")
                    .sub("App activity and a support bundle")
                    .opens(),
                t,
            )
            .on_press(|s: &mut State| super::go(s, "DW-0790")),
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
            lists::item(
                2,
                1,
                &Item::new(Some(&ic::DOWNLOAD_TRAY), "Export my data").opens().ruled(),
                t,
            )
            .on_press(|s: &mut State| s.settings.ask(Intent::Open("export-my-data"))),
            lists::item(
                2,
                2,
                &Item::new(Some(&ic::GLOBE), "Storage location")
                    .trailing(Trailing::Value(g.storage_location.clone()))
                    .opens(),
                t,
            )
            .on_press(|s: &mut State| super::go(s, "DW-0788")),
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
            .on_press(|s: &mut State| super::go(s, "DW-0751")),
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

    let menu = super::menu_overlay(ui, MenuKind::Theme, t);
    let overlay = match ui.dialog {
        Some(Dialog::ReportProblem) => Some(report_problem(t)),
        _ => menu,
    };
    (shell::pane(children, t), overlay)
}

/// Report a problem (DW-0782): what the support bundle holds and leaves out, before it is
/// saved; the person sends the email themselves.
fn report_problem(t: &Theme) -> El {
    const WORDS: &str = "Beebeeb saves a support bundle on this PC and opens an email to support@beebeeb.io in your email app. Read the file, then attach it and send the email yourself.";
    const ADDRESS: &str = "support@beebeeb.io";
    let at = WORDS.find(ADDRESS).unwrap_or(0);
    let plain = |l: usize, i: usize, icon: &'static rune::Icon, title: &str, ruled: bool| {
        let mut it = Item::new(Some(icon), title);
        if ruled {
            it = it.ruled();
        }
        lists::item(l, i, &it, t)
    };
    let body = vec![
        dialogs::para_rich(0, WORDS, &[at..at + ADDRESS.len()], t),
        lists::list(
            4,
            Some("In the bundle"),
            vec![
                plain(4, 0, &ic::CHECK, "Queue counts", false),
                plain(4, 1, &ic::CHECK, "An error code and the last error message", true),
            ],
            None,
            t,
        ),
        lists::list(
            5,
            Some("Removed first"),
            vec![
                plain(5, 0, &ic::CLOSE_X, "Paths, file and folder names", false),
                plain(5, 1, &ic::CLOSE_X, "Sign-in tokens", true),
            ],
            Some(
                "Standard error words, numbers, opaque ids and the server address are kept, so a name the app does not recognise can stay only if it is itself a standard word or a number.",
            ),
            t,
        ),
        lists::list(
            6,
            Some("Your email"),
            vec![lists::item(
                6,
                0,
                &Item::new(Some(&ic::MAIL), "Sent from your own address")
                    .sub("Support sees it, as with any email you send."),
                t,
            )],
            None,
            t,
        ),
    ];
    let foot = vec![
        dialogs::button(0, "Save and write email", Weight::Primary, t)
            .default_button()
            .on_press(|s: &mut State| {
                s.settings.ask(Intent::SaveBundleAndWriteEmail);
                // The press exports, once; showing the page (`go`) never does.
                s.settings.drive.export_bundle();
                super::go(s, "DW-0790b");
            }),
        dialogs::button(1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    dialogs::dialog("Report a problem", 460.0, "Report a problem", None, body, foot, t)
        .modal(false, |s: &mut State| s.settings.dialog = None)
}
