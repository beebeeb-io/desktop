//! Sync and File Explorer (DW-0746): Beebeeb in File Explorer, sync, and the space Beebeeb
//! uses on this PC. Over it: turning the File Explorer location off (DW-0746b), the repair
//! question (DW-0746c), free up space (DW-0780), the limit (DW-0714). States: a repair that
//! did not finish (DW-0746d), sync waiting on a metered connection (DW-0746e).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::scale::space as sp;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_dialog::{self as dialogs, Weight};
use crate::ui::settings_list::{self as lists, FOOT, Item, Role, Trailing};
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::settings_shell as shell;
use crate::ui::{dialog, kit};
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
    let mut children = vec![shell::heading(concat!("Sync and ", beebeeb_engine::file_manager!()), None, t)];

    if sy.repair == Repair::Failed {
        let again = button("Repair…").on_press(|s: &mut State| s.settings.dialog = Some(Dialog::Repair));
        children.push(notices::note_with(
            0,
            Tone::Red,
            &ic::WARN_CIRCLE,
            concat!("Couldn't repair Beebeeb in ", beebeeb_engine::file_manager!(), "."),
            "Nothing was changed that you need to undo. Try again.",
            Some(again),
            t,
        ));
    }

    // File Explorer.
    let mut explorer = Vec::new();
    let mut place = Item::new(Some(&ic::FOLDER_TAB), concat!("Beebeeb in ", beebeeb_engine::file_manager!())).sub(if sy.in_explorer {
        concat!("In ", beebeeb_engine::file_manager!(), "'s navigation pane")
    } else {
        "Off"
    });
    let show = sy.in_explorer.then(|| {
        button(concat!("Show in ", beebeeb_engine::file_manager!())).on_press(|s: &mut State| s.settings.ask(Intent::Open("file-explorer")))
    });
    if !sy.in_explorer {
        place = place.trailing(Trailing::Value("Turned off".into()));
    }
    explorer.push(lists::item_with(0, 0, &place, show.into_iter().collect(), t));
    let repairing = sy.repair == Repair::Running;
    let repair =
        (!repairing).then(|| button("Repair…").on_press(|s: &mut State| s.settings.dialog = Some(Dialog::Repair)));
    explorer.push(lists::item_with(
        0,
        1,
        &Item::new(Some(&ic::REBUILD), concat!("Repair Beebeeb in ", beebeeb_engine::file_manager!()))
            .sub(if repairing {
                "Repairing…"
            } else {
                concat!("If ", beebeeb_engine::file_manager!(), " stops showing Beebeeb's files or their state")
            })
            .ruled(),
        repair.into_iter().collect(),
        t,
    ));
    children.push(lists::list(0, Some(concat!(beebeeb_engine::file_manager!())), explorer, None, t));

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
                crate::ui::switch::switch_sized(
                    &format!("{}-switch", "item-1-1"),
                    "",
                    login,
                    true,
                    crate::ui::button::Size::Md,
                    t,
                )
                .label("Open Beebeeb when you log in")
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
            &Item::new(Some(&ic::FOLDER), concat!("Keep on ", beebeeb_engine::device_word!())).sub(concat!("Choose folders that stay on ", beebeeb_engine::device_word!())),
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
        Some(concat!("Space on ", beebeeb_engine::device_word!())),
        space,
        Some(
            concat!("Files download when you open them. Files you make available offline stay on ", beebeeb_engine::device_word!(), ", also without a connection."),
        ),
        t,
    ));

    if sy.in_explorer {
        children.push(lists::roving_list(
            3,
            None,
            vec![
                lists::item_danger(3, 0, &ic::FOLDER_TAB, concat!("Turn off Beebeeb in ", beebeeb_engine::file_manager!(), "…"), t)
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
        Some(Dialog::Keep) => Some(super::keep::dialog(ui, n, t)),
        Some(Dialog::KeepConfirm) => Some(super::keep::question(ui, n, t)),
        _ => ui.toast.map(|words| notices::toast(0, words, t)),
    };
    (shell::pane(children, t), overlay)
}

/// The words before the link in the metered foot, and the link's own word.
const BEFORE: &str = "Sync on metered connections is off in";
const LINK: &str = "Bandwidth";

/// The metered foot's sentence and the bytes of it that are the link: its underline and its
/// press target are both this one range.
pub(super) fn metered_words() -> (String, std::ops::Range<usize>) {
    let words = format!("{BEFORE} {LINK}. Sync resumes by itself on a connection that isn't metered.");
    let at = BEFORE.len() + 1;
    (words, at..at + LINK.len())
}

/// The Sync list's foot while sync waits on a metered connection (DW-0746e): why, and a link
/// to the setting. The board's export keys the words before the link (`list-1-foot`), so a
/// marker as wide as those words stands where they are drawn.
fn metered_foot(t: &Theme) -> El {
    let (words, link) = metered_words();
    kit::note_text("list-1-foot", "list-1-foot-mark", BEFORE, FOOT, t.text_tertiary);
    col()
        .key("list-1-foot-box")
        .w_full()
        .items_start()
        .px(sp::S16)
        .child(
            el().key("list-1-foot-mark")
                .pin(Some(16.0), Some(0.5), None, None)
                .size(rune::text::advance(&format!("{BEFORE} "), FOOT), 15.0),
        )
        .child(
            rich(words, vec![Span::new(link.start, link.end).underline(t.text_tertiary)])
                .key("list-1-foot-words")
                .font(FOOT)
                .color(t.text_tertiary)
                .link(Link::new(link, "bandwidth").label("Bandwidth settings"))
                .on_link(|s: &mut State, _: &str, _| super::go(s, "DW-0785f")),
        )
}

/// Turn off Beebeeb in File Explorer (DW-0746b): the alert counts what has not uploaded.
fn turn_off(not_uploaded: Option<&(u32, String)>, first: usize, t: &Theme) -> El {
    let says = match not_uploaded {
        Some((count, size)) => format!(
            concat!("{count} files made in ", beebeeb_engine::file_manager!(), " haven't uploaded yet ({size}). Turning off removes them from ", beebeeb_engine::device_word!(), ". Everything else stays in Beebeeb."),
            count = count,
            size = size
        ),
        None => concat!("Beebeeb leaves ", beebeeb_engine::file_manager!(), ". Everything stays in Beebeeb.").to_string(),
    };
    let foot = vec![
        dialogs::button_danger(first, "Turn off", t).on_press(|s: &mut State| {
            s.settings.drive.turn_off_explorer();
            s.settings.dialog = None;
        }),
        dialogs::button(first + 1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    let card = dialog::dialog(
        dialog::Dialog {
            alert: true,
            head: dialog::Head::Alert {
                tile: notices::tile(0, Tone::Red, &ic::FOLDER_TAB, t),
                title: concat!("Turn off Beebeeb in ", beebeeb_engine::file_manager!(), "?"),
                sub: &says,
            },
            foot: dialog::Foot::Even(foot),
            ..dialog::Dialog::new("dlg", concat!("Turn off Beebeeb in ", beebeeb_engine::file_manager!()), 380.0)
        },
        t,
    );
    // Turned off, the row that opened this is gone: focus goes to the list above it.
    dialog::modal_back(
        0.0,
        card,
        TURN_OFF_ROW,
        SPACE_LIST,
        |s: &mut State| s.settings.dialog = None,
        t,
    )
}

/// The row that opens the turn-off question, gone once Beebeeb leaves File Explorer.
pub const TURN_OFF_ROW: &str = "item-3-0";
/// Where focus goes when that row is gone: the list above it (Space on this PC).
pub const SPACE_LIST: &str = "list-2";

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
        concat!("Repair Beebeeb in ", beebeeb_engine::file_manager!()),
        380.0,
        notices::tile(0, Tone::Neutral, &ic::FOLDER_TAB, t),
        concat!("Repair Beebeeb in ", beebeeb_engine::file_manager!(), "?"),
        concat!("Beebeeb removes its ", beebeeb_engine::file_manager!(), " location and turns off Open Beebeeb at login, then adds itself back to ", beebeeb_engine::file_manager!(), ". Files waiting to upload are kept."),
        foot,
        t,
    )
    .modal(false, |s: &mut State| s.settings.dialog = None)
}
