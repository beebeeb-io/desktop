//! The first-time guide (DW-0380 and DW-0301 to DW-0305): a coach card over the main window, with
//! a ring around the real control each tip explains. It starts when the first run ends ("Open my
//! Beebeeb") and from Settings › General › "Show the guide again".
//!
//! [`Guide`] is the whole state (`State::guide`): which step shows, or none. The shell draws
//! [`over`] on top of the frame of the window; nothing under it is covered for the pointer (the
//! ring and its dim take no press), so the window stays usable while a tip shows.
//!
//! A tip belongs to a place (Files, Shared, Photos). While the window shows another place, a
//! viewer, settings, or anything the place has in front of itself (a menu, a dialog, a name being
//! typed, a photo opened), the tip is not drawn and takes no key: what is in front gets Esc first.
//! The tip is there again when the window is back on its place with nothing in front.
//!
//! While a tip shows, the keyboard is the card's: focus goes to its button, Tab moves between
//! Skip and Next, Enter presses the focused one, Esc is Skip.
//!
//! The guide ends when the account signs out, locks or changes: another account never resumes it,
//! and nothing is kept. Finished ("Done") or skipped (Skip, Esc), it is done for that ACCOUNT on this
//! PC and does not start by itself again (`guide_marks`; owner ruling 2026-11-13, a deviation from
//! the board's "Shown once per PC", recorded in `FRAMES.md`).
//!
//! Nothing here is a secret.

use std::f32::consts::FRAC_PI_4;

use rune::{Color, Cx, El, Key, KeyEvent, Shadow, TextStyle, Tx, col, el, row};

use super::guide_marks;
use crate::State;
use crate::shell::app::{GateState, Screen, Section};
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
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Guide {
    pub step: Option<Step>,
    /// The row that was selected when the guide started: tip 2 points at it when it is a file.
    pub row: Option<usize>,
    /// The account the guide started for. Another account never sees its step.
    pub(super) account: Option<String>,
    /// The step changed: focus goes to the card's button once it is drawn ([`poll`]).
    focus: bool,
    /// The last poll saw a tip on screen. A tip that comes back (what was in front of it closed) takes the
    /// keyboard again.
    shown: bool,
}

/// The row tip 2 points at: the file that was selected when the guide started (the board points
/// at the file its Files frame has selected), otherwise the first file; among the folder's first
/// [`TIP_2_ROWS`] rows.
pub fn file_row(s: &State) -> Option<usize> {
    let is_file = |i: &usize| *i < TIP_2_ROWS && s.listing.items.get(*i).is_some_and(|item| !item.is_folder);
    if let Some(row) = s.guide.row.filter(is_file) {
        return Some(row);
    }
    s.listing.items.iter().take(TIP_2_ROWS).position(|item| !item.is_folder)
}

/// Start the guide on Files, at DW-0380. From the first run's end and from Settings.
pub fn start(s: &mut State) {
    s.guide.row = s.selected;
    s.guide.account = s.shell.app.account().map(str::to_string);
    show(s, Step::Tray);
}

fn show(s: &mut State, step: Step) {
    let on_place =
        matches!(s.shell.app.screen(), Screen::Drive(route) if route.section == step.place() && route.item.is_none());
    if !on_place {
        crate::frames::go(s, step.place());
    }
    s.guide.step = Some(step);
    s.guide.focus = true;
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

/// "Skip", Esc and "Done": the whole guide ends; the window stays where it is. The person saw it or chose not to:
/// that is kept for the account the guide ran for (`guide_marks`: once per account on this PC). Only here: a guide
/// that ends because the account locked, signed out or changed ([`poll`]) keeps nothing.
pub fn skip(s: &mut State) {
    if s.guide.step.is_some() && s.guide.account.as_deref() == s.shell.app.account() {
        let account = s.guide.account.take();
        s.guide_marks.mark_done(account.as_deref());
    }
    s.guide = Guide::default();
}

/// Whether the place under a tip has something of its own in front: a menu, a dialog, a name
/// being typed in a row, an opened photo, search. It gets the keys (Esc closes it), not the tip.
fn in_front(s: &State, place: Section) -> bool {
    // Search takes Esc for itself ("Esc: ends search") on every place it shows.
    if crate::frames::search::is_open(s) {
        return true;
    }
    match place {
        // The Files window's own list of what lies over it, and the New file dialog (DW-1521), which is the
        // Files window's too and drawn by `viewers::new_file`: open, or asked for and opened by the next poll (a
        // key that arrives in between is not the tip's).
        Section::Files => {
            crate::frames::files::has_in_front(s)
                || s.viewer.new_file.is_some()
                || s.actions
                    .intents
                    .contains(&crate::frames::files::Intent::Frame("DW-1521"))
        }
        Section::Shared => s.frames.shared.menu.is_some() || s.frames.shared.dialog.is_some(),
        Section::Photos => s.photos.menu.is_some() || s.photos.dialog.is_some() || s.photos.viewer.is_some(),
        _ => false,
    }
}

/// The step the window shows now: the guide's step, while the window is on its place with
/// nothing in front.
pub fn showing(s: &State) -> Option<Step> {
    let step = s.guide.step?;
    let app = &s.shell.app;
    let on_place =
        matches!(app.screen(), Screen::Drive(route) if route.section == step.place() && route.item.is_none());
    // A session the server ended draws its own state over the window (DW-1109): no tip over that, and no step.
    (on_place && !s.session_ended() && app.dialog().is_none() && s.tray.is_none() && !in_front(s, step.place()))
        .then_some(step)
}

/// The keys of a tip that shows: Esc is Skip, Tab moves between the card's buttons (it stays in
/// the card), Enter presses the focused one. `true`: the key was the guide's. The card's own
/// key handler (focus is in the card), and `shell::host::shell_keys` before the shell's own keys.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(step) = showing(s) else { return false };
    let m = ev.mods;
    if m.cmd || m.ctrl || m.alt || m.logo {
        return false;
    }
    match ev.key {
        Key::Escape if !m.shift => skip(s),
        Key::Tab => {
            // Skip is on tips 1 to 4.
            let has_skip = matches!(step, Step::Tip(n) if n < 5);
            let to = if has_skip && tx.focused() == Some(NEXT_KEY) {
                SKIP_KEY
            } else {
                NEXT_KEY
            };
            tx.focus(to, true);
        }
        Key::Enter if !m.shift => match tx.focused() {
            Some(NEXT_KEY) => go_on(s, tx),
            Some(SKIP_KEY) => skip(s),
            // Focus is on something of the window's (the person clicked there): its Enter.
            _ => return false,
        },
        _ => return false,
    }
    true
}

/// Runs every frame (`shell::host::poll`). The guide ends when the account signs out, locks or
/// changes (nothing is kept then); when the first run ends ("Open my Beebeeb": "then the guide
/// (DW-0380)") it asks whether the signed-in account has done the guide, and starts it on a definite
/// "not done" for that account; and focus goes to the card's button after a step changed, and when a
/// tip comes back from behind what was in front of it. `true`: something changed.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    if s.guide.step.is_some() {
        let gone = matches!(s.shell.app.gate(), GateState::SignedOut | GateState::Locked)
            || s.shell.app.account() != s.guide.account.as_deref();
        if gone {
            s.guide = Guide::default();
            changed = true;
        }
    }
    let account = s.shell.app.account().map(str::to_string);
    // The first run's own flag, not its `Intent::OpenDrive`: that intent stays for the shell.
    if s.setup.as_mut().is_some_and(super::Ui::just_finished) {
        s.guide_marks.ask(account.as_deref());
    }
    // Also the source's poll. Only "not done", said of the account that is signed in now, starts the guide.
    if s.guide_marks.answer(account.as_deref()) == Some(guide_marks::Known::NotDone)
        && account.is_some()
        && s.guide.step.is_none()
    {
        s.guide = Guide {
            step: Some(Step::Tray),
            row: None,
            account,
            focus: true,
            shown: false,
        };
        changed = true;
    }
    let shows = showing(s).is_some();
    if shows && (s.guide.focus || !s.guide.shown) {
        s.guide.focus = false;
        tx.focus(NEXT_KEY, tx.focus_visible());
    }
    s.guide.shown = shows;
    changed
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
    /// The ring's own width, where the board gives one: the ring then stands from the target's right edge, and
    /// its left edge does not move with the target's measured width.
    width: Option<f32>,
    radius: f32,
    arrow: Arrow,
}

fn spot(n: u8, s: &State) -> Option<Spot> {
    Some(match n {
        // The board places the ring at whole points (left 964, top 37, 88 x 36) around New
        // (967.73, 40.75, 80.27 x 28): 3.73 left of it, 3.75 over it, 4 right of it, 4.25 under it. New's label is
        // measured by the font at hand (80.5 wide on one machine), so the ring is the board's 88 wide from 4 right
        // of New: its left edge is the board's 964 whatever the label measures.
        1 => Spot {
            target: "new-btn".into(),
            out: [3.73, 3.75, 4.0, 4.25],
            width: Some(88.0),
            radius: 10.0,
            arrow: Arrow::Up(202.0),
        },
        2 => Spot {
            target: format!("row-{}", file_row(s)?),
            out: [0.0; 4],
            width: None,
            radius: 7.0,
            arrow: Arrow::Up(40.0),
        },
        // The board's ring around a sidebar row stands 1 over it and 3 under it.
        3 => Spot {
            target: "Shared-row".into(),
            out: [2.0, 1.0, 1.5, 3.0],
            width: None,
            radius: 8.0,
            arrow: Arrow::Left(20.0),
        },
        4 => Spot {
            target: "Photos-row".into(),
            out: [2.0, 1.0, 1.5, 3.0],
            width: None,
            radius: 8.0,
            arrow: Arrow::Left(20.0),
        },
        // The board's ring stands 9.5 over the gear and ends 1.5 above its lower edge.
        _ => Spot {
            target: "settings-btn".into(),
            out: [4.5, 9.5, 3.5, -1.5],
            width: None,
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
        s.guide.focus = false;
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
        // With focus on one of its buttons the card gets the key before Rune's own Tab walk.
        .on_key(on_key)
}

/// DW-0380's card: the heading, the words, "Got it". A view of its own (no place, no arrow
/// target), so a window of its own by the notification area can hold it unchanged (task 2479).
pub fn tray_card(t: &Theme) -> El {
    card(300.0, TRAY_TITLE, t)
        .child(arrow(Arrow::Down(150.0), t))
        .child(kit::label("coach-title", TRAY_TITLE, type_::HEAD, t.text))
        .child(
            kit::label("coach-text", TRAY_TEXT, SUB, t.text_tertiary)
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
        // Interim (the lead, 2026-11-12): the board draws this card on the desktop, by the b, where
        // it covers only the window's lower right corner. Until it has a window of its own (task
        // 2479) it stands in that corner of the main window, over the status line's end, clear of
        // the details panel's buttons.
        Step::Tray => {
            return layer.child(
                el().key("coach-place")
                    .pin(None, None, Some(16.0), Some(44.0))
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
    // Never narrower than the target with its right-hand distance.
    let l = spot.width.map_or(l, |width| (width - w - r).max(0.0));
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
    at(s, step);
}

/// The guide on `step`, for the account that is signed in.
fn at(s: &mut State, step: Step) {
    s.guide.step = Some(step);
    s.guide.account = s.shell.app.account().map(str::to_string);
}

/// The six board frames of the guide, for `frames::scenes`.
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    // The board's desktop holds the window as DW-0401 draws it (a file selected, its details).
    ("DW-0380", |s| at(s, Step::Tray)),
    ("DW-0301", |s| on_files(s, Step::Tip(1))),
    ("DW-0302", |s| {
        // The file DW-0401 has selected ("Contract draft.pdf"), as when the guide starts there.
        s.guide.row = s.selected;
        on_files(s, Step::Tip(2));
    }),
    ("DW-0303", |s| {
        crate::frames::shared::scenes::dw_0501(s);
        at(s, Step::Tip(3));
    }),
    ("DW-0304", |s| {
        crate::frames::photos::set_scene(s, "DW-0601");
        at(s, Step::Tip(4));
    }),
    ("DW-0305", |s| on_files(s, Step::Tip(5))),
];
