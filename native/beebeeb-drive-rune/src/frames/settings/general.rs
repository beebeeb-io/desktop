//! General (DW-0701), with a new version available (DW-0781), and the Theme menu open (DW-0787).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::settings_shell as shell;
use rune::El;

use super::source::{Press, UPDATE_CAPTION, Update};
use super::{Dialog, Intent, MenuKind, SettingsUi};
use crate::ui::settings_dialog::{self as dialogs, Weight};

/// Where the Theme menu drops, from its row's top-left corner: over its pop-up button, the
/// board's place (DW-0787: the menu at 700, 118 over the row at 254, 115).
const THEME_MENU_AT: (f32, f32) = (446.0, 3.0);

/// The pane, and the menu over it when one is open.
pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let g = ui.source.general();
    let menu_open = ui.menu.is_some_and(|m| m.kind == MenuKind::Theme);

    let appearance = lists::roving_list(
        0,
        Some("Appearance"),
        vec![super::menu_keys(
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
            MenuKind::Theme,
        )],
        None,
        t,
    );

    let help = lists::roving_list(
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

    let privacy = lists::roving_list(
        2,
        Some("Privacy and data"),
        vec![
            lists::item(
                2,
                0,
                &Item::new(Some(&ic::SHIELD), "Privacy")
                    .trailing(Trailing::External)
                    .role(Role::Link),
                t,
            )
            .on_press(|s: &mut State| s.settings.ask(Intent::Open("privacy"))),
            lists::item(
                2,
                1,
                &Item::new(Some(&ic::DOWNLOAD_TRAY), "Export my data")
                    .trailing(Trailing::External)
                    .role(Role::Link)
                    .ruled(),
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
    let about = lists::roving_list(
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
    if let Update::Available { version, press, says } = &g.update {
        // The one primary action of this view: Amber. One press, Restart to update: download, check, install (owner
        // ruling 2026-11-15); while it is worked on the button waits and the banner says the step it is at.
        let press = *press;
        let mut action = controls::button(0, press.label(), None, Emphasis::Primary, t);
        action = if press == Press::Wait {
            action.disabled(true).focusable(false)
        } else {
            action.on_press(move |s: &mut State| s.settings.press_update(press))
        };
        children.push(controls::banner(
            0,
            &ic::DOWNLOAD_TRAY,
            &format!("Version {version} is available"),
            says.as_deref().unwrap_or(UPDATE_CAPTION),
            Some(action),
            t,
        ));
    }
    children.extend([help, privacy, about]);

    let menu = super::menu_overlay(ui, MenuKind::Theme, t);
    let overlay = match ui.dialog {
        Some(Dialog::ReportProblem) => Some(report_problem(ui.crash_record.as_ref(), t)),
        _ => menu,
    };
    (shell::pane(children, t), overlay)
}

/// Report a problem (DW-0782): what the support bundle holds and leaves out, before it is
/// saved; the person sends the email themselves. `crash`: the record "Send a report" attached after a crash
/// (`shell::crash_notice`), shown here word for word before it goes into the bundle.
fn report_problem(crash: Option<&crate::log_file::crash::CrashRecord>, t: &Theme) -> El {
    const WORDS: &str = concat!("Beebeeb saves a support bundle on ", beebeeb_engine::device_word!(), " and opens an email to support@beebeeb.io in your email app. Read the file, then attach it and send the email yourself.");
    const ADDRESS: &str = "support@beebeeb.io";
    let at = WORDS.find(ADDRESS).unwrap_or(0);
    let plain = |l: usize, i: usize, icon: &'static rune::Icon, title: &str, ruled: bool| {
        let mut it = Item::new(Some(icon), title);
        if ruled {
            it = it.ruled();
        }
        lists::item(l, i, &it, t)
    };
    let address = at..at + ADDRESS.len();
    let mut body = vec![dialogs::para_rich(0, WORDS, std::slice::from_ref(&address), t)];
    if let Some(record) = crash {
        let text = crate::shell::crash_notice::attached_text(record);
        body.push(lists::list(
            7,
            Some("Attached: why Beebeeb closed last time"),
            text.lines().enumerate().map(|(i, line)| lists::item(7, i, &Item::new(None, line), t)).collect(),
            Some(
                "The app version, which part stopped and where in Beebeeb's own code. No file names, paths, email address, keys or tokens.",
            ),
            t,
        ));
    }
    body.extend([
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
    ]);
    let foot = vec![
        dialogs::button(0, "Save and write email", Weight::Primary, t)
            .default_button()
            .on_press(|s: &mut State| {
                s.settings.ask(Intent::SaveBundleAndWriteEmail);
                // The press exports, once; showing the page (`go`) never does. An attached crash record goes in the
                // bundle, and its file is deleted once the bundle is saved.
                match s.settings.crash_record.take() {
                    Some(record) => s.settings.drive.export_bundle_with_crash(record),
                    None => s.settings.drive.export_bundle(),
                }
                super::go(s, "DW-0790b");
            }),
        dialogs::button(1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    dialogs::dialog("Report a problem", 460.0, "Report a problem", None, body, foot, t)
        .modal(false, |s: &mut State| s.settings.dialog = None)
}

#[cfg(test)]
mod tests {
    use super::super::source::{Os, ThemeChoice};

    #[test]
    fn theme_row_names_the_os_it_follows() {
        assert_eq!(ThemeChoice::System.says_on(Os::Windows), "Follows Windows.");
        assert_eq!(ThemeChoice::System.says_on(Os::Mac), "Follows macOS.");
        assert_eq!(ThemeChoice::System.says_on(Os::Other), "Follows the system.");
        assert_eq!(ThemeChoice::Light.says_on(Os::Mac), "Always light.");
        assert_eq!(ThemeChoice::Dark.says_on(Os::Mac), "Always dark.");
        #[cfg(target_os = "macos")]
        assert_eq!(ThemeChoice::System.says(), "Follows macOS.");
    }

    /// Every settings frame, drawn with this Mac's words, holds no Windows-only word.
    #[cfg(target_os = "macos")]
    #[test]
    fn no_settings_frame_says_a_windows_word_on_a_mac() {
        let bad = ["Windows", "Explorer", "this PC", "This PC", " PC ", "Hello", "Start menu", "OneDrive"];
        let mut found = Vec::new();
        for (id, _) in super::super::FRAMES {
            let mut state = crate::init();
            if !super::super::scene(id, &mut state) {
                continue;
            }
            let env = rune::Env { system_dark: false, scale: 2.0 };
            let mut rt = rune::Runtime::new(state, crate::view, (958.0, 718.0), env);
            rt.frame(0.0);
            rt.frame(0.016);
            for e in rt.elements() {
                for said in [e.text.as_deref(), e.label.as_deref()].into_iter().flatten() {
                    let padded = format!(" {said} ");
                    if let Some(w) = bad.iter().find(|w| padded.contains(**w)) {
                        found.push(format!("{id} {}: {w}: {said}", e.key.as_deref().unwrap_or("")));
                    }
                }
            }
        }
        found.sort();
        found.dedup();
        assert!(found.is_empty(), "Windows words on a Mac:\n{}", found.join("\n"));
    }
}
