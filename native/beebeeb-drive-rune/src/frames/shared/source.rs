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
    /// How much it holds, in words ("212 items"), when the source knows: People with access
    /// says it under the name (DW-0511).
    pub contents: Option<String>,
}

/// What a person may do with a share (DW-0512's switch, DW-0514's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    View,
    Edit,
}

impl Access {
    /// The board's words: "Can view", "Can edit".
    pub fn words(self) -> &'static str {
        match self {
            Access::View => "Can view",
            Access::Edit => "Can edit",
        }
    }
}

/// The pronoun a sentence about a person uses (DW-0512b: "goes from his list"). Beebeeb does
/// not ask anyone for one: a person the engine lists is `They`; only the board's sample people
/// carry the board's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pronoun {
    He,
    She,
    #[default]
    They,
}

impl Pronoun {
    /// "his", "her", "their".
    pub fn possessive(self) -> &'static str {
        match self {
            Pronoun::He => "his",
            Pronoun::She => "her",
            Pronoun::They => "their",
        }
    }
    /// "he", "she", "they".
    pub fn subject(self) -> &'static str {
        match self {
            Pronoun::He => "he",
            Pronoun::She => "she",
            Pronoun::They => "they",
        }
    }
    /// "him", "her", "them".
    pub fn object(self) -> &'static str {
        match self {
            Pronoun::He => "him",
            Pronoun::She => "her",
            Pronoun::They => "them",
        }
    }
}

/// One person on a share with people (DW-0511's rows, DW-0512's dialog).
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub initials: String,
    /// The board's avatar hue (`.kit-avatar.hue-*`).
    pub hue: char,
    pub email: String,
    /// `None`: the owner, who can do everything.
    pub access: Option<Access>,
    /// The person using this app.
    pub you: bool,
    /// Invited and not signed up or accepted yet ("Invited, waiting").
    pub invited: bool,
    /// The person compared safety numbers with them. Only ever read here: nothing in this
    /// window computes or compares a safety number.
    pub compared: bool,
    pub pronoun: Pronoun,
}

impl Member {
    /// The name a sentence uses: the first word of the name ("Jonas").
    pub fn first_name(&self) -> &str {
        self.name.split_whitespace().next().unwrap_or(&self.name)
    }

    /// The row's second line (DW-0511): "You · Owner", "Can edit", "Can view · Invited, waiting".
    pub fn line(&self) -> String {
        let role = match (self.access, self.you) {
            (None, true) => "You \u{00B7} Owner".to_string(),
            (None, false) => "Owner".to_string(),
            (Some(access), _) => access.words().to_string(),
        };
        if self.invited {
            format!("{role} \u{00B7} Invited, waiting")
        } else {
            role
        }
    }
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
    /// `false`: nothing answers yet (`Unwired`), and the view draws the shell's "not built yet"
    /// frame instead of a section of sample values.
    fn wired(&self) -> bool {
        true
    }
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
    /// The people of a share with people, the owner first (DW-0511). Empty: no such share.
    fn members(&self, share: &str) -> Vec<Member>;
    /// What one person may do with a share, changed at once (DW-0512). `false`: no such
    /// person on that share, or the owner.
    fn set_access(&mut self, share: &str, person: &str, access: Access) -> bool;
    /// Take one person off a share (DW-0512b): nothing new opens for them and the share goes
    /// from their list; copies they saved stay with them. `false`: no such person, or the owner.
    fn remove_access(&mut self, share: &str, person: &str) -> bool;
    /// Share with someone by their email address (DW-0514); they show as invited until they
    /// accept. `false`: no such share, or they are on it already.
    fn invite(&mut self, share: &str, email: &str, access: Access) -> bool;
}

/// The board's sample (DW-0501, DW-0502, DW-0503), or nothing at all (DW-0508, DW-0509, DW-0510).
#[derive(Default)]
pub struct BoardShared {
    invitations: Vec<Invitation>,
    with_me: Vec<Row>,
    people: Vec<PeopleShare>,
    /// The people of each share with people, by the share's id (DW-0511).
    members: Vec<(String, Vec<Member>)>,
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
        let share = |id: &str, name: &str, kind, names: &str, people, pending, contents: Option<&str>| PeopleShare {
            id: id.into(),
            name: name.into(),
            kind,
            names: names.into(),
            people,
            pending,
            contents: contents.map(Into::into),
        };
        // DW-0511 draws the people of Lisbon trip; the other shares' people are the ones their
        // rows name (DW-0502), with the access an invitation starts with (DW-0514: Can view).
        let member = |id: &str, name: &str, initials: &str, hue: char, email: &str, access: Option<Access>| Member {
            id: id.into(),
            name: name.into(),
            initials: initials.into(),
            hue,
            email: email.into(),
            access,
            you: access.is_none(),
            invited: false,
            compared: false,
            pronoun: Pronoun::They,
        };
        let anna = || member("m-anna", "Anna Visser", "AV", 'a', "anna@northwind.studio", None);
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
                    Some("212 items"),
                ),
                share(
                    "p-invoices",
                    "Invoices 2026",
                    Kind::Folder,
                    "Priya Shah",
                    vec![person("PS", 'c')],
                    false,
                    None,
                ),
                share(
                    "p-contract",
                    "Contract draft.pdf",
                    Kind::Document,
                    "Sam Okafor",
                    vec![person("SO", 'b')],
                    true,
                    None,
                ),
            ],
            members: vec![
                (
                    "p-lisbon".into(),
                    vec![
                        anna(),
                        // The board's sentences about Jonas say "his" (DW-0512b).
                        Member {
                            pronoun: Pronoun::He,
                            ..member(
                                "m-jonas",
                                "Jonas Meier",
                                "JM",
                                'c',
                                "jonas@northwind.studio",
                                Some(Access::Edit),
                            )
                        },
                        Member {
                            invited: true,
                            ..member(
                                "m-mia",
                                "Mia de Vries",
                                "MD",
                                'a',
                                "mia@northwind.studio",
                                Some(Access::View),
                            )
                        },
                    ],
                ),
                (
                    "p-invoices".into(),
                    vec![
                        anna(),
                        member(
                            "m-priya",
                            "Priya Shah",
                            "PS",
                            'c',
                            "priya@northwind.studio",
                            Some(Access::View),
                        ),
                    ],
                ),
                (
                    "p-contract".into(),
                    vec![
                        anna(),
                        Member {
                            invited: true,
                            ..member(
                                "m-sam",
                                "Sam Okafor",
                                "SO",
                                'b',
                                "sam@northwind.studio",
                                Some(Access::View),
                            )
                        },
                    ],
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

    fn people_of(&mut self, share: &str) -> Option<&mut Vec<Member>> {
        self.members
            .iter_mut()
            .find(|(id, _)| id == share)
            .map(|(_, people)| people)
    }

    /// The share's row in By me names its people as People with access lists them.
    fn sync_row(&mut self, share: &str) {
        let others: Vec<Member> = self.members(share).into_iter().filter(|m| m.access.is_some()).collect();
        if let Some(row) = self.people.iter_mut().find(|p| p.id == share) {
            row.names = others.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ");
            row.people = others
                .iter()
                .map(|m| Person {
                    initials: m.initials.clone(),
                    hue: m.hue,
                })
                .collect();
        }
    }
}

/// The initial an invited address shows until its person has a name: its first letter.
fn initial_of(email: &str) -> String {
    email
        .chars()
        .find(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_default()
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
    fn members(&self, share: &str) -> Vec<Member> {
        self.members
            .iter()
            .find(|(id, _)| id == share)
            .map(|(_, people)| people.clone())
            .unwrap_or_default()
    }
    fn set_access(&mut self, share: &str, person: &str, access: Access) -> bool {
        let Some(people) = self.people_of(share) else {
            return false;
        };
        match people.iter_mut().find(|m| m.id == person && m.access.is_some()) {
            Some(m) => {
                m.access = Some(access);
                true
            }
            None => false,
        }
    }
    fn remove_access(&mut self, share: &str, person: &str) -> bool {
        let Some(people) = self.people_of(share) else {
            return false;
        };
        let before = people.len();
        // The owner is never removed.
        people.retain(|m| m.id != person || m.access.is_none());
        let gone = people.len() != before;
        if gone {
            self.sync_row(share);
        }
        gone
    }
    fn invite(&mut self, share: &str, email: &str, access: Access) -> bool {
        let email = email.trim();
        let Some(people) = self.people_of(share) else {
            return false;
        };
        if email.is_empty() || people.iter().any(|m| m.email.eq_ignore_ascii_case(email)) {
            return false;
        }
        people.push(Member {
            // Its own kind of id: never one of a member's (`m-…`), and one address is invited once.
            id: format!("invite-{}", email.to_ascii_lowercase()),
            name: email.to_string(),
            initials: initial_of(email),
            hue: 'e',
            email: email.to_string(),
            access: Some(access),
            you: false,
            invited: true,
            compared: false,
            pronoun: Pronoun::They,
        });
        self.sync_row(share);
        true
    }
}

/// The source of a build that ships before the engine lane has wired Shared: it answers
/// nothing, and the view draws the shell's "not built yet" frame instead of the section.
/// No value of it is ever shown.
pub struct Unwired;

impl SharedSource for Unwired {
    fn wired(&self) -> bool {
        false
    }
    fn invitations(&self) -> Vec<Invitation> {
        Vec::new()
    }
    fn with_me(&self) -> Vec<Row> {
        Vec::new()
    }
    fn people_shares(&self) -> Vec<PeopleShare> {
        Vec::new()
    }
    fn links(&self) -> Vec<LinkShare> {
        Vec::new()
    }
    fn requests(&self) -> Vec<Request> {
        Vec::new()
    }
    fn accept(&mut self, _id: &str) -> bool {
        false
    }
    fn decline(&mut self, _id: &str) -> bool {
        false
    }
    fn keep_offline(&mut self, _id: &str) -> bool {
        false
    }
    fn stop_sharing(&mut self, _id: &str) -> bool {
        false
    }
    fn close_request(&mut self, _id: &str) -> bool {
        false
    }
    fn remove_request(&mut self, _id: &str) -> bool {
        false
    }
    fn members(&self, _share: &str) -> Vec<Member> {
        Vec::new()
    }
    fn set_access(&mut self, _share: &str, _person: &str, _access: Access) -> bool {
        false
    }
    fn remove_access(&mut self, _share: &str, _person: &str) -> bool {
        false
    }
    fn invite(&mut self, _share: &str, _email: &str, _access: Access) -> bool {
        false
    }
}
