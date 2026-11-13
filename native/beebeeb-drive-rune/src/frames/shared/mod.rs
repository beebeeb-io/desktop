//! Shared (`DW-0501` With me, `DW-0502` By me, `DW-0503` Requests, their empty states
//! `DW-0508`, `DW-0509`, `DW-0510`, and an item's menu `DW-0505`, `DW-0506`, `DW-0507`), and
//! the lane's dialogs (`dialogs`: DW-0504, DW-0511 to DW-0515, Trash's DW-0749). The checklist
//! is `FRAMES.md`.

pub mod dialogs;
pub mod parts;
pub mod scenes;
pub mod source;
#[cfg(test)]
pub(crate) mod tests;

use crate::State;
use crate::frames::{self, Intent};
use crate::shell::app::Section;
use crate::theme::tokens::{Theme, type_};
use crate::theme::{icons, tokens};
use crate::ui::kit;
use parts::controls::{ButtonKind, ButtonSize, Tone, button, button_keyed, pill, segments};
use parts::menu::{Entry, Open, Took, Trail};
use parts::table::{Columns, header, rows, table_row, tile};
use parts::toolbar::{Bar, slot_sep, toolbar};
use parts::{chrome, empty, list, style};
use rune::{Cx, El, Key, KeyEvent, Tx, col, el, row};
use source::{BoardShared, Loaded, Notice, RequestState, SharedSource};

const COLUMNS: Columns = Columns {
    names: ["Name", "Shared on", "Size", "From", "PC"],
    extra_w: 160.0,
    pc_w: 34.0,
};

/// The toolbar's switch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    WithMe,
    ByMe,
    Requests,
}

const TABS: [(Tab, &str); 3] = [
    (Tab::WithMe, "With me"),
    (Tab::ByMe, "By me"),
    (Tab::Requests, "Requests"),
];

pub struct SharedUi {
    pub source: Box<dyn SharedSource>,
    pub tab: Tab,
    /// The selected item of the tab that shows.
    pub selected: Option<usize>,
    /// The selected item's menu, while it is open (DW-0505, DW-0506, DW-0507).
    pub menu: Option<Open>,
    /// The lane's open dialog, over Shared or over Trash (`dialogs`).
    pub dialog: Option<dialogs::Dialog>,
    /// What the open dialog's fields hold.
    pub fields: dialogs::Fields,
    /// What the view says over the lists (a control not available yet, a change Beebeeb refused).
    pub notice: Option<Notice>,
    /// The gate and account the last poll saw: a change drops every name at once.
    seen: Option<(crate::shell::app::GateState, Option<String>)>,
    /// Whether the Shared section was open at the last poll.
    was_open: bool,
}

/// How often the poll runs again while a read or a change is on its way.
const POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// The window's poll (`shell::host::poll`), as Trash's: a gate or account change drops the source's names at once,
/// on the window's side, with the selection, the menu and any dialog about them; opening the Shared section asks for
/// the lists (the source decides whether they are old); what arrived is taken in. `true`: something drawn changed.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let gate = (s.shell.app.gate(), s.shell.app.account().map(str::to_owned));
    let open = s.sidebar_section == crate::ui::sidebar::SidebarSection::Shared
        && gate.0 == crate::shell::app::GateState::Ready;
    let ui = &mut s.frames.shared;
    let mut changed = false;
    if ui.seen.as_ref() != Some(&gate) {
        if ui.seen.is_some() {
            ui.source.forget();
            ui.selected = None;
            ui.menu = None;
            ui.notice = None;
            if ui.dialog_over(Section::Shared) {
                ui.close_dialog();
            }
            // The section must re-open on the next poll, so a new read starts for the new account.
            ui.was_open = false;
            changed = true;
        }
        ui.seen = Some(gate);
    }
    if open && !ui.was_open {
        ui.notice = None;
        ui.source.opened();
        changed = true;
    }
    ui.was_open = open;
    let pumped = ui.source.pump();
    if let Some(notice) = pumped.notice {
        ui.notice = Some(notice);
    }
    if pumped.busy {
        tx.wake_after(POLL_EVERY);
    }
    if pumped.changed {
        // An item that went takes the selection with it.
        let left = ui.count();
        ui.selected = ui.selected.filter(|i| *i < left);
        if ui.selected.is_none() {
            ui.menu = None;
        }
    }
    changed | pumped.changed
}

/// The words of a notice.
fn notice_words(notice: Notice) -> &'static str {
    match notice {
        Notice::NotAvailable => "Not available yet in this version. Nothing was changed.",
        Notice::ChangeRefused => "Couldn\u{2019}t make that change. Nothing was changed.",
        Notice::RefreshFailed => "Couldn\u{2019}t refresh Shared. What\u{2019}s shown may be out of date.",
    }
}

/// The notice over the lists (Trash's). Keys: `notice`, `notice-words`.
fn notice_note(notice: Notice, t: &Theme) -> El {
    const WORDS: rune::TextStyle = rune::TextStyle::new(12.5, 400.0).lh(17.0);
    el().key("notice-wrap").w_full().px(14.0).pt(10.0).child(
        col()
            .key("notice")
            .role("status")
            .label(notice_words(notice))
            .w_full()
            .pt(9.0)
            .pb(9.0)
            .pl(12.0)
            .pr(10.0)
            .radius(10.0)
            .fill(t.text.alpha(0.06))
            .child(
                rune::rich(notice_words(notice), Vec::new())
                    .key("notice-words")
                    .font(WORDS)
                    .color(t.text_secondary)
                    .w_full(),
            ),
    )
}

/// The words of lists that are not here (`None`: they are, draw them).
fn not_here(loaded: Loaded) -> Option<(&'static str, &'static str)> {
    match loaded {
        Loaded::Ready => None,
        Loaded::Loading => Some(("Loading Shared\u{2026}", "Getting your shares from Beebeeb.")),
        Loaded::Offline => Some((
            "Can\u{2019}t reach Beebeeb",
            "Shared shows again once you\u{2019}re online.",
        )),
        Loaded::Failed => Some((
            "Couldn\u{2019}t load Shared",
            "Open Shared again in a moment to try again.",
        )),
        Loaded::SessionEnded => Some(("Your session ended", "Sign in again to see Shared.")),
        Loaded::SignedOut => Some(("Signed out", "Sign in to see Shared.")),
    }
}

impl Default for SharedUi {
    fn default() -> Self {
        SharedUi {
            source: Box::new(BoardShared::new()),
            tab: Tab::WithMe,
            selected: None,
            menu: None,
            dialog: None,
            fields: dialogs::Fields::default(),
            notice: None,
            seen: None,
            was_open: false,
        }
    }
}

impl SharedUi {
    pub fn show(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.selected = None;
            self.menu = None;
        }
    }

    /// Enter (or a double-click) on the selected item: a share with people opens People with
    /// access here (DW-0511); anything else asks for what it points at.
    pub fn open_selected(&mut self) -> Option<Intent> {
        let i = self.selected?;
        if self.tab == Tab::ByMe && i < self.source.people_shares().len() {
            let id = self.selected_id()?;
            self.show_people(&id);
            return None;
        }
        self.open_intent()
    }

    /// How many items the tab that shows lists.
    pub fn count(&self) -> usize {
        match self.tab {
            Tab::WithMe => self.source.with_me().len(),
            Tab::ByMe => self.source.people_shares().len() + self.source.links().len(),
            Tab::Requests => self.source.requests().len(),
        }
    }

    /// What opening the selected item asks of another frame. A share with people asks
    /// nothing: People with access is drawn here (`open_selected`).
    pub fn open_intent(&self) -> Option<Intent> {
        let i = self.selected?;
        match self.tab {
            Tab::WithMe => self.source.with_me().get(i).map(|r| Intent::OpenItem(r.id.clone())),
            // A link or a request opens what it points at.
            Tab::ByMe if i < self.source.people_shares().len() => None,
            Tab::ByMe => self
                .source
                .links()
                .get(i - self.source.people_shares().len())
                .map(|l| Intent::OpenItem(l.id.clone())),
            Tab::Requests => self.source.requests().get(i).map(|r| Intent::OpenItem(r.id.clone())),
        }
    }

    /// The id of the selected item.
    fn selected_id(&self) -> Option<String> {
        let i = self.selected?;
        let people = self.source.people_shares();
        match self.tab {
            Tab::WithMe => self.source.with_me().get(i).map(|r| r.id.clone()),
            Tab::ByMe if i < people.len() => Some(people[i].id.clone()),
            Tab::ByMe => self.source.links().get(i - people.len()).map(|l| l.id.clone()),
            Tab::Requests => self.source.requests().get(i).map(|r| r.id.clone()),
        }
    }

    /// The key of item `i` of the tab that shows: focus goes where the selection goes.
    pub fn item_key(&self, i: usize) -> String {
        let people = self.source.people_shares().len();
        match self.tab {
            Tab::WithMe => format!("row-{i}"),
            Tab::ByMe if i < people => format!("item-0-{i}"),
            Tab::ByMe => format!("item-1-{}", i - people),
            Tab::Requests => format!("item-0-{i}"),
        }
    }

    /// The key of the selected item.
    pub fn selected_key(&self) -> Option<String> {
        self.selected.map(|i| self.item_key(i))
    }

    /// The item of the selected item's menu that shows these keys ("Space", "Alt+Enter"): a
    /// key the menu shows does what that item does, also while the menu is closed.
    pub fn hinted(&self, keys: &str) -> Option<usize> {
        let (_, entries, _) = self.menu_entries();
        entries
            .iter()
            .position(|e| matches!(e.trail, Trail::Keys(k) if k == keys))
    }

    /// Move the selection to item `i`.
    fn select(&mut self, i: usize, tx: &mut Tx) {
        self.selected = Some(i);
        tx.focus(self.item_key(i), true);
    }

    /// The selected item's menu (DW-0505, DW-0506, DW-0507) and its name.
    pub fn menu_entries(&self) -> (&'static str, &'static [Entry], f32) {
        let Some(i) = self.selected else {
            return ("", &[], 0.0);
        };
        match self.tab {
            Tab::WithMe => ("Shared with you", MENU_WITH_ME, 220.0),
            // A share with people offers People and Stop sharing instead of the link items.
            Tab::ByMe if i < self.source.people_shares().len() => ("Shared with people", MENU_PEOPLE, 210.0),
            Tab::ByMe => ("Link", MENU_LINK, 210.0),
            Tab::Requests => match self.source.requests().get(i).map(|r| r.state) {
                Some(RequestState::Open) => ("File request", MENU_REQUEST, 200.0),
                // A closed or expired request only offers View files and Remove.
                _ => ("File request", MENU_REQUEST_OVER, 200.0),
            },
        }
    }

    /// Do what the menu's item `n` says, and close the menu.
    pub fn pick(&mut self, n: usize) -> Option<Intent> {
        let (_, entries, _) = self.menu_entries();
        self.menu = None;
        let words = entries.get(n)?.words;
        let id = self.selected_id()?;
        let mut gone = false;
        let intent = match words {
            "Open" | "View files" => self.open_intent(),
            "Preview" => Some(Intent::Do("preview", id)),
            "Make available offline" => {
                self.source.keep_offline(&id);
                None
            }
            "Show in File Explorer" => Some(Intent::About("DW-1180", id)),
            "Details" => Some(Intent::About("DW-0411", id)),
            // The confirmation is the shell's (the board draws none); the engine call follows it.
            "Leave\u{2026}" => Some(Intent::Do("leave", id)),
            "Copy link" => Some(Intent::Do("copy-link", id)),
            "Change expiry" => Some(Intent::Do("change-expiry", id)),
            "Password\u{2026}" => Some(Intent::Do("link-password", id)),
            // People with access is drawn here for a share with people; a link's People is
            // another frame's to decide (the board draws none).
            "People" if self.show_people(&id) => None,
            "People" => Some(Intent::About("DW-0511", id)),
            "Stop sharing" => {
                gone = self.source.stop_sharing(&id);
                None
            }
            "Close request" => {
                self.source.close_request(&id);
                None
            }
            "Remove" => {
                gone = self.source.remove_request(&id);
                None
            }
            _ => None,
        };
        if gone {
            let left = self.count();
            self.selected = self.selected.filter(|_| left > 0).map(|i| i.min(left - 1));
        }
        intent
    }
}

const MENU_WITH_ME: &[Entry] = &[
    Entry::new("Open").keys("Enter"),
    Entry::new("Preview").keys("Space"),
    Entry::new("Make available offline").rule(),
    Entry::new("Show in File Explorer"),
    Entry::new("Details").keys("Alt+Enter"),
    Entry::new("Leave\u{2026}").danger().rule(),
];
const MENU_LINK: &[Entry] = &[
    Entry::new("Copy link"),
    Entry::new("Change expiry").more(),
    Entry::new("Password\u{2026}"),
    Entry::new("Open").keys("Enter"),
    Entry::new("People").rule(),
    Entry::new("Stop sharing").danger(),
];
const MENU_PEOPLE: &[Entry] = &[Entry::new("People"), Entry::new("Stop sharing").danger()];
const MENU_REQUEST: &[Entry] = &[
    Entry::new("Copy link"),
    Entry::new("View files"),
    Entry::new("Change expiry").more(),
    Entry::new("Close request").danger().rule(),
];
const MENU_REQUEST_OVER: &[Entry] = &[Entry::new("View files"), Entry::new("Remove").danger().rule()];

/// Do what the menu's item `n` says; focus goes back to the item the menu was about (or to
/// the one that took its place), unless the item opened a dialog: focus is the dialog's then.
fn pick(s: &mut State, n: usize, tx: &mut Tx) {
    if let Some(intent) = s.frames.shared.pick(n) {
        s.frames.intents.push(intent);
    }
    if !s.frames.shared.dialog_over(Section::Shared) {
        return_focus(s, tx);
    }
}

fn close_menu(s: &mut State, tx: &mut Tx) {
    s.frames.shared.menu = None;
    return_focus(s, tx);
}

fn return_focus(s: &State, tx: &mut Tx) {
    if let Some(key) = s.frames.shared.selected_key() {
        tx.return_focus(key, tx.focus_visible());
    }
}

/// The pointer is on the menu's item `n`.
fn light(s: &mut State, n: usize) {
    if let Some(open) = s.frames.shared.menu.as_mut() {
        open.hot = n;
    }
}

fn select(i: usize) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| s.frames.shared.selected = Some(i)
}

fn open(i: usize) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        s.frames.shared.selected = Some(i);
        if let Some(intent) = s.frames.shared.open_selected() {
            s.frames.intents.push(intent);
        }
    }
}

/// A secondary press on item `i`: its menu at the pointer (or, from the keyboard through
/// Rune, at the item's corner).
fn menu(i: usize) -> impl Fn(&mut State, rune::PointerPress, &mut Tx) + 'static {
    move |s: &mut State, press, _| {
        s.frames.shared.selected = Some(i);
        s.frames.shared.menu = Some(Open::at(press.x, press.y));
    }
}

/// The menu of item `i`, opened under the element keyed `under` (the Menu key, the More button).
fn menu_under(i: usize, under: String) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        s.frames.shared.selected = Some(i);
        s.frames.shared.menu = Some(Open::under(under.clone()));
    }
}

/// A click on the people of share `i`: People with access (DW-0502's action, DW-0511).
fn open_people(i: usize) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        s.frames.shared.selected = Some(i);
        if let Some(id) = s.frames.shared.selected_id() {
            s.frames.shared.show_people(&id);
        }
    }
}

/// An item of a list is a Tab stop only when it is the selected one (the first, when none is):
/// the list is one stop, and the arrows move inside it.
fn stop(ui: &SharedUi, i: usize) -> i32 {
    if ui.selected.unwrap_or(0) == i { 0 } else { -1 }
}

fn new_request(t: &Theme) -> El {
    button("New request", ButtonKind::Outline, Some(&icons::PLUS), t)
        .on_press(|s: &mut State| s.frames.intents.push(Intent::Frame("DW-0504")))
}

/// The "Waiting for you" card (`.kit-card`): one invitation, Decline and Accept.
fn invitation_card(n: usize, inv: &source::Invitation, t: &Theme) -> El {
    let k = format!("card-{n}");
    let (accept, decline) = (inv.id.clone(), inv.id.clone());
    let words = col()
        .key("card-text")
        .grow(1.0)
        .min_w(0.0)
        .child(kit::label(
            format!("{k}-label"),
            "WAITING FOR YOU",
            type_::SECTION_LABEL,
            t.text_tertiary,
        ))
        .child(
            kit::label(format!("{k}-head"), inv.name.clone(), type_::HEAD, t.text)
                .mt(2.0)
                .truncate(),
        )
        .child(kit::label(format!("{k}-sub"), inv.from_line.clone(), style::SUB, t.text_tertiary).truncate());
    el().key(format!("{k}-wrap")).w_full().px(16.0).pt(12.0).child(
        row()
            .key(k.clone())
            .role("group")
            .label(format!("Waiting for you: {}", inv.name))
            .w_full()
            .padding(12.0)
            .gap(12.0)
            .center_y()
            .radius(10.0)
            .fill(t.card)
            .box_shadow(kit::drop(1.0, 2.0, rune::Color::BLACK.alpha(0.04)))
            .box_shadow(kit::ring(kit::HAIR, t.separator))
            .child(tile(&k, format!("{k}-icon"), inv.kind, 36.0, 9.0, 22.0, t))
            .child(words)
            .child(
                button("Decline", ButtonKind::Outline, None, t).on_press(move |s: &mut State| {
                    s.frames.shared.source.decline(&decline);
                }),
            )
            .child(
                button("Accept", ButtonKind::Outline, None, t).on_press(move |s: &mut State| {
                    s.frames.shared.source.accept(&accept);
                }),
            ),
    )
}

fn with_me(ui: &SharedUi, t: &Theme) -> (El, String, usize) {
    let invitations = ui.source.invitations();
    let items = ui.source.with_me();
    let count = items.len() + invitations.len();
    let status = format!(
        "Shared with you: {} \u{00B7} waiting: {}",
        items.len(),
        invitations.len()
    );
    if count == 0 {
        let body = empty::empty(
            &icons::SHARED,
            "Nothing shared with you",
            "When someone shares a file or folder with you, it appears here.",
            None,
            t,
        );
        return (body, status, 0);
    }
    let mut body = col().key("list-body");
    for (n, inv) in invitations.iter().enumerate() {
        body = body.child(invitation_card(n, inv, t));
    }
    let list = items
        .iter()
        .enumerate()
        .map(|(i, r)| {
            table_row(i, r, ui.selected == Some(i), &COLUMNS, t)
                .tab_index(stop(ui, i))
                .on_press(select(i))
                .on_double_press(open(i))
                .on_secondary_press(menu(i))
        })
        .collect();
    body =
        body.child(el().key("label-wrap").px(20.0).pt(16.0).child(list::label(
            "shared-with-you",
            "Shared with you",
            t,
        )))
        .child(header(&COLUMNS, t))
        .child(rows("Shared with you", list));
    (body, status, count)
}

fn by_me(ui: &SharedUi, t: &Theme) -> (El, String, usize) {
    let people = ui.source.people_shares();
    let links = ui.source.links();
    let count = people.len() + links.len();
    let status = format!("{} shared with people \u{00B7} {} links", people.len(), links.len());
    if count == 0 {
        let body = empty::empty(
            &icons::SHARE,
            "You haven't shared anything",
            "Right-click any file or folder in Files and choose Share\u{2026} to share it.",
            None,
            t,
        );
        return (body, status, 0);
    }
    let people_items = people
        .iter()
        .enumerate()
        .map(|(m, p)| {
            let k = format!("item-0-{m}");
            // "Click the people": People with access. Not a Tab stop: Enter on the item does the same.
            let mut avatars = row()
                .key(format!("{k}-people"))
                .role("button")
                .label(format!("People with access to {}", p.name))
                .focusable(true)
                .tab_index(-1)
                .on_press(open_people(m));
            for (j, person) in p.people.iter().enumerate() {
                let a = kit::avatar(
                    format!("{k}-avatar-{j}"),
                    &person.initials,
                    22.0,
                    parts::hue(person.hue),
                    t.card,
                    t,
                );
                avatars = avatars.child(if j > 0 { a.ml(-7.0) } else { a });
            }
            let mut trailing = vec![avatars];
            if p.pending {
                trailing.push(pill(&k, "Pending", Tone::Neutral, true, t));
            }
            list::item(
                0,
                m,
                p.kind,
                None,
                &p.name,
                &p.names,
                trailing,
                list::more(0, m, &p.name, t).on_press(menu_under(m, list::more_key(0, m))),
                ui.selected == Some(m),
                t,
            )
            .tab_index(stop(ui, m))
            .on_press(select(m))
            .on_double_press(open(m))
            .on_secondary_press(menu(m))
        })
        .collect();
    let link_items = links
        .iter()
        .enumerate()
        .map(|(m, l)| {
            let k = format!("item-1-{m}");
            let i = people.len() + m;
            let mut trailing = vec![
                row()
                    .key("link")
                    .center_y()
                    .gap(4.0)
                    .child(kit::glyph(
                        format!("{k}-link-icon"),
                        &icons::LINK,
                        12.0,
                        t.text_secondary,
                    ))
                    .child(kit::label(
                        format!("{k}-link"),
                        "Link",
                        type_::STATUS_STRONG.lh(14.0),
                        t.text_secondary,
                    )),
            ];
            if l.expired {
                trailing.push(pill(&k, "Expired", Tone::Neutral, false, t));
            }
            list::item(
                1,
                m,
                l.kind,
                None,
                &l.name,
                &l.line,
                trailing,
                list::more(1, m, &l.name, t).on_press(menu_under(i, list::more_key(1, m))),
                ui.selected == Some(i),
                t,
            )
            .tab_index(stop(ui, i))
            .on_press(select(i))
            .on_double_press(open(i))
            .on_secondary_press(menu(i))
        })
        .collect();
    let body = col()
        .key("list-body")
        .pt(12.0)
        .gap(8.0)
        .child(
            el().key("label-wrap-0")
                .px(20.0)
                .child(list::label("with-people", "With people", t)),
        )
        .child(list::list(0, "With people", people_items, t))
        .child(
            el().key("label-wrap-1")
                .px(20.0)
                .pt(12.0)
                .child(list::label("links", "Links", t)),
        )
        .child(list::list(1, "Links", link_items, t));
    (body, status, count)
}

fn requests(ui: &SharedUi, t: &Theme) -> (El, String, usize) {
    let requests = ui.source.requests();
    let of = |state| requests.iter().filter(|r| r.state == state).count();
    let status = format!(
        "{} open \u{00B7} {} closed \u{00B7} {} expired",
        of(RequestState::Open),
        of(RequestState::Closed),
        of(RequestState::Expired)
    );
    if requests.is_empty() {
        let body = empty::empty(
            &parts::icons::REQUEST,
            "No file requests",
            "Ask anyone to send you files, even without a Beebeeb account.",
            Some(
                button_keyed(
                    "btn-new-request-2".into(),
                    "New request",
                    ButtonKind::Outline,
                    ButtonSize::Medium,
                    Some(&icons::PLUS),
                    t,
                )
                .on_press(|s: &mut State| s.frames.intents.push(Intent::Frame("DW-0504"))),
            ),
            t,
        );
        return (body, status, 0);
    }
    let items = requests
        .iter()
        .enumerate()
        .map(|(m, r)| {
            let k = format!("item-0-{m}");
            let state = match r.state {
                RequestState::Open => pill(&k, "Open", Tone::Green, true, t),
                RequestState::Closed => pill(&k, "Closed", Tone::Neutral, false, t),
                RequestState::Expired => pill(&k, "Expired", Tone::Neutral, false, t),
            };
            list::item(
                0,
                m,
                parts::table::Kind::Folder,
                Some(&parts::icons::REQUEST),
                &r.title,
                &r.line,
                vec![state],
                list::more(0, m, &r.title, t).on_press(menu_under(m, list::more_key(0, m))),
                ui.selected == Some(m),
                t,
            )
            .tab_index(stop(ui, m))
            .on_press(select(m))
            .on_double_press(open(m))
            .on_secondary_press(menu(m))
        })
        .collect();
    let body = col()
        .key("list-body")
        .pt(12.0)
        .gap(8.0)
        .child(list::list(0, "File requests", items, t));
    (body, status, requests.len())
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    // Nothing answers for Shared yet (a build that ships before the engine lane wires it):
    // the shell's "not built yet" frame, never a section of sample values.
    if !s.frames.shared.source.wired() {
        return crate::shell::placeholder::window(s, cx, crate::shell::registry::group_for(&s.shell.app));
    }
    let t = tokens::theme(cx.system_dark());
    let ui = &s.frames.shared;
    // A source that answers late: its loading, offline, error and signed-out states instead of an empty list.
    let away = not_here(ui.source.loaded());
    let (body, status, count) = match (away, ui.tab) {
        (Some((title, words)), _) => (empty::empty(&icons::SHARED, title, words, None, &t), String::new(), 0),
        (None, Tab::WithMe) => with_me(ui, &t),
        (None, Tab::ByMe) => by_me(ui, &t),
        (None, Tab::Requests) => requests(ui, &t),
    };
    let body = match ui.notice {
        Some(notice) => col()
            .key("notice-body")
            .w_full()
            .grow(1.0)
            .child(notice_note(notice, &t))
            .child(body),
        None => body,
    };
    let sub = match (ui.tab, count) {
        (_, 0) if away.is_some() => String::new(),
        (_, 0) => "No items".to_string(),
        (Tab::WithMe, 1) => "1 item".to_string(),
        (Tab::WithMe, n) => format!("{n} items"),
        (Tab::ByMe, 1) => "1 share".to_string(),
        (Tab::ByMe, n) => format!("{n} shares"),
        (Tab::Requests, 1) => "1 request".to_string(),
        (Tab::Requests, n) => format!("{n} requests"),
    };
    let tabs: Vec<(&'static str, bool)> = TABS.iter().map(|(tab, words)| (*words, *tab == ui.tab)).collect();
    let mut slot = vec![segments(0, "Shared", &tabs, &t, |i, tab| {
        let to = TABS[i].0;
        tab.on_press(move |s: &mut State| s.frames.shared.show(to))
    })];
    // New request sits beside the switch on Requests, also while there is none yet (DW-0510 draws both).
    let slot_has_sep = ui.tab == Tab::Requests;
    if slot_has_sep {
        slot.push(slot_sep(&t));
        slot.push(new_request(&t));
    }
    let bar = toolbar(
        s,
        Bar {
            title: "Shared",
            sub,
            slot,
            slot_sep: slot_has_sep,
            new_off: false,
        },
        &t,
    );
    // With nothing to count, the status line is the PC's summary, as in Files (DW-0508, DW-0509, DW-0510).
    let status = if count == 0 { s.frames.pc.summary() } else { status };
    let window = cx.layout_size("window");
    let layer = ui.menu.as_ref().map(|open| {
        let (name, entries, min_w) = ui.menu_entries();
        parts::menu::menu(name, entries, open, min_w, window, &t, pick, close_menu, light, on_key)
    });
    chrome::window(s, cx, &t, bar, body, &status, layer)
}

/// Shared's keys. A menu takes every key while it is open. Otherwise: on the switch, Left and
/// Right change the tab and Down goes into the list; in the list (or with focus nowhere) the
/// arrows, Home and End move the selection AND focus, Enter opens, the keys an item's menu
/// shows (Space, Alt+Enter) do what that item does, and the Menu key or Shift+F10 opens the
/// menu of the SELECTED item under it. In the search field, Enter asks for the search.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // A dialog in front takes every key that reaches the section: the list never moves under it.
    if let Some(took) = dialogs::section_keys(s, ev, tx, Section::Shared) {
        return took;
    }
    if let Some(mut open) = s.frames.shared.menu.take() {
        let entries = s.frames.shared.menu_entries().1;
        let took = open.on_key(ev, entries, tx);
        s.frames.shared.menu = Some(open);
        match took {
            Took::Pick(n) => pick(s, n, tx),
            Took::Close => close_menu(s, tx),
            Took::Key => {}
        }
        return true;
    }
    if ev.is_command('f') {
        tx.focus(crate::ui::keys::SEARCH_KEY, true);
        return true;
    }
    let focused = tx.focused().map(str::to_owned);
    match focused.as_deref() {
        Some(key) if key == crate::ui::keys::SEARCH_KEY => return parts::toolbar::search_key(s, ev),
        Some(key) if key.starts_with("seg-0-") => return tab_key(s, ev, tx),
        Some(key) if !key.starts_with("row-") && !key.starts_with("item-") => return false,
        _ => {}
    }
    let count = s.frames.shared.count();
    if count == 0 {
        return false;
    }
    let ui = &mut s.frames.shared;
    if frames::is_menu_key(ev) {
        return match ui.selected_key() {
            Some(key) => {
                ui.menu = Some(Open::under(key));
                true
            }
            None => false,
        };
    }
    if frames::is_alt_enter(ev) || frames::is_space(ev) {
        let keys = if frames::is_space(ev) { "Space" } else { "Alt+Enter" };
        return match ui.hinted(keys) {
            Some(n) => {
                pick(s, n, tx);
                true
            }
            None => false,
        };
    }
    if !frames::plain(ev) {
        return false;
    }
    match ev.key {
        Key::Down => ui.select(ui.selected.map_or(0, |i| (i + 1).min(count - 1)), tx),
        Key::Up => ui.select(ui.selected.map_or(0, |i| i.saturating_sub(1)), tx),
        Key::Home => ui.select(0, tx),
        Key::End => ui.select(count - 1, tx),
        Key::Enter => match ui.open_selected() {
            Some(intent) => s.frames.intents.push(intent),
            // People with access opened here: the key is taken.
            None if ui.dialog_over(Section::Shared) => {}
            None => return false,
        },
        _ => return false,
    }
    true
}

/// The keys of the switch while one of its tabs has focus.
fn tab_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if !frames::plain(ev) {
        return false;
    }
    let ui = &mut s.frames.shared;
    let at = TABS.iter().position(|(tab, _)| *tab == ui.tab).unwrap_or(0);
    let to = match ev.key {
        Key::Left => at.saturating_sub(1),
        Key::Right => (at + 1).min(TABS.len() - 1),
        Key::Home => 0,
        Key::End => TABS.len() - 1,
        // Into the list: the selected item, or the first.
        Key::Down if ui.count() > 0 => {
            ui.select(ui.selected.unwrap_or(0), tx);
            return true;
        }
        _ => return false,
    };
    ui.show(TABS[to].0);
    tx.focus(parts::controls::segment_key(0, TABS[to].1), true);
    true
}
