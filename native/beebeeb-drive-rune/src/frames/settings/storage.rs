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
use crate::ui::view_state::{ViewState, try_again, view_state};
use rune::tok::Family;
use rune::{El, col, el, row};

use super::drive::{Control, Insights, Kept, KeptOnPc, UseKind};
use super::{Dialog, SettingsUi};

fn icon_of(kind: UseKind) -> &'static rune::Icon {
    match kind {
        UseKind::Photos => &ic::IMAGE,
        UseKind::Files => &ic::FOLDER,
        UseKind::Previews => &ic::GRID,
        UseKind::Versions => &ic::CLOCK,
    }
}

/// The "Thumbnails and previews" row has a true figure only when previews report a size and a limit: otherwise it
/// would read " of 0 GB", so it stays out.
fn previews_row_shown(shows: bool, previews: &KeptOnPc) -> bool {
    shows && !previews.used.is_empty() && previews.limit.max_gb > 0
}

/// Storage has no answer of its own: its usage arrives with the insights. Loading while that is out, Failed
/// (with a retry that really asks again) when it did not come, never a blank " used of ".
fn unanswered(ui: &SettingsUi, st: &super::drive::Storage, t: &Theme) -> Option<El> {
    let why = match ui.drive.insights() {
        Insights::Loading => {
            return Some(view_state(ViewState::Loading { words: "Loading your storage…" }, t));
        }
        Insights::Failed(why) => why.as_str(),
        _ if st.used.is_empty() || st.total.is_empty() => "Beebeeb could not read how much space you use.",
        _ => return None,
    };
    Some(view_state(
        ViewState::Failed {
            key: "storage-failed",
            title: "Couldn't load your storage",
            why,
            retry: Some(try_again("storage-failed", t).on_press(|s: &mut State| s.settings.drive.reload_insights())),
        },
        t,
    ))
}

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let st = ui.drive.storage();
    if let Some(state) = unanswered(ui, &st, t) {
        return (shell::pane(vec![shell::heading("Storage", None, t), state], t), None);
    }

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
    let mut pc = vec![kept(0, 0, Kept::Downloaded, &ic::DOWNLOAD_TRAY, "Clear…")];
    if previews_row_shown(ui.drive.shows(Control::PreviewsRow), &st.previews) {
        pc.push(kept(1, 2, Kept::Previews, &ic::GRID, "Clear"));
    }
    pc.push(
        lists::item(
            1,
            2,
            &Item::new(Some(&ic::OFFLINE_PIN), "Available offline")
                .trailing(Trailing::Mono(st.offline.clone()))
                .opens(),
            t,
        )
        .on_press(|s: &mut State| super::go(s, "DW-0715")),
    );

    let children = vec![
        shell::heading("Storage", None, t),
        col()
            .key("account-use")
            .w_full()
            .gap(8.0)
            .child(super::list(0, Some("In your Beebeeb account"), account, None, t))
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
        super::list(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::settings::drive::{BoardDrive, DriveSettings};

    #[test]
    fn previews_row_stays_out_without_a_true_figure() {
        let board = BoardDrive::new().storage().previews;
        assert!(previews_row_shown(true, &board));
        assert!(!previews_row_shown(false, &board));
        let mut none = board.clone();
        none.used = String::new();
        assert!(!previews_row_shown(true, &none));
        let mut no_limit = board;
        no_limit.limit.max_gb = 0;
        assert!(!previews_row_shown(true, &no_limit));
    }

    fn keys_with(insights: crate::frames::settings::drive::Insights) -> Vec<String> {
        let mut state = crate::init();
        assert!(crate::frames::settings::scene("DW-0712", &mut state));
        state.settings.drive = Box::new(BoardDrive::new().with_insights(insights));
        let env = rune::Env { system_dark: false, scale: 2.0 };
        let mut rt = rune::Runtime::new(state, crate::view, (958.0, 718.0), env);
        rt.frame(0.0);
        rt.frame(0.016);
        rt.elements().into_iter().filter_map(|e| e.key).collect()
    }

    #[test]
    fn storage_says_loading_and_failed_never_a_blank_figure() {
        use crate::frames::settings::drive::Insights;
        let loading = keys_with(Insights::Loading);
        assert!(loading.iter().any(|k| k == "loading") && !loading.iter().any(|k| k == "usage"));
        let failed = keys_with(Insights::Failed("offline".into()));
        assert!(failed.iter().any(|k| k == "storage-failed-retry") && !failed.iter().any(|k| k == "usage"));
        let ready = keys_with(Insights::Empty);
        assert!(ready.iter().any(|k| k == "usage"));
    }
}
