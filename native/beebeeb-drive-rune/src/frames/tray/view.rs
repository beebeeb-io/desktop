//! The flyout, drawn (`.mk-pop` in `.wn-fly`): the status block (`.mk-pst`), the Recent changes
//! label (`.mk-plabel`) and list (`.mk-plist`), and the three buttons (`.mk-pfoot`).
//!
//! Every measurement is the board's CSS (`design/desktop-v2/board.html`, `.mk-p*` and
//! `.wn-fly`), read from the goldens' `css` records. Keys are `scripts/keymap-tray.js`'s.

use rune::tok::Family;
use rune::{Align, Color, Cursor, El, Icon, ScrollFade, ScrollPos, ScrollSpec, TextStyle, col, el, oklch, row};

use super::source::{Change, Changes, MAX_ROWS, Origin, Snapshot, Status, Tile, Tone, What};
use super::{Host, Ui, icons};
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::kit;
use crate::ui::settings_controls::{self, Emphasis};

/// The flyout's size (`.mk-pop`): fixed in every state (DW-1201).
pub const SIZE: (f32, f32) = (340.0, 520.0);

mod type_fly {
    use super::{Family, TextStyle};

    /// 13.5px/650, tracked -0.006em: the state (`.mk-pst-tx b`).
    pub const TITLE: TextStyle = TextStyle::new(13.5, 650.0).lh(17.0).tracking(-0.081);
    /// 12px/400: the line under it (`.mk-pst-tx span`).
    pub const LINE: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
    /// 10.5px/400 mono: the reason, for support (DW-1216).
    pub const CODE: TextStyle = TextStyle::new(10.5, 400.0).family(Family::Mono).lh(16.0);
    /// 11px/400 mono: the upload speed.
    pub const SPEED: TextStyle = TextStyle::new(11.0, 400.0).family(Family::Mono).lh(17.0);
    /// 11px/600: "Recent changes" (`.mk-plabel`).
    pub const LABEL: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0);
    /// 11px/500: "last 50" (`.mk-plabel span`).
    pub const LABEL_COUNT: TextStyle = TextStyle::new(11.0, 500.0).lh(17.0);
    /// 12.5px/500: a file's name (`.mk-pr-tx b`).
    pub const NAME: TextStyle = TextStyle::new(12.5, 500.0).lh(15.0);
    /// 11.5px/400: what happened to it (`.mk-pr-tx span`).
    pub const WHAT: TextStyle = TextStyle::new(11.5, 400.0).lh(14.0);
    /// 11px/400 mono: when (`.mk-pr-t`).
    pub const WHEN: TextStyle = TextStyle::new(11.0, 400.0).family(Family::Mono).lh(14.5);
    /// 12px/400 on a 15px line: the sentence of an empty list (`.t-sub` in the flyout).
    pub const EMPTY: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
    /// 11.5px/400: "That's the last 50 changes." (`.mk-pend small`).
    pub const END: TextStyle = TextStyle::new(11.5, 400.0).lh(17.0);
    /// 11.5px/500 on a 13px line: the three buttons (`.wn-fly .mk-pbtn`).
    pub const FOOT: TextStyle = TextStyle::new(11.5, 500.0).lh(13.0);
}

/// `--mk-warn`: "color-mix(in oklch, var(--color-amber-deep) 45%, var(--color-red))", the
/// orange of a state that needs a decision.
fn warn(t: &Theme) -> Color {
    if t.dark {
        oklch(0.6995, 0.183, 46.15)
    } else {
        oklch(0.575, 0.1785, 46.15)
    }
}

/// `--color-red-text`: a row's reason when it could not sync.
fn red_text(t: &Theme) -> Color {
    crate::ui::au::red_text(t)
}

/// `--kit-ground`: the surface under the three buttons.
fn ground(t: &Theme) -> Color {
    t.surface
}

/// A bar's track (`.mk-bar`): ink at 9 %.
fn track(t: &Theme) -> Color {
    t.text.alpha(0.09)
}

pub fn row_key(row: usize) -> String {
    format!("row-{row}")
}

/// The whole flyout.
pub fn flyout<S: 'static>(ui: &Ui, host: Host<S>, dark: bool) -> El {
    let t = tokens::theme(dark);
    let snapshot = ui.snapshot();
    col()
        .key("fly")
        .role("dialog")
        .label(crate::shell::identity::AppIdentity::CURRENT.flyout_window_title())
        .size(SIZE.0, SIZE.1)
        .fill(t.card)
        .on_key(super::on_key(host))
        .child(status_block(snapshot, host, &t))
        .child(changes_label(snapshot, &t))
        .child(changes_list(ui, host, &t))
        .child(footer(snapshot, host, &t))
}

// ---------------------------------------------------------------------------
// The status block
// ---------------------------------------------------------------------------

fn status_icon(status: &Status) -> &'static Icon {
    match status {
        Status::Checking | Status::UpToDate { .. } => &icons::CHECK,
        Status::Syncing { .. } => &icons::SYNC,
        Status::Paused { .. } | Status::WaitingMetered => &icons::PAUSE,
        Status::NeedsDecision { .. } | Status::Stopped { .. } => &icons::ALERT,
        Status::Offline => &icons::CLOUD_OFF,
        Status::StorageAlmostFull { .. } | Status::StorageFull { .. } => &icons::STORAGE,
        Status::UpdateAvailable { .. } | Status::UpdateRequired => &icons::DOWNLOAD,
        Status::Locked => &icons::LOCK,
        Status::NoPlan { .. } => &icons::INFO,
        Status::AddingToExplorer | Status::ExplorerFailed { .. } => &icons::EXPLORER,
        Status::SignedOut => &icons::KEY,
    }
}

/// `(mark, wash)` of the icon tile (`.mk-pst-ic.tone-*`).
fn tone(tone: Tone, t: &Theme) -> (Color, Color) {
    match tone {
        Tone::Neutral => (t.text_secondary, t.tile),
        Tone::Green => (t.green, t.green.alpha(0.13)),
        Tone::Warn => (warn(t), warn(t).alpha(0.14)),
        Tone::Red => (t.red, t.red.alpha(0.12)),
    }
}

/// A small button of the kit (`ui::settings_controls::button`: `.kit-btn.kit-btn-sm`, 22 high),
/// with the flyout's focus ring: amber when it is the one main action, paper with a line
/// otherwise. Keys: `btn-<index>`, `btn-<index>-label`.
fn small_button(index: usize, label: &str, primary: bool, t: &Theme) -> El {
    let emphasis = if primary { Emphasis::Primary } else { Emphasis::Plain };
    settings_controls::button(index, label, None, emphasis, t)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(Cursor::Pointer)
}

/// A progress bar (`.mk-bar`): `lead` of the track is empty, then `fill` of it is filled.
fn bar(key: &str, lead: f32, fill: f32, color: Color, t: &Theme) -> El {
    let mut track_el = row()
        .key(key)
        .role("progressbar")
        .h(4.0)
        .radius(2.0)
        .clip_rounded(2.0)
        .fill(track(t));
    if lead > 0.0 {
        track_el = track_el.child(el().key(format!("{key}-lead")).w_frac(lead).h_full());
    }
    track_el.child(
        el().key(format!("{key}-fill"))
            .w_frac(fill.clamp(0.0, 1.0))
            .h_full()
            .radius(2.0)
            .fill(color),
    )
}

fn status_block<S: 'static>(snapshot: &Snapshot, host: Host<S>, t: &Theme) -> El {
    let status = &snapshot.status;
    let (mark, wash) = tone(status.tone(), t);
    let checking = *status == Status::Checking;
    let skeleton = t.text.alpha(0.08);

    let tile = el()
        .key("st-tile")
        .size(34.0, 34.0)
        .radius(10.0)
        .fill(wash)
        .center()
        .child(if checking {
            el().key("st-skel-0").size(16.0, 16.0).radius(8.0).fill(skeleton)
        } else {
            kit::glyph("st-icon", status_icon(status), 18.0, mark)
        });

    let mut words = col().key("st-text").grow(1.0).min_w(0.0);
    if checking {
        // No words until the state is known (DW-1212): two bars where they will be.
        words = words
            .gap(6.0)
            .child(el().key("st-skel-1").w_frac(0.62).h(11.0).radius(5.0).fill(skeleton))
            .child(el().key("st-skel-2").w_frac(0.84).h(9.0).radius(5.0).fill(skeleton));
    } else {
        words = words.gap(1.0).child(
            kit::label("st-title", status.title(), type_fly::TITLE, t.text)
                .w_full()
                .wrap()
                .max_lines(2),
        );
        if let Some(line) = status.line() {
            words = words.child(
                kit::label("st-line", line, type_fly::LINE, t.text_tertiary)
                    .w_full()
                    .wrap()
                    .max_lines(3),
            );
        }
        if let Some(code) = status.code() {
            words = words.child(kit::label("st-code", code, type_fly::CODE, t.text_tertiary).w_full());
        }
    }

    let button = status.action().map(|(action, primary)| {
        (
            small_button(0, action.label(), primary, t).on_press_tx(host.act(Ui::press_action)),
            // "A button label over 12 characters moves under the text instead of beside it" (DW-1221).
            action.label().chars().count() > 12,
            action,
        )
    });

    let mut top = row()
        .key("st-row")
        .w_full()
        .min_h(36.0)
        .gap(11.0)
        .center_y()
        .child(tile)
        .child(words);
    let mut under = None;
    match button {
        Some((button, true, _)) => under = Some(row().key("st-under").w_full().pl(45.0).child(button)),
        Some((button, false, _)) => top = top.child(button),
        None => {}
    }

    let mut block = col()
        .key("st")
        .role("status")
        .label(match status.line() {
            Some(line) => format!("{}. {line}", status.title()),
            None => status.title(),
        })
        .w_full()
        .pt(14.0)
        .px(14.0)
        .pb(12.0)
        .gap(9.0)
        .child(top);
    if let Some(under) = under {
        block = block.child(under);
    }
    match status {
        Status::Syncing { fraction, speed, .. } => {
            block = block.child(
                row()
                    .key("st-progress")
                    .w_full()
                    .h(17.0)
                    .gap(8.0)
                    .center_y()
                    .child(
                        bar("st-bar", 0.0, *fraction, t.accent, t)
                            .label("Synced so far")
                            .grow(1.0),
                    )
                    .child(
                        row()
                            .key("st-speed-row")
                            .label(format!("Upload speed {speed}"))
                            .gap(2.0)
                            .center_y()
                            .child(kit::glyph("st-speed-icon", &icons::ARROW_UP, 11.0, t.text_tertiary))
                            .child(kit::label("st-speed", speed.clone(), type_fly::SPEED, t.text_tertiary)),
                    ),
            );
        }
        // "An indeterminate bar, because Windows gives no progress" (DW-1215).
        Status::AddingToExplorer => {
            block = block.child(
                bar("st-bar", 0.22, 0.30, t.text_secondary, t)
                    .label("Adding Beebeeb to File Explorer")
                    .w_full(),
            );
        }
        _ => {}
    }
    block
}

// ---------------------------------------------------------------------------
// Recent changes
// ---------------------------------------------------------------------------

fn changes_label(snapshot: &Snapshot, t: &Theme) -> El {
    let _ = snapshot;
    row()
        .key("label")
        .w_full()
        .h(28.5)
        .pt(6.0)
        .pb(5.0)
        .px(16.0)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .justify_between()
        .center_y()
        .child(kit::label(
            "label-title",
            "Recent changes",
            type_fly::LABEL,
            t.text_tertiary,
        ))
        .child(kit::label(
            "label-count",
            format!("last {MAX_ROWS}"),
            type_fly::LABEL_COUNT,
            t.text_tertiary,
        ))
}

fn tile_icon(tile: Tile) -> Option<&'static Icon> {
    Some(match tile {
        Tile::Document => &icons::FILE_DOC,
        Tile::Sheet => &icons::FILE_SHEET,
        Tile::Deck => &icons::FILE_DECK,
        Tile::Video => &icons::FILE_VIDEO,
        Tile::Audio => &icons::FILE_AUDIO,
        Tile::Archive => &icons::FILE_ARCHIVE,
        Tile::Folder => &icons::FOLDER,
        Tile::Trashed => &icons::FILE_TRASHED,
        Tile::Photo(_) => return None,
    })
}

/// The board's sample photos (`.mk-ph1` to `.mk-ph6`): the angle in CSS degrees and the stops.
fn photo(n: u8) -> (f32, [(f32, &'static str); 3]) {
    match n {
        1 => (160.0, [(0.0, "#e9c9a0"), (0.55, "#b97a50"), (1.0, "#5e4a4a")]),
        2 => (170.0, [(0.0, "#9cc4dc"), (0.60, "#e9e2cf"), (1.0, "#8d9a78")]),
        3 => (150.0, [(0.0, "#33435c"), (0.55, "#7a7f96"), (1.0, "#e3b58c")]),
        4 => (200.0, [(0.0, "#c7d6b0"), (0.60, "#6c8d62"), (1.0, "#2f4634")]),
        6 => (120.0, [(0.0, "#d9dfe4"), (0.60, "#8ea4b4"), (1.0, "#4a5866")]),
        _ => (180.0, [(0.0, "#f3d9a8"), (0.50, "#d58c6a"), (1.0, "#3f3a58")]),
    }
}

/// A row's tile (`.mk-ftile`, 30 in the flyout): the file's glyph, or the photo.
fn file_tile(key: &str, tile: Tile, t: &Theme) -> El {
    let base = el()
        .key(format!("{key}-tile"))
        .size(30.0, 30.0)
        .radius(7.0)
        .clip_rounded(7.0)
        .center();
    match (tile, tile_icon(tile)) {
        (_, Some(icon)) => base
            .fill(t.tile)
            .child(kit::glyph(format!("{key}-icon"), icon, 19.0, t.text_secondary)),
        (Tile::Photo(n), _) => {
            let (degrees, stops) = photo(n);
            base.linear_gradient(
                degrees.to_radians(),
                stops.map(|(at, hex)| (at, rune::color::hex(hex).unwrap_or(Color::BLACK))),
            )
        }
        _ => base.fill(t.tile),
    }
}

fn origin_icon(origin: Origin) -> &'static Icon {
    match origin {
        Origin::ThisPc => &icons::FROM_PC,
        Origin::Phone => &icons::FROM_PHONE,
        Origin::Tablet => &icons::FROM_TABLET,
        Origin::People => &icons::FROM_PEOPLE,
    }
}

fn change_row<S: 'static>(i: usize, change: &Change, selected: bool, host: Host<S>, t: &Theme) -> El {
    let key = row_key(i);
    let mut words = col().key(format!("{key}-text")).grow(1.0).min_w(0.0).gap(2.0).child(
        kit::label(format!("{key}-name"), change.name.clone(), type_fly::NAME, t.text)
            .w_full()
            .truncate(),
    );
    let said = match &change.what {
        What::Done { origin, text } => {
            words = words.child(
                row()
                    .key(format!("{key}-what"))
                    .w_full()
                    .gap(4.0)
                    .center_y()
                    .child(kit::glyph(
                        format!("{key}-sub-icon"),
                        origin_icon(*origin),
                        12.0,
                        t.text_tertiary,
                    ))
                    .child(
                        kit::label(format!("{key}-sub"), text.clone(), type_fly::WHAT, t.text_tertiary)
                            .min_w(0.0)
                            .truncate(),
                    ),
            );
            text.clone()
        }
        What::Moving { text, fraction, upload } => {
            // Amber for an upload (the encryption step), grey for a download (DW-1201).
            let color = if *upload { t.accent } else { t.text_tertiary };
            words = words
                .child(
                    kit::label(format!("{key}-sub"), text.clone(), type_fly::WHAT, t.text_tertiary)
                        .w_full()
                        .truncate(),
                )
                .child(bar(&format!("{key}-bar"), 0.0, *fraction, color, t).w_full());
            text.clone()
        }
        What::Failed { text } => {
            words = words.child(
                kit::label(format!("{key}-sub"), text.clone(), type_fly::WHAT, red_text(t))
                    .w_full()
                    .wrap()
                    .max_lines(2),
            );
            text.clone()
        }
    };
    let mut r = row()
        .key(key.clone())
        .role("listitem")
        .label(format!("{}, {said}, {}", change.name, change.when))
        .help(if change.in_trash {
            "Open Trash"
        } else {
            "Show in File Explorer"
        })
        .w_full()
        .h(44.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(8.0)
        .hover(t.selection)
        .cursor(Cursor::Pointer)
        .on_press_tx(host.act(move |ui| ui.open_row(i)))
        .child(file_tile(&key, change.tile, t))
        .child(words)
        .child(
            kit::label(
                format!("{key}-time"),
                change.when.clone(),
                type_fly::WHEN,
                t.text_tertiary,
            )
            .self_start()
            .mt(7.0),
        );
    if selected {
        r = r.fill(t.selection);
    }
    r
}

/// The list with nothing to show (`.kit-tile` over `.t-head` and `.t-sub`, centred).
fn empty(icon: &'static Icon, title: &str, sentence: &str, t: &Theme) -> El {
    col()
        .key("empty")
        .w_full()
        .h_full()
        // The list's own 6 px, and the 32 px of the empty state (`px-8`).
        .px(38.0)
        .gap(8.0)
        .center()
        .child(
            el().key("empty-tile")
                .size(38.0, 38.0)
                .radius(11.0)
                .fill(t.tile)
                .center()
                .child(kit::glyph("empty-icon", icon, 19.0, t.text_secondary)),
        )
        .child(kit::label("empty-title", title, type_::HEAD, t.text))
        .child(
            kit::label("empty-text", sentence, type_fly::EMPTY, t.text_tertiary)
                .w_full()
                .wrap()
                .align(Align::Center),
        )
}

/// The scroll thumb's length and the room it keeps from the list's ends (`.mk-scroll`).
const THUMB: (f32, f32) = (34.0, 4.0);

fn changes_list<S: 'static>(ui: &Ui, host: Host<S>, t: &Theme) -> El {
    let list = el().key("list").w_full().grow(1.0).min_h(0.0);
    let (rows, complete) = match &ui.snapshot().changes {
        Changes::Hidden => {
            return list.child(empty(
                &icons::LOCK,
                "Recent changes are hidden",
                "Unlock Beebeeb to see file names. Nothing shows while it's locked.",
                t,
            ));
        }
        Changes::Empty { text } => return list.child(empty(&icons::CLOCK, "No changes yet", text, t)),
        Changes::Rows { rows, complete } => (rows, *complete),
    };
    let shown = rows.len().min(MAX_ROWS);
    let mut content = col()
        .key("list-rows")
        .role("list")
        .label(format!(
            "Recent changes, {shown} {}",
            if shown == 1 { "item" } else { "items" }
        ))
        .w_full()
        .h_full()
        .px(6.0)
        .focusable(true)
        .focus_ring(2.0, -2.0, t.accent_deep)
        .scroll(ScrollSpec {
            stick_bottom: ui.opens_at_end,
        })
        // `.mk-plist.is-more` / `.is-end`: the 30 px next to hidden rows fade out, in a straight ramp.
        .scroll_fade(ScrollFade::strength(30.0).linear().full_at(1.0));
    if let Some(row) = ui.selected() {
        content = content.active_descendant(row_key(row));
    }
    let (thumb_at, thumb_shown) = (ui.thumb_at.clone(), ui.thumb_shown.clone());
    content = content.on_scroll_offset(move |at: ScrollPos| {
        let travel = (at.viewport[1] - 2.0 * THUMB.1 - THUMB.0).max(0.0);
        let part = if at.max[1] > 0.0 { at.offset[1] / at.max[1] } else { 0.0 };
        thumb_at.set([0.0, travel * part.clamp(0.0, 1.0)]);
        // An overlay scrollbar: it is not there while the list is at its top (DW-1202).
        thumb_shown.set(if at.offset[1] > 0.5 { 1.0 } else { 0.0 });
    });
    for (i, change) in rows.iter().take(MAX_ROWS).enumerate() {
        content = content.child(change_row(i, change, ui.selected() == Some(i), host, t));
    }
    // The end of the list: where the rest is (DW-1220).
    let mut end = col().key("end").w_full().pt(12.0).pb(14.0).gap(8.0).center();
    if complete {
        end = end.child(kit::label(
            "end-text",
            format!("That's the last {MAX_ROWS} changes."),
            type_fly::END,
            t.text_tertiary,
        ));
    }
    content = content.child(
        end.child(
            small_button(
                usize::from(ui.snapshot().status.action().is_some()),
                "View full history",
                false,
                t,
            )
            .on_press_tx(host.act(Ui::open_full_history)),
        ),
    );
    // The end block stops 6 px short of the list's end (DW-1220).
    content = content.child(el().key("end-room").w_full().h(6.0));
    list.child(content).child(
        el().key("scroll-thumb")
            .absolute(SIZE.0 - 3.0 - 6.0, THUMB.1)
            .size(6.0, THUMB.0)
            .radius(3.0)
            .fill(t.text.alpha(0.28))
            .live_offset(&ui.thumb_at)
            .live_opacity(&ui.thumb_shown),
    )
}

// ---------------------------------------------------------------------------
// The three buttons
// ---------------------------------------------------------------------------

/// The b as a template image: the plate in `color`, the b cut out of it (`surface` shows).
fn b_glyph(key: String, size: f32, color: Color, surface: Color) -> El {
    kit::state_glyph(
        key,
        &icons::B_TEMPLATE,
        Some((&icons::B_PLATE, &icons::B_CUT, surface)),
        "template",
        size,
        color,
    )
}

fn footer<S: 'static>(snapshot: &Snapshot, host: Host<S>, t: &Theme) -> El {
    let surface = ground(t);
    let button = |i: usize, label: &'static str, enabled: bool| {
        let ink = if enabled { t.text_secondary } else { t.text_quaternary };
        let key = format!("foot-{i}");
        let b = col()
            .key(key.clone())
            .role("button")
            .label(label)
            .grow(1.0)
            .w(0.0)
            .h(52.0)
            .pt(8.0)
            .px(6.0)
            .gap(3.0)
            .items_center()
            .radius(8.0);
        let b = if enabled {
            b.focusable(true)
                .focus_ring(2.0, -2.0, t.accent_deep)
                .hover(t.selection)
                .press(t.press)
                .cursor(Cursor::Pointer)
        } else {
            b.disabled(true)
        };
        (b, key, ink)
    };
    let label = |key: &str, text: &'static str, ink: Color| {
        kit::label(format!("{key}-label"), text, type_fly::FOOT, ink)
            .wrap()
            .align(Align::Center)
    };

    let (open, key, ink) = button(0, "Open Beebeeb", true);
    let open = open
        .child(b_glyph(format!("{key}-icon"), 16.0, ink, surface))
        .child(label(&key, "Open Beebeeb", ink))
        .on_press_tx(host.act(Ui::open_beebeeb));

    // "Open in File Explorer is disabled only while Beebeeb is not in File Explorer" (DW-1201).
    let (explorer, key, ink) = button(1, "Open in File Explorer", snapshot.explorer_ready);
    let mut explorer = explorer
        .child(
            // The board's glyph box is 17 x 15: the glyph is 15, centred in it.
            kit::glyph(format!("{key}-icon"), &icons::EXPLORER, 15.0, ink).w(17.0),
        )
        .child(label(&key, "Open in File Explorer", ink));
    if snapshot.explorer_ready {
        explorer = explorer.on_press_tx(host.act(Ui::open_in_file_explorer));
    }

    let (settings, key, ink) = button(2, "Settings", true);
    let settings = settings
        .child(kit::glyph(format!("{key}-icon"), &icons::SETTINGS, 16.0, ink))
        .child(label(&key, "Settings", ink))
        .on_press_tx(host.act(Ui::open_settings));

    row()
        .key("foot")
        .role("toolbar")
        .w_full()
        .h(64.5)
        .padding(6.0)
        .gap(4.0)
        .fill(surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(open)
        .child(explorer)
        .child(settings)
}
