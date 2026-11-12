//! Sync history (DW-1301 and its filters DW-1302, DW-1303, DW-0428; the search DW-1304; the
//! empty history DW-1305): every change of the last 30 days, grouped by day, newest first.

use chrono::{Datelike, NaiveDate, Weekday};
use rune::{Align, Color, Cx, El, Icon, TextStyle, col, el, row};

use super::chrome::{self, FilesControls};
use super::source::{Change, Kind, Origin, What};
use super::{Filter, Section, Ui, icons};
use crate::State;
use crate::theme::icons as base;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::kit;

const SEARCH_PLACEHOLDER: &str = "Search history";

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    // Nothing answers for This PC yet (a build that ships before the engine lane wires it):
    // the shell's "not built yet" frame, never sample sync history.
    if !s.this_pc.wired() {
        return crate::shell::placeholder::window(s, cx, crate::shell::registry::group_for(&s.shell.app));
    }
    let t = tokens::theme(cx.system_dark());
    let ui = &s.this_pc;
    let focused = chrome::search_focused(cx);

    let mut right = vec![filter(ui, &t)];
    right.extend(chrome::files_controls(FilesControls::AsFiles, &t));
    right.push(chrome::search_field(ui, focused, SEARCH_PLACEHOLDER, &t));
    let toolbar = chrome::toolbar(
        "Sync history",
        vec![
            chrome::nav(s, ui, &t),
            chrome::title("Sync history", "Last 30 days".into(), &t),
        ],
        right,
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
        main = main.child(problems_bar(&t));
    }
    main = main
        .child(if ui.no_history() { empty(&t) } else { list(ui, &t) })
        .child(chrome::status_line(
            ui,
            "History of the last 30 days, on this PC only".into(),
            &t,
        ));
    super::window(s, cx, Section::SyncHistory, main)
}

/// The filter (`.kit-seg.kit-seg-sm`): one of four, the chosen one raised.
fn filter(ui: &Ui, t: &Theme) -> El {
    const SEG: TextStyle = TextStyle::new(12.0, 550.0).lh(17.0);
    let problems = ui.problem_count();
    let mut seg = row()
        .key("seg-0")
        .role("tablist")
        .label("Show")
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
fn problems_bar(t: &Theme) -> El {
    const WORDS: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
    const TITLE: TextStyle = TextStyle::new(12.5, 600.0).lh(17.0);
    row()
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
        .child(el().key("problems-bar-spacer").grow(1.0))
        .child(
            chrome::outline_button("btn-try-all-again", "Try all again", Some(&icons::RETRY), t)
                .on_press(|s: &mut State| s.this_pc.retry_all()),
        )
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

fn list(ui: &Ui, t: &Theme) -> El {
    let changes = ui.changes();
    let today = ui.today();
    let mut days = col().key("history").w_full();
    let mut day: Option<NaiveDate> = None;
    let mut d = 0;
    // Try again buttons are counted through the frame, as the gate keys them by their words.
    let mut retries = 0;
    for (i, change) in changes.iter().enumerate() {
        if day != Some(change.date) {
            day = Some(change.date);
            let words = day_label(change.date, today);
            days = days.child(
                el().key(format!("day-{d}-box"))
                    .role("heading")
                    .label(words.clone())
                    .w_full()
                    .pt(12.0)
                    .px(12.0)
                    .pb(4.0)
                    .child(kit::label(
                        format!("label-{}", crate::ui::settings_shell::slug(&words)),
                        words.to_uppercase(),
                        type_::SECTION_LABEL,
                        t.text_tertiary,
                    )),
            );
            d += 1;
        }
        if change.problem().is_some() {
            retries += 1;
        }
        days = days.child(change_row(i, change, retries, t));
    }
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
        .child(days)
}

fn kind_icon(change: &Change) -> &'static Icon {
    if matches!(change.what, What::MovedToTrash { .. }) {
        return &base::TRASH;
    }
    match change.kind {
        Kind::Document | Kind::Photo(_) => &base::DOC_GENERIC,
        Kind::Sheet => &base::DOC_SPREADSHEET,
        Kind::Deck => &base::DOC_MINIMAL,
        Kind::Video => &icons::DOC_VIDEO,
        Kind::Audio => &icons::DOC_AUDIO,
        Kind::Folder => &base::FILES,
    }
}

fn origin_icon(origin: &Origin) -> &'static Icon {
    match origin {
        Origin::ThisPc => &icons::LAPTOP,
        Origin::Phone(_) => &icons::PHONE,
        Origin::Tablet(_) => &icons::TABLET,
        Origin::SharedFolder => &base::SHARED,
    }
}

/// A 30 pt tile (`.mk-ftile` in the history): the kind's glyph, or the picture.
fn tile(k: &str, change: &Change, t: &Theme) -> El {
    let tile = el()
        .key(format!("{k}-tile"))
        .size(30.0, 30.0)
        .radius(7.0)
        .clip_rounded(7.0)
        .center();
    if let Kind::Photo(n) = change.kind {
        let (angle, stops) = icons::sample_photo(n);
        let rgb =
            |c: [u8; 3]| rune::color::hex(&format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).unwrap_or(Color::BLACK);
        return tile.linear_gradient(angle.to_radians(), stops.map(|(at, c)| (at, rgb(c))));
    }
    tile.fill(t.tile).child(kit::glyph(
        format!("{k}-icon"),
        kind_icon(change),
        19.0,
        t.text_secondary,
    ))
}

/// One change (`.grid` 30 · rest · 150 · 96 · 48, 44 pt). Keys: `change-<i>` and
/// `change-<i>-tile|icon|name|sub|device-icon|device|time`; a problem's button is
/// `btn-try-again`, the second in the frame `btn-try-again-2` (+ `-icon`, `-label`).
fn change_row(i: usize, change: &Change, retry_no: usize, t: &Theme) -> El {
    let k = format!("change-{i}");
    let line = change.what.line();
    let failed = change.problem().map(str::to_string);
    let shown = change.clone();

    let mut retry = el().key("retry-cell").w(96.0);
    if let Some(problem) = failed.clone() {
        let key = if retry_no > 1 {
            format!("btn-try-again-{retry_no}")
        } else {
            "btn-try-again".to_string()
        };
        retry = retry.child(
            chrome::outline_button(&key, "Try again", Some(&icons::RETRY), t)
                .on_press(move |s: &mut State| s.this_pc.retry(&problem)),
        );
    }

    row()
        .key(k.clone())
        .role("listitem")
        .label(format!(
            "{}, {}, {}, {}",
            change.name,
            line,
            change.origin.label(),
            change.time
        ))
        .h(44.0)
        .w_full()
        .px(12.0)
        .gap(12.0)
        .center_y()
        .radius(8.0)
        .hover(t.hover)
        .focusable(true)
        .on_double_press(move |s: &mut State| s.this_pc.show_change(&shown))
        .child(tile(&k, change, t))
        .child(
            col()
                .key("words")
                .grow(1.0)
                .min_w(0.0)
                .child(
                    kit::label(format!("{k}-name"), change.name.clone(), type_::LIST_ITEM, t.text)
                        .w_full()
                        .truncate(),
                )
                .child(
                    kit::label(
                        format!("{k}-sub"),
                        line,
                        type_::CAPTION,
                        if failed.is_some() {
                            icons::red_text(t)
                        } else {
                            t.text_tertiary
                        },
                    )
                    .w_full()
                    .truncate(),
                ),
        )
        .child(
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
        )
        .child(retry)
        .child(
            kit::label(format!("{k}-time"), change.time.clone(), type_::KBD, t.text_tertiary)
                .w(48.0)
                .align(Align::End),
        )
}

/// DW-1305: a PC that just signed in has no history yet.
fn empty(t: &Theme) -> El {
    super::offline::empty_state(
        &base::SYNC_HISTORY,
        "No changes yet",
        "Changes from this PC and your other devices show here for 30 days.",
        t,
    )
}
