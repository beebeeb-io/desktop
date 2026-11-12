//! Storage (DW-0712): what uses the account's space, and what Beebeeb keeps on this PC; the
//! two limit dialogs (DW-0713 thumbnails and previews, DW-0714 downloaded files) and the
//! question before downloaded copies are removed (DW-0780).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_dialog::{self as dialogs, DIALOG_TITLE, Weight};
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::settings_meter as meters;
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::settings_shell::{self as shell, PANE_TITLE, SUB};
use rune::tok::Family;
use rune::{El, col, el, row};

use super::drive::{Kept, UseKind};
use super::{Dialog, SettingsUi};

fn icon_of(kind: UseKind) -> &'static rune::Icon {
    match kind {
        UseKind::Photos => &ic::IMAGE,
        UseKind::Files => &ic::FOLDER,
        UseKind::Previews => &ic::GRID,
        UseKind::Versions => &ic::CLOCK,
    }
}

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let st = ui.drive.storage();

    // In your Beebeeb account: the figure and its meter, then what it is made of.
    let head = col()
        .key("usage")
        .role("status")
        .label(format!("{} used of {}", st.used, st.total))
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .child(
            row()
                .key("usage-line")
                .w_full()
                .items_start()
                .child(meters::value_then(
                    "usage-used",
                    &st.used,
                    " used",
                    DIALOG_TITLE,
                    t.text,
                ))
                .child(el().key("usage-gap").grow(1.0))
                .child(el().key("usage-total-box").pt(5.5).child(meters::then_value(
                    "usage-total",
                    "of ",
                    &st.total,
                    SUB,
                    t.text_tertiary,
                ))),
        )
        .child(meters::meter(
            0,
            516.0,
            8.0,
            &[(st.shares[0], t.text), (st.shares[1], t.text_quaternary)],
            t,
        ));
    let mut account = vec![head];
    for (i, (kind, title, size)) in st.kinds.iter().enumerate() {
        let mut it = Item::new(Some(icon_of(*kind)), *title).trailing(Trailing::Mono(size.clone()));
        if i > 0 {
            it = it.ruled();
        }
        account.push(lists::item(0, i, &it, t));
    }
    let n = st.kinds.len();
    account.push(
        lists::item(
            0,
            n,
            &Item::new(Some(&ic::TRASH), "Trash")
                .trailing(Trailing::Mono(st.trash.clone()))
                .opens(),
            t,
        )
        .on_press(|s: &mut State| super::go(s, "DW-0747")),
    );
    account.push(
        lists::item(
            0,
            n + 1,
            &Item::new(Some(&ic::ACTIVITY), "Storage insights")
                .sub("By type, and your largest files")
                .opens(),
            t,
        )
        .on_press(|s: &mut State| super::go(s, "DW-0783")),
    );

    // On this PC: what each cache holds against its limit.
    let kept = |index: usize, first: usize, which: Kept, icon: &'static rune::Icon, clear: &str| {
        let k = match which {
            Kept::Downloaded => &st.downloaded,
            Kept::Previews => &st.previews,
        };
        let asks = which == Kept::Downloaded;
        let clear = controls::button(first, clear, None, Emphasis::Plain, t).on_press(move |s: &mut State| {
            if asks {
                s.settings.dialog = Some(Dialog::FreeUp);
            } else {
                s.settings.drive.clear(which);
                s.settings.toast = Some("Thumbnails and previews cleared");
            }
        });
        let change = controls::button(first + 1, "Change limit", None, Emphasis::Plain, t)
            .on_press(move |s: &mut State| s.settings.open_limit(which));
        let mut it = Item::new(Some(icon), which.title()).sub(format!("{} of {} GB", k.used, k.limit.gb));
        if index > 0 {
            it = it.ruled();
        }
        lists::item_with(
            1,
            index,
            &it,
            vec![
                row()
                    .key(format!("item-1-{index}-actions"))
                    .gap(6.0)
                    .child(clear)
                    .child(change),
            ],
            t,
        )
    };
    let pc = vec![
        kept(0, 0, Kept::Downloaded, &ic::DOWNLOAD_TRAY, "Clear…"),
        kept(1, 2, Kept::Previews, &ic::GRID, "Clear"),
        lists::item(
            1,
            2,
            &Item::new(Some(&ic::OFFLINE_PIN), "Available offline")
                .trailing(Trailing::Mono(st.offline.clone()))
                .opens(),
            t,
        )
        .on_press(|s: &mut State| super::go(s, "DW-0715")),
    ];

    let children = vec![
        shell::heading("Storage", None, t),
        col()
            .key("account-use")
            .w_full()
            .gap(8.0)
            .child(lists::list(0, Some("In your Beebeeb account"), account, None, t))
            .child(
                el().key("cap-0-box").w_full().px(4.0).child(
                    kit::label(
                        "cap-0",
                        "Need more space? Plans are on beebeeb.io.",
                        type_::CAPTION,
                        t.text_tertiary,
                    )
                    .w_full(),
                ),
            ),
        lists::list(
            1,
            Some("On this PC"),
            pc,
            Some(
                "Clearing only removes copies on this PC. Your files stay safe in Beebeeb, and your file list is always kept.",
            ),
            t,
        ),
    ];
    let overlay = match ui.dialog {
        Some(Dialog::Limit(which)) => Some(limit(ui, which, 4, t)),
        Some(Dialog::FreeUp) => Some(free_up(ui, 4, t)),
        _ => ui.toast.map(|words| notices::toast(0, words, t)),
    };
    (shell::pane(children, t), overlay)
}

/// The limit dialog (DW-0713, DW-0714): the figure, a slider over the range this PC allows,
/// and what the limit means. `first` is the index of its first button in the frame.
pub fn limit(ui: &SettingsUi, which: Kept, first: usize, t: &Theme) -> El {
    const W: f32 = 392.0;
    let st = ui.drive.storage();
    let l = match which {
        Kept::Downloaded => st.downloaded.limit,
        Kept::Previews => st.previews.limit,
    };
    let gb = ui.draft_gb.clamp(l.min_gb, l.max_gb);
    let (lo, hi) = (l.min_gb, l.max_gb);
    let range = l.clone();
    let slider = meters::slider(which.title(), W, l.at_for(gb), t)
        .range_value(f64::from(gb), f64::from(lo), f64::from(hi))
        .range_step(1.0)
        .on_step(move |s: &mut State, d: i32, _| {
            let now = i64::from(s.settings.draft_gb) + i64::from(d);
            s.settings.draft_gb = now.clamp(i64::from(lo), i64::from(hi)) as u32;
        })
        .on_step_limit(move |s: &mut State, max: bool, _| s.settings.draft_gb = if max { hi } else { lo })
        .on_drag(move |s: &mut State, drag: &mut rune::Drag, _| {
            if drag.w > 0.0 {
                s.settings.draft_gb = range.gb_at(drag.x / drag.w);
            }
        });
    let mono_cap = |key: &str, gb: u32| kit::label(key, format!("{gb} GB"), type_::KBD, t.text_tertiary);
    let body = vec![
        kit::label(
            "dlg-figure",
            format!("{gb} GB"),
            PANE_TITLE.family(Family::Mono),
            t.text,
        ),
        col().key("slider-box").w_full().gap(4.0).child(slider).child(
            row()
                .key("slider-ends")
                .w_full()
                .child(mono_cap("slider-min", lo))
                .child(el().key("slider-ends-gap").grow(1.0))
                .child(mono_cap("slider-max", hi)),
        ),
        kit::label("dlg-text-0", l.says.clone(), SUB, t.text_tertiary).wrap(),
    ];
    let foot = vec![
        dialogs::button(first, "Save", Weight::Primary, t)
            .default_button()
            .on_press(move |s: &mut State| {
                let gb = s.settings.draft_gb;
                s.settings.drive.set_limit(which, gb);
                s.settings.dialog = None;
            }),
        dialogs::button(first + 1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    dialogs::dialog(which.title(), 440.0, which.title(), None, body, foot, t)
        .modal(false, |s: &mut State| s.settings.dialog = None)
}

/// Free up space (DW-0780): asked before the downloaded copies leave this PC.
pub fn free_up(ui: &SettingsUi, first: usize, t: &Theme) -> El {
    let used = ui.drive.storage().downloaded.used;
    let foot = vec![
        dialogs::button(first, "Free up space", Weight::Primary, t)
            .default_button()
            .on_press(|s: &mut State| {
                s.settings.drive.clear(Kept::Downloaded);
                s.settings.dialog = None;
                s.settings.toast = Some("Downloaded copies removed from this PC");
            }),
        dialogs::button(first + 1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    dialogs::alert(
        "Free up space",
        380.0,
        notices::tile(0, Tone::Neutral, &ic::DOWNLOAD_TRAY, t),
        &format!("Free up {used}?"),
        "Downloaded copies are removed from this PC. Your files stay in Beebeeb and download again when you open them. Available offline files stay.",
        foot,
        t,
    )
    .modal(false, |s: &mut State| s.settings.dialog = None)
}
