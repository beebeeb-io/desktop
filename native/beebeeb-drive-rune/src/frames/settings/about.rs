//! About Beebeeb (DW-0751), the Release channel menu (DW-0789), the release notes (DW-0789b).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::settings_card as cards;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_dialog::{self as dialogs, Weight};
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::settings_shell as shell;
use rune::{El, col};

use super::{Dialog, Intent, MenuKind, SettingsUi};

/// Where the Release channel menu drops, in the window's body (the board's place).
const CHANNEL_MENU_AT: (f32, f32) = (702.0, 313.0);

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let a = ui.source.about();
    let menu_open = ui.menu.is_some_and(|m| m.kind == MenuKind::Channel);

    let copy_text = format!("{} {} ({})\nEncryption core {}", a.product, a.version, a.build, a.core);
    let product = cards::card(0, t)
        .child(kit::mark("card-0-mark", 44.0))
        .child(
            col()
                .key("card-0-words")
                .grow(1.0)
                .min_w(0.0)
                .child(cards::head(0, &a.product, t))
                .child(cards::sub(0, &a.tagline, t)),
        )
        .child(
            controls::button(0, "Copy", Some(&ic::COPY), Emphasis::Plain, t)
                .on_press(move |s: &mut State| s.settings.ask(Intent::Copy(copy_text.clone()))),
        );

    let versions = lists::list(
        0,
        None,
        vec![
            lists::item(
                0,
                0,
                &Item::new(None, "Version").trailing(Trailing::Mono(format!("{} ({})", a.version, a.build))),
                t,
            ),
            lists::item(
                0,
                1,
                &Item::new(None, "Encryption core")
                    .trailing(Trailing::Mono(a.core.clone()))
                    .ruled(),
                t,
            ),
        ],
        None,
        t,
    );

    let updates = lists::list(
        1,
        Some("Updates"),
        vec![
            lists::item_with(
                1,
                0,
                &Item::new(Some(&ic::DOWNLOAD_TRAY), "Check for updates").sub(a.last_check.clone()),
                vec![
                    controls::button(1, "Check now", None, Emphasis::Plain, t)
                        .on_press(|s: &mut State| s.settings.ask(Intent::CheckForUpdates)),
                ],
                t,
            ),
            lists::item(
                1,
                1,
                &Item::new(Some(&ic::SYNC), "Release channel")
                    .sub(a.channel.says())
                    .trailing(Trailing::Pop {
                        choice: a.channel.label().into(),
                        open: menu_open,
                    })
                    .role(Role::Button),
                t,
            )
            .on_press(|s: &mut State| s.settings.toggle_menu(MenuKind::Channel)),
            lists::item(
                1,
                2,
                &Item::new(Some(&ic::DOCUMENT), "Release notes")
                    .sub("What changed in each version")
                    .opens(),
                t,
            )
            .on_press(|s: &mut State| super::go(s, "DW-0789b")),
        ],
        Some("Beebeeb checks for updates at launch and every 4 hours."),
        t,
    );

    let web = |l: usize, i: usize, title: &str, page: &'static str, ruled: bool| {
        let mut it = Item::new(None, title).trailing(Trailing::External).role(Role::Link);
        if ruled {
            it = it.ruled();
        }
        lists::item(l, i, &it, t).on_press(move |s: &mut State| s.settings.ask(Intent::Web(page)))
    };
    let open_source = lists::list(
        2,
        Some("Open source"),
        vec![
            web(2, 0, "Encryption core on GitHub", "encryption-core-on-github", false),
            lists::item(2, 1, &Item::new(None, "Open-source licences").opens().ruled(), t)
                .on_press(|s: &mut State| s.settings.ask(Intent::Open("open-source-licences"))),
        ],
        None,
        t,
    );
    let more = lists::list(
        3,
        Some("More"),
        vec![
            web(3, 0, "About audits", "about-audits", false),
            web(3, 1, "beebeeb.io", "beebeeb.io", true),
        ],
        Some(&a.company),
        t,
    );

    let children = vec![
        shell::crumb("General", t).on_press(|s: &mut State| super::go(s, "DW-0701")),
        shell::heading("About Beebeeb", None, t),
        product,
        versions,
        updates,
        open_source,
        more,
    ];

    let overlay = match ui.dialog {
        Some(Dialog::ReleaseNotes) => Some(release_notes(ui, t)),
        _ => super::menu_overlay(ui, MenuKind::Channel, CHANNEL_MENU_AT, t),
    };
    (shell::pane(children, t), overlay)
}

/// What a version changes, before downloading it (DW-0789b).
fn release_notes(ui: &SettingsUi, t: &Theme) -> El {
    let n = ui.source.about().notes;
    let title = format!("Beebeeb {}", n.version);
    let sub = format!("{} channel · released {}", n.channel.label(), n.released);
    let notes = if n.lines.is_empty() {
        dialogs::bullets(&["No release notes were provided for this version.".to_string()], t)
    } else {
        dialogs::bullets(&n.lines, t)
    };
    let body = vec![
        notes,
        dialogs::caption(
            0,
            "Download it, then open it to install. Beebeeb never installs an update by itself.",
            t,
        ),
    ];
    let foot = vec![
        dialogs::button(2, "Restart to update", Weight::Primary, t).on_press(|s: &mut State| {
            s.settings.dialog = None;
            s.settings.ask(Intent::DownloadUpdate);
        }),
        dialogs::button(3, "Close", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    dialogs::dialog(&title, 480.0, &title, Some(&sub), body, foot, t)
        .modal(false, |s: &mut State| s.settings.dialog = None)
}
