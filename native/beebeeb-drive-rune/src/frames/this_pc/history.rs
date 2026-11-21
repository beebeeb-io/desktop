//! Sync history (DW-1301 and its filters DW-1302, DW-1303, DW-0428; the search DW-1304; the
//! empty history DW-1305): every change of the last 30 days, grouped by day, newest first.

use chrono::{Datelike, NaiveDate, Weekday};
use rune::{Align, Cx, El, Icon, TextStyle, col, el, row};

use super::chrome::{self, FilesControls};
use super::source::{Change, Exit, Kind, Origin};
use super::{Filter, Section, Ui, icons};
use crate::State;
use crate::ui::button;
use crate::theme::icons as base;
use crate::theme::tokens::{self, Theme, type_};
use crate::theme::file_icons::icon_for;
use crate::ui::file_list::ROW_H;
use crate::ui::file_row::{self, RowLook};
use crate::ui::file_tile::{self, TilePicture, TileSize};
use crate::ui::view_state::{self, ViewState};
use crate::ui::kit;

const SEARCH_PLACEHOLDER: &str = "Search history";

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    // This PC turned off (`engine_host::this_pc::TURNED_ON`):
    // the shell's "not built yet" frame, never sample sync history.
    if !s.this_pc.wired() {
        return crate::shell::placeholder::window(s, cx, crate::shell::registry::group_for(&s.shell.app));
    }
    let t = tokens::theme(cx.system_dark());
    let ui = &s.this_pc;

    let toolbar = chrome::toolbar(
        s,
        ui,
        "Sync history",
        "Last 30 days".into(),
        vec![filter(ui, &t)],
        FilesControls::AsFiles,
        chrome::search_field(ui, SEARCH_PLACEHOLDER, &t),
        &t,
    );

    let mut main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar);
    if ui.filter == Filter::Problems && ui.problem_count() > 0 {
        main = main.child(problems_bar(ui.retryable_problem_count() > 0, &t));
    }
    main = main
        .child(match super::offline::not_ready(ui.standing(), &t) {
            // No lists: the section says why, never "No changes yet".
            Some(state) => state,
            None if ui.no_history() => empty(&t),
            None => list(ui, &t),
        })
        .child(chrome::status_line(
            s,
            ui,
            "History of the last 30 days, on this PC only".into(),
            &t,
        ));
    let window = super::window(s, cx, Section::SyncHistory, main);
    // Task 2530: a problem's question, over the window (not on the board: `settle.rs`).
    match &ui.asking {
        Some(asking) => el()
            .key("window-with-ask")
            .w_full()
            .h_full()
            .child(window)
            .child(super::settle::ask(asking, &t)),
        None => window,
    }
}

/// The filter (`.kit-seg.kit-seg-sm`): one of four, the chosen one raised.
fn filter(ui: &Ui, t: &Theme) -> El {
    const SEG: TextStyle = TextStyle::new(12.0, 550.0).lh(17.0);
    let problems = ui.problem_count();
    let mut seg = row()
        .key("seg-0")
        .role("tablist")
        .label("Show")
        .focus_group()
        .padding(2.0)
        .radius(10.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator));
    for f in Filter::ALL {
        let on = ui.filter == f;
        let words = f.label(problems);
        let mut tab = el()
            .key(format!("seg-0-{}", crate::ui::settings_shell::slug(&words)))
            .role("tab")
            .label(words.clone())
            .checked(on)
            .h(24.0)
            .px(10.0)
            .center()
            .radius(8.0)
            .focusable(true)
            .focus_selected(on)
            .on_press(move |s: &mut State| s.this_pc.set_filter(f))
            .child(kit::label(
                format!("seg-0-{}-label", crate::ui::settings_shell::slug(&words)),
                words,
                SEG,
                if on { t.text } else { t.text_secondary },
            ));
        if on {
            tab = tab
                .fill(t.card)
                .box_shadow(kit::drop(1.0, 2.0, t.shadow_raised))
                .box_shadow(kit::ring(kit::HAIR, t.separator));
        } else {
            tab = tab.hover(t.hover).press(t.press);
        }
        seg = seg.child(tab);
    }
    seg
}

/// The bar under the toolbar on the Problems filter (`.mk-bulk`).
fn problems_bar(try_all: bool, t: &Theme) -> El {
    const WORDS: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
    const TITLE: TextStyle = TextStyle::new(12.5, 600.0).lh(17.0);
    let mut bar = row()
        .key("problems-bar")
        .role("group")
        .label("Can't sync")
        .w_full()
        .py(6.0)
        .px(14.0)
        .gap(8.0)
        .center_y()
        .fill(icons::bar_bg(t))
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(kit::label("problems-bar-title", "Can't sync", TITLE, t.text))
        .child(kit::label(
            "problems-bar-text",
            "These didn't sync. Everything else is up to date.",
            WORDS,
            t.text_secondary,
        ))
        .child(el().key("problems-bar-spacer").grow(1.0));
    // Task 2524: held edits have no Try again; the bar offers it only while something can be tried again.
    if try_all {
        bar = bar.child(
            button::button(button::keys("btn-try-all-again"), "Try all again", Some(&icons::RETRY), button::Variant::Secondary, button::Size::Sm, button::ButtonState::Enabled, t)
                .on_press(|s: &mut State| s.this_pc.retry_all()),
        );
    }
    bar
}

/// "Today", "Yesterday", then the day in words: "Sunday 1 Nov".
pub fn day_label(date: NaiveDate, today: NaiveDate) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    match (today - date).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => {
            let weekday = match date.weekday() {
                Weekday::Mon => "Monday",
                Weekday::Tue => "Tuesday",
                Weekday::Wed => "Wednesday",
                Weekday::Thu => "Thursday",
                Weekday::Fri => "Friday",
                Weekday::Sat => "Saturday",
                Weekday::Sun => "Sunday",
            };
            format!("{weekday} {} {}", date.day(), MONTHS[date.month0() as usize])
        }
    }
}

/// One line of the history list: a day's heading, or a change (its index among the changes) and its Try again number.
enum Line {
    Day(String, usize),
    Change(usize, usize),
}

fn list(ui: &Ui, t: &Theme) -> El {
    let changes = std::rc::Rc::new(ui.changes());
    let today = ui.today();
    let mut lines = Vec::new();
    let mut day: Option<NaiveDate> = None;
    let mut d = 0;
    // Try again buttons are counted through the frame, as the gate keys them by their words.
    let mut retries = 0;
    for (i, change) in changes.iter().enumerate() {
        if day != Some(change.date) {
            day = Some(change.date);
            lines.push(Line::Day(day_label(change.date, today), d));
            d += 1;
        }
        if change.can_retry() {
            retries += 1;
        }
        lines.push(Line::Change(i, retries));
    }
    let lines = std::rc::Rc::new(lines);
    let t = *t;
    let count = lines.len();
    let (keys, built) = (lines.clone(), (lines, changes));
    col()
        .key("history-scroll")
        .role("list")
        .label("Sync history")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .py(8.0)
        .px(16.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(
            col()
                .key("history")
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    count,
                    ROW_H,
                    move |_: &State, i| match keys.get(i) {
                        Some(Line::Day(_, d)) => std::borrow::Cow::Owned(format!("day-{d}-box")),
                        Some(Line::Change(c, _)) => std::borrow::Cow::Owned(format!("change-{c}")),
                        None => std::borrow::Cow::Borrowed("row-oob"),
                    },
                    move |_: &State, i| match built.0.get(i) {
                        Some(Line::Day(words, _)) => day_heading(words, &t),
                        Some(Line::Change(c, retry_no)) => change_row(*c, &built.1[*c], *retry_no, built.1.len(), &t),
                        None => el().key("empty"),
                    },
                ),
        )
}

/// A day's heading, one row high.
fn day_heading(words: &str, t: &Theme) -> El {
    row()
        .key(format!("label-{}-box", crate::ui::settings_shell::slug(words)))
        .role("heading")
        .label(words)
        .w_full()
        .h(ROW_H)
        .px(14.0)
        .items_end()
        .pb(4.0)
        .child(kit::label(
            format!("label-{}", crate::ui::settings_shell::slug(words)),
            words.to_uppercase(),
            type_::SECTION_LABEL,
            t.text_tertiary,
        ))
}

fn origin_icon(origin: &Origin) -> &'static Icon {
    match origin {
        Origin::ThisPc => &icons::LAPTOP,
        Origin::Phone(_) => &icons::PHONE,
        Origin::Tablet(_) => &icons::TABLET,
        Origin::SharedFolder => &base::SHARED,
    }
}

/// One change (`.grid` 30 · rest · 150 · 96 · 48, 44 pt). Keys: `change-<i>` and
/// `change-<i>-tile|icon|name|sub|device-icon|device|time`; a problem's button is
/// `btn-try-again`, the second in the frame `btn-try-again-2` (+ `-icon`, `-label`). Task 2530: the button is only
/// a way out the engine listed for the row: Try again; else Choose (`change-<i>-choose`), which asks the two-versions
/// question; else Remove (`change-<i>-remove`), which asks once. A row with none has no button.
fn change_row(i: usize, change: &Change, retry_no: usize, count: usize, t: &Theme) -> El {
    let k = format!("change-{i}");
    let line = change.what.line();
    let failed = change.problem().map(str::to_string);
    let shown = change.clone();

    let mut retry = el().key("retry-cell").w(96.0);
    // A held edit is a problem with no Try again (task 2524); a parked one has the ways out the engine lists (2530).
    let exits = change.exits();
    if let Some(problem) = failed.clone().filter(|_| change.can_retry()) {
        let key = if retry_no > 1 {
            format!("btn-try-again-{retry_no}")
        } else {
            "btn-try-again".to_string()
        };
        retry = retry.child(
            button::button(button::keys(&key), "Try again", Some(&icons::RETRY), button::Variant::Secondary, button::Size::Sm, button::ButtonState::Enabled, t)
                .on_press(move |s: &mut State| s.this_pc.retry(&problem)),
        );
    } else if let Some(problem) = failed.clone().filter(|_| exits.iter().any(|exit| exit.is_keep())) {
        retry = retry.child(
            button::button(button::keys(&format!("{k}-choose")), "Choose", None, button::Variant::Secondary, button::Size::Sm, button::ButtonState::Enabled, t)
                .on_press(move |s: &mut State| s.this_pc.ask_exit(&problem, false)),
        );
    } else if let Some(problem) = failed.clone().filter(|_| exits.contains(&Exit::RemoveFromList)) {
        retry = retry.child(
            button::button(button::keys(&format!("{k}-remove")), "Remove", None, button::Variant::Secondary, button::Size::Sm, button::ButtonState::Enabled, t)
                .on_press(move |s: &mut State| s.this_pc.ask_exit(&problem, true)),
        );
    }

    let icon = icon_for(&change.name, matches!(change.kind, Kind::Folder), None);
    let tile = file_tile::tile(
        &format!("{k}-tile"),
        &change.name,
        TilePicture::Icon(icon),
        &change.name,
        None,
        RowLook::default(),
        TileSize::Small,
        t,
    );
    let cells = vec![
        kit::label(
            format!("{k}-sub"),
            line.clone(),
            type_::CAPTION,
            if failed.is_some() { icons::red_text(t) } else { t.text_tertiary },
        )
        .grow(1.0)
        .min_w(0.0)
        .truncate(),
        row()
            .key("device-cell")
            .w(150.0)
            .min_w(0.0)
            .gap(6.0)
            .center_y()
            .child(kit::glyph(
                format!("{k}-device-icon"),
                origin_icon(&change.origin),
                14.0,
                t.text_tertiary,
            ))
            .child(
                kit::label(
                    format!("{k}-device"),
                    change.origin.label().to_string(),
                    type_::BODY.lh(15.0),
                    t.text_tertiary,
                )
                .truncate(),
            ),
        retry,
        kit::label(format!("{k}-time"), change.time.clone(), type_::KBD, t.text_tertiary)
            .w(48.0)
            .align(Align::End),
    ];
    let label = format!("{}, {}, {}, {}", change.name, line, change.origin.label(), change.time);
    let look = RowLook {
        position: Some((i + 1, count)),
        ..RowLook::default()
    };
    file_row::row_with(&k, &label, tile, &change.name, Vec::new(), cells, look, t)
        .on_double_press(move |s: &mut State| s.this_pc.show_change(&shown))
}

/// DW-1305: a PC that just signed in has no history yet.
fn empty(t: &Theme) -> El {
    col().key("empty-area").grow(1.0).min_h(0.0).w_full().center().child(view_state::view_state(
        ViewState::Empty {
            key: "empty",
            icon: &base::SYNC_HISTORY,
            title: "No changes yet",
            body: "Changes from this PC and your other devices show here for 30 days.",
            action: None,
        },
        t,
    ))
}

#[cfg(test)]
#[path = "icon_tests.rs"]
mod icon_tests;
