//! What Backups draws: the section (DW-0737, DW-1411), one device's backup (DW-0739), what
//! could not back up (DW-0741), and the two dialogs (DW-1410, DW-0742).

use rune::{Cx, El, col, row};

use super::source::{Backed, BackupsSource, Device, DeviceKind, FileKind, Folder, NotBackedUp, Part, Status};
use super::{BackupsUi, Dialog, Page, mail_view};
use crate::State;
use crate::frames::settings;
use crate::frames::this_pc::icons as pc_icons;
use crate::shell::{placeholder, registry};
use crate::theme::icons as drive_icons;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{self, Theme};
use crate::theme::file_icons::icon_for;
use crate::ui::file_row::RowLook;
use crate::ui::file_tile::{self, TilePicture, TileSize};
use crate::ui::view_state::{ViewState, try_again, view_state};
use crate::ui::backups_kit::{self as bk, Leading, Md, RowItem, TileTone};
use crate::ui::settings_card as cards;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_dialog as dialogs;
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::{caption, settings_shell as shell};

/// `1820` as "1,820".
pub(super) fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The settings sidebar with Backups selected: the shared list; Backups itself goes back to the
/// section's own page, the rest through `settings::go`, which keeps the shell's route in step.
fn sidebar(t: &Theme) -> El {
    fn go(s: &mut State, frame: &'static str) {
        if frame == "DW-0737" {
            s.backups.show(Page::Section);
        } else {
            settings::go(s, frame);
        }
    }
    shell::app_sidebar(Some("Backups"), true, go, t)
}

/// The status card: all backed up with Back up now, or a backup running with its bar.
/// Returns how many buttons it drew.
fn status_card(status: &Status, t: &Theme) -> (El, usize) {
    match status {
        Status::AllBackedUp { last, from_this_pc } => {
            // A source that has no count of the files says nothing of them (never "from this PC" with nothing before it).
            let words = if from_this_pc.is_empty() {
                format!("Last backup {last}")
            } else {
                format!("Last backup {last} \u{00B7} {from_this_pc} from this PC")
            };
            let at = "Last backup ".len();
            let card = bk::card(0, t)
                .child(bk::tile(0, TileTone::Green, &ic::CHECK, 34.0, t))
                .child(
                    col()
                        .key("card-0-words")
                        .grow(1.0)
                        .min_w(0.0)
                        .child(cards::head(0, "All backed up", t))
                        .child(bk::sub_rich(0, &words, &[at..at + last.len()], t)),
                )
                .child(
                    controls::button(0, "Back up now", None, Emphasis::Plain, t)
                        .on_press(|s: &mut State| s.backups.back_up_now()),
                );
            (card, 1)
        }
        Status::Plain { title, sub, can_run } => {
            let mut card = bk::card(0, t)
                .child(bk::tile(0, TileTone::Neutral, &ic::SYNC, 34.0, t))
                .child(
                    col()
                        .key("card-0-words")
                        .grow(1.0)
                        .min_w(0.0)
                        .child(cards::head(0, title, t))
                        .child(bk::sub_rich(0, sub, &[], t)),
                );
            if *can_run {
                card = card.child(
                    controls::button(0, "Back up now", None, Emphasis::Plain, t)
                        .on_press(|s: &mut State| s.backups.back_up_now()),
                );
            }
            (card, usize::from(*can_run))
        }
        Status::Running {
            folder,
            done,
            total,
            minutes_left,
            first,
        } => {
            let count = format!("{} of {}", thousands(*done), thousands(*total));
            let mut words = format!("{count} files \u{00B7} about {minutes_left} min left");
            if *first {
                words.push_str(" \u{00B7} the first backup takes longest");
            }
            // Whole per cents, as the board draws 612 of 1,820 at 34 %.
            let percent = if *total == 0 {
                0.0
            } else {
                (f64::from(*done) * 100.0 / f64::from(*total)).round()
            };
            let card = bk::card(0, t)
                .child(bk::tile(0, TileTone::Neutral, &ic::SYNC, 34.0, t))
                .child(
                    col()
                        .key("card-0-words")
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(6.0)
                        .child(cards::head(0, &format!("Backing up {folder}"), t))
                        .child(bk::sub_rich(0, &words, &[0..count.len()], t))
                        .child(bk::bar(
                            "card-0-bar",
                            percent as f32 / 100.0,
                            &format!("Backing up {folder}, {percent:.0} per cent"),
                            t,
                        )),
                );
            (card, 0)
        }
    }
}

fn folder_row(index: usize, folder: &Folder, button: usize, t: &Theme) -> El {
    let last = folder.last_backup.clone().unwrap_or_else(|| "off".into());
    let chosen = if folder.chosen_by_you {
        " \u{00B7} chosen by you"
    } else {
        ""
    };
    let sub = format!(
        "{} items \u{00B7} {}{chosen} \u{00B7} {last}",
        thousands(folder.items),
        folder.size
    );
    let mut item = Item::new(Some(&ic::FOLDER), folder.name.clone()).sub(sub);
    if index > 0 {
        item = item.ruled();
    }
    let id = folder.id.clone();
    let control = if folder.chosen_by_you {
        bk::ghost_button(button, "Remove", &format!("Remove {}", folder.name), t)
            .on_press(move |s: &mut State| s.backups.ask_stop_folder(&id))
    } else {
        lists::switch(&format!("item-0-{index}"), &folder.name, folder.on, t)
            .on_press(move |s: &mut State| s.backups.toggle_folder(&id))
    };
    lists::item_with(0, index, &item, vec![control], t)
}

/// What a device row and page say for a device whose status file this PC cannot read.
pub(super) const CANT_READ: &str = "Can't read this device's backup.";
/// Why a device that may be this PC's own backup has no "Delete this backup" (the engine refuses it too).
pub(super) const MAY_BE_THIS_PC: &str = "This may be this PC's own backup, so it can't be deleted from this PC.";

fn device_icon(kind: DeviceKind) -> &'static rune::Icon {
    match kind {
        DeviceKind::Phone => &ic::PHONE,
        DeviceKind::Tablet => &ic::TABLET,
        DeviceKind::Computer | DeviceKind::Unknown => &ic::LAPTOP,
    }
}

/// "12,418 photos", "2 folders", or that it cannot be read.
fn backed_words(backed: Backed) -> String {
    match backed {
        Backed::Photos(n) => format!("{} photos", thousands(n)),
        Backed::Folders(1) => "1 folder".to_string(),
        Backed::Folders(n) => format!("{} folders", thousands(n)),
        Backed::Unreadable => CANT_READ.to_string(),
    }
}

/// What may put a deleted backup back: the device itself, while its backup is on.
fn backs_up_again(device: &Device) -> String {
    match device.kind {
        DeviceKind::Phone | DeviceKind::Tablet => format!(
            "Photos still on the {} may back up again unless backup is off there.",
            device.model
        ),
        DeviceKind::Computer => format!("{} may back it up again unless backup is off there.", device.name),
        DeviceKind::Unknown => "The device may back it up again unless backup is off there.".to_string(),
    }
}

fn device_row(index: usize, device: &Device, t: &Theme) -> El {
    let icon = device_icon(device.kind);
    let when = match (&device.last_backup, device.signed_in) {
        (_, Some(false)) => "Not signed in".to_string(),
        (Some(last), _) => last.clone(),
        (None, _) => String::new(),
    };
    let mut sub = backed_words(device.backed);
    if !when.is_empty() && device.backed != Backed::Unreadable {
        sub = format!("{sub} \u{00B7} {when}");
    }
    let id = device.id.clone();
    lists::item(
        1,
        index,
        &Item::new(Some(icon), device.name.clone()).sub(sub).opens(),
        t,
    )
    .on_press(move |s: &mut State| s.backups.show(Page::Device(id.clone())))
}

/// DW-0737, DW-1411. Returns the pane's children and how many buttons they hold.
fn section(source: &dyn BackupsSource, t: &Theme) -> (Vec<El>, usize) {
    // Nothing is drawn as read before the engine answered: the loading state, then the answer or why there is none.
    if source.loading() {
        return (
            vec![
                shell::heading("Backups", None, t),
                view_state(ViewState::Loading { words: "Reading this PC's backup\u{2026}" }, t),
            ],
            0,
        );
    }
    let refusal = source.refusal();
    if let (Some(words), true) = (refusal, source.folders().is_empty()) {
        let retry = try_again("backups-failed", t).on_press(|s: &mut State| s.backups.read_again());
        return (
            vec![
                shell::heading("Backups", None, t),
                view_state(
                    ViewState::Failed {
                        key: "backups-failed",
                        title: "Couldn't read this PC's backup",
                        why: words,
                        retry: Some(retry),
                    },
                    t,
                ),
            ],
            1,
        );
    }
    let (mut children, mut buttons) = if source.wired(Part::Status) {
        let (card, buttons) = status_card(&source.status(), t);
        (vec![shell::heading("Backups", None, t), card], buttons)
    } else {
        (vec![shell::heading("Backups", None, t)], 0)
    };
    if let Some(words) = refusal {
        children.push(controls::banner_toned(
            0,
            true,
            &ic::WARN_CIRCLE,
            words,
            "What is shown below was read before.",
            None,
            t,
        ));
    }

    let failures = if source.wired(Part::NotBackedUp) {
        source.not_backed_up().iter().filter(|f| !f.excluded).count()
    } else {
        0
    };
    if failures > 0 {
        let strong = if failures == 1 {
            "1 item".to_string()
        } else {
            format!("{} items", thousands(failures as u32))
        };
        children.push(
            bk::link_card(1, &ic::WARN_CIRCLE, "Couldn't back up", &strong, t)
                .on_press(|s: &mut State| s.backups.show(Page::Failures)),
        );
    }

    let folders = source.folders();
    let mut rows: Vec<El> = Vec::new();
    for (i, folder) in folders.iter().enumerate() {
        rows.push(folder_row(i, folder, buttons, t));
        buttons += usize::from(folder.chosen_by_you);
    }
    // "Add a folder" is drawn only where a source adds one (the engine's settings hold the six known folders only).
    if source.wired(Part::AddFolder) {
        rows.push(
            lists::item(
                0,
                folders.len(),
                &Item::new(Some(&drive_icons::PLUS), "Add a folder\u{2026}").role(Role::Link),
                t,
            )
            .on_press(|s: &mut State| s.backups.add_folder()),
        );
    }
    if folders.is_empty() {
        children.push(view_state(
            ViewState::Empty {
                key: "backups-empty",
                icon: &drive_icons::BACKUPS,
                title: "No folders are backed up",
                body: if source.wired(Part::AddFolder) {
                    "Choose a folder with Add a folder\u{2026} and Beebeeb copies it here."
                } else {
                    "This PC has none of the folders Beebeeb backs up."
                },
                action: None,
            },
            t,
        ));
    }
    let root = source.root();
    children.push(lists::list(
        0,
        Some("This PC"),
        rows,
        Some(&format!(
            "Beebeeb copies these folders into Backups \u{203A} {}; they stay where they are. Deleting a file from this PC keeps its backup.",
            root.device_folder
        )),
        t,
    ));
    if source.wired(Part::AddFolder) {
        children.push(controls::banner(
            0,
            &ic::CLOUD,
            "OneDrive may back up Desktop, Documents and Pictures too",
            "With OneDrive's folder backup on, these folders live in the OneDrive folder: add it with Add a folder\u{2026} to back it up.",
            None,
            t,
        ));
    }

    let (devices, note) = if source.wired(Part::Devices) {
        (source.devices(), source.devices_note())
    } else {
        (Vec::new(), None)
    };
    if !devices.is_empty() {
        children.push(lists::list(
            1,
            Some("Other devices"),
            devices.iter().enumerate().map(|(i, d)| device_row(i, d, t)).collect(),
            note,
            t,
        ));
    } else if let Some(note) = note {
        // Still reading, or why there is no list: said in the list's place, never an empty card.
        children.push(lists::list(
            1,
            Some("Other devices"),
            vec![lists::item(1, 0, &Item::new(None, note), t)],
            None,
            t,
        ));
    }
    if source.mail_backup_offered() {
        children.push(lists::list(
            2,
            Some("Beebeeb services"),
            vec![
                lists::item(
                    2,
                    0,
                    &Item::new(Some(&ic::MAIL), "Mail Backup")
                        .trailing(Trailing::Value(
                            if source.mail_backup_on() { "On" } else { "Off" }.into(),
                        ))
                        .opens(),
                    t,
                )
                .on_press(|s: &mut State| s.backups.mail_backup()),
            ],
            Some(
                "Each service is off until you turn it on, and keeps its files in its own protected folder inside Backups.",
            ),
            t,
        ));
    }
    (children, buttons)
}

/// DW-0739: another device's backup. `links`: the source answers for "Open in Photos" and "Open in Files".
fn device_page(device: &Device, links: bool, t: &Theme) -> Vec<El> {
    let backed_up = match &device.size {
        Some(size) => format!("{} \u{00B7} {size}", backed_words(device.backed)),
        None => backed_words(device.backed),
    };
    let mut facts = vec![lists::item(
        0,
        0,
        &Item::new(None, "Backed up").trailing(Trailing::Mono(backed_up)),
        t,
    )];
    if let Some(last) = &device.last_backup {
        facts.push(lists::item(
            0,
            facts.len(),
            &Item::new(None, "Last backup")
                .trailing(Trailing::Mono(last.clone()))
                .ruled(),
            t,
        ));
    }
    // Whether another device is signed in is not known on this PC (no server part says it): no Status then.
    if let Some(signed_in) = device.signed_in {
        facts.push(lists::item(
            0,
            facts.len(),
            &Item::new(None, "Status")
                .trailing(Trailing::Value(
                    if signed_in { "Signed in" } else { "Not signed in" }.into(),
                ))
                .ruled(),
            t,
        ));
    }
    let (photos, files, delete) = (device.id.clone(), device.id.clone(), device.id.clone());
    let unreadable = device.backed == Backed::Unreadable;
    let mut page = vec![
        shell::crumb("Backups", t, |s: &mut State| s.backups.show(Page::Section)),
        shell::heading(&device.name, unreadable.then_some(CANT_READ), t),
    ];
    if !unreadable {
        page.push(lists::list(0, None, facts, None, t));
    }
    if links {
        page.push(lists::list(
            1,
            Some("See it"),
            vec![
                lists::item(1, 0, &Item::new(Some(&ic::IMAGE), "Open in Photos").opens(), t)
                    .on_press(move |s: &mut State| s.backups.open_in_photos(&photos)),
                lists::item(1, 1, &Item::new(Some(&ic::FOLDER), "Open in Files").opens(), t)
                    .on_press(move |s: &mut State| s.backups.open_in_files(&files)),
            ],
            None,
            t,
        ));
    }
    // A folder that may be this PC's own (no layout names this PC's yet): never offered for delete, and said why.
    if !device.deletable {
        page.push(lists::list(
            2,
            Some("Remove"),
            vec![bk::row_item(
                2,
                0,
                RowItem {
                    leading: Leading::Icon(&ic::TRASH),
                    title: "Can't delete this backup here".into(),
                    title_color: None,
                    sub: None,
                    ruled: false,
                    link: false,
                    trailing: Vec::new(),
                },
                t,
            )],
            Some(MAY_BE_THIS_PC),
            t,
        ));
        return page;
    }
    page.push(lists::list(
        2,
        Some("Remove"),
        vec![
            bk::row_item(
                2,
                0,
                RowItem {
                    leading: Leading::Icon(&ic::TRASH),
                    title: "Delete this backup".into(),
                    title_color: Some(bk::red_text(t)),
                    sub: None,
                    ruled: false,
                    link: true,
                    trailing: Vec::new(),
                },
                t,
            )
            .on_press(move |s: &mut State| s.backups.ask_delete_device(&delete)),
        ],
        Some(&format!(
            "Moves these copies to Trash for all your devices. {}",
            backs_up_again(device)
        )),
        t,
    ));
    page
}

/// A file row's tile: the shared small tile with the registry's icon for the name.
fn file_tile(index: usize, name: &str, t: &Theme) -> El {
    file_tile::tile(
        &format!("tile-{index}"),
        name,
        TilePicture::Icon(icon_for(name, false, None)),
        name,
        None,
        RowLook::default(),
        TileSize::Small,
        t,
    )
}

/// DW-0741: what could not back up, and what never does. Returns the children and how many
/// buttons they hold.
fn failures_page(source: &dyn BackupsSource, t: &Theme) -> (Vec<El>, usize) {
    let all = source.not_backed_up();
    let failed: Vec<&NotBackedUp> = all.iter().filter(|f| !f.excluded).collect();
    let excluded: Vec<&NotBackedUp> = all.iter().filter(|f| f.excluded).collect();
    let mut children = vec![
        shell::crumb("Backups", t, |s: &mut State| s.backups.show(Page::Section)),
        shell::heading(
            "Couldn't back up",
            Some("Beebeeb stopped trying these after several tries. Everything else is backed up."),
            t,
        ),
    ];
    let mut buttons = 0;
    let mut list = 0;
    if !failed.is_empty() {
        let rows = failed
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let id = f.id.clone();
                let again = controls::button(i, "Try again", Some(&ic::REBUILD), Emphasis::Plain, t)
                    .label(format!("Try again: {}", f.name))
                    .on_press(move |s: &mut State| s.backups.retry(&id));
                bk::row_item(
                    list,
                    i,
                    RowItem {
                        leading: Leading::El(file_tile(i, &f.name, t)),
                        title: f.name.clone(),
                        title_color: None,
                        sub: Some(format!("{} \u{00B7} {}", f.folder, f.why)),
                        ruled: i > 0,
                        link: false,
                        trailing: vec![again],
                    },
                    t,
                )
            })
            .collect();
        buttons = failed.len();
        children.push(lists::list(list, None, rows, None, t));
        children.push(
            row().key("try-all-line").w_full().child(
                bk::button_md(buttons, "Try all again", Some(&ic::REBUILD), Md::Plain, t)
                    .on_press(|s: &mut State| s.backups.retry_all()),
            ),
        );
        buttons += 1;
        list += 1;
    }
    if !excluded.is_empty() {
        let rows = excluded
            .iter()
            .enumerate()
            .map(|(i, f)| {
                bk::row_item(
                    list,
                    i,
                    RowItem {
                        leading: Leading::El(file_tile(failed.len() + i, &f.name, t)),
                        title: f.name.clone(),
                        title_color: None,
                        sub: Some(f.folder.clone()),
                        ruled: i > 0,
                        link: false,
                        trailing: Vec::new(),
                    },
                    t,
                )
            })
            .collect();
        children.push(lists::list(
            list,
            Some("Not backed up, by design"),
            rows,
            Some("Files larger than 4 GB are never backed up from this PC. Everything else in the folders you back up is."),
            t,
        ));
    }
    (children, buttons)
}

/// DW-1410: stop backing up a chosen folder, and what happens to its copy.
fn stop_folder(folder: &Folder, first_button: usize, t: &Theme) -> El {
    let buttons = vec![
        bk::button_md(first_button, "Stop and keep the backup", None, Md::Plain, t)
            .default_button()
            .on_press(|s: &mut State| s.backups.stop_folder(false)),
        bk::button_md(
            first_button + 1,
            "Stop and move the backup to Trash",
            None,
            Md::QuietRed,
            t,
        )
        .on_press(|s: &mut State| s.backups.stop_folder(true)),
        bk::button_md(first_button + 2, "Cancel", None, Md::Quiet, t)
            .on_press(|s: &mut State| s.backups.close_dialog()),
    ];
    let title = format!("Stop backing up {}?", folder.name);
    // The engine has no size of a backed-up folder: the sentence then says what without a number, never a blank.
    let kept = if folder.size.is_empty() {
        "What is already backed up".to_string()
    } else {
        format!("The {} already backed up", folder.size)
    };
    bk::alert(
        &title,
        bk::tile(1, TileTone::Neutral, &ic::FOLDER, 36.0, t),
        &title,
        &format!(
            "New changes in {} stop backing up. {kept} can stay in Backups or go to Trash.",
            folder.name
        ),
        buttons,
        t,
    )
    .modal(false, |s: &mut State| s.backups.close_dialog())
}

/// The password field of DW-1410's second step (focus, tests).
pub(super) const STOP_PASSWORD_KEY: &str = "stop-password";

/// DW-1410's second step (NOT ON THE BOARD, flagged in `frames/this_pc/FRAMES.md`; lead ruling 2026-11-15): moving a
/// chosen folder's backup to Trash proves the ACCOUNT PASSWORD first, drawn as "Delete this backup?" (DW-0742) draws
/// it. Only where the source proves it (`BackupsSource::trash_needs_password`); the board's scene never opens it.
fn stop_folder_password(ui: &BackupsUi, folder: &Folder, first_button: usize, t: &Theme) -> El {
    let rest = " will be moved to Trash. It leaves every device and is deleted for good when Trash empties. \
                Nothing on this PC is touched.";
    let (words, mono) = if folder.size.is_empty() {
        (format!("What is backed up from {}{rest}", folder.name), Vec::new())
    } else {
        let at = "The ".len();
        (
            format!("The {} backed up from {}{rest}", folder.size, folder.name),
            vec![at..at + folder.size.len()],
        )
    };
    // While the move is on its way the button is off (one at a time) and says so.
    let moving = ui.stopping();
    let ready = ui.code_len() > 0 && !moving;
    let refusal = ui.stop_refusal();
    let body = vec![
        bk::para(0, &words, &mono, t),
        col()
            .key("stop-password-room")
            .w_full()
            .pb(14.0)
            .child(crate::ui::account_form::field(
                0,
                &crate::ui::account_form::Field {
                    label: "Account password",
                    input_key: STOP_PASSWORD_KEY,
                    message: refusal,
                    error: refusal.is_some(),
                    ..Default::default()
                },
                &ui.code,
                |s: &mut State| &mut s.backups.code,
                t,
            )),
    ];
    let label = if moving { "Moving\u{2026}" } else { "Move to Trash" };
    let mut trash = bk::button_md(first_button, label, None, Md::Danger(ready), t);
    if ready {
        trash = trash.on_press(|s: &mut State| {
            s.backups.stop_folder_to_trash();
        });
    }
    let foot = vec![
        trash,
        bk::button_md(first_button + 1, "Cancel", None, Md::Plain, t)
            .on_press(|s: &mut State| s.backups.close_dialog()),
    ];
    let title = format!("Move the backup of {} to Trash?", folder.name);
    dialogs::dialog(&title, 420.0, &title, None, body, foot, t).modal(false, |s: &mut State| s.backups.close_dialog())
}

/// The password field of "Delete this backup?" (focus, tests).
pub(super) const PASSWORD_KEY: &str = "delete-password";

/// DW-0742: deleting a device's backup is a risky action, the account password first (owner ruling Q4).
fn delete_device(ui: &BackupsUi, device: &Device, first_button: usize, t: &Theme) -> El {
    let (what, rest) = match device.backed {
        Backed::Unreadable => (
            "This device's backup".to_string(),
            " will be moved to Trash. It leaves every device and is deleted for good when Trash empties. ",
        ),
        backed => (
            backed_words(backed),
            " will be moved to Trash. They leave every device and are deleted for good when Trash empties. ",
        ),
    };
    let rest = format!("{rest}{}", backs_up_again(device));
    let (words, mono) = match &device.size {
        Some(size) => {
            let at = what.len() + " and ".len();
            (format!("{what} and {size}{rest}"), vec![at..at + size.len()])
        }
        None => (format!("{what}{rest}"), Vec::new()),
    };
    // While a delete is on its way the button is off (one at a time) and, in its own device's dialog, says so.
    let deleting = ui.deleting_for(&device.id);
    let ready = ui.code_len() > 0 && !ui.deleting();
    let refusal = ui.delete_refusal();
    let body = vec![
        bk::para(0, &words, &mono, t),
        // The password field keeps the room the board's code field had (14 pt taller), so the buttons stay where the board has them.
        col()
            .key("delete-password-room")
            .w_full()
            .pb(14.0)
            .child(crate::ui::account_form::field(
                0,
                &crate::ui::account_form::Field {
                    label: "Account password",
                    input_key: PASSWORD_KEY,
                    message: refusal,
                    error: refusal.is_some(),
                    ..Default::default()
                },
                &ui.code,
                |s: &mut State| &mut s.backups.code,
                t,
            )),
    ];
    let label = if deleting { "Deleting\u{2026}" } else { "Delete backup" };
    let mut delete = bk::button_md(first_button, label, None, Md::Danger(ready), t);
    if ready {
        delete = delete.on_press(|s: &mut State| {
            s.backups.delete_device();
        });
    }
    let foot = vec![
        delete,
        bk::button_md(first_button + 1, "Cancel", None, Md::Plain, t)
            .on_press(|s: &mut State| s.backups.close_dialog()),
    ];
    dialogs::dialog("Delete this backup?", 420.0, "Delete this backup?", None, body, foot, t)
        .modal(false, |s: &mut State| s.backups.close_dialog())
}

/// The window while settings shows Backups.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let ui = &s.backups;
    let Some(source) = ui.source() else {
        // Nothing answers for Backups yet: the shell's "not built yet" frame, never sample values.
        return placeholder::window(s, cx, registry::group_for(&s.shell.app));
    };
    // Keyboard as Windows has it: Tab visits every control, Space and Enter press the focused one.
    cx.web_focus();
    let t = tokens::theme(s.settings.dark(cx.system_dark()));
    let devices = source.devices();
    let device = |id: &str| devices.iter().find(|d| d.id == id);
    // Mail Backup's page, while a source answers for it; the section otherwise.
    let mail = if ui.page == Page::Mail { source.mail() } else { None };
    let mut mail_lists = 0;
    let (children, buttons) = match &ui.page {
        Page::Device(id) => match device(id) {
            Some(d) => (device_page(d, source.wired(Part::OpenDevice), &t), 0),
            None => section(source, &t),
        },
        Page::Failures => failures_page(source, &t),
        Page::Mail => match &mail {
            Some(mail) => {
                let (children, lists, buttons) = mail_view::page(mail, ui.dialog.is_some(), &t);
                mail_lists = lists;
                (children, buttons)
            }
            None => section(source, &t),
        },
        Page::Section => section(source, &t),
    };
    let mut children = children;
    if let (Page::Section, Some((words, sub))) = (&ui.page, ui.notice()) {
        // What the source refused to do (a folder that did not switch, a delete that was not done): said in the
        // section, for a few seconds.
        children.insert(
            1,
            controls::banner_toned(1, true, &ic::WARN_CIRCLE, words, sub, None, &t),
        );
    }
    let overlay = match &ui.dialog {
        Some(
            Dialog::MailTurnOn
            | Dialog::MailAccount
            | Dialog::MailFolders
            | Dialog::MailTurnOff
            | Dialog::MailKeepFiles,
        ) => mail
            .as_ref()
            .and_then(|mail| mail_view::overlay(ui, source, mail, mail_lists, buttons, &t)),
        Some(Dialog::StopFolder(id)) => source
            .folders()
            .iter()
            .find(|f| &f.id == id && f.chosen_by_you)
            .map(|f| {
                if ui.stop_asks_password() {
                    stop_folder_password(ui, f, buttons, &t)
                } else {
                    stop_folder(f, buttons, &t)
                }
            }),
        Some(Dialog::DeleteDevice(id)) => device(id).map(|d| delete_device(ui, d, buttons, &t)),
        None => None,
    };
    let mut body = row()
        .key("body")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(sidebar(&t))
        .child(shell::pane(children, &t));
    if let Some(overlay) = overlay {
        body = body.child(overlay);
    }
    col()
        .key("window")
        .role("window")
        .label(format!("{}, Settings, Backups", s.identity.main_window_title()))
        .on_key(super::on_key)
        .w_full()
        .h_full()
        .fill(t.surface)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(body)
}

#[cfg(test)]
#[path = "icon_tests.rs"]
mod icon_tests;
