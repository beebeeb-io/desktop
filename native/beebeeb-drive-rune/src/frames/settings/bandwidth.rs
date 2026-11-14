//! Bandwidth (DW-0785): what Beebeeb sends and receives on this PC now and over the last 20
//! hours, each device's sync, the two speed limits, the metered switch, and the speed test.
//! States: nothing moving (DW-0785b), a limit on (DW-0785c), the limit menu (DW-0785d),
//! nothing recorded yet (DW-0785e), waiting on a metered connection (DW-0785f); the speed
//! test running (DW-0786, DW-0786b), its result (DW-0786c), and its failure (DW-0786d).
//!
//! The pane is ONE page that scrolls. Seven board frames draw a part of it only (`Crop`);
//! the app never sets a crop, a scene does.

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::settings_card as cards;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Role, Trailing};
use crate::ui::settings_meter as meters;
use crate::ui::settings_notice::{self as notices, PILL, Tone};
use crate::ui::settings_shell::{self as shell, SUB};
use rune::tok::Family;
use rune::{Align, Color, El, LineCap, Shape, Side, Span, TextStyle, canvas, col, el, oklch, rich, row};

use super::bandwidth_data::{
    Bandwidth, Device, DeviceKind, DeviceState, Direction, Flow, History, PRESET_LABELS, PRESETS, STEP_SECONDS,
    SpeedTest, Step, limit_label,
};
use super::drive::BoardDrive;
use super::{Menu, MenuKind, SettingsUi};

/// Where each limit's menu goes: the point its row marks (the Limits list is the pane's second).
pub const UPLOAD_MENU_AT: &str = "item-1-0-menu-at";
pub const DOWNLOAD_MENU_AT: &str = "item-1-1-menu-at";
/// The menu's top-left corner from its row's own (DW-0785d).
const MENU_AT: (f32, f32) = (436.0, -5.0);

/// 11.5px/600 on a 17px line: a figure's name (`.mk-fig-k`, `.mk-step-k`).
const FIG_NAME: TextStyle = TextStyle::new(11.5, 600.0).lh(17.0);
/// 24px/650 mono on a 30px line, tracked -.02em: a live figure (`.mk-fig-v`).
const FIG: TextStyle = TextStyle::new(24.0, 650.0)
    .family(Family::Mono)
    .lh(30.0)
    .tracking(-0.48);
/// 12px/500 mono: a live figure's unit (`.mk-fig-v small`).
const FIG_UNIT: TextStyle = TextStyle::new(12.0, 500.0).family(Family::Mono).lh(16.0);
/// 11.5px/400 on a 15px line: what a figure is about (`.mk-fig-s`).
const FIG_SAYS: TextStyle = TextStyle::new(11.5, 400.0).lh(15.0);
/// 10px/400 mono: the chart's axis (`svg text`).
const AXIS_Y: TextStyle = TextStyle::new(10.0, 400.0).family(Family::Mono).lh(12.0);
/// 10.5px/400 mono: the chart's times (`.mk-chart-x`).
const AXIS_X: TextStyle = TextStyle::new(10.5, 400.0).family(Family::Mono).lh(13.5);
/// 11px/500 mono on a 15px line: the chart's hover note (`.mk-chart-tip`).
const TIP: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(15.0);
/// 13px/500 on a 17px line: a device's name (`.mk-drow-tx b`).
const DEVICE: TextStyle = TextStyle::new(13.0, 500.0).lh(17.0);
/// 11.5px/400 mono on a 15px line: a device's line (`.mk-drow-tx span`).
const DEVICE_SAYS: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(15.0);
/// 18px/650 mono on a 24px line: a step's figure (`.mk-step-v`).
const STEP: TextStyle = TextStyle::new(18.0, 650.0).family(Family::Mono).lh(24.0);
/// 11px/500 mono: a step's unit (`.mk-step-v small`).
const STEP_UNIT: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);
/// 11px/400 mono: the line under a step's figure (`.mk-step-s`).
const STEP_SAYS: TextStyle = TextStyle::new(11.0, 400.0).family(Family::Mono).lh(14.5);

/// The chart's plot, in points (the board's `svg`), and the room above its tallest value.
const PLOT_W: f32 = 448.0;
const PLOT_H: f32 = 104.0;
const PLOT_TOP: f32 = 4.0;

const SPEED_HINT: &str = "5 pings, then 3 uploads and 3 downloads of 10 MB \u{b7} 30 to 90 s";

/// The part of the pane a board frame draws. The app draws `Whole`, always.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Crop {
    #[default]
    Whole,
    /// The heading, Right now and Last 20 hours (DW-0785b, DW-0785e).
    Head,
    /// The same and the Limits (DW-0785f).
    HeadAndLimits,
    /// Devices, Limits and the speed test, without what is above them (DW-0785d, DW-0786 to d).
    FromDevices,
}

/// What the Bandwidth pane holds between draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Held {
    /// The bucket of the chart under the pointer.
    pub tip: Option<usize>,
    /// Set by a scene only.
    pub crop: Crop,
}

/// Show the board frame `frame` on `s`. Every state is reached through what the pane's own
/// controls call; only the crop and the pointer's place are set here.
pub fn scene(s: &mut State, frame: &'static str) {
    let board = BoardDrive::new();
    let (drive, crop) = match frame {
        "DW-0785b" => (board.with_bandwidth_idle(), Crop::Head),
        "DW-0785e" => (board.with_no_traffic_recorded(), Crop::Head),
        "DW-0785f" => (board.with_metered_wait(), Crop::HeadAndLimits),
        "DW-0785" | "DW-0785c" => (board, Crop::Whole),
        // The test as Run test starts it, and one and two steps later.
        "DW-0786" => (board.with_speed_test_at(0.0), Crop::FromDevices),
        "DW-0786b" => (board.with_speed_test_at(STEP_SECONDS), Crop::FromDevices),
        "DW-0786c" => (board.with_speed_test_at(2.0 * STEP_SECONDS), Crop::FromDevices),
        "DW-0786d" => (
            board.with_beebeeb_unreachable().with_speed_test_at(STEP_SECONDS),
            Crop::FromDevices,
        ),
        _ => (board, Crop::FromDevices),
    };
    s.settings.drive = Box::new(drive);
    super::open_on(s, frame);
    let ui = &mut s.settings;
    ui.bandwidth = Held { tip: None, crop };
    let upload = MenuKind::SpeedLimit(Direction::Upload);
    match frame {
        // The board draws the pointer on the chart at 06:20.
        "DW-0785" => ui.bandwidth.tip = Some(50),
        "DW-0785c" => ui.choose(upload, 1),
        // The board draws the pointer over 1 Mbps.
        "DW-0785d" => ui.menu = Some(Menu { kind: upload, hot: 1 }),
        _ => {}
    }
}

/// What arrived since the last frame (the speed test's steps). The shell calls it at the start
/// of every frame; while a test runs it asks for another frame, so the steps show without a press.
pub fn poll(s: &mut State, seconds: f64, tx: &mut rune::Tx) -> bool {
    let (changed, more) = s.settings.drive.pump(seconds);
    if more {
        tx.wake_after(std::time::Duration::from_millis(100));
    }
    changed
}

/// The index of a direction's limit among the presets. `None`: a limit an older version
/// saved that is not a preset (the menu then holds it as a ninth item of its own, checked).
pub fn chosen(ui: &SettingsUi, direction: Direction) -> Option<usize> {
    let bw = ui.drive.bandwidth();
    let limit = match direction {
        Direction::Upload => bw.upload_limit,
        Direction::Download => bw.download_limit,
    };
    PRESETS.iter().position(|preset| *preset == limit)
}

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let bw = ui.drive.bandwidth();
    let crop = ui.bandwidth.crop;
    let mut children = Vec::new();
    if crop != Crop::FromDevices {
        children.push(shell::heading(
            "Bandwidth",
            Some(
                "What Beebeeb sends and receives on this PC, how fast it may go, and how fast this PC reaches Beebeeb.",
            ),
            t,
        ));
        children.push(right_now(&bw, t));
        children.push(last_20_hours(&bw.history, ui.bandwidth.tip, t));
    }
    // Lists are numbered in the order the pane draws them.
    let mut list = 0;
    if matches!(crop, Crop::Whole | Crop::FromDevices) {
        children.push(devices(list, &bw.devices, t));
        list += 1;
    }
    if crop != Crop::Head {
        children.push(limits(list, &bw, ui.menu, ui.drive.metered_control(), t));
        list += 1;
    }
    if matches!(crop, Crop::Whole | Crop::FromDevices) {
        children.push(speed_test(list, &bw.test, t));
    }
    let overlay = [Direction::Upload, Direction::Download]
        .into_iter()
        .find_map(|direction| limit_menu(ui, direction, t));
    (shell::pane(children, t), overlay)
}

/// A speed's unit, said in full ("MB/s" is read "megabytes per second").
fn unit_words(unit: &str) -> &str {
    match unit {
        "MB/s" => "megabytes per second",
        "KB/s" => "kilobytes per second",
        "B/s" => "bytes per second",
        "Mbps" => "megabits per second",
        "ms" => "milliseconds",
        other => other,
    }
}

/// A tint of the ink under a figure (`color-mix(in oklch, var(--color-ink) 4%, transparent)`).
fn wash(t: &Theme) -> Color {
    t.text.alpha(0.04)
}

/// `--color-green-text`.
fn green_text(t: &Theme) -> Color {
    if t.dark {
        oklch(0.72, 0.16, 155.0)
    } else {
        oklch(0.46, 0.13, 155.0)
    }
}

/// A figure and its unit on one baseline: the unit sits `drop` points lower than the figure's
/// box starts, `gap` after it.
fn figure(key: &str, value: &str, unit: &str, styles: (TextStyle, TextStyle), gap: f32, drop: f32, t: &Theme) -> El {
    let mut line =
        row()
            .key(format!("{key}-line"))
            .items_start()
            .child(kit::label(key.to_string(), value, styles.0, t.text));
    if !unit.is_empty() {
        line = line.child(
            kit::label(format!("{key}-unit"), unit, styles.1, t.text_tertiary)
                .ml(gap)
                .mt(drop),
        );
    }
    line
}

/// Right now (`.kit-card`): the two live figures, and how many devices are syncing.
fn right_now(bw: &Bandwidth, t: &Theme) -> El {
    let tile = |i: usize, direction: Direction, flow: &Flow| {
        let (icon, ink) = match direction {
            Direction::Upload => (&ic::ARROW_UP, t.accent_deep),
            Direction::Download => (&ic::ARROW_DOWN, t.text_secondary),
        };
        let name = direction.label();
        let said = match &flow.speed {
            Some((value, unit)) => format!("{name}, {value} {}. {}", unit_words(unit), flow.says),
            None => format!("{name}, nothing is moving. {}", flow.says),
        };
        let value = match &flow.speed {
            Some((value, unit)) => figure(&format!("fig-{i}"), value, unit, (FIG, FIG_UNIT), 4.0, 11.3, t),
            // An idle direction is a dash, never "0 B/s".
            None => figure(&format!("fig-{i}"), "\u{2014}", "", (FIG, FIG_UNIT), 0.0, 0.0, t),
        };
        col()
            .key(format!("fig-{i}-tile"))
            .role("group")
            .label(said)
            .w(248.0)
            .gap(3.0)
            .py(12.0)
            .px(14.0)
            .radius(12.0)
            .fill(wash(t))
            .child(
                row()
                    .key(format!("fig-{i}-name-row"))
                    .gap(6.0)
                    .center_y()
                    .child(kit::glyph(format!("fig-{i}-icon"), icon, 14.0, ink))
                    .child(kit::label(format!("fig-{i}-name"), name, FIG_NAME, t.text_secondary)),
            )
            .child(value)
            .child(kit::label(format!("fig-{i}-says"), flow.says.clone(), FIG_SAYS, t.text_tertiary).wrap())
    };
    cards::column(0, 16.0, 12.0, t)
        .role("group")
        .label("Right now")
        .child(
            row()
                .key("card-0-top")
                .w_full()
                .justify_between()
                .items_start()
                .child(kit::label("card-0-head", "Right now", type_::HEAD, t.text))
                // The two share a baseline (`items-baseline`).
                .child(kit::label("card-0-now", bw.now.clone(), type_::CAPTION, t.text_tertiary).mt(2.2)),
        )
        .child(
            row()
                .key("card-0-figs")
                .w_full()
                .gap(12.0)
                .child(tile(0, Direction::Upload, &bw.upload))
                .child(tile(1, Direction::Download, &bw.download)),
        )
}

/// A swatch and a total in the chart's legend.
fn legend(key: &str, direction: Direction, total: &str, t: &Theme) -> El {
    let mut swatch = el().key(format!("{key}-swatch")).size(10.0, 10.0).radius(3.0);
    swatch = match direction {
        Direction::Upload => swatch.fill(t.accent).border(1.0, t.accent_deep),
        Direction::Download => swatch.fill(t.text_tertiary),
    };
    // The swatch, the name and the total are 6 points apart (`gap-1.5`); the total is a machine's value.
    row()
        .key(key.to_string())
        .gap(6.0)
        .center_y()
        .child(swatch)
        .child(kit::label(
            format!("{key}-name"),
            direction.label(),
            type_::CAPTION,
            t.text_tertiary,
        ))
        .child(
            kit::label(
                format!("{key}-total"),
                total,
                type_::CAPTION.family(Family::Mono),
                t.text_tertiary,
            )
            // The exact advance (0.6 em a character): a hug width is snapped up to a device pixel,
            // and the legend is anchored at its right end.
            .w(total.chars().count() as f32 * 6.6),
        )
}

/// Last 20 hours (`.kit-card`): the legend with the totals, and the chart; or that nothing
/// is recorded yet (DW-0785e).
fn last_20_hours(h: &History, tip: Option<usize>, t: &Theme) -> El {
    let mut top = row().key("card-1-top").w_full().justify_between().center_y().child(
        col()
            .key("card-1-names")
            .child(kit::label("card-1-head", "Last 20 hours", type_::HEAD, t.text).w_full())
            .child(kit::label(
                "card-1-cap",
                "Data moved per 20 minutes",
                type_::CAPTION,
                t.text_tertiary,
            )),
    );
    let body = if h.is_empty() {
        col()
            .key("chart-empty")
            .w_full()
            .h(PLOT_H + 22.0)
            .center()
            .gap(4.0)
            .child(kit::label("card-1-sub", "Nothing recorded yet.", SUB, t.text_tertiary))
            .child(kit::label(
                "card-1-empty",
                "Beebeeb records traffic from when it starts syncing on this PC.",
                type_::CAPTION,
                t.text_tertiary,
            ))
    } else {
        top = top.child(
            row()
                .key("legend")
                .gap(12.0)
                .center_y()
                .child(legend("legend-up", Direction::Upload, &h.up_total, t))
                .child(legend("legend-down", Direction::Download, &h.down_total, t)),
        );
        chart(h, tip, t)
    };
    cards::column(1, 16.0, 12.0, t)
        .role("group")
        .label("Last 20 hours")
        .child(top)
        .child(body)
}

/// Where bucket `i` of `n` is along the plot.
fn plot_x(i: usize, n: usize) -> f32 {
    i as f32 / (n.max(2) - 1) as f32 * PLOT_W
}

/// Where a value is up the plot: the axis' top is `PLOT_TOP` below the plot's own.
fn plot_y(mb: f32, top_mb: f32) -> f32 {
    PLOT_H - (mb.min(top_mb) / top_mb.max(1.0)) * (PLOT_H - PLOT_TOP)
}

/// The megabytes of an axis mark: "400 MB" at the top, bare figures below it.
fn axis_mark(mb: f32, with_unit: bool) -> String {
    if with_unit {
        format!("{} MB", mb.round())
    } else {
        format!("{}", mb.round())
    }
}

/// The chart: upload (Amber, the encryption step) over download (grey), one axis, the times
/// under it. Hover shows a crosshair and that bucket's time with both amounts. Identity is in
/// the legend's words, never in colour alone; the chart's name carries the totals.
fn chart(h: &History, tip: Option<usize>, t: &Theme) -> El {
    let n = h.up_mb.len();
    // The canvas is a little larger than the plot, so a line on its edge is not cut.
    const PAD: f32 = 2.0;
    let line = |values: &[f32]| -> Vec<[f32; 2]> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| [plot_x(i, n) + PAD, plot_y(*v, h.top_mb) + PAD])
            .collect()
    };
    let area = |values: &[f32]| -> Vec<[f32; 2]> {
        let mut points = line(values);
        points.push([PLOT_W + PAD, PLOT_H + PAD]);
        points.push([PAD, PLOT_H + PAD]);
        points
    };
    let mut shape = Shape::new();
    for f in [0.25, 0.5, 0.75] {
        let y = PLOT_H - f * (PLOT_H - PLOT_TOP) + PAD;
        shape = shape.line([PAD, y], [PLOT_W + PAD, y], 1.0, LineCap::Butt, t.separator);
    }
    let floor = PLOT_H + PAD;
    shape = shape
        .line(
            [PAD, floor],
            [PLOT_W + PAD, floor],
            1.0,
            LineCap::Butt,
            t.text_quaternary,
        )
        .fill(area(&h.down_mb), t.text_tertiary.alpha(0.14))
        .fill(area(&h.up_mb), t.accent.alpha(0.26))
        .stroke(line(&h.down_mb), 2.0, t.text_tertiary)
        .stroke(line(&h.up_mb), 2.0, t.accent_deep);
    let tip = tip.filter(|i| *i < n);
    if let Some(i) = tip {
        // The crosshair: 3 on, 3 off.
        let x = plot_x(i, n) + PAD;
        let mut y = PLOT_TOP + PAD;
        while y < floor {
            shape = shape.line([x, y], [x, (y + 3.0).min(floor)], 1.0, LineCap::Butt, t.text_tertiary);
            y += 6.0;
        }
    }
    let mut plot = el().key("chart-plot").size(PLOT_W, PLOT_H).child(
        canvas(shape)
            .key("chart-lines")
            .absolute(-PAD, -PAD)
            .size(PLOT_W + 2.0 * PAD, PLOT_H + 2.0 * PAD),
    );
    // The axis: the top, the middle, the floor (`text` baselines at 8, 57 and 107).
    for (j, (mb, baseline)) in [(h.top_mb, 8.0), (h.top_mb / 2.0, 57.0), (0.0, 107.0)]
        .into_iter()
        .enumerate()
    {
        plot = plot.child(
            kit::label(format!("chart-y-{j}"), axis_mark(mb, j == 0), AXIS_Y, t.text_tertiary)
                .absolute(PLOT_W + 6.0, baseline - 9.6),
        );
    }
    // One strip per bucket takes the pointer.
    let strip = PLOT_W / (n.max(2) - 1) as f32;
    for i in 0..n {
        plot = plot.child(
            el().key(format!("chart-at-{i}"))
                .absolute(plot_x(i, n) - strip / 2.0, 0.0)
                .size(strip, PLOT_H)
                .on_hover(move |s: &mut State| s.settings.bandwidth.tip = Some(i)),
        );
    }
    if let Some((i, (when, moved))) = tip.and_then(|i| Some((i, h.tip(i)?))) {
        let x = plot_x(i, n);
        let dot = |key: &str, mb: f32, ink: Color| {
            el().key(key.to_string())
                .absolute(x - 5.0, plot_y(mb, h.top_mb) - 5.0)
                .size(10.0, 10.0)
                .radius(5.0)
                .fill(t.card)
                .center()
                .child(el().key(format!("{key}-ink")).size(6.0, 6.0).radius(3.0).fill(ink))
        };
        plot = plot
            .child(dot("chart-dot-up", h.up_mb[i], t.accent))
            .child(dot("chart-dot-down", h.down_mb[i], t.text_tertiary))
            .child(
                col()
                    .key("chart-tip")
                    .role("tooltip")
                    .label(format!("{when}. {moved}"))
                    // Left of the crosshair where there is room, right of it otherwise.
                    .absolute(if x >= 228.0 { x - 228.0 } else { x + 12.0 }, 18.0)
                    .py(6.0)
                    .px(9.0)
                    .radius(8.0)
                    .fill(t.card)
                    .box_shadow(kit::drop(4.0, 14.0, Color::BLACK.alpha(0.14)))
                    .box_shadow(kit::ring(kit::HAIR, t.separator))
                    .child(kit::label("chart-tip-when", when, TIP, t.text))
                    .child(kit::label("chart-tip-moved", moved, TIP, t.text)),
            );
    }
    let mut times = row().key("chart-x").w(PLOT_W).pt(4.0).justify_between();
    for (j, mark) in h.marks().into_iter().enumerate() {
        times = times.child(kit::label(format!("chart-x-{j}"), mark, AXIS_X, t.text_tertiary));
    }
    col()
        .key("chart")
        .role("img")
        .label(format!(
            "Upload and download over the last 20 hours: upload {}, download {}",
            h.up_total, h.down_total
        ))
        .w_full()
        .items_start()
        .child(plot)
        .child(times)
}

/// A thin bar (`.mk-bar`): how far along, 0 to 1. Upload is Amber, download grey.
fn bar(key: &str, share: f32, direction: Direction, t: &Theme) -> El {
    // The board rounds the share to a whole per cent.
    let share = (share.clamp(0.0, 1.0) * 100.0).round() / 100.0;
    el().key(key.to_string())
        .role("progressbar")
        .label(format!("{} per cent", (share * 100.0).round()))
        .w_full()
        .h(4.0)
        .radius(2.0)
        .fill(t.text.alpha(0.09))
        .child(
            el().key(format!("{key}-fill"))
                .w_frac(share)
                .h(4.0)
                .radius(2.0)
                .fill(match direction {
                    Direction::Upload => t.accent,
                    Direction::Download => t.text_tertiary,
                }),
        )
}

/// A device's state as a pill with a dot (`.kit-pill`): green when it is up to date, neutral
/// otherwise. The word says the state; the colour only repeats it.
fn state_pill(i: usize, state: DeviceState, t: &Theme) -> El {
    if state != DeviceState::UpToDate {
        return notices::pill(i, state.label(), t);
    }
    let key = format!("pill-{i}");
    row()
        .key(key.clone())
        .h(20.0)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(10.0)
        .fill(t.green.alpha(0.14))
        .child(meters::disc(&format!("{key}-dot"), 6.0, t.green))
        .child(kit::label(format!("{key}-label"), state.label(), PILL, green_text(t)))
}

/// Your devices right now: one row per device. A row leads to Devices and sessions.
fn devices(list: usize, devices: &[Device], t: &Theme) -> El {
    let rows = devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let key = format!("device-{i}");
            let icon = match d.kind {
                DeviceKind::Computer => &ic::LAPTOP,
                DeviceKind::Phone => &ic::PHONE,
                DeviceKind::Tablet => &ic::TABLET,
            };
            let name = format!("{} \u{b7} {}", d.name, d.tag);
            let tag = Span::new(d.name.len(), name.len())
                .size(11.5)
                .weight(400.0)
                .color(t.text_tertiary);
            kit::note_text(&format!("{key}-name"), &format!("{key}-name"), &name, DEVICE, t.text);
            let mut words = col().key(format!("{key}-text")).grow(1.0).min_w(0.0).gap(3.0).child(
                rich(name, vec![tag])
                    .key(format!("{key}-name"))
                    .font(DEVICE)
                    .color(t.text),
            );
            if let Some(progress) = d.progress {
                words = words.child(bar(&format!("{key}-bar"), progress, Direction::Upload, t));
            }
            words =
                words.child(kit::label(format!("{key}-says"), d.says.clone(), DEVICE_SAYS, t.text_tertiary).truncate());
            let mut r = row()
                .key(key.clone())
                .role("link")
                .label(format!("{}, {}, {}. {}", d.name, d.tag, d.state.label(), d.says))
                .w_full()
                .py(9.0)
                .px(14.0)
                .gap(12.0)
                .center_y()
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(|s: &mut State| super::go(s, "DW-A55"));
            if i > 0 {
                // `.mk-drow + .mk-drow`: a hairline that takes its own half point.
                r = r.pt(9.5).child(
                    el().key(format!("{key}-rule"))
                        .pin(Some(0.0), Some(0.0), Some(0.0), None)
                        .h(kit::HAIR)
                        .fill(t.separator),
                );
            }
            r.child(
                el().key(format!("{key}-tile"))
                    .size(30.0, 30.0)
                    .radius(8.0)
                    .fill(t.text.alpha(0.07))
                    .center()
                    .child(kit::glyph(format!("{key}-icon"), icon, 15.0, t.text_secondary)),
            )
            .child(words)
            .child(state_pill(i, d.state, t))
        })
        .collect();
    lists::list(
        list,
        Some("Your devices right now"),
        rows,
        Some(
            "Each device reports in about every 20 seconds while Beebeeb runs on it. Devices that are off show when they last reported.",
        ),
        t,
    )
}

/// Limits: the two pop-up buttons and the metered switch.
fn limits(list: usize, bw: &Bandwidth, menu: Option<Menu>, metered_control: bool, t: &Theme) -> El {
    let pop = |index: usize, direction: Direction, limit: Option<u32>| {
        let kind = MenuKind::SpeedLimit(direction);
        let icon = match direction {
            Direction::Upload => &ic::ARROW_UP,
            Direction::Download => &ic::ARROW_DOWN,
        };
        let mut item = Item::new(Some(icon), direction.label());
        if limit.is_some() {
            item = item.sub("Sync takes longer while this is lower than your connection");
        }
        if index > 0 {
            item = item.ruled();
        }
        item = item
            .trailing(Trailing::Pop {
                choice: limit_label(limit),
                open: menu.is_some_and(|m| m.kind == kind),
                menu_at: MENU_AT,
            })
            .role(Role::PopUp);
        lists::item(list, index, &item, t).on_press(move |s: &mut State| s.settings.toggle_menu(kind))
    };
    let on = bw.sync_on_metered;
    let metered = Item::new(Some(&ic::WIFI), "Sync on metered connections")
        .sub(if on {
            "A network Windows marks as metered, such as a phone's hotspot"
        } else {
            "Off: sync waits on networks Windows marks as metered"
        })
        .ruled();
    let mut rows = vec![
        pop(0, Direction::Upload, bw.upload_limit),
        pop(1, Direction::Download, bw.download_limit),
    ];
    // No switch where nothing detects a metered network: it would store a choice nothing honours.
    if metered_control {
        rows.push(lists::item_with(
            list,
            2,
            &metered,
            vec![
                lists::switch(&format!("item-{list}-2"), "Sync on metered connections", on, t)
                    .on_press(move |s: &mut State| s.settings.drive.set_sync_on_metered(!on)),
            ],
            t,
        ));
    }
    lists::list(
        list,
        Some("Limits"),
        rows,
        Some("For this PC only. Each device keeps its own limits."),
        t,
    )
}

/// One step of the speed test (`.mk-step`): its name and mark, its figure, and the line or
/// the bar under it.
fn step(i: usize, name: &str, step: &Step, t: &Theme) -> El {
    let key = format!("step-{i}");
    let mut top = row()
        .key(format!("{key}-top"))
        .w_full()
        .justify_between()
        .center_y()
        .child(kit::label(format!("{key}-name"), name, FIG_NAME, t.text_secondary));
    let direction = if i == 2 { Direction::Download } else { Direction::Upload };
    let (said, value, under) = match step {
        Step::Done(value, unit, says) => {
            top = top.child(kit::glyph(format!("{key}-check"), &ic::CHECK, 13.0, green_text(t)));
            (
                format!("{name}, {value} {}", unit_words(unit)),
                figure(&format!("{key}-value"), value, unit, (STEP, STEP_UNIT), 3.0, 7.5, t),
                kit::label(format!("{key}-says"), says.clone(), STEP_SAYS, t.text_tertiary),
            )
        }
        Step::Measuring(count, share) => {
            top = top.child(kit::label(
                format!("{key}-measuring"),
                "measuring",
                type_::CAPTION,
                t.text_tertiary,
            ));
            (
                format!("{name}, measuring, {count}"),
                figure(&format!("{key}-value"), count, "", (STEP, STEP_UNIT), 0.0, 0.0, t),
                bar(&format!("{key}-bar"), *share, direction, t),
            )
        }
        Step::Waiting(why) => (
            format!("{name}, not started. {why}"),
            kit::label(format!("{key}-value"), "\u{2014}", STEP, t.text_tertiary),
            kit::label(format!("{key}-says"), why.clone(), STEP_SAYS, t.text_tertiary),
        ),
    };
    col()
        .key(key)
        .role("group")
        .label(said)
        .w((shell::COLUMN_W - 28.0 - 16.0) / 3.0)
        .min_h(83.5)
        .gap(4.0)
        .py(10.0)
        .px(12.0)
        .radius(10.0)
        .fill(wash(t))
        .child(top)
        .child(value)
        .child(under)
}

/// The speed test: the row with Run test, the three steps while it runs and after, or the
/// sentence that names the step that failed.
fn speed_test(list: usize, test: &SpeedTest, t: &Theme) -> El {
    let running = test.running();
    let button = controls::button(
        0,
        if running { "Running\u{2026}" } else { "Run test" },
        None,
        Emphasis::Plain,
        t,
    );
    // One test at a time: the button does nothing while a test runs.
    let button = if running {
        button.disabled(true)
    } else {
        button.on_press(|s: &mut State| s.settings.drive.run_speed_test())
    };
    let head = Item::new(Some(&ic::GAUGE), "Test connection speed").sub(if running {
        "Testing\u{2026} this takes 30 to 90 seconds."
    } else {
        SPEED_HINT
    });
    let mut rows = vec![lists::item_with(list, 0, &head, vec![button], t)];
    let steps = |steps: &[Step; 3]| {
        let mut line = row().key("steps").w_full().pt(2.0).px(14.0).pb(14.0).gap(8.0);
        for (i, name) in ["Latency", "Upload", "Download"].into_iter().enumerate() {
            line = line.child(step(i, name, &steps[i], t));
        }
        line
    };
    let mut foot = "Measures how fast this PC reaches Beebeeb's own servers in Europe: the same test as the bb command line and the iPhone.";
    match test {
        SpeedTest::Idle => {}
        SpeedTest::Running(now) => rows.push(steps(now)),
        SpeedTest::Done(result, measured) => {
            rows.push(steps(result));
            foot = measured;
        }
        SpeedTest::Failed(why) => rows.push(el().key("test-failed").w_full().px(12.0).pb(12.0).child(notices::note(
            0,
            Tone::Red,
            &ic::WARN_CIRCLE,
            "Couldn't run the speed test.",
            why,
            t,
        ))),
    }
    lists::list(list, Some("Speed test"), rows, Some(foot), t)
}

/// A limit's menu, when it is the open one (DW-0785d): No limit, a rule, then the presets.
fn limit_menu(ui: &SettingsUi, direction: Direction, t: &Theme) -> Option<El> {
    let kind = MenuKind::SpeedLimit(direction);
    let open = ui.menu.filter(|m| m.kind == kind)?;
    let checked = chosen(ui, direction);
    let mut items = Vec::new();
    for (i, label) in PRESET_LABELS.into_iter().enumerate() {
        if i == 1 {
            items.push(
                el().key("menu-0-sep-0")
                    .w_full()
                    .h(kit::HAIR)
                    .mt(4.0)
                    .mb(4.0)
                    .fill(t.separator),
            );
        }
        items.push(
            controls::menu_item(0, i, label, checked == Some(i), i == open.hot, t)
                .on_press(move |s: &mut State| s.settings.choose(kind, i)),
        );
    }
    if checked.is_none() {
        // A limit an older version saved: kept, and shown as its own choice (DW-0785c).
        let bw = ui.drive.bandwidth();
        let saved = limit_label(match direction {
            Direction::Upload => bw.upload_limit,
            Direction::Download => bw.download_limit,
        });
        let own = PRESETS.len();
        items.push(
            controls::menu_item(0, own, &saved, true, open.hot == own, t)
                .on_press(move |s: &mut State| s.settings.choose(kind, own)),
        );
    }
    Some(
        controls::menu(0, kind.name(), 150.0, items, t)
            .anchored(kind.anchor(), Side::Bottom, Align::Start, 0.0)
            .modal(true, |s: &mut State| s.settings.menu = None),
    )
}
