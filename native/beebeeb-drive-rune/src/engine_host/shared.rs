//! Shared on the real engine: [`EngineSharedSource`], the `SharedSource` of the running app.
//!
//! Where each list comes from: the SERVER only, for the session in memory ([`Engine::shares_overview`], one call for
//! all of them); nothing is read from this computer's local state. Names are decrypted in the engine.
//! - `invitations`: incoming invites waiting for an answer. `with_me`: incoming invites the owner approved.
//! - `people_shares` / `members`: the invites the account sent, by item; a person's id is the invite's id, the
//!   owner's row is the session's own email. A person is named by their email address (Beebeeb has no other name).
//! - `links`: the account's live links, one per item. `requests`: its file requests.
//! - `Member::compared` is always `false`: the safety-number marks are not in the engine crate yet (see the PR).
//!
//! Bound changes ([`Engine::share_change`], by id, for the session the row was drawn under): `accept`, `set_access`,
//! `remove_access` (the view confirms it, DW-0512b). NOT bound, each answers `false` and a notice the view draws
//! ("Not available yet"): `decline`, `stop_sharing`, `close_request` (the view asks no confirmation), `invite` (needs
//! the safety marks and the key flows), `keep_offline`, `remove_request` (no server route).
//!
//! When it reads: when the Shared section opens and the lists are missing or older than [`FRESH_FOR`], and after a
//! change was answered. Never from a draw call: every accessor only reads what is here. So the sidebar's count of
//! invitations is known only once Shared was opened.
//!
//! Session rule: the lists carry the session they were read for; an answer of another session is dropped, and every
//! accessor compares the lists' session with [`Engine::session_turn`] before it hands out a name. `forget` (the
//! window's poll, on a gate or account change) clears everything.
//!
//! Thread rule: the read and each change run on a worker of their own and answer down one channel, taken in by
//! [`SharedSource::pump`]; the worker wakes the window ([`Engine::wake_host`]).
//!
//! Must not: log a name, an address, an id or a server's words. Holds no token, key, link or link secret.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::shares_api::{ShareChange, ShareChangeError, ShareMember, SharesOverview};
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::drive_v2::model::mime_for_name;
use chrono::{DateTime, Local};

use super::log;
use super::trash::{kind_of, seconds_of, when};
use crate::frames::shared::parts::table::{Extra, Row};
use crate::frames::shared::source::{
    Access, Invitation, LinkShare, Loaded, Member, Notice, PeopleShare, Person, Pronoun, Pumped, Request, RequestState,
    SharedSource,
};

#[cfg(test)]
use crate::frames::shared::parts::table::Kind;

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;

/// Lists this recent are not read again when the Shared section opens.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// The owner's row in People with access. Never an invite's id (those are UUIDs).
pub const OWNER: &str = "owner";

/// Shared on the real engine. See the module's doc.
pub struct EngineSharedSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The lists last read, and the session they belong to.
    held: Option<Held>,
    /// What the view draws while there are no lists.
    loaded: Loaded,
    /// When the last read was answered (a failure too).
    read_at: Option<Instant>,
    /// The read on its way, by number.
    reading: Option<u64>,
    reads: u64,
    /// A read was asked for while one was on its way: start it when that one lands.
    again: bool,
    /// Changes on their way.
    changing: usize,
    notice: Option<Notice>,
    /// Something changed outside an answer: the next pump says so.
    dirty: bool,
}

/// The lists as the view draws them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Held {
    pub(crate) session: SessionGeneration,
    pub(crate) invitations: Vec<Invitation>,
    pub(crate) with_me: Vec<Row>,
    pub(crate) people: Vec<PeopleShare>,
    pub(crate) members: HashMap<String, Vec<Member>>,
    pub(crate) links: Vec<LinkShare>,
    pub(crate) requests: Vec<Request>,
}

enum Answer {
    Read(u64, Result<SharesOverview, AccountCallError>),
    Changed(SessionGeneration, Result<(), ShareChangeError>),
}

impl<C: CredentialStores> EngineSharedSource<C> {
    /// Nothing is asked until the Shared section opens.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            held: None,
            loaded: Loaded::Loading,
            read_at: None,
            reading: None,
            reads: 0,
            again: false,
            changing: 0,
            notice: None,
            dirty: false,
        }
    }

    /// The lists, while they are the current session's. Asked on every accessor.
    fn current(&self) -> Option<&Held> {
        let now = self.engine.session_turn();
        self.held
            .as_ref()
            .filter(|held| held.session == now && self.engine.has_session())
    }

    fn current_mut(&mut self) -> Option<&mut Held> {
        self.current()?;
        self.held.as_mut()
    }

    fn read(&mut self) {
        if self.reading.is_some() {
            self.again = true;
            return;
        }
        self.again = false;
        self.reads += 1;
        let (engine, answers, number) = (self.engine.clone(), self.answers.clone(), self.reads);
        let started = std::thread::Builder::new().name("shared-read".into()).spawn(move || {
            // A panic in this worker must not leave `reading` set for good: send an error.
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.shares_overview()))
                .unwrap_or(Err(AccountCallError::Offline));
            let _ = answers.send(Answer::Read(number, read));
            engine.wake_host();
        });
        match started {
            Ok(_) => {
                self.reading = Some(number);
                if self.current().is_none() {
                    self.loaded = Loaded::Loading;
                }
            }
            Err(_) => {
                log("shared: could not start a thread for the lists");
                self.read_at = Some(Instant::now());
                self.loaded = Loaded::Failed;
            }
        }
    }

    /// Send one change on a worker, for the session the lists were read for. `false`: no worker could start.
    fn change(&mut self, session: SessionGeneration, change: ShareChange) -> bool {
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let started = std::thread::Builder::new().name("shared-change".into()).spawn(move || {
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.share_change(session, &change)))
                .unwrap_or(Err(ShareChangeError::Refused));
            let _ = answers.send(Answer::Changed(session, done));
            engine.wake_host();
        });
        if started.is_err() {
            log("shared: could not start a thread for a change");
            return self.refuse(Notice::ChangeRefused);
        }
        self.changing += 1;
        self.dirty = true;
        true
    }

    /// Nothing was done: say so over the lists.
    fn refuse(&mut self, notice: Notice) -> bool {
        self.notice = Some(notice);
        self.dirty = true;
        false
    }

    fn take_in(&mut self, answer: Answer) {
        let now = self.engine.session_turn();
        match answer {
            Answer::Read(number, result) => {
                if self.reading == Some(number) {
                    self.reading = None;
                }
                let again = std::mem::take(&mut self.again);
                self.read_at = Some(Instant::now());
                match result {
                    // Read for a session that is gone: dropped, never drawn.
                    Ok(overview) if overview.session != now => {}
                    Ok(overview) => {
                        self.held = Some(held_of(overview, Local::now()));
                        self.loaded = Loaded::Ready;
                    }
                    Err(kind) => {
                        if kind != AccountCallError::NotSignedIn {
                            log(format!("shared: the lists did not load ({kind})"));
                        }
                        let have = self.current().is_some();
                        match kind {
                            AccountCallError::NotSignedIn => {
                                self.held = None;
                                self.loaded = Loaded::SignedOut;
                            }
                            AccountCallError::SessionEnded => {
                                self.held = None;
                                self.loaded = Loaded::SessionEnded;
                            }
                            // Lists are shown: they stay, and the view says they may be old.
                            _ if have => self.notice = Some(Notice::RefreshFailed),
                            AccountCallError::Offline => self.loaded = Loaded::Offline,
                            _ => self.loaded = Loaded::Failed,
                        }
                    }
                }
                if again && self.engine.has_session() {
                    self.read();
                }
            }
            Answer::Changed(session, done) => {
                self.changing = self.changing.saturating_sub(1);
                if session != now {
                    return;
                }
                if let Err(kind) = done {
                    log(format!("shared: a change was not made ({kind})"));
                    self.notice = Some(Notice::ChangeRefused);
                }
                // What Beebeeb has now is the truth, whatever was drawn ahead of it.
                self.read();
            }
        }
    }

    /// The member `person` of `share` in the current lists, when it is a person and not the owner.
    fn person_at(&self, share: &str, person: &str) -> Option<SessionGeneration> {
        let held = self.current()?;
        held.members
            .get(share)?
            .iter()
            .any(|m| m.id == person && m.access.is_some())
            .then_some(held.session)
    }
}

#[cfg(test)]
impl<C: CredentialStores> EngineSharedSource<C> {
    fn busy(&self) -> bool {
        self.reading.is_some() || self.changing > 0
    }
}

impl<C: CredentialStores> SharedSource for EngineSharedSource<C> {
    fn invitations(&self) -> Vec<Invitation> {
        self.current().map(|h| h.invitations.clone()).unwrap_or_default()
    }
    fn with_me(&self) -> Vec<Row> {
        self.current().map(|h| h.with_me.clone()).unwrap_or_default()
    }
    fn people_shares(&self) -> Vec<PeopleShare> {
        self.current().map(|h| h.people.clone()).unwrap_or_default()
    }
    fn links(&self) -> Vec<LinkShare> {
        self.current().map(|h| h.links.clone()).unwrap_or_default()
    }
    fn requests(&self) -> Vec<Request> {
        self.current().map(|h| h.requests.clone()).unwrap_or_default()
    }
    fn members(&self, share: &str) -> Vec<Member> {
        self.current()
            .and_then(|h| h.members.get(share).cloned())
            .unwrap_or_default()
    }

    /// Sent once, for an invitation of the current session's lists; its card goes at once. Once the owner approves,
    /// the item is listed under Shared with you.
    fn accept(&mut self, id: &str) -> bool {
        let Some(held) = self.current_mut() else { return false };
        let Some(at) = held.invitations.iter().position(|i| i.id == id) else {
            return false;
        };
        held.invitations.remove(at);
        let session = held.session;
        self.change(session, ShareChange::Accept { invite_id: id.into() })
    }

    /// Sent once, for a person (never the owner) of a share in the current session's lists.
    fn set_access(&mut self, share: &str, person: &str, access: Access) -> bool {
        let Some(session) = self.person_at(share, person) else {
            return false;
        };
        if let Some(m) = self
            .held
            .as_mut()
            .and_then(|h| h.members.get_mut(share))
            .and_then(|people| people.iter_mut().find(|m| m.id == person))
        {
            m.access = Some(access);
        }
        self.change(
            session,
            ShareChange::SetAccess {
                invite_id: person.into(),
                can_edit: access == Access::Edit,
            },
        )
    }

    /// Sent once (the view confirmed it, DW-0512b), for a person (never the owner) of a share in the current
    /// session's lists; the person leaves the share's rows at once.
    fn remove_access(&mut self, share: &str, person: &str) -> bool {
        let Some(session) = self.person_at(share, person) else {
            return false;
        };
        if let Some(held) = self.held.as_mut() {
            let people = held.members.entry(share.to_string()).or_default();
            people.retain(|m| m.id != person);
            let others: Vec<Member> = people.iter().filter(|m| m.access.is_some()).cloned().collect();
            if others.is_empty() {
                held.people.retain(|p| p.id != share);
                held.members.remove(share);
            } else if let Some(row) = held.people.iter_mut().find(|p| p.id == share) {
                (row.names, row.people, row.pending) = people_line(&others);
            }
        }
        self.change(
            session,
            ShareChange::RemoveMember {
                invite_id: person.into(),
            },
        )
    }

    // Not bound (the module's doc says why): nothing is sent, and the view says so.
    fn decline(&mut self, _id: &str) -> bool {
        self.refuse(Notice::NotAvailable)
    }
    fn keep_offline(&mut self, _id: &str) -> bool {
        self.refuse(Notice::NotAvailable)
    }
    fn stop_sharing(&mut self, _id: &str) -> bool {
        self.refuse(Notice::NotAvailable)
    }
    fn close_request(&mut self, _id: &str) -> bool {
        self.refuse(Notice::NotAvailable)
    }
    fn remove_request(&mut self, _id: &str) -> bool {
        self.refuse(Notice::NotAvailable)
    }
    fn invite(&mut self, _share: &str, _email: &str, _access: Access) -> bool {
        self.refuse(Notice::NotAvailable)
    }

    fn loaded(&self) -> Loaded {
        if !self.engine.has_session() {
            Loaded::SignedOut
        } else if self.current().is_some() {
            Loaded::Ready
        } else {
            self.loaded
        }
    }

    fn opened(&mut self) {
        if !self.engine.has_session() {
            return;
        }
        let fresh = self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
        }
    }

    fn pump(&mut self) -> Pumped {
        let mut changed = std::mem::take(&mut self.dirty);
        // The session moved on since the lists were read: they go now, on the window's side.
        if self.held.is_some() && self.current().is_none() {
            self.forget();
            self.dirty = false;
            changed = true;
        }
        while let Ok(answer) = self.inbox.try_recv() {
            self.take_in(answer);
            changed = true;
        }
        let notice = self.notice.take();
        Pumped {
            changed: changed || notice.is_some(),
            busy: self.reading.is_some() || self.changing > 0,
            notice,
        }
    }

    fn forget(&mut self) {
        self.held = None;
        self.again = false;
        self.read_at = None;
        self.notice = None;
        self.loaded = Loaded::Loading;
        self.dirty = true;
    }
}

/// The engine's overview as the view's lists. Nothing here reads anything.
pub(crate) fn held_of(overview: SharesOverview, now: DateTime<Local>) -> Held {
    let kind = |is_folder: bool, name: &str| kind_of(is_folder, mime_for_name(name));
    let date = |stamp: &str| seconds_of(stamp).map(|at| when(at, now)).unwrap_or_default();
    let (mut invitations, mut with_me) = (Vec::new(), Vec::new());
    for item in overview.incoming {
        let row = Row {
            id: item.item_id,
            kind: kind(item.is_folder, &item.name),
            date: if item.waiting {
                String::new()
            } else {
                date(&item.shared_at)
            },
            size: if item.is_folder {
                String::new()
            } else {
                crate::source::format_size(item.size_bytes.max(0))
            },
            extra: Extra::Person {
                initials: initial_of(&item.from_email),
                name: item.from_email.clone(),
                hue: 'e',
            },
            // Whether it is on this PC is not known here: no claim.
            pc: None,
            name: item.name,
        };
        if item.waiting {
            let what = if item.is_folder { "folder" } else { "file" };
            invitations.push(Invitation {
                id: item.invite_id,
                name: row.name.clone(),
                kind: row.kind,
                from_line: format!("{} wants to share this {what}", item.from_email),
                row,
            });
        } else {
            with_me.push(row);
        }
    }
    let owner = overview.owner_email.unwrap_or_default();
    let (mut people, mut members) = (Vec::new(), HashMap::new());
    for share in overview.by_me {
        let mut rows = vec![Member {
            id: OWNER.into(),
            name: owner.clone(),
            initials: initial_of(&owner),
            hue: 'a',
            email: owner.clone(),
            access: None,
            you: true,
            invited: false,
            compared: false,
            pronoun: Pronoun::They,
        }];
        rows.extend(share.members.into_iter().map(member_of));
        let (names, avatars, pending) = people_line(&rows[1..]);
        people.push(PeopleShare {
            kind: kind(share.is_folder, &share.name),
            id: share.item_id.clone(),
            name: share.name,
            names,
            people: avatars,
            pending,
            contents: None,
        });
        members.insert(share.item_id, rows);
    }
    let links = overview
        .links
        .into_iter()
        .map(|link| {
            let expiry = link.expires_at.as_deref().and_then(seconds_of);
            let expired = expiry.is_some_and(|at| at <= now.timestamp());
            let mut line = match expiry {
                None => "No expiry".to_string(),
                Some(at) if expired => format!("Expired {}", day(at)),
                Some(at) => expires_in(at - now.timestamp()),
            };
            if link.has_password && !expired {
                line.push_str(", with password");
            }
            LinkShare {
                kind: kind(false, &link.name),
                id: link.item_id,
                name: link.name,
                line,
                expired,
            }
        })
        .collect();
    let requests = overview
        .requests
        .into_iter()
        .map(|r| {
            let expiry = r.expires_at.as_deref().and_then(seconds_of);
            let state = match expiry {
                _ if r.closed => RequestState::Closed,
                Some(at) if at <= now.timestamp() => RequestState::Expired,
                _ => RequestState::Open,
            };
            let mut line = match (r.files_received, r.max_files) {
                (0, _) => "No files".to_string(),
                (n, Some(max)) => format!("{n} of {max} files"),
                (1, None) => "1 file".to_string(),
                (n, None) => format!("{n} files"),
            };
            match (state, expiry) {
                (RequestState::Expired, Some(at)) => line.push_str(&format!(" \u{00B7} Expired {}", day(at))),
                (RequestState::Open, Some(at)) => line.push_str(&format!(" \u{00B7} Expires {}", day(at))),
                _ => {}
            }
            Request {
                id: r.id,
                title: r.title,
                line,
                state,
            }
        })
        .collect();
    Held {
        session: overview.session,
        invitations,
        with_me,
        people,
        members,
        links,
        requests,
    }
}

fn member_of(member: ShareMember) -> Member {
    Member {
        id: member.invite_id,
        name: member.email.clone(),
        initials: initial_of(&member.email),
        hue: 'e',
        access: Some(if member.can_edit { Access::Edit } else { Access::View }),
        you: false,
        invited: !member.accepted,
        // Not read here: the safety-number marks are not in the engine crate (the module's doc).
        compared: false,
        pronoun: Pronoun::They,
        email: member.email,
    }
}

/// A share's row in By me from its people (the owner left out): their names, their avatars, and whether nobody has
/// accepted yet.
fn people_line(others: &[Member]) -> (String, Vec<Person>, bool) {
    (
        others.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "),
        others
            .iter()
            .map(|m| Person {
                initials: m.initials.clone(),
                hue: m.hue,
            })
            .collect(),
        others.iter().all(|m| m.invited),
    )
}

/// The initial of an address: its first letter or digit.
fn initial_of(email: &str) -> String {
    email
        .chars()
        .find(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_default()
}

/// A day as the board writes it in a line: "3 Oct".
fn day(seconds: i64) -> String {
    DateTime::from_timestamp(seconds, 0)
        .map(|at| at.with_timezone(&Local).format("%-d %b").to_string())
        .unwrap_or_default()
}

fn expires_in(seconds: i64) -> String {
    match (seconds + 86_399) / 86_400 {
        ..=1 => "Expires within a day".to_string(),
        days => format!("Expires in {days} days"),
    }
}
