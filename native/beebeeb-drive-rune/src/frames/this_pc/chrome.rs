//! The Drive window around a This PC section: the sidebar with ITS item lit and its counts, the
//! toolbar's fixed controls, and the status line.
//!
//! `ui::sidebar::sidebar` lights one of the five Drive sections and has no count for Sync
//! history; `ui::toolbar::toolbar` and `ui::status_bar` hold Files' own words. These are the
//! same pieces with the section's content (see the PR body, "Requests to the design-system lane").

use rune::{El, Icon, col, el, row, text_input};

use super::{Section, Ui};
use crate::State;
use crate::shell::app::{Event, Route, SettingsPane};
use crate::shell::host;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::kit;
use crate::ui::sidebar::{SidebarSection, brand, side_item};

/// Whether the search field has the keyboard (Rune reports a focused field's caret).
pub fn search_focused(cx: &rune::Cx<State>) -> bool {
    cx.caret(crate::ui::keys::SEARCH_KEY).is_some()
}
use crate::ui::toolbar::{self as tb, kbd};

const SIDEBAR_W: f32 = 220.0;

fn go(section: Section) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        host::dispatch(s, Event::Navigate(Route::section(section)));
    }
}

fn drive_section(section: SidebarSection) -> Section {
    match section {
        SidebarSection::Files => Section::Files,
        SidebarSection::Shared => Section::Shared,
        SidebarSection::Photos => Section::Photos,
        SidebarSection::Backups => Section::Backups,
        SidebarSection::Trash => Section::Trash,
    }
}

/// A count that needs the person (`.mk-badge.is-red`): the items that did not sync.
fn red_pill(label: &str, n: usize, t: &Theme) -> El {
    el().key(format!("{label}-pill"))
        .role("image")
        .label(if n == 1 {
            "1 item didn't sync".to_string()
        } else {
            format!("{n} items didn't sync")
        })
        .min_w(18.0)
        .h(18.0)
        .px(5.0)
        .radius(9.0)
        .fill(super::icons::red_badge())
        .center()
        .child(kit::label(
            format!("{label}-badge"),
            n.to_string(),
            type_::BADGE,
            t.white,
        ))
}

pub fn sidebar(s: &State, ui: &Ui, here: Section, t: &Theme) -> El {
    let mut sections = col().key("sidebar-sections").gap(1.0);
    for &section in SidebarSection::all() {
        let count = crate::ui::sidebar::badge_for(s, section);
        sections = sections.child(
            side_item(section.label(), section.icon(), count, false, section.shortcut(), t)
                .on_press(go(drive_section(section))),
        );
    }

    let problems = ui.problem_count();
    let kept = ui.kept_count();
    let mut history = side_item(
        "Sync history",
        &icons::SYNC_HISTORY,
        None,
        here == Section::SyncHistory,
        "Ctrl+6",
        t,
    )
    .on_press(go(Section::SyncHistory));
    if problems > 0 {
        history = history.child(red_pill("Sync history", problems, t));
    }
    let offline_on = here == Section::AvailableOffline;
    let mut offline = side_item(
        "Available offline",
        &icons::AVAILABLE_OFFLINE,
        None,
        offline_on,
        "Ctrl+7",
        t,
    )
    .on_press(go(Section::AvailableOffline));
    if kept > 0 {
        // `side_item`'s plain count is ink-3 always; the board's is ink-2 on the lit item.
        offline = offline.child(kit::label(
            "Available offline-badge",
            kept.to_string(),
            type_::KBD,
            if offline_on { t.text_secondary } else { t.text_tertiary },
        ));
    }

    let this_pc = col()
        .key("this-pc")
        .gap(1.0)
        .child(el().key("this-pc-title").px(10.0).pb(3.0).child(kit::label(
            "this-pc-label",
            "THIS PC",
            type_::SECTION_LABEL,
            t.text_tertiary,
        )))
        .child(history)
        .child(offline);

    let nav = col()
        .key("sidebar-nav")
        .role("navigation")
        .label("Sections")
        .gap(12.0)
        .grow(1.0)
        .min_h(0.0)
        .child(sections)
        .child(this_pc);

    col()
        .key("sidebar")
        .w(SIDEBAR_W)
        .h_full()
        .fill(t.sidebar_bg)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .gap(12.0)
        .pt(12.0)
        .px(10.0)
        .pb(10.0)
        .child(brand("Drive", t))
        .child(nav)
        .child(foot(s, t))
}

/// The storage meter and the account row (`.mk-side-foot`), as `ui::sidebar` draws them, from the same answers:
/// the storage the window read (`State::storage_known`, else "Unavailable" and no fill), the Files/Photos split
/// only when the source gives one (`State::storage_split`, else one segment and no breakdown), and the signed-in
/// account as the window's source names it (none known: the row keeps its place and names no one). Never the
/// board's sample beside a real account.
fn foot(s: &State, t: &Theme) -> El {
    let storage = s.storage;
    let quota = storage.quota_bytes.max(1) as f64;
    let (files_frac, photos_frac) = match s.storage_split {
        _ if !s.storage_known => (0.0, 0.0),
        Some((files, _)) => (
            (files.min(storage.used_bytes) as f64 / quota) as f32,
            ((storage.used_bytes - files).max(0) as f64 / quota) as f32,
        ),
        None => ((storage.used_bytes as f64 / quota) as f32, 0.0),
    };
    let track_w = SIDEBAR_W - 2.0 * (10.0 + 6.0 + 4.0) - kit::HAIR;
    // The figures as every other section's sidebar writes them (a real account is rarely a round number).
    let gb = crate::ui::sidebar::format_storage;
    let value = if s.storage_known {
        format!("{} of {}", gb(storage.used_bytes), gb(storage.quota_bytes))
    } else {
        "Unavailable".to_string()
    };
    let mut meter = col()
        .key("storage-meter")
        .role("group")
        .label(format!("Storage: {value}"))
        .gap(5.0)
        .px(4.0)
        .child(
            row()
                .key("storage-header")
                .justify_between()
                .child(kit::label("storage-label", "Storage", type_::CAPTION, t.text_secondary))
                .child(kit::label("storage-value", value.clone(), type_::KBD, t.text_tertiary)),
        )
        .child(
            row()
                .key("meter-track")
                .w_full()
                .h(5.0)
                .radius(2.5)
                .clip_rounded(2.5)
                .fill(t.separator)
                .child(el().key("meter-fill-0").h_full().w(track_w * files_frac).fill(t.text))
                .child(
                    el().key("meter-fill-1")
                        .h_full()
                        .w(track_w * photos_frac)
                        .fill(t.text_quaternary),
                ),
        );
    if let (true, Some((files, _))) = (s.storage_known, s.storage_split) {
        meter = meter.child(
            row()
                .key("storage-breakdown")
                .justify_between()
                .child(kit::label(
                    "storage-files",
                    format!("Files {}", gb(files)),
                    type_::KBD,
                    t.text_tertiary,
                ))
                .child(kit::label(
                    "storage-photos",
                    format!("Photos {}", gb((storage.used_bytes - files).max(0))),
                    type_::KBD,
                    t.text_tertiary,
                )),
        );
    }
    let account = row().key("account-row").center_y().gap(8.0).pt(2.0).px(2.0);
    let account = match s.actions.source.account() {
        Some(who) => account
            .child(kit::avatar(
                "avatar",
                &who.initials,
                26.0,
                rune::oklch(0.88, 0.05, 250.0),
                t.card,
                t,
            ))
            .child(
                col()
                    .key("account-info")
                    .grow(1.0)
                    .min_w(0.0)
                    .child(kit::label("account-name", &who.name, type_::BODY_SEMI, t.text).truncate())
                    .child(kit::label("account-email", &who.email, type_::BODY_SM, t.text_tertiary).truncate()),
            ),
        // No account known: the row keeps its height and names no one (as `ui::sidebar`).
        None => account.child(
            col()
                .key("account-info")
                .grow(1.0)
                .min_w(0.0)
                .child(kit::label("account-name", "", type_::BODY_SEMI, t.text).truncate())
                .child(el().key("account-email-off")),
        ),
    };
    let account = account.child(
        el().key("settings-btn")
            .role("button")
            .label("Settings")
            .shortcut_hint("Ctrl+,")
            .help("Settings  Ctrl+,")
            .size(26.0, 26.0)
            .radius(radius::CONTROL)
            .center()
            .hover(t.selection)
            .press(t.press)
            .focusable(true)
            .on_press(|s: &mut State| {
                host::dispatch(s, Event::OpenSettings(SettingsPane::General));
            })
            .child(kit::glyph("settings-icon", &icons::SETTINGS, 15.0, t.text_secondary)),
    );
    col()
        .key("sidebar-foot")
        .gap(10.0)
        .pt(10.0)
        .px(6.0)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(meter)
        .child(account)
}

/// The section's name and the line under it (`.mk-title`).
pub fn title(name: &str, sub: String, t: &Theme) -> El {
    col()
        .key("folder-info")
        .px(4.0)
        .min_w(0.0)
        .child(kit::label("folder-title", name, type_::TITLE, t.text).truncate())
        .child(kit::label("item-count", sub, type_::TITLE_SUB, t.text_tertiary))
}

/// How the toolbar's Files controls show in a section that has no use for them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FilesControls {
    /// Drawn as in Files (the board's Sync history), and inert (`ui::toolbar::Bar::inert`).
    AsFiles,
    /// The view switch at 45 % and the panel button off (the board's Available offline).
    Dimmed,
}

/// The search field (`.mk-search`) over the section's own text. With words in it the field is
/// lit (`.is-focus`) and the hint gives way to a clear button.
pub fn search_field(ui: &Ui, focused: bool, placeholder: &'static str, t: &Theme) -> El {
    let has_words = !ui.search.text.is_empty();
    let lit = focused || has_words;
    if !has_words {
        kit::note_text(
            "search-placeholder",
            "search-input",
            placeholder,
            type_::SEARCH,
            t.text_tertiary,
        );
    } else {
        kit::note_text(
            "search-placeholder",
            "search-input",
            &ui.search.text,
            type_::SEARCH,
            t.text,
        );
    }
    let mut field = row()
        .key("search-box")
        .role("search")
        .w(210.0)
        .h(28.0)
        .gap(6.0)
        .pl(9.0)
        .pr(6.0)
        .radius(8.0)
        .center_y()
        .child(kit::glyph(
            "search-icon",
            &icons::SEARCH,
            13.0,
            if lit { t.text } else { t.text_tertiary },
        ))
        .child(
            text_input(&ui.search, type_::SEARCH)
                .key(crate::ui::keys::SEARCH_KEY)
                .label(placeholder)
                .shortcut_hint("Ctrl+F")
                .single_line()
                .color(t.text)
                // The board's caret (`.kit-caret`); the selection stays neutral.
                .caret(t.accent_deep, t.text.alpha(0.15))
                .placeholder(placeholder, t.text_tertiary)
                .on_edit(|s: &mut State| &mut s.this_pc.search)
                .grow(1.0),
        );
    if lit {
        field = field
            .fill(t.card)
            .box_shadow(kit::ring(3.0, t.accent.alpha(0.3)))
            .box_shadow(rune::Shadow {
                spread: 1.0,
                ..kit::ring_inset(t.accent_deep)
            });
    } else {
        field = field.fill(t.field).box_shadow(kit::ring_inset(t.separator));
    }
    if has_words {
        field = field.child(
            el().key("search-clear-btn")
                .role("button")
                .label("Clear search")
                .size(12.0, 12.0)
                .focusable(true)
                .on_press(|s: &mut State| {
                    s.this_pc.clear_search();
                })
                .child(kit::glyph("search-clear", &super::icons::CLOSE, 12.0, t.text_tertiary)),
        );
    } else {
        field = field.child(kbd("kbd-hint", "Ctrl+F", t));
    }
    field
}

/// The toolbar (`.mk-tb`): the one window toolbar (`ui::toolbar::bar`) with the section's title,
/// its own controls in the slot (`slot`), the Files controls as `how` says, and its search field.
pub fn toolbar(
    s: &State,
    ui: &Ui,
    name: &'static str,
    sub: String,
    slot: Vec<El>,
    how: FilesControls,
    search: El,
    t: &Theme,
) -> El {
    // Back and Forward go through the route machine.
    let (back, forward) = if ui.first_place {
        (false, false)
    } else {
        (s.shell.app.can_go_back(), s.shell.app.can_go_forward())
    };
    let dimmed = how == FilesControls::Dimmed;
    tb::bar(
        tb::Bar {
            name: name.into(),
            back: (
                back,
                tb::press(|s, _| {
                    host::dispatch(s, Event::Back);
                }),
            ),
            forward: (
                forward,
                tb::press(|s, _| {
                    host::dispatch(s, Event::Forward);
                }),
            ),
            title: title(name, sub, t),
            slot: Some(row().key("toolbar-slot").center_y().gap(8.0).children(slot)),
            slot_sep: false,
            // The board draws these and the section has nothing for them: there to be seen,
            // disabled to assistive technology, no focus (`Bar::inert`).
            view: if dimmed { tb::View::NotHere } else { tb::View::List },
            on_view: None,
            sort: tb::Sort::Idle,
            on_sort: None,
            details: if dimmed {
                tb::Details::NotHere
            } else {
                tb::Details::Open
            },
            on_details: None,
            inert: true,
            // These sections show no folder: the Beebeeb location.
            on_explorer: Box::new(tb::open_beebeeb_location),
            new: tb::New::Idle,
            on_new: tb::press(|s, _| s.this_pc.ask(super::Intent::New)),
            search,
            trailing_inset: 0.0,
        },
        t,
    )
}

/// An outlined small button (`.kit-btn-sm`, 22 pt): an optional glyph and its words. Keys:
/// `<key>`, `<key>-icon`, `<key>-label`.
pub fn outline_button(key: &str, words: &str, ic: Option<&'static Icon>, t: &Theme) -> El {
    let mut button = row()
        .key(key)
        .role("button")
        .label(words)
        .h(22.0)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(radius::MD)
        .fill(t.paper)
        .border(1.0, t.card_border)
        .border_layout()
        .hover(t.paper2)
        .press(t.paper3)
        .focusable(true);
    if let Some(ic) = ic {
        button = button.child(kit::glyph(format!("{key}-icon"), ic, 14.0, t.text));
    }
    button.child(kit::label(
        format!("{key}-label"),
        words,
        type_::BUTTON_SM.lh(17.14),
        t.text,
    ))
}

/// The status line (`.mk-foot`): the sync state at the left, the section's words at the right. The state is "Sync
/// stopped" while an item of this PC did not sync, else the word the window's own source has for sync (the one
/// Files' status line says); a source with none gets no dot and no word, never "Up to date" nobody read.
pub fn status_line(s: &State, ui: &Ui, right: String, t: &Theme) -> El {
    use crate::ui::status_line::{SYNC_STOPPED, SYNCING, UP_TO_DATE};
    let stopped = ui.problem_count() > 0;
    let state = if stopped {
        Some(SYNC_STOPPED.to_string())
    } else {
        s.actions.source.sync_state()
    };
    let mut left = row().key("status-left").center_y().gap(7.0);
    if let Some(state) = state {
        let dot = match state.as_str() {
            _ if stopped => t.red,
            SYNCING | UP_TO_DATE => t.green,
            _ => t.text_quaternary,
        };
        left = left
            .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(dot))
            .child(kit::label("status-text", state, type_::STATUS_STRONG, t.text_secondary));
    }
    row()
        .key("status-bar")
        .role("status")
        .h(28.0)
        .w_full()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .px(16.0)
        .gap(7.0)
        .center_y()
        .child(left)
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label("status-details", right, type_::STATUS, t.text_tertiary))
}
