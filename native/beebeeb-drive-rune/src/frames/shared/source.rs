//! What Shared shows and what it can do: a small trait with the board's samples as its fixture.

use super::parts::table::{Extra, Kind, Row, Thumb};
use crate::ui::file_list::PcState;

/// Someone wants to share something with the person (DW-0501's card).
#[derive(Clone, Debug, PartialEq)]
pub struct Invitation {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// "Lotte de Wit wants to share this folder"
    pub from_line: String,
    /// The row it becomes once accepted.
    pub row: Row,
}

/// A person on a share: initials and the board's avatar hue.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub initials: String,
    pub hue: char,
}

/// Something the person shares with people (DW-0502, "With people").
#[derive(Clone, Debug, PartialEq)]
pub struct PeopleShare {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// The people, as People with access lists them.
    pub names: String,
    pub people: Vec<Person>,
    /// Not accepted yet.
    pub pending: bool,
}

/// A link the person made (DW-0502, "Links").
#[derive(Clone, Debug, PartialEq)]
pub struct LinkShare {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// "Expires in 12 days", "Expired 3 Oct"
    pub line: String,
    pub expired: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestState {
    Open,
    Closed,
    Expired,
}

/// A file request (DW-0503).
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: String,
    pub title: String,
    pub line: String,
    pub state: RequestState,
}

pub trait SharedSource {
    fn invitations(&self) -> Vec<Invitation>;
    /// Shared with the person, newest first.
    fn with_me(&self) -> Vec<Row>;
    fn people_shares(&self) -> Vec<PeopleShare>;
    fn links(&self) -> Vec<LinkShare>;
    fn requests(&self) -> Vec<Request>;
    /// Accept an invitation: it joins `with_me`. `false`: no such invitation.
    fn accept(&mut self, id: &str) -> bool;
    /// Decline an invitation: it goes; the sender is not told why.
    fn decline(&mut self, id: &str) -> bool;
    /// Keep something shared with the person on this PC.
    fn keep_offline(&mut self, id: &str) -> bool;
    /// End a share with people or a link, straight away.
    fn stop_sharing(&mut self, id: &str) -> bool;
    /// Close a file request: no new uploads, what arrived stays.
    fn close_request(&mut self, id: &str) -> bool;
    /// Take a closed or expired request out of the list.
    fn remove_request(&mut self, id: &str) -> bool;
}

/// The board's sample (DW-0501, DW-0502, DW-0503), or nothing at all (DW-0508, DW-0509, DW-0510).
#[derive(Default)]
pub struct BoardShared {
    invitations: Vec<Invitation>,
    with_me: Vec<Row>,
    people: Vec<PeopleShare>,
    links: Vec<LinkShare>,
    requests: Vec<Request>,
}

fn from(id: &str, name: &str, kind: Kind, date: &str, size: &str, who: (&str, &str, char), pc: PcState) -> Row {
    Row {
        id: id.into(),
        name: name.into(),
        kind,
        date: date.into(),
        size: size.into(),
        extra: Extra::Person {
            initials: who.0.into(),
            name: who.1.into(),
            hue: who.2,
        },
        pc: Some(pc),
    }
}

impl BoardShared {
    pub fn new() -> Self {
        let person = |initials: &str, hue: char| Person {
            initials: initials.into(),
            hue,
        };
        let share = |id: &str, name: &str, kind, names: &str, people, pending| PeopleShare {
            id: id.into(),
            name: name.into(),
            kind,
            names: names.into(),
            people,
            pending,
        };
        let link = |id: &str, name: &str, kind, line: &str, expired| LinkShare {
            id: id.into(),
            name: name.into(),
            kind,
            line: line.into(),
            expired,
        };
        let request = |id: &str, title: &str, line: &str, state| Request {
            id: id.into(),
            title: title.into(),
            line: line.into(),
            state,
        };
        BoardShared {
            invitations: vec![Invitation {
                id: "inv-autumn-shoot".into(),
                name: "Autumn shoot".into(),
                kind: Kind::Folder,
                from_line: "Lotte de Wit wants to share this folder".into(),
                // The board shows this invitation only as a card; once accepted it lists like the others,
                // with what the card says: the name and who shares it.
                row: Row {
                    id: "s-autumn-shoot".into(),
                    name: "Autumn shoot".into(),
                    kind: Kind::Folder,
                    date: String::new(),
                    size: String::new(),
                    extra: Extra::Person {
                        initials: "LW".into(),
                        name: "Lotte de Wit".into(),
                        hue: 'd',
                    },
                    pc: Some(PcState::OnlineOnly),
                },
            }],
            with_me: vec![
                from(
                    "s-studio-assets",
                    "Studio assets",
                    Kind::Folder,
                    "30 Oct 2026",
                    "34 items",
                    ("JM", "Jonas Meier", 'c'),
                    PcState::OnlineOnly,
                ),
                from(
                    "s-brand-review",
                    "Brand review.pdf",
                    Kind::Document,
                    "27 Oct 2026",
                    "2.4 MB",
                    ("PS", "Priya Shah", 'c'),
                    PcState::Downloaded,
                ),
                from(
                    "s-harbourlight",
                    "Harbourlight shoot",
                    Kind::Folder,
                    "19 Oct 2026",
                    "118 items",
                    ("MJ", "Mara Jansen", 'e'),
                    PcState::OnlineOnly,
                ),
                from(
                    "s-rates-2026",
                    "Rates 2026.xlsx",
                    Kind::Sheet,
                    "12 Oct 2026",
                    "62 KB",
                    ("TL", "Tobias Lang", 'd'),
                    PcState::OnlineOnly,
                ),
            ],
            people: vec![
                share(
                    "p-lisbon",
                    "Lisbon trip",
                    Kind::Folder,
                    "Jonas Meier, Mia de Vries",
                    vec![person("JM", 'c'), person("MD", 'a')],
                    false,
                ),
                share(
                    "p-invoices",
                    "Invoices 2026",
                    Kind::Folder,
                    "Priya Shah",
                    vec![person("PS", 'c')],
                    false,
                ),
                share(
                    "p-contract",
                    "Contract draft.pdf",
                    Kind::Document,
                    "Sam Okafor",
                    vec![person("SO", 'b')],
                    true,
                ),
            ],
            links: vec![
                link("l-budget", "Budget 2026.xlsx", Kind::Sheet, "Expires in 12 days", false),
                link(
                    "l-harbour",
                    "Harbour.heic",
                    Kind::Photo(Some(Thumb::BoardDusk)),
                    "Expires in 27 days, with password",
                    false,
                ),
                link("l-quote", "Quote 2025.pdf", Kind::Document, "Expired 3 Oct", true),
            ],
            requests: vec![
                request(
                    "r-contracts",
                    "Signed contracts",
                    "3 files \u{00B7} Into Documents",
                    RequestState::Open,
                ),
                request(
                    "r-selects",
                    "Shoot selects",
                    "12 of 50 files \u{00B7} Expires 14 Nov",
                    RequestState::Open,
                ),
                request(
                    "r-invoices",
                    "Invoices for Q3",
                    "5 files \u{00B7} Into Invoices 2026",
                    RequestState::Closed,
                ),
                request(
                    "r-logos",
                    "Logo files",
                    "No files \u{00B7} Expired 3 Oct",
                    RequestState::Expired,
                ),
            ],
        }
    }

    /// An account nobody has shared with and that shares nothing.
    pub fn empty() -> Self {
        BoardShared::default()
    }
}

impl SharedSource for BoardShared {
    fn invitations(&self) -> Vec<Invitation> {
        self.invitations.clone()
    }
    fn with_me(&self) -> Vec<Row> {
        self.with_me.clone()
    }
    fn people_shares(&self) -> Vec<PeopleShare> {
        self.people.clone()
    }
    fn links(&self) -> Vec<LinkShare> {
        self.links.clone()
    }
    fn requests(&self) -> Vec<Request> {
        self.requests.clone()
    }
    fn accept(&mut self, id: &str) -> bool {
        let Some(at) = self.invitations.iter().position(|i| i.id == id) else {
            return false;
        };
        let invitation = self.invitations.remove(at);
        self.with_me.insert(0, invitation.row);
        true
    }
    fn decline(&mut self, id: &str) -> bool {
        let before = self.invitations.len();
        self.invitations.retain(|i| i.id != id);
        self.invitations.len() != before
    }
    fn keep_offline(&mut self, id: &str) -> bool {
        match self.with_me.iter_mut().find(|r| r.id == id) {
            Some(row) => {
                row.pc = Some(PcState::AvailableOffline);
                true
            }
            None => false,
        }
    }
    fn stop_sharing(&mut self, id: &str) -> bool {
        let before = self.people.len() + self.links.len();
        self.people.retain(|p| p.id != id);
        self.links.retain(|l| l.id != id);
        self.people.len() + self.links.len() != before
    }
    fn close_request(&mut self, id: &str) -> bool {
        match self
            .requests
            .iter_mut()
            .find(|r| r.id == id && r.state == RequestState::Open)
        {
            Some(request) => {
                request.state = RequestState::Closed;
                true
            }
            None => false,
        }
    }
    fn remove_request(&mut self, id: &str) -> bool {
        let before = self.requests.len();
        self.requests.retain(|r| r.id != id || r.state == RequestState::Open);
        self.requests.len() != before
    }
}
