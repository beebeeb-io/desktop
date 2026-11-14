//! What Backups draws: the section (DW-0737, DW-1411), one device's backup (DW-0739), what
//! could not back up (DW-0741), and the two dialogs (DW-1410, DW-0742).

use rune::{Cx, El, col, row};

use super::source::{BackupsSource, Device, DeviceKind, FileKind, Folder, NotBackedUp, Status};
use super::{BackupsUi, Dialog, Page, mail_view};
use crate::State;
use crate::frames::settings;
use crate::frames::this_pc::icons as pc_icons;
use crate::shell::{placeholder, registry};
use crate::theme::icons as drive_icons;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{self, Theme};
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

/// The settings sidebar with Backups selected. The same items, in the same order, as
/// `frames::settings` draws them; a press goes through `settings::go`, which keeps the
/// shell's route in step.
fn sidebar(t: &Theme) -> El {
    const SETTINGS: &[(&str, &rune::Icon, &str)] = &[
        ("General", &ic::GEAR, "DW-0701"),
        ("Account", &ic::PEOPLE, "DW-A51"),
        ("Security", &ic::SHIELD, "DW-A52"),
        ("Devices and sessions", &ic::LAPTOP, "DW-A55"),
        ("Notifications", &ic::BELL, "DW-0750"),
    ];
    const DRIVE: &[(&str, &rune::Icon, &str)] = &[
        ("Storage", &ic::CHEV_RIGHT, "DW-0712"),
        ("Backups", &ic::CHEV_RIGHT, "DW-0737"),
        ("Sync and File Explorer", &ic::CHEV_RIGHT, "DW-0746"),
        ("Thumbnails and previews", &ic::CHEV_RIGHT, "DW-0744"),
        ("Bandwidth", &ic::CHEV_RIGHT, "DW-0785"),
    ];
    let items = |sections: &'static [(&'static str, &'static rune::Icon, &'static str)]| -> Vec<El> {
        sections
            .iter()
            .map(|&(label, icon, frame)| {
                let item = shell::nav_item(label, icon, 16.0, label == "Backups", t);
                if label == "Backups" {
                    // Already here: back to the section's own page.
                    item.on_press(|s: &mut State| s.backups.show(Page::Section))
                } else {
                    item.on_press(move |s: &mut State| settings::go(s, frame))
                }
            })
            .collect()
    };
    let back = shell::nav_item("Back to Drive", &ic::BACK, 15.0, false, t)
        .shortcut_hint("Esc")
        .on_press(|s: &mut State| settings::go(s, "DW-0401"));
    shell::sidebar(
        back,
        vec![("Settings", items(SETTINGS)), ("Drive", items(DRIVE))],
        "Settings open inside the app. Esc goes back.",
        t,
    )
}

/// The status card: all backed up with Back up now, or a backup running with its bar.
/// Returns how many buttons it drew.
fn status_card(status: &Status, t: &Theme) -> (El, usize) {
    match status {
        Status::AllBackedUp { last, from_this_pc } => {
            let words = format!("Last backup {last} \u{00B7} {from_this_pc} from this PC");
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

fn device_row(index: usize, device: &Device, t: &Theme) -> El {
    let icon = match device.kind {
        DeviceKind::Phone => &ic::PHONE,
        DeviceKind::Tablet => &ic::TABLET,
    };
    let when = match (&device.last_backup, device.signed_in) {
        (_, false) => "Not signed in".to_string(),
        (Some(last), true) => last.clone(),
        (None, true) => String::new(),
    };
    let mut sub = format!("{} photos", thousands(device.photos));
    if !when.is_empty() {
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
    let (card, mut buttons) = status_card(&source.status(), t);
    let mut children = vec![shell::heading("Backups", None, t), card];

    let failures = source.not_backed_up().iter().filter(|f| !f.excluded).count();
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
    rows.push(
        lists::item(
            0,
            folders.len(),
            &Item::new(Some(&drive_icons::PLUS), "Add a folder\u{2026}").role(Role::Link),
            t,
        )
        .on_press(|s: &mut State| s.backups.add_folder()),
    );
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
    children.push(controls::banner(
        0,
        &ic::CLOUD,
        "OneDrive may back up Desktop, Documents and Pictures too",
        "With OneDrive's folder backup on, these folders live in the OneDrive folder: add it with Add a folder\u{2026} to back it up.",
        None,
        t,
    ));

    let devices = source.devices();
    if !devices.is_empty() {
        children.push(lists::list(
            1,
            Some("Other devices"),
            devices.iter().enumerate().map(|(i, d)| device_row(i, d, t)).collect(),
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

/// DW-0739: another device's backup.
fn device_page(device: &Device, t: &Theme) -> Vec<El> {
    let backed_up = match &device.size {
        Some(size) => format!("{} photos \u{00B7} {size}", thousands(device.photos)),
        None => format!("{} photos", thousands(device.photos)),
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
    facts.push(lists::item(
        0,
        facts.len(),
        &Item::new(None, "Status")
            .trailing(Trailing::Value(
                if device.signed_in { "Signed in" } else { "Not signed in" }.into(),
            ))
            .ruled(),
        t,
    ));
    let (photos, files, delete) = (device.id.clone(), device.id.clone(), device.id.clone());
    vec![
        shell::crumb("Backups", t, |s: &mut State| s.backups.show(Page::Section)),
        shell::heading(&device.name, None, t),
        lists::list(0, None, facts, None, t),
        lists::list(
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
        ),
        lists::list(
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
                "Moves these copies to Trash for all your devices. Photos still on the {} may back up again unless backup is off there.",
                device.model
            )),
            t,
        ),
    ]
}

fn file_glyph(kind: FileKind) -> &'static rune::Icon {
    match kind {
        FileKind::Video => &pc_icons::DOC_VIDEO,
        FileKind::Document => &ic::DOCUMENT,
        FileKind::Sheet => &drive_icons::DOC_SPREADSHEET,
        FileKind::Image => &ic::IMAGE,
    }
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
                        leading: Leading::El(bk::tile(i, TileTone::Neutral, file_glyph(f.kind), 28.0, t)),
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
                        leading: Leading::El(bk::tile(
                            failed.len() + i,
                            TileTone::Neutral,
                            file_glyph(f.kind),
                            28.0,
                            t,
                        )),
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
    bk::alert(
        &title,
        bk::tile(1, TileTone::Neutral, &ic::FOLDER, 36.0, t),
        &title,
        &format!(
            "New changes in {} stop backing up. The {} already backed up can stay in Backups or go to Trash.",
            folder.name, folder.size
        ),
        buttons,
        t,
    )
    .modal(false, |s: &mut State| s.backups.close_dialog())
}

/// DW-0742: deleting a device's backup is a risky action, a code first.
fn delete_device(ui: &BackupsUi, device: &Device, first_button: usize, t: &Theme) -> El {
    let photos = format!("{} photos", thousands(device.photos));
    let rest = format!(
        " will be moved to Trash. They leave every device and are deleted for good when Trash empties. Photos still on the {} may back up again unless backup is off there.",
        device.model
    );
    let (words, mono) = match &device.size {
        Some(size) => {
            let at = photos.len() + " and ".len();
            (format!("{photos} and {size}{rest}"), vec![at..at + size.len()])
        }
        None => (format!("{photos}{rest}"), Vec::new()),
    };
    let ready = ui.code_len() == bk::CODE_LEN;
    let body = vec![
        bk::para(0, &words, &mono, t),
        bk::code_boxes("Authenticator code", ui.code_digits(), t),
    ];
    let mut delete = bk::button_md(first_button, "Delete backup", None, Md::Danger(ready), t);
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
            Some(d) => (device_page(d, &t), 0),
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
            .map(|f| stop_folder(f, buttons, &t)),
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
