//! The one Drive sidebar (`.mk-side`), in every window that has one: the brand, the five sections,
//! "This PC", the storage meter and the account row. Counts come from one place ([`counts`]), every
//! item goes through `host::dispatch`, and the window says which row is lit ([`Lit`]).

use crate::State;
use crate::frames::files::source::Account;
use crate::shell::app::{Event, Route, Screen, Section, SettingsPane};
use crate::shell::host;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use rune::{El, col, el, row};

use super::kit;
use super::sidebar::{Count, SidebarSection, Standing, brand, format_storage, pinned_item, section_item, side_item};

const SIDEBAR_W: f32 = 220.0;

/// Which row of the sidebar is the window's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lit {
    Section(SidebarSection),
    SyncHistory,
    AvailableOffline,
    Nothing,
}

/// The sidebar's figures. `None`: no figure (nothing there, or nobody said a number).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveCounts {
    pub shared_waiting: Option<u32>,
    pub trash: Option<u32>,
    pub available_offline: Option<u32>,
    pub sync_problems: Option<u32>,
}

fn some(n: u32) -> Option<u32> {
    (n > 0).then_some(n)
}

/// The figures every window's sidebar shows. Shared and Trash are the Files source's; in the Shared
/// and Trash windows themselves they are that window's own list (an accepted invitation or an
/// emptied Trash changes the figure at once, as ux-31 had it). Available offline and the sync
/// problems are This PC's. Never a sample.
pub fn counts(s: &State) -> DriveCounts {
    let files = s.actions.source.counts();
    let mut shared_waiting = files.and_then(|c| some(c.shared_waiting));
    let mut trash = files.and_then(|c| some(c.trash));
    match s.sidebar_section {
        SidebarSection::Shared | SidebarSection::Trash => {
            shared_waiting = some(s.frames.shared.source.invitations().len() as u32);
            trash = s.frames.trash.source.count().and_then(|n| some(n as u32));
        }
        // Another window: the engine's Trash figure once the server has said one (the Files source knows
        // none of its own, ux-55); never a sample.
        _ => {
            trash = s.frames.trash.source.server_count().and_then(|n| some(n as u32)).or(trash);
        }
    }
    DriveCounts {
        shared_waiting,
        trash,
        available_offline: some(s.this_pc.kept_count() as u32),
        sync_problems: some(s.this_pc.problem_count() as u32),
    }
}

/// The route a sidebar section goes to.
fn section(section: SidebarSection) -> Section {
    match section {
        SidebarSection::Files => Section::Files,
        SidebarSection::Shared => Section::Shared,
        SidebarSection::Photos => Section::Photos,
        SidebarSection::Backups => Section::Backups,
        SidebarSection::Trash => Section::Trash,
    }
}

/// Goes to `to` through the route machine (which points `s.sidebar_section` at a Drive section).
///
/// Files goes to the top of Files (dm-73), as a place in Finder's sidebar goes to its top: a folder opened inside it is
/// left to Back.
fn go(to: Section) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        host::dispatch(s, Event::Navigate(Route::section(to)));
        let in_files = matches!(s.shell.app.screen(), Screen::Drive(route) if route.section == Section::Files);
        if to == Section::Files && in_files && s.listing.folder.id.is_some() {
            s.open_folder(None);
        }
    }
}

/// An item while the gate is closed: drawn, and neither focusable nor pressable.
fn closed(item: El, open: bool) -> El {
    if open {
        item
    } else {
        item.disabled(true).focusable(false)
    }
}

/// A sidebar row as a tree item: role, the count in its name, selected when it is the window's own,
/// and the one roving Tab stop of the tree when lit.
fn tree_item(item: El, name: &str, count: Option<String>, lit: bool) -> El {
    let label = match count {
        Some(c) => format!("{name}, {c}"),
        None => name.to_string(),
    };
    let item = item.role("treeitem").label(label).focus_selected(lit);
    if lit { item.a11y_value("selected") } else { item }
}

/// A count that needs the person (`.mk-badge.is-red`): the items that did not sync.
fn red_pill(label: &str, n: u32, t: &Theme) -> El {
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
        .fill(crate::frames::this_pc::icons::red_badge())
        .center()
        .child(kit::label(
            format!("{label}-badge"),
            n.to_string(),
            type_::BADGE,
            t.white,
        ))
}

/// The Drive sidebar with `lit` the window's own row, in `standing`. Closed standings draw it
/// dimmed and inert with nothing lit.
pub fn drive_sidebar(s: &State, lit: Lit, standing: Standing, t: &Theme) -> El {
    let open = standing == Standing::Open;
    let no_plan = standing == Standing::NoPlan;
    let details = standing.shows_account_details();
    let lit = if open { lit } else { Lit::Nothing };
    let c = if no_plan || !details {
        DriveCounts::default()
    } else {
        counts(s)
    };

    let mut sections = col().key("sidebar-sections").gap(1.0);
    for &place in SidebarSection::all() {
        let count = match place {
            SidebarSection::Shared => c.shared_waiting.map(Count::Pill),
            SidebarSection::Trash => c.trash.map(Count::Plain),
            _ => None,
        };
        let words = match count {
            Some(Count::Pill(n)) => Some(format!("{n} waiting")),
            Some(Count::Plain(n)) => Some(format!("{n} items")),
            None => None,
        };
        let item = tree_item(
            section_item(place, count, lit == Lit::Section(place), t),
            place.label(),
            words,
            lit == Lit::Section(place),
        );
        sections = sections.child(if open {
            item.on_press(go(section(place)))
        } else {
            closed(item, open)
        });
    }

    // Pinned: the account's pinned folders (the web's Quick access), under the places. Not drawn while there are none,
    // nor while the window hides the account's details (names are the person's).
    let pinned: Vec<_> = if details && !no_plan { s.actions.source.pinned() } else { Vec::new() };
    let pinned_list = (!pinned.is_empty()).then(|| {
        let mut list = col()
            .key("pinned")
            .gap(1.0)
            .child(el().key("pinned-title").px(10.0).pb(3.0).child(kit::label(
                "pinned-label",
                "PINNED",
                type_::SECTION_LABEL,
                t.text_tertiary,
            )));
        for folder in &pinned {
            let id = folder.id.clone();
            let item = tree_item(pinned_item(&id, &folder.name, t), &folder.name, None, false)
                .description("Pinned folder");
            list = list.child(if open {
                item.on_press(move |s: &mut State| crate::frames::files::pins::open(s, &id))
            } else {
                closed(item, open)
            });
        }
        list
    });

    let mut history = side_item(
        "Sync history",
        &icons::SYNC_HISTORY,
        None,
        lit == Lit::SyncHistory,
        concat!(beebeeb_engine::primary_key!(), "6"),
        t,
    );
    if let Some(n) = c.sync_problems {
        history = history.child(red_pill("Sync history", n, t));
    }
    let offline_on = lit == Lit::AvailableOffline;
    let mut offline = side_item(
        "Available offline",
        &icons::AVAILABLE_OFFLINE,
        None,
        offline_on,
        concat!(beebeeb_engine::primary_key!(), "7"),
        t,
    );
    if let Some(n) = c.available_offline {
        // `side_item`'s plain count is ink-3 always; the board's is ink-2 on the lit item.
        offline = offline.child(kit::label(
            "Available offline-badge",
            n.to_string(),
            type_::KBD,
            if offline_on { t.text_secondary } else { t.text_tertiary },
        ));
    }
    let history = tree_item(
        history,
        "Sync history",
        c.sync_problems.map(|n| format!("{n} didn't sync")),
        lit == Lit::SyncHistory,
    );
    let offline = tree_item(
        offline,
        "Available offline",
        c.available_offline.map(|n| format!("{n} items")),
        offline_on,
    );
    let (history, offline) = if open {
        (
            history.on_press(go(Section::SyncHistory)),
            offline.on_press(go(Section::AvailableOffline)),
        )
    } else {
        (closed(history, open), closed(offline, open))
    };
    let this_pc = col()
        .key("this-pc")
        .gap(1.0)
        .child(el().key("this-pc-title").px(10.0).pb(3.0).child(kit::label(
            "this-pc-label",
            if cfg!(target_os = "macos") { "THIS MAC" } else { "THIS PC" },
            type_::SECTION_LABEL,
            t.text_tertiary,
        )))
        .child(history)
        .child(offline);

    let nav = col()
        .key("sidebar-nav")
        .role("tree")
        .label("Drive")
        .focus_group()
        .gap(12.0)
        .grow(1.0)
        .min_h(0.0)
        .child(sections)
        .children(pinned_list)
        .child(this_pc);

    let side = crate::ui::focus::pane(col(), "sidebar")
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
        .child(foot(s, standing, t));
    // `.mk-side` at 55 % while the gate is closed (the board).
    if open { side } else { side.opacity(0.55) }
}

/// The storage meter and the account row (`.mk-side-foot`). The figure is the storage the window
/// read (else "Unavailable" and no fill); the Files/Photos split only when the source gives one
/// (else one segment and no breakdown); the account as the source names it (none known: the row
/// keeps its place and names no one). Locked (`LOCKED_BLANKS_ACCOUNT`) keeps the shapes and shows
/// no figure and no account; NoPlan says "No plan" over an empty meter.
fn foot(s: &State, standing: Standing, t: &Theme) -> El {
    let open = standing == Standing::Open;
    let no_plan = standing == Standing::NoPlan;
    let details = standing.shows_account_details();
    let blanked = !details && !no_plan;
    let storage = s.storage;
    let listing = s.global.source.status().loading.is_some();

    let storage_value = if blanked {
        String::new()
    } else if no_plan {
        "No plan".to_string()
    } else if s.global.source.status().loading.is_some() {
        // The first listing runs: the totals are not known yet, never "0 B".
        "Calculating\u{2026}".to_string()
    } else if s.storage_known {
        format!(
            "{} of {}",
            format_storage(storage.used_bytes),
            format_storage(storage.quota_bytes)
        )
    } else {
        // No figure without the server (a `--db` source never asks it).
        "Unavailable".to_string()
    };
    let split = s
        .actions
        .source
        .storage_split()
        .filter(|_| s.storage_known && !no_plan && !blanked && !listing);
    let quota = storage.quota_bytes.max(1) as f64;
    let (files_bytes, photos_bytes, backups_bytes) = match split {
        Some(split) => (split.files_bytes, split.photos_bytes, split.backups_bytes),
        None if s.storage_known && !no_plan && !blanked => (storage.used_bytes, 0, 0),
        None => (0, 0, 0),
    };
    let files_frac = (files_bytes.max(0) as f64 / quota).min(1.0) as f32;
    let photos_frac = (photos_bytes.max(0) as f64 / quota).min(1.0) as f32;
    let backups_frac = (backups_bytes.max(0) as f64 / quota).min(1.0) as f32;
    let track_w = SIDEBAR_W - 2.0 * (10.0 + 6.0 + 4.0) - kit::HAIR;
    let mut meter = col()
        .key("storage-meter")
        .role("group")
        .label(format!("Storage: {storage_value}"))
        .gap(5.0)
        .px(4.0)
        .child(
            row()
                .key("storage-header")
                .justify_between()
                .child(kit::label("storage-label", "Storage", type_::CAPTION, t.text_secondary))
                .child(kit::label(
                    "storage-value",
                    storage_value.clone(),
                    // A figure is set in mono; "No plan" and blank are words.
                    if no_plan || blanked { type_::CAPTION } else { type_::KBD },
                    t.text_tertiary,
                )),
        )
        .child(
            row()
                .key("meter-track")
                .w_full()
                .h(5.0)
                .radius(2.5)
                .clip_rounded(2.5)
                .fill(t.separator)
                // Neutral fills: Amber is only encryption state and the primary action.
                .child(
                    el().key("meter-fill-0")
                        .h_full()
                        .w(track_w * files_frac)
                        // `.tone-neutral` on the empty meter of an account with no plan.
                        .fill(if no_plan { t.text_quaternary } else { t.text }),
                )
                .child(
                    el().key("meter-fill-1")
                        .h_full()
                        .w(track_w * photos_frac)
                        .fill(t.text_quaternary),
                )
                .child(
                    el().key("meter-fill-2")
                        .h_full()
                        .w(track_w * backups_frac)
                        .fill(t.text_tertiary),
                ),
        );
    if no_plan {
        meter = meter.child(kit::label(
            "storage-hint",
            "Choose a plan on beebeeb.io",
            type_::CAPTION,
            t.text_tertiary,
        ));
    } else if split.is_some() {
        // Files, Photos and Backups add up to the local total Insights shows. A drive with no Backups says none.
        let mut breakdown = row()
            .key("storage-breakdown")
            .justify_between()
            .child(kit::label(
                "storage-files",
                format!("Files {}", format_storage(files_bytes)),
                type_::KBD,
                t.text_tertiary,
            ))
            .child(kit::label(
                "storage-photos",
                format!("Photos {}", format_storage(photos_bytes)),
                type_::KBD,
                t.text_tertiary,
            ));
        if backups_bytes > 0 {
            breakdown = breakdown.child(kit::label(
                "storage-backups",
                format!("Backups {}", format_storage(backups_bytes)),
                type_::KBD,
                t.text_tertiary,
            ));
        }
        meter = meter.child(breakdown);
    }

    // Whose account: the source's own answer. Locked: none shown, the row keeps its shape.
    let who: Option<Account> = if details { s.actions.source.account() } else { None };
    let account = row().key("account-row").center_y().gap(8.0).pt(2.0).px(2.0);
    let account = match &who {
        // dc-167: the pill (avatar, name, email) opens the account switcher (`frames::account::switcher`).
        Some(who) => account.child(
            row()
                .key("account-pill")
                .role("button")
                .label(format!("{}, switch account", who.email))
                .center_y()
                .gap(8.0)
                .grow(1.0)
                .min_w(0.0)
                .radius(radius::CONTROL)
                .hover(t.selection)
                .press(t.press)
                .focusable(open)
                .on_press(crate::frames::account::switcher::toggle)
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
        ),
        // No account known, or blanked: the row keeps its height and names no one.
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
            .shortcut_hint(concat!(beebeeb_engine::primary_key!(), ","))
            .help(concat!("Settings  ", beebeeb_engine::primary_key!(), ","))
            .size(26.0, 26.0)
            .radius(radius::CONTROL)
            .center()
            .hover(t.selection)
            .press(t.press)
            .focusable(open)
            .disabled(!open)
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
        .children(crate::frames::account::switcher::panel(t).filter(|_| who.is_some()))
        .child(account)
}

#[cfg(test)]
#[path = "drive_sidebar_tests.rs"]
mod drive_sidebar_tests;
