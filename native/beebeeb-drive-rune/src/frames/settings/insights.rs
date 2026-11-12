//! Storage insights (DW-0783): where the storage goes, by type and by the largest files;
//! loading (DW-0783b), could not load (DW-0783c), nothing stored yet (DW-0783d).

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::settings_card as cards;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_meter as meters;
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::settings_shell::{self as shell, SUB};
use rune::tok::Family;
use rune::{El, TextStyle, col, el, row};

use super::drive::{Insights, InsightsData};
use super::{Intent, SettingsUi};

/// 24px/650 mono on a 30px line, tracked -0.02em: a figure (`.mk-fig-v`).
pub const FIGURE: TextStyle = TextStyle::new(24.0, 650.0)
    .family(Family::Mono)
    .lh(30.0)
    .tracking(-0.48);
/// 12px/500 on a 15px line: a kind's name (`.t-sub` in ink 2, medium).
const KIND: TextStyle = TextStyle::new(12.0, 500.0).lh(15.0);
/// 12px mono: a path, and its size in semibold (`.mk-lrow .p`, `.s`).
const PATH: TextStyle = TextStyle::new(12.0, 400.0).family(Family::Mono).lh(15.5);
const SIZE: TextStyle = TextStyle::new(12.0, 600.0).family(Family::Mono).lh(15.5);

/// The inner width of a card on the pane's column (540 less 16 each side).
const INNER: f32 = 508.0;

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let mut children = vec![
        shell::crumb("Storage", t, |s: &mut State| super::go(s, "DW-0712")),
        shell::heading(
            "Storage insights",
            Some(
                "Where your storage goes, by type and by your largest files. Worked out on this PC, from files that are already encrypted.",
            ),
            t,
        ),
    ];
    match ui.drive.insights() {
        Insights::Loading => children.extend(loading(t)),
        Insights::Failed(why) => children.push(failed(&why, t)),
        Insights::Empty => children.push(notices::empty_sized(
            0,
            0,
            &ic::CLOUD,
            "Nothing is stored yet",
            "Once you add files, you'll see where your storage goes, by type and by your largest files.",
            333.09,
            t,
        )),
        Insights::Ready(data) => children.extend(ready(&data, t)),
    }
    (shell::pane(children, t), None)
}

/// A caption in mono (`.t-cap > .font-mono`), numbered within its card.
fn cap(card: usize, index: usize, words: &str, t: &Theme) -> El {
    kit::label(format!("card-{card}-cap-{index}"), words, type_::KBD, t.text_tertiary)
}

fn ready(d: &InsightsData, t: &Theme) -> Vec<El> {
    let spacer = |key: &str| el().key(key.to_string()).grow(1.0);

    // Encrypted storage: the figure, the plan, the meter, and the meter in words.
    let usage = cards::column(0, 16.0, 10.0, t)
        .role("status")
        .label(format!("Encrypted storage: {} of {}", d.used, d.total))
        .child(
            row()
                .key("card-0-top")
                .w_full()
                .items_start()
                .child(
                    col()
                        .key("card-0-figure")
                        .gap(4.0)
                        .child(kit::label(
                            "card-0-cap-0",
                            "Encrypted storage",
                            type_::CAPTION,
                            t.text_tertiary,
                        ))
                        .child(
                            row()
                                .key("card-0-figure-line")
                                .gap(8.0)
                                .items_start()
                                .child(kit::label("fig-0", d.used.clone(), FIGURE, t.text))
                                .child(el().key("card-0-sub-box").pt(12.0).child(meters::then_value(
                                    "card-0-sub",
                                    "of ",
                                    &d.total,
                                    SUB,
                                    t.text_tertiary,
                                ))),
                        ),
                )
                .child(spacer("card-0-top-gap"))
                .child(meters::chip(0, Some(&ic::SHIELD), &d.plan, t)),
        )
        .child(meters::meter(0, INNER, 9.0, &[(d.share, t.text)], t))
        .child(
            row()
                .key("card-0-words")
                .w_full()
                .child(cap(0, 1, &d.used_says, t))
                .child(spacer("card-0-words-gap"))
                .child(cap(0, 2, &d.free_says, t)),
        );

    // By type: each kind's bar against the largest kind's; the largest is darkest.
    let inks = [t.text_secondary, t.text_tertiary, t.text_quaternary];
    let mut by_type = cards::column(1, 16.0, 14.0, t).child(
        row()
            .key("card-1-top")
            .w_full()
            .items_start()
            .child(kit::label("card-1-head", "By type", type_::HEAD, t.text))
            .child(spacer("card-1-top-gap"))
            .child(el().key("card-1-cap-0-box").pt(2.0).child(cap(1, 0, &d.totals, t))),
    );
    for (i, kind) in d.kinds.iter().enumerate() {
        // The first name is also the card's own second line in the board's export.
        let name_key = if i == 0 {
            kit::note_text("insbar-0-name", "card-1-sub", &kind.name, KIND, t.text_secondary);
            "card-1-sub".to_string()
        } else {
            format!("insbar-{i}-name")
        };
        by_type = by_type.child(
            col()
                .key(format!("kind-{i}"))
                .role("listitem")
                .label(format!("{}: {}", kind.name, kind.says))
                .w_full()
                .gap(6.0)
                .child(
                    row()
                        .key(format!("kind-{i}-line"))
                        .w_full()
                        .items_start()
                        .child(kit::label(name_key, kind.name.clone(), KIND, t.text_secondary))
                        .child(spacer(&format!("kind-{i}-gap")))
                        .child(
                            el().key(format!("kind-{i}-says-box"))
                                .pt(1.0)
                                .child(cap(1, i + 1, &kind.says, t)),
                        ),
                )
                .child(meters::bar(i, INNER, kind.share, inks[i.min(inks.len() - 1)], t)),
        );
    }

    // Largest files: the path, its kind, its size; a row shows the file in Files.
    let mut rows = col().key("card-2-rows").w_full().clip_rounded(10.0).child(
        row()
            .key("card-2-top")
            .w_full()
            .items_start()
            .pt(12.0)
            .px(16.0)
            .pb(8.0)
            .child(kit::label("card-2-head", "Largest files", type_::HEAD, t.text))
            .child(spacer("card-2-top-gap"))
            .child(
                el().key("card-2-cap-0-box")
                    .pt(2.0)
                    .child(cap(2, 0, &d.largest_says, t)),
            ),
    );
    for (i, file) in d.largest.iter().enumerate() {
        let path = file.path.clone();
        rows = rows.child(
            row()
                .key(format!("lrow-{i}"))
                .role("link")
                .label(format!("{}, {}, {}", file.path, file.kind, file.size))
                .help(file.path.clone())
                .w_full()
                .h(36.0)
                .px(16.0)
                .gap(12.0)
                .center_y()
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(move |s: &mut State| {
                    s.settings.ask(Intent::ShowInFiles(path.clone()));
                    super::go(s, "DW-0401");
                })
                .child(
                    kit::label(format!("lrow-{i}-path"), file.shown.clone(), PATH, t.text_secondary)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(meters::chip(i + 1, None, &file.kind, t))
                .child(
                    kit::label(format!("lrow-{i}-size"), file.size.clone(), SIZE, t.text)
                        .w(72.0)
                        .align(rune::Align::End),
                ),
        );
    }
    if let Some((words, size)) = &d.more {
        rows = rows.child(
            row()
                .key("lrow-more-row")
                .w_full()
                .h(30.0)
                .px(16.0)
                .center_y()
                .child(meters::then_value("lrow-more", words, size, type_::CAPTION, t.text_tertiary).w(412.0)),
        );
    }
    let largest = cards::column(2, 0.0, 0.0, t).child(rows);
    vec![usage, by_type, largest]
}

/// The page's shape while it loads (DW-0783b): three cards of skeletons.
fn loading(t: &Theme) -> Vec<El> {
    let shapes: [[(Option<f32>, f32); 3]; 3] = [
        [(Some(120.0), 10.0), (Some(220.0), 24.0), (None, 26.0)],
        [(Some(120.0), 10.0), (Some(INNER * 0.7), 11.0), (None, 50.0)],
        [(Some(120.0), 10.0), (Some(INNER * 0.7), 11.0), (None, 80.0)],
    ];
    shapes
        .iter()
        .enumerate()
        .map(|(c, lines)| {
            let mut card = cards::column(c, 16.0, 12.0, t).role("status").label("Loading");
            for (i, (w, h)) in lines.iter().enumerate() {
                card = card.child(meters::skeleton(c * 3 + i, *w, *h, t));
            }
            card
        })
        .collect()
}

/// Could not load (DW-0783c): said plainly, with the engine's reason and one way to try again.
fn failed(why: &str, t: &Theme) -> El {
    const HEAD: &str = "Couldn't load your insights";
    let again = controls::button(0, "Try again", None, Emphasis::Plain, t)
        .on_press(|s: &mut State| s.settings.drive.reload_insights());
    cards::card(0, t)
        .items_start()
        .role("alert")
        .label(format!("{HEAD}. {why}"))
        .child(notices::tile(0, Tone::Red, &ic::WARN_CIRCLE, t))
        .child(
            col()
                .key("card-0-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(cards::head(0, HEAD, t))
                .child(kit::label("card-0-sub", why, SUB.family(Family::Mono), t.text_tertiary))
                .child(row().key("card-0-actions").pt(8.0).child(again)),
        )
}
