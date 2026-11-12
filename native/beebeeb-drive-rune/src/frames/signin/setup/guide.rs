//! The first-time guide (DW-0380 and DW-0301 to DW-0305): a coach card over the main window, with
//! a ring around the real control each tip explains. It starts when the first run ends ("Open my
//! Beebeeb") and from Settings › General › "Show the guide again".
//!
//! [`Guide`] is the whole state (`State::guide`): which step shows, or none. The shell draws
//! [`over`] on top of the frame of the window; nothing under it is covered for the pointer (the
//! ring and its dim take no press), so the window stays usable while a tip shows.
//!
//! A tip belongs to a place (Files, Shared, Photos). While the window shows another place, a
//! viewer, settings or a dialog, the tip is not drawn and takes no key; it is there again when the
//! window is back on its place.
//!
//! Nothing here is a secret.

use std::f32::consts::FRAC_PI_4;

use rune::{Color, Cx, El, Shadow, TextStyle, Tx, col, el, row};

use crate::State;
use crate::shell::app::{Screen, Section};
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::kit;

/// DW-0380's heading.
pub const TRAY_TITLE: &str = "Beebeeb lives in the notification area";
/// DW-0380's words, as the board DRAWS them. The frame's note in `frames.json` has other words
/// ("Click the b to see what's syncing. Closing this window keeps Beebeeb running."): the
/// contradiction is with the owner (decision 2478); a ruling changes this one line.
pub const TRAY_TEXT: &str = "Windows puts a new icon under ^ at first. Drag the b next to the clock to keep it in view, or turn Beebeeb on in Windows Settings \u{203A} Personalization \u{203A} Taskbar \u{203A} Other system tray icons.";

/// The words of the five tips (DW-0301 to DW-0305).
pub const TIPS: [&str; 5] = [
    "Add files and photos here. They're encrypted before they upload. You can also drop files from File Explorer.",
    "Right-click a file to share it, move it or keep it on this PC.",
    "Files and folders you've shared, or that others shared with you.",
    "Photos your iPhone and iPad backed up. New ones arrive on their own.",
    "Storage, devices, backups and app lock. Ctrl+, opens them, and you can see this guide again there.",
];

/// The card (`.mk-coach`), its ring (`.mk-ring`), and the two buttons.
pub const COACH_KEY: &str = "coach";
pub const RING_KEY: &str = "coach-ring";
pub const NEXT_KEY: &str = "coach-next";
pub const SKIP_KEY: &str = "coach-skip";

/// Tip 2 points at the first file of the folder, when it is among this many first rows (the rows
/// a window at its smallest size shows without scrolling); otherwise tip 2 is left out, as it is
/// for an account with no file yet (DW-0302's rule).
pub const TIP_2_ROWS: usize = 10;

/// `.t-body`: a tip's words.
const TIP: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// `.t-sub`: DW-0380's words.
const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// `.t-cap.font-mono`: "1 of 5".
const COUNTER: TextStyle = type_::KBD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// DW-0380: where the b is.
    Tray,
    /// DW-0301 to DW-0305: tip 1 to 5.
    Tip(u8),
}

impl Step {
    /// The section of the window the step sits over.
    pub fn place(self) -> Section {
        match self {
            Step::Tip(3) => Section::Shared,
            Step::Tip(4) => Section::Photos,
            _ => Section::Files,
        }
    }

    /// The board frame that draws the step.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn frame(self) -> &'static str {
        match self {
            Step::Tray => "DW-0380",
            Step::Tip(1) => "DW-0301",
            Step::Tip(2) => "DW-0302",
            Step::Tip(3) => "DW-0303",
            Step::Tip(4) => "DW-0304",
            Step::Tip(_) => "DW-0305",
        }
    }
}

/// The guide's state: the step that shows, or none (the guide is not running).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Guide {
    pub step: Option<Step>,
}

/// The row tip 2 points at: the first file among the folder's first [`TIP_2_ROWS`] rows.
pub fn file_row(s: &State) -> Option<usize> {
    s.listing.items.iter().take(TIP_2_ROWS).position(|item| !item.is_folder)
}

/// Start the guide on Files, at DW-0380. From the first run's end and from Settings.
pub fn start(s: &mut State) {
    show(s, Step::Tray);
}

fn show(s: &mut State, step: Step) {
    let on_place =
        matches!(s.shell.app.screen(), Screen::Drive(route) if route.section == step.place() && route.item.is_none());
    if !on_place {
        crate::frames::go(s, step.place());
    }
    s.guide.step = Some(step);
}

/// "Got it", "Next" and "Done": the next step (its view comes by itself), or the end on Files.
pub fn next(s: &mut State) {
    let Some(step) = s.guide.step else { return };
    let to = match step {
        Step::Tray => Some(Step::Tip(1)),
        Step::Tip(1) if file_row(s).is_some() => Some(Step::Tip(2)),
        Step::Tip(1) | Step::Tip(2) => Some(Step::Tip(3)),
        Step::Tip(3) => Some(Step::Tip(4)),
        Step::Tip(4) => Some(Step::Tip(5)),
        Step::Tip(_) => None,
    };
    match to {
        Some(to) => show(s, to),
        // "Done": "Ends the guide on Files DW-0401".
        None => skip(s),
    }
}

/// "Skip" and Esc: the whole guide ends; the window stays where it is.
pub fn skip(s: &mut State) {
    s.guide.step = None;
}

/// The step the window shows now: the guide's step, while the window is on its place with
/// nothing in front.
pub fn showing(s: &State) -> Option<Step> {
    let step = s.guide.step?;
    let app = &s.shell.app;
    let on_place =
        matches!(app.screen(), Screen::Drive(route) if route.section == step.place() && route.item.is_none());
    (on_place && app.dialog().is_none() && s.tray.is_none()).then_some(step)
}

/// Esc is Skip while a tip shows. `true`: the key was the guide's. For `shell::host::shell_keys`.
pub fn escape(s: &mut State) -> bool {
    let shown = showing(s).is_some();
    if shown {
        skip(s);
    }
    shown
}

/// "Open my Beebeeb" at the end of the first run starts the guide ("then the guide (DW-0380)").
/// Reads only that intent; a row of "You're set up" stays queued for its own reader. For
/// `shell::host::poll`. `true`: the guide started.
pub fn poll(s: &mut State) -> bool {
    let Some(setup) = s.setup.as_mut() else { return false };
    let before = setup.intents.len();
    setup.intents.retain(|intent| *intent != super::Intent::OpenDrive);
    if setup.intents.len() == before {
        return false;
    }
    s.guide.step = Some(Step::Tray);
    true
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// Which way the card's arrow points, and where along the card's edge its tip is.
#[derive(Clone, Copy)]
enum Arrow {
    /// The card is under its target; `ax` from the card's left edge.
    Up(f32),
    /// The card is right of its target; `ay` from the card's top edge.
    Left(f32),
    /// The card is over its target.
    Down(f32),
}

/// One tip's geometry, from the board's CSS: the target's key, how far the ring stands out from
/// the target on each side (left, top, right, bottom), the ring's corner radius, and the arrow.
struct Spot {
    target: String,
    out: [f32; 4],
    radius: f32,
    arrow: Arrow,
}

fn spot(n: u8, s: &State) -> Option<Spot> {
    Some(match n {
        1 => Spot {
            target: "new-btn".into(),
            out: [4.0; 4],
            radius: 10.0,
            arrow: Arrow::Up(202.0),
        },
        2 => Spot {
            target: format!("row-{}", file_row(s)?),
            out: [0.0; 4],
            radius: 7.0,
            arrow: Arrow::Up(40.0),
        },
        // The board's ring around a sidebar row stands 1 over it and 3 under it.
        3 => Spot {
            target: "Shared-row".into(),
            out: [2.0, 1.0, 1.5, 3.0],
            radius: 8.0,
            arrow: Arrow::Left(20.0),
        },
        4 => Spot {
            target: "Photos-row".into(),
            out: [2.0, 1.0, 1.5, 3.0],
            radius: 8.0,
            arrow: Arrow::Left(20.0),
        },
        // The board's ring stands 9.5 over the gear and ends 1.5 above its lower edge.
        _ => Spot {
            target: "settings-btn".into(),
            out: [4.5, 9.5, 3.5, -1.5],
            radius: 9.0,
            arrow: Arrow::Down(32.0),
        },
    })
}

/// The card's 12 pt square on its edge (`.mk-coach::before`): the arrow.
fn arrow(arrow: Arrow, t: &Theme) -> El {
    let (left, top, bottom, hair) = match arrow {
        Arrow::Up(ax) => (Some(ax), Some(-6.0), None, (-kit::HAIR, -kit::HAIR)),
        Arrow::Left(ay) => (Some(-6.0), Some(ay), None, (-kit::HAIR, kit::HAIR)),
        Arrow::Down(ax) => (Some(ax), None, Some(-6.0), (kit::HAIR, kit::HAIR)),
    };
    el().key("coach-arrow")
        .pin(left, top, None, bottom)
        .w(12.0)
        .h(12.0)
        .fill(t.card)
        .box_shadow(Shadow {
            x: hair.0,
            y: hair.1,
            blur: 0.0,
            spread: 0.0,
            color: t.separator,
            inset: false,
        })
        .rotate(FRAC_PI_4)
        .a11y_hidden()
}

/// A 22 pt button of the card (`.kit-btn.kit-btn-sm`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ghost,
    Outline,
    Amber,
}

fn button(key: &str, label: &str, kind: Kind, t: &Theme) -> El {
    let b = row()
        .key(key.to_string())
        .role("button")
        .label(label)
        .h(22.0)
        .px(12.0)
        .center()
        .radius(6.0)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer);
    let (b, ink) = match kind {
        Kind::Ghost => (b.hover(t.hover).press(t.press), t.text_secondary),
        Kind::Outline => (
            b.fill(t.paper)
                .border(1.0, t.card_border)
                .border_layout()
                .hover(t.paper2)
                .press(t.paper3),
            t.text,
        ),
        Kind::Amber => (
            b.fill(t.accent)
                .hover(t.accent.mix(Color::BLACK, 0.05))
                .press(t.accent.mix(Color::BLACK, 0.10)),
            t.accent_text_on,
        ),
    };
    b.child(kit::label(
        format!("{key}-label"),
        label,
        type_::BUTTON_SM.lh(17.1429),
        ink,
    ))
}

fn go_on(s: &mut State, tx: &mut Tx) {
    next(s);
    if s.guide.step.is_some() {
        tx.focus(NEXT_KEY, tx.focus_visible());
    }
}

fn card(width: f32, name: &str, t: &Theme) -> El {
    col()
        .key(COACH_KEY)
        .role("group")
        .label(name)
        .w(width)
        .pt(14.0)
        .px(14.0)
        .pb(12.0)
        .gap(10.0)
        .radius(12.0)
        .fill(t.card)
        .box_shadow(kit::drop(16.0, 44.0, Color::BLACK.alpha(0.26)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
}

/// DW-0380's card: the heading, the words, "Got it". A view of its own (no place, no arrow
/// target), so a window of its own by the notification area can hold it unchanged (task 2479).
pub fn tray_card(t: &Theme) -> El {
    card(300.0, TRAY_TITLE, t)
        .child(arrow(Arrow::Down(150.0), t))
        .child(kit::label("coach-title", TRAY_TITLE, type_::HEAD, t.text))
        .child(
            kit::label("coach-text", TRAY_TEXT, SUB, t.text_secondary)
                .wrap()
                .w_full(),
        )
        .child(
            row()
                .justify_end()
                .child(button(NEXT_KEY, "Got it", Kind::Amber, t).on_press_tx(go_on)),
        )
}

/// A tip's card: the words, the five dots, "n of 5", Skip (not on the last) and Next or Done.
fn tip_card(n: u8, arrow_at: Arrow, t: &Theme) -> El {
    let last = n == 5;
    let mut dots = row().key("coach-dots").gap(6.0).a11y_hidden();
    for i in 0..5u8 {
        let on = i + 1 == n;
        dots = dots.child(
            el().key(format!("coach-dot-{i}"))
                .w(6.0)
                .h(6.0)
                .radius(3.0)
                .fill(if on { t.text_secondary } else { t.text.alpha(0.18) }),
        );
    }
    let mut foot = row()
        .items_center()
        .gap(8.0)
        .child(dots)
        .child(kit::label(
            "coach-counter",
            format!("{n} of 5"),
            COUNTER,
            t.text_tertiary,
        ))
        .child(el().grow(1.0));
    if !last {
        foot = foot.child(button(SKIP_KEY, "Skip", Kind::Ghost, t).on_press(|s: &mut State| skip(s)));
    }
    foot = foot.child(button(NEXT_KEY, if last { "Done" } else { "Next" }, Kind::Outline, t).on_press_tx(go_on));
    card(280.0, &format!("Guide, tip {n} of 5"), t)
        .child(arrow(arrow_at, t))
        .child(
            kit::label("coach-text", TIPS[usize::from(n) - 1], TIP, t.text)
                .wrap()
                .w_full(),
        )
        .child(foot)
}

/// The ring around the target, and the dim over everything else (`.mk-ring`: "0 0 0 2px ink-2,
/// 0 0 0 9999px black 22%"), in window points. The dim ends under the caption, which keeps its own
/// colours (the board's overlay is the window's body). It takes no press: what is under it stays
/// usable.
fn ring(rect: [f32; 4], radius: f32, t: &Theme) -> El {
    el().key("coach-dim")
        .pin(Some(0.0), Some(crate::TITLEBAR_H), Some(0.0), Some(0.0))
        .clip_rounded(0.0)
        .inert()
        .a11y_hidden()
        .child(
            el().key(RING_KEY)
                .absolute(rect[0], rect[1] - crate::TITLEBAR_H)
                .w(rect[2])
                .h(rect[3])
                .radius(radius)
                // The ring is drawn over the dim, as the first of two CSS shadows is.
                .box_shadow(kit::ring(9999.0, Color::BLACK.alpha(0.22)))
                .box_shadow(kit::ring(2.0, t.text_secondary)),
        )
}

/// The frame of the window with the guide's step over it, when one shows. For
/// `shell::registry::view`: ONE call around the frame behind.
pub fn over(s: &State, cx: &mut Cx<State>, behind: El) -> El {
    let Some(step) = showing(s) else { return behind };
    let t = tokens::theme(cx.system_dark());
    let layer = el().key("window-with-guide").w_full().h_full().child(behind);
    let n = match step {
        // Interim (the lead, 2026-11-12): the board draws this card on the desktop, by the b;
        // until it has a window of its own it stands in the main window at the board's offsets.
        Step::Tray => {
            return layer.child(
                el().key("coach-place")
                    .pin(None, None, Some(94.0), Some(120.0))
                    .child(tray_card(&t)),
            );
        }
        Step::Tip(n) => n,
    };
    let Some(spot) = spot(n, s) else { return layer };
    // Where the real control is in the window now (a row moves when its list scrolls). Unknown
    // until it is laid out once: the view runs again then.
    let Some([x, y, w, h]) = cx.window_layout(&spot.target) else {
        return layer;
    };
    let [l, top, r, b] = spot.out;
    let rect = [x - l, y - top, w + l + r, h + top + b];
    let card = tip_card(n, spot.arrow, &t);
    // The card's corner, from the ring (the board's distances).
    let place = el().key("coach-place");
    let placed = match spot.arrow {
        // Under the ring: the arrow's tip under the ring's centre (tip 1), or 120 in (tip 2).
        Arrow::Up(ax) => {
            let (left, gap) = if n == 2 {
                (rect[0] + 74.0, 14.0)
            } else {
                (rect[0] + rect[2] / 2.0 - ax - 6.0, 13.0)
            };
            place.absolute(left, rect[1] + rect[3] + gap).child(card)
        }
        Arrow::Left(_) => place.absolute(rect[0] + rect[2] + 13.0, rect[1] - 9.0).child(card),
        // Over the ring: the card's lower edge 12 above it.
        Arrow::Down(ax) => place
            .absolute(rect[0] + rect[2] / 2.0 - ax - 6.0, rect[1] - 12.0)
            .child(card.pin(Some(0.0), None, None, Some(0.0))),
    };
    layer.child(ring(rect, spot.radius, &t)).child(placed)
}

// ---------------------------------------------------------------------------
// Scenes: each board frame
// ---------------------------------------------------------------------------

/// Files with nothing selected and the details closed, as the board draws tips 1, 2 and 5.
fn on_files(s: &mut State, step: Step) {
    s.clear_selection();
    s.show_detail_pane = false;
    s.guide.step = Some(step);
}

/// The six board frames of the guide, for `frames::scenes`.
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    // The board's desktop holds the window as DW-0401 draws it (a file selected, its details).
    ("DW-0380", |s| s.guide.step = Some(Step::Tray)),
    ("DW-0301", |s| on_files(s, Step::Tip(1))),
    ("DW-0302", |s| on_files(s, Step::Tip(2))),
    ("DW-0303", |s| {
        crate::frames::shared::scenes::dw_0501(s);
        s.guide.step = Some(Step::Tip(3));
    }),
    ("DW-0304", |s| {
        crate::frames::photos::set_scene(s, "DW-0601");
        s.guide.step = Some(Step::Tip(4));
    }),
    ("DW-0305", |s| on_files(s, Step::Tip(5))),
];
