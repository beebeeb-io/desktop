//! Sync and File Explorer (DW-0746): Beebeeb in File Explorer, sync, and the space Beebeeb
//! uses on this PC. Over it: turning the File Explorer location off (DW-0746b), the repair
//! question (DW-0746c), free up space (DW-0780), the limit (DW-0714). States: a repair that
//! did not finish (DW-0746d), sync waiting on a metered connection (DW-0746e).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_dialog::{self as dialogs, Weight};
use crate::ui::settings_list::{self as lists, FOOT, Item, Role, Trailing};
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::settings_shell as shell;
use rune::{El, Link, Span, col, el, rich};

use super::drive::{Kept, Repair, SyncStatus};
use super::{Dialog, Intent, SettingsUi};

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let sy = ui.drive.sync();
    // Buttons are numbered in the order the pane draws them.
    let mut n = 0;
    let mut button = |label: &str| {
        n += 1;
        controls::button(n - 1, label, None, Emphasis::Plain, t)
    };
    let mut children = vec![shell::heading("Sync and File Explorer", None, t)];

    if sy.repair == Repair::Failed {
        let again = button("Repair…").on_press(|s: &mut State| s.settings.dialog = Some(Dialog::Repair));
        children.push(notices::note_with(
            0,
            Tone::Red,
            &ic::WARN_CIRCLE,
            "Couldn't repair Beebeeb in File Explorer.",
            "Nothing was changed that you need to undo. Try again.",
            Some(again),
            t,
        ));
    }

    // File Explorer.
    let mut explorer = Vec::new();
    let mut place = Item::new(Some(&ic::FOLDER_TAB), "Beebeeb in File Explorer").sub(if sy.in_explorer {
        "In File Explorer's navigation pane"
    } else {
        "Off"
    });
    let show = sy.in_explorer.then(|| {
        button("Show in File Explorer").on_press(|s: &mut State| s.settings.ask(Intent::Open("file-explorer")))
    });
    if !sy.in_explorer {
        place = place.trailing(Trailing::Value("Turned off".into()));
    }
    explorer.push(lists::item_with(0, 0, &place, show.into_iter().collect(), t));
    let on = sy.status_on_files;
    explorer.push(lists::item_with(
        0,
        1,
        &Item::new(Some(&ic::SYNC_BADGE), "Show sync status on files").ruled(),
        vec![
            lists::switch("item-0-1", "Show sync status on files", on, t)
                .on_press(move |s: &mut State| s.settings.drive.set_status_on_files(!on)),
        ],
        t,
    ));
    let repairing = sy.repair == Repair::Running;
    let repair =
        (!repairing).then(|| button("Repair…").on_press(|s: &mut State| s.settings.dialog = Some(Dialog::Repair)));
    explorer.push(lists::item_with(
        0,
        2,
        &Item::new(Some(&ic::REBUILD), "Repair Beebeeb in File Explorer")
            .sub(if repairing {
                "Repairing…"
            } else {
                "If File Explorer stops showing Beebeeb's files or their state"
            })
            .ruled(),
        repair.into_iter().collect(),
        t,
    ));
    children.push(lists::list(0, Some("File Explorer"), explorer, None, t));

    // Sync: where it stands, and the one thing to do about it.
    let (icon, says, act): (&'static rune::Icon, String, El) = match &sy.status {
        SyncStatus::Running(says) => (
            &ic::SYNC,
            says.clone(),
            button("Pause syncing").on_press(|s: &mut State| {
                s.settings.drive.set_paused(true);
                s.settings.ask(Intent::Show("DW-1204"));
            }),
        ),
        SyncStatus::Paused => (
            &ic::SYNC,
            "Paused".into(),
            button("Resume syncing").on_press(|s: &mut State| s.settings.drive.set_paused(false)),
        ),
        SyncStatus::WaitingMetered(says) => (
            &ic::WIFI,
            says.clone(),
            button("Sync anyway").on_press(|s: &mut State| {
                s.settings.drive.sync_anyway();
                s.settings.ask(Intent::Show("DW-1203"));
            }),
        ),
    };
    let login = sy.open_at_login;
    let sync_rows = vec![
        lists::item_with(1, 0, &Item::new(Some(icon), "Status").sub(says), vec![act], t),
        lists::item_with(
            1,
            1,
            &Item::new(Some(&ic::POWER), "Open Beebeeb when you log in").ruled(),
            vec![
                lists::switch("item-1-1", "Open Beebeeb when you log in", login, t)
                    .on_press(move |s: &mut State| s.settings.drive.set_open_at_login(!login)),
            ],
            t,
        ),
    ];
    let mut sync_list = lists::list(1, Some("Sync"), sync_rows, None, t);
    if matches!(sy.status, SyncStatus::WaitingMetered(_)) {
        sync_list = sync_list.child(metered_foot(t));
    }
    children.push(sync_list);

    // Space on this PC.
    let space = vec![
        lists::item(
            2,
            0,
            &Item::new(Some(&ic::DOWNLOAD_TRAY), "Downloaded files").trailing(Trailing::Mono(sy.downloaded.clone())),
            t,
        ),
        lists::item(
            2,
            1,
            &Item::new(Some(&ic::SLIDERS), "Limit")
                .sub("Beebeeb removes the oldest downloads past this, never offline files")
                .trailing(Trailing::Pop {
                    choice: format!("{} GB", sy.limit_gb),
                    open: false,
                    menu_at: (0.0, 0.0),
                })
                .role(Role::Link),
            t,
        )
        .on_press(|s: &mut State| s.settings.open_limit(Kept::Downloaded)),
        lists::item_with(
            2,
            2,
            &Item::new(Some(&ic::FOLDER), "Keep on this PC").sub("Choose folders that stay on this PC"),
            vec![button("Choose folders…").on_press(|s: &mut State| super::go(s, "DW-0784"))],
            t,
        ),
        lists::item(
            2,
            3,
            &Item::new(Some(&ic::OFFLINE_PIN), "Available offline")
                .trailing(Trailing::Value(sy.offline.clone()))
                .opens(),
            t,
        )
        .on_press(|s: &mut State| super::go(s, "DW-0715")),
        lists::item_with(
            2,
            4,
            &Item::new(Some(&ic::TRASH), "Free up space").sub("Removes downloaded copies. Files stay in Beebeeb."),
            vec![button("Free up space…").on_press(|s: &mut State| s.settings.dialog = Some(Dialog::FreeUp))],
            t,
        ),
    ];
    children.push(lists::list(
        2,
        Some("Space on this PC"),
        space,
        Some(
            "Files download when you open them. Files you make available offline stay on this PC, also without a connection.",
        ),
        t,
    ));

    if sy.in_explorer {
        children.push(lists::list(
            3,
            None,
            vec![
                lists::item_danger(3, 0, &ic::FOLDER_TAB, "Turn off Beebeeb in File Explorer…", t)
                    .on_press(|s: &mut State| s.settings.dialog = Some(Dialog::TurnOffExplorer)),
            ],
            None,
            t,
        ));
    }

    let overlay = match ui.dialog {
        Some(Dialog::TurnOffExplorer) => Some(turn_off(sy.not_uploaded.as_ref(), n, t)),
        Some(Dialog::Repair) => Some(repair_question(n, t)),
        Some(Dialog::FreeUp) => Some(super::storage::free_up(ui, n, t)),
        Some(Dialog::Limit(which)) => Some(super::storage::limit(ui, which, n, t)),
        _ => ui.toast.map(|words| notices::toast(0, words, t)),
    };
    (shell::pane(children, t), overlay)
}

/// The Sync list's foot while sync waits on a metered connection (DW-0746e): why, and a link
/// to the setting. The board's export keys the words before the link (`list-1-foot`), so a
/// marker as wide as those words stands where they are drawn.
fn metered_foot(t: &Theme) -> El {
    const BEFORE: &str = "Sync on metered connections is off in";
    const LINK: &str = "Bandwidth";
    let words = format!("{BEFORE} {LINK}. Sync resumes by itself on a connection that isn't metered.");
    let at = BEFORE.len() + 1;
    kit::note_text("list-1-foot", "list-1-foot-mark", BEFORE, FOOT, t.text_tertiary);
    col()
        .key("list-1-foot-box")
        .w_full()
        .items_start()
        .px(16.0)
        .child(
            el().key("list-1-foot-mark")
                .pin(Some(16.0), Some(0.5), None, None)
                .size(rune::text::advance(&format!("{BEFORE} "), FOOT), 15.0),
        )
        .child(
            rich(words, vec![Span::new(at, at + LINK.len()).underline(t.text_tertiary)])
                .key("list-1-foot-words")
                .font(FOOT)
                .color(t.text_tertiary)
                .link(Link::new(at..at + LINK.len(), "bandwidth").label("Bandwidth settings"))
                .on_link(|s: &mut State, _: &str, _| super::go(s, "DW-0785f")),
        )
}

/// Turn off Beebeeb in File Explorer (DW-0746b): the alert counts what has not uploaded.
fn turn_off(not_uploaded: Option<&(u32, String)>, first: usize, t: &Theme) -> El {
    let says = match not_uploaded {
        Some((count, size)) => format!(
            "{count} files made in File Explorer haven't uploaded yet ({size}). Turning off removes them from this PC. Everything else stays in Beebeeb."
        ),
        None => "Beebeeb leaves File Explorer. Everything stays in Beebeeb.".to_string(),
    };
    let foot = vec![
        dialogs::button_danger(first, "Turn off", t).on_press(|s: &mut State| {
            s.settings.drive.turn_off_explorer();
            s.settings.dialog = None;
        }),
        dialogs::button(first + 1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    dialogs::alert(
        "Turn off Beebeeb in File Explorer",
        380.0,
        notices::tile(0, Tone::Red, &ic::FOLDER_TAB, t),
        "Turn off Beebeeb in File Explorer?",
        &says,
        foot,
        t,
    )
    .modal(false, |s: &mut State| s.settings.dialog = None)
}

/// Repair Beebeeb in File Explorer (DW-0746c): what the repair does, before it does it.
fn repair_question(first: usize, t: &Theme) -> El {
    let foot = vec![
        dialogs::button(first, "Repair", Weight::Primary, t)
            .default_button()
            .on_press(|s: &mut State| {
                s.settings.drive.repair();
                s.settings.dialog = None;
                s.settings.ask(Intent::Show("DW-1212"));
            }),
        dialogs::button(first + 1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    dialogs::alert(
        "Repair Beebeeb in File Explorer",
        380.0,
        notices::tile(0, Tone::Neutral, &ic::FOLDER_TAB, t),
        "Repair Beebeeb in File Explorer?",
        "Beebeeb removes its File Explorer location and turns off Open Beebeeb at login, then adds itself back to File Explorer. Files waiting to upload are kept.",
        foot,
        t,
    )
    .modal(false, |s: &mut State| s.settings.dialog = None)
}
