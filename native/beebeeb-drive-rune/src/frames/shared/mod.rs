//! Shared (`DW-0501` With me, `DW-0502` By me, `DW-0503` Requests, and their empty states
//! `DW-0508`, `DW-0509`, `DW-0510`). Its menus and dialogs are listed in `FRAMES.md`.

pub mod parts;
pub mod scenes;
pub mod source;
#[cfg(test)]
mod tests;

use crate::State;
use crate::frames::Intent;
use crate::theme::tokens::{Theme, type_};
use crate::theme::{icons, tokens};
use crate::ui::kit;
use parts::controls::{ButtonKind, ButtonSize, Tone, button, button_keyed, pill, segments};
use parts::table::{Columns, header, rows, table_row, tile};
use parts::toolbar::{Bar, slot_sep, toolbar};
use parts::{chrome, empty, list, style};
use rune::{Cx, El, Key, KeyEvent, Tx, col, el, row};
use source::{BoardShared, RequestState, SharedSource};

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
}

impl Default for SharedUi {
    fn default() -> Self {
        SharedUi {
            source: Box::new(BoardShared::new()),
            tab: Tab::WithMe,
            selected: None,
        }
    }
}

impl SharedUi {
    pub fn show(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.selected = None;
        }
    }

    /// How many items the tab that shows lists.
    pub fn count(&self) -> usize {
        match self.tab {
            Tab::WithMe => self.source.with_me().len(),
            Tab::ByMe => self.source.people_shares().len() + self.source.links().len(),
            Tab::Requests => self.source.requests().len(),
        }
    }

    /// What Enter (or a double-click) on the selected item asks for.
    pub fn open_intent(&self) -> Option<Intent> {
        let i = self.selected?;
        match self.tab {
            Tab::WithMe => self.source.with_me().get(i).map(|r| Intent::OpenItem(r.id.clone())),
            // A share with people opens People with access; a link or a request opens what it points at.
            Tab::ByMe if i < self.source.people_shares().len() => Some(Intent::Frame("DW-0511")),
            Tab::ByMe => self
                .source
                .links()
                .get(i - self.source.people_shares().len())
                .map(|l| Intent::OpenItem(l.id.clone())),
            Tab::Requests => self.source.requests().get(i).map(|r| Intent::OpenItem(r.id.clone())),
        }
    }

    /// The menu of the selected item: DW-0505, DW-0506 or DW-0507.
    pub fn menu_intent(&self) -> Intent {
        Intent::Frame(match self.tab {
            Tab::WithMe => "DW-0505",
            Tab::ByMe => "DW-0506",
            Tab::Requests => "DW-0507",
        })
    }
}

fn select(i: usize) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| s.frames.shared.selected = Some(i)
}

fn open(i: usize) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        s.frames.shared.selected = Some(i);
        if let Some(intent) = s.frames.shared.open_intent() {
            s.frames.intents.push(intent);
        }
    }
}

fn menu(i: usize) -> impl Fn(&mut State, rune::PointerPress, &mut Tx) + 'static {
    move |s: &mut State, _, _| {
        s.frames.shared.selected = Some(i);
        let intent = s.frames.shared.menu_intent();
        s.frames.intents.push(intent);
    }
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
            let mut avatars = row().key("avatars").role("group").label("Shared with");
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
                ui.selected == Some(m),
                t,
            )
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
                ui.selected == Some(i),
                t,
            )
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
                ui.selected == Some(m),
                t,
            )
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
    let t = tokens::theme(cx.system_dark());
    let ui = &s.frames.shared;
    let (body, status, count) = match ui.tab {
        Tab::WithMe => with_me(ui, &t),
        Tab::ByMe => by_me(ui, &t),
        Tab::Requests => requests(ui, &t),
    };
    let sub = match (ui.tab, count) {
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
    // With nothing to count, the board's status line is the PC's summary, as in Files (DW-0508, DW-0509, DW-0510).
    let status = if count == 0 {
        chrome::PC_SUMMARY.to_string()
    } else {
        status
    };
    chrome::window(s, cx, &t, bar, body, &status, None)
}

/// Shared's keys: the arrows select, Enter opens. The menu key is Rune's (it calls the row's
/// `on_secondary_press`).
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.is_command('f') {
        tx.focus(crate::ui::keys::SEARCH_KEY, true);
        return true;
    }
    if tx
        .focused()
        .is_some_and(|key| !key.starts_with("row-") && !key.starts_with("item-"))
    {
        return false;
    }
    let count = s.frames.shared.count();
    if count == 0 {
        return false;
    }
    let ui = &mut s.frames.shared;
    match ev.key {
        Key::Down => {
            ui.selected = Some(ui.selected.map_or(0, |i| (i + 1).min(count - 1)));
            true
        }
        Key::Up => {
            ui.selected = Some(ui.selected.map_or(0, |i| i.saturating_sub(1)));
            true
        }
        Key::Enter => match ui.open_intent() {
            Some(intent) => {
                s.frames.intents.push(intent);
                true
            }
            None => false,
        },
        _ => false,
    }
}
