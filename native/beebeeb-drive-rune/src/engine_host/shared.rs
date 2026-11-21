//! Shared on the real engine: [`EngineSharedSource`], the `SharedSource` of the running app.
//!
//! Where each list comes from: the SERVER only, for the session in memory ([`Engine::shares_overview`], one call for
//! all of them); nothing is read from this computer's local state. Names are decrypted in the engine.
//! - `invitations`: incoming invites waiting for an answer. `with_me`: incoming invites the owner approved.
//! - `people_shares` / `members`: the invites the account sent, by item; a person's id is the invite's id, the
//!   owner's row is the session's own email. A person is named by their email address (Beebeeb has no other name).
//! - `links`: the account's live links, one per item. `requests`: its file requests.
//! - `Member::compared` is the engine's trust read after each read of the lists (`shared_safety.rs`, which also holds
//!   People's safety number, the two marks and "Share anyway").
//!
//! Bound changes ([`Engine::share_change`], by id, for the session the row was drawn under): `accept`, `set_access`,
//! `remove_access` (the view confirms it, DW-0512b), `decline` (`/decline`: the sender sees it declined; 409 is "Already
//! accepted."), `stop_sharing` (a people share: `CancelInvites` for every invite; a link row: `RevokeLinks` for every live link
//! of the item), `close_request` (`CloseRequest`, an open request only), `remove_request` ("Remove from my list" is the server's `/hide`, never DELETE, which destroys the key;
//! 409 is "Close the request first."). A change is never drawn before Beebeeb confirmed it: the rows
//! are the server's, read again after the answer; one change per id at a time. NOT bound, each answers `false` and a
//! notice the view draws ("Not available yet"): the old synchronous `invite` (it cannot wait for the server). `keep_offline` is bound (`shared_offline.rs`):
//! the engine's pin change for a row of Shared with me, answered like a change.
//!
//! Invite someone (DW-0514, [`Engine::invite_for`]): `start_invite` stamps the press's session and account, runs the
//! invite on a worker (one per share at a time) and answers through `invite_answer`; an answer under another session
//! or account is dropped. After an invite that reached Beebeeb (and after a changed key, whose invite stays) the lists
//! are read again: a person is never inserted here. The access is Beebeeb's default for a new invitation (Can view):
//! core's invite carries none.
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

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::share_invite_api::{InviteAnswer, InviteRefused};
use beebeeb_engine::app::shares_api::{ShareChange, ShareChangeError, ShareMember, SharesOverview};
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::drive_v2::model::mime_for_name;
use chrono::{DateTime, Local};

use super::log;
use super::stamp::{Stamp, stamp};
use super::trash::{kind_of, seconds_of, when};
use crate::frames::files::link::LinkText;
use crate::frames::shared::parts::table::{Extra, Row};
use crate::frames::shared::source::{
    Access, Invitation, InviteResult, LinkEdit, LinkShare, Loaded, Member, Notice, PeopleShare, Person, Pronoun,
    Pumped, Request, RequestForm, RequestState, RowTarget, SafetyAsk, SafetyState, SharedSource,
};
use crate::ui::file_list::PcState;

#[cfg(test)]
use crate::frames::shared::parts::table::Kind;

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
// plan40 batch 1: Make available offline on someone else's file (slice B4).
#[path = "shared_leave.rs"]
mod shared_leave;
#[path = "shared_links.rs"]
mod shared_links;
#[path = "shared_offline.rs"]
mod shared_offline;
#[path = "shared_requests.rs"]
mod shared_requests;
#[path = "shared_safety.rs"]
mod shared_safety;

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
    /// The section was opened in this session: only then is a finished check followed by a read.
    asked: bool,
    /// The ids (an invitation's, a person's) with a change on its way: one change per id at a time.
    changing: HashSet<String>,
    notice: Option<Notice>,
    /// The last change answered was refused and no read has answered since: a failed read keeps saying the change
    /// was not made. A change Beebeeb made clears it.
    unmade: bool,
    /// Something changed outside an answer: the next pump says so.
    dirty: bool,
    /// Changes with every change to the lists held (`SharedSource::rev`).
    rev: u64,
    /// The shares with an invitation on its way: one per share at a time.
    inviting: HashSet<String>,
    /// The invitations' answers of the current session and account, in order, for the view.
    invite_answers: VecDeque<(String, InviteResult)>,
    /// Slice B4 (`shared_offline.rs`): what it keeps between a press and its answer. Empty until it binds.
    shared_offline: shared_offline::SharedOfflineCalls,
    /// `shared_links.rs`: the links Copy link made, waiting for the clipboard.
    shared_links: shared_links::SharedLinkCalls,
    /// `shared_requests.rs`: the links a new request made, until the window takes them.
    shared_requests: shared_requests::RequestCalls,
    /// `shared_safety.rs`: the safety numbers, marks and "Share anyway" of People.
    safety: shared_safety::SafetyCalls,
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
    /// A link row's id to every live link behind it (what Stop sharing revokes).
    pub(crate) link_shares: HashMap<String, Vec<String>>,
    /// A link row's id to the item it is a link to (OwnedLink.item_id).
    pub(crate) link_items: HashMap<String, String>,
    pub(crate) requests: Vec<Request>,
    /// A with-me row's item id to the invite it came with (what Leave sends).
    pub(crate) invites: HashMap<String, String>,
    /// A request's id to the folder its uploads land in, when it has one (what View files opens).
    pub(crate) request_folders: HashMap<String, String>,
}

enum Answer {
    /// The read's number, the overview, and which of its with-me rows the engine says are kept on this PC.
    Read(u64, Result<SharesOverview, AccountCallError>, HashSet<String>),
    /// The change for this id, the session it was sent for, and what Beebeeb said.
    Changed(String, SessionGeneration, Result<(), ShareChangeError>),
    /// The invitation for this share, the session and account it was sent under, and how it ended.
    Invited(String, Stamp, InviteAnswer),
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
            asked: false,
            changing: HashSet::new(),
            notice: None,
            unmade: false,
            dirty: false,
            rev: 0,
            inviting: HashSet::new(),
            invite_answers: VecDeque::new(),
            shared_offline: Default::default(),
            shared_links: Default::default(),
            shared_requests: Default::default(),
            safety: Default::default(),
        }
    }

    /// The lists, while they are the current session's. Asked on every accessor.
    fn current(&self) -> Option<&Held> {
        let now = self.engine.session_turn();
        self.held
            .as_ref()
            .filter(|held| held.session == now && self.engine.has_session())
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
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let overview = engine.shares_overview();
                // Which with-me rows are kept comes from the engine's own state, on this same worker.
                let kept = match &overview {
                    Ok(overview) => {
                        let ids: Vec<String> = overview
                            .incoming
                            .iter()
                            .filter(|item| !item.waiting)
                            .map(|item| item.item_id.clone())
                            .collect();
                        engine.shared_kept_of_for(overview.session, &ids).unwrap_or_else(|_| {
                            log("shared: whether shared files are kept could not be read");
                            HashSet::new()
                        })
                    }
                    Err(_) => HashSet::new(),
                };
                (overview, kept)
            }))
            .unwrap_or((Err(AccountCallError::Offline), HashSet::new()));
            let (read, kept) = read;
            let _ = answers.send(Answer::Read(number, read, kept));
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
                self.loaded = Loaded::Failed;
            }
        }
    }

    /// Send one change on a worker, for the session the lists were read for. Nothing in the lists is edited here:
    /// they change only when Beebeeb's answer was read back. `false`: a change for `id` is already on its way (a
    /// second press sends nothing), or no worker could start.
    fn change(&mut self, session: SessionGeneration, id: &str, change: ShareChange) -> bool {
        if self.changing.contains(id) {
            return false;
        }
        let (engine, answers, sent_for) = (self.engine.clone(), self.answers.clone(), id.to_string());
        let started = std::thread::Builder::new().name("shared-change".into()).spawn(move || {
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.share_change(session, &change)))
                .unwrap_or(Err(ShareChangeError::Refused));
            let _ = answers.send(Answer::Changed(sent_for, session, done));
            engine.wake_host();
        });
        if started.is_err() {
            log("shared: could not start a thread for a change");
            return self.refuse(Notice::ChangeRefused);
        }
        self.changing.insert(id.to_string());
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
        self.rev += 1;
        let now = self.engine.session_turn();
        match answer {
            Answer::Read(number, result, kept) => {
                if self.reading == Some(number) {
                    self.reading = None;
                }
                let again = std::mem::take(&mut self.again);
                // Only an answer makes the lists fresh: after a failed read they are old, and the next opening reads
                // again.
                self.read_at = result.is_ok().then(Instant::now);
                match result {
                    // Read for a session that is gone: dropped, never drawn.
                    Ok(overview) if overview.session != now => {}
                    Ok(overview) => {
                        let mut held = held_of(overview, Local::now());
                        mark_kept(&mut held, &kept);
                        self.held = Some(held);
                        self.unmade = false;
                        self.loaded = Loaded::Ready;
                        self.shared_safety_after_read();
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
                            // A change Beebeeb refused keeps its own words.
                            _ if have => {
                                let words = if self.unmade {
                                    Notice::ChangeRefused
                                } else {
                                    Notice::RefreshFailed
                                };
                                self.notice.get_or_insert(words);
                            }
                            AccountCallError::Offline => self.loaded = Loaded::Offline,
                            _ => self.loaded = Loaded::Failed,
                        }
                    }
                }
                if again && self.engine.has_session() {
                    self.read();
                }
            }
            Answer::Changed(id, session, done) => {
                self.changing.remove(&id);
                if session != now {
                    return;
                }
                match done {
                    // Beebeeb made this change: an earlier refusal is no longer what a failed read is about.
                    Ok(()) => self.unmade = false,
                    Err(kind) => {
                        log(format!("shared: a change was not made ({kind})"));
                        self.notice = Some(match kind {
                            ShareChangeError::AlreadyAccepted => Notice::AlreadyAccepted,
                            ShareChangeError::RequestOpen => Notice::RequestStillOpen,
                            _ => Notice::ChangeRefused,
                        });
                        self.unmade = true;
                    }
                }
                // Nothing was drawn ahead of the answer: the lists change when Beebeeb's are read again.
                self.read();
            }
            Answer::Invited(share, asked, answer) => {
                self.inviting.remove(&share);
                // Sent under another session or account: dropped, nothing opens and nothing is read for it.
                if asked != stamp(&self.engine) {
                    return;
                }
                let result = invite_result(&answer);
                if let InviteAnswer::PeerKeyChanged { invite_id } = &answer {
                    self.shared_safety_key_changed(&share, invite_id);
                }
                if let InviteAnswer::Refused(kind) = answer {
                    log(format!("shared: an invitation was not sent ({kind})"));
                }
                // The person is on the share now (a changed key's invite stays too, and so does one that could not be
                // finished): the lists are Beebeeb's again.
                if result.sent()
                    || result == InviteResult::KeyChanged
                    || matches!(answer, InviteAnswer::Unfinished { .. })
                {
                    self.read();
                }
                self.invite_answers.push_back((share, result));
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
        self.reading.is_some() || !self.changing.is_empty() || !self.inviting.is_empty() || self.shared_safety_busy()
    }
}

impl<C: CredentialStores> SharedSource for EngineSharedSource<C> {
    fn rev(&self) -> Option<u64> {
        self.current().map(|_| self.rev)
    }
    fn retry(&mut self) {
        self.read_at = None;
        self.opened();
    }
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
        let mut members = self
            .current()
            .and_then(|h| h.members.get(share).cloned())
            .unwrap_or_default();
        self.shared_safety_overlay(&mut members);
        members
    }

    fn safety_wired(&self) -> bool {
        true
    }
    fn safety_start(&mut self, share: &str, person: &str, ask: SafetyAsk) -> bool {
        self.shared_safety_start(share, person, ask)
    }
    fn safety_state(&self, share: &str, person: &str) -> SafetyState {
        self.shared_safety_state(share, person)
    }
    fn safety_clear(&mut self, share: &str, person: &str) {
        self.shared_safety_clear(share, person)
    }
    fn key_changed_person(&self, share: &str) -> Option<String> {
        self.shared_safety_key_changed_person(share)
    }

    /// Sent once per press, for an invitation of the current session's lists. Nothing is drawn as accepted before
    /// Beebeeb answered: the card goes when the lists are read again after it. `false`: no such invitation, or its
    /// answer is still on its way.
    fn accept(&mut self, id: &str) -> bool {
        let Some(session) = self
            .current()
            .filter(|held| held.invitations.iter().any(|i| i.id == id))
            .map(|held| held.session)
        else {
            return false;
        };
        self.change(session, id, ShareChange::Accept { invite_id: id.into() })
    }

    /// Sent once per press, for a person (never the owner) of a share in the current session's lists. The row keeps
    /// what Beebeeb last said until Beebeeb confirmed the change and the lists were read again.
    fn set_access(&mut self, share: &str, person: &str, access: Access) -> bool {
        let Some(session) = self.person_at(share, person) else {
            return false;
        };
        self.change(
            session,
            person,
            ShareChange::SetAccess {
                invite_id: person.into(),
                can_edit: access == Access::Edit,
            },
        )
    }

    /// Sent once per press (the view confirmed it, DW-0512b), for a person (never the owner) of a share in the
    /// current session's lists. The person stays in the rows until Beebeeb confirmed it and the lists were read
    /// again: access Beebeeb did not take away is never drawn as taken away.
    fn remove_access(&mut self, share: &str, person: &str) -> bool {
        let Some(session) = self.person_at(share, person) else {
            return false;
        };
        self.change(
            session,
            person,
            ShareChange::RemoveMember {
                invite_id: person.into(),
            },
        )
    }

    /// Sent once per press, for an invitation of the current session's lists (the web's Decline: the invitation
    /// leaves the inbox). The card goes when the lists are read again after Beebeeb answered.
    fn decline(&mut self, id: &str) -> bool {
        let Some(session) = self
            .current()
            .filter(|held| held.invitations.iter().any(|i| i.id == id))
            .map(|held| held.session)
        else {
            return false;
        };
        self.change(session, id, ShareChange::Decline { invite_id: id.into() })
    }

    // w17-shared-links (`shared_links.rs`).
    fn copy_row_link(&mut self, id: &str) -> bool {
        self.shared_links_copy_row_link(id)
    }
    fn change_link(&mut self, share_id: &str, change: LinkEdit) -> bool {
        self.shared_links_change_link(share_id, change)
    }
    fn change_request_expiry(&mut self, id: &str, hours: Option<u32>) -> bool {
        self.shared_links_change_request_expiry(id, hours)
    }

    // Slice B4 (`shared_offline.rs`): sent once per press for a row of "Shared with me".
    fn keep_offline(&mut self, id: &str) -> bool {
        self.shared_offline_keep_offline(id)
    }

    /// Sent once per press: a share with people (cancels every invite of it) or a link (revokes every live link of
    /// the item). Both stay in the rows until Beebeeb answered and the lists were read again.
    fn stop_sharing(&mut self, id: &str) -> bool {
        let Some(held) = self.current() else {
            return false;
        };
        let session = held.session;
        let change = if held.people.iter().any(|p| p.id == id) {
            let invite_ids: Vec<String> = held
                .members
                .get(id)
                .into_iter()
                .flatten()
                .filter(|m| m.access.is_some())
                .map(|m| m.id.clone())
                .collect();
            if invite_ids.is_empty() {
                return false;
            }
            ShareChange::CancelInvites { invite_ids }
        } else if let Some(share_ids) = held.link_shares.get(id) {
            ShareChange::RevokeLinks {
                share_ids: share_ids.clone(),
            }
        } else {
            return false;
        };
        self.change(session, id, change)
    }

    // Slice w17 (`shared_leave.rs`): Leave a share someone made with the person; where a row points.
    fn leave(&mut self, id: &str) -> bool {
        self.shared_leave(id)
    }

    fn target_of(&self, id: &str) -> Option<RowTarget> {
        self.shared_target_of(id)
    }

    /// Sent once per press, for an open request of the current session's lists.
    fn close_request(&mut self, id: &str) -> bool {
        let Some(session) = self
            .current()
            .filter(|held| {
                held.requests
                    .iter()
                    .any(|r| r.id == id && r.state == RequestState::Open)
            })
            .map(|held| held.session)
        else {
            return false;
        };
        self.change(session, id, ShareChange::CloseRequest { request_id: id.into() })
    }

    /// "Remove from my list": the server's HIDE (the row stays there with its key, so what arrived can still be
    /// opened), never the DELETE route that destroys the key. A request that is still open answers 409
    /// ("Close the request first."). The row leaves when the lists are read again after the answer.
    fn remove_request(&mut self, id: &str) -> bool {
        let Some(session) = self
            .current()
            .filter(|held| held.requests.iter().any(|r| r.id == id))
            .map(|held| held.session)
        else {
            return false;
        };
        self.change(session, id, ShareChange::HideRequest { request_id: id.into() })
    }

    /// Sent once per press, on a worker, under the session and account of the press. `share` is an item's id (one not
    /// shared with anyone yet too: the engine decides whether it is the account's own). `false`: nothing typed, an
    /// invitation for this share still on its way, no session, or no worker could start; nothing was sent.
    fn start_invite(&mut self, share: &str, email: &str) -> bool {
        let email = email.trim();
        if email.is_empty() || self.inviting.contains(share) || !self.engine.has_session() {
            return false;
        }
        let asked = stamp(&self.engine);
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let (sent_for, address) = (share.to_string(), email.to_string());
        let started = std::thread::Builder::new().name("shared-invite".into()).spawn(move || {
            let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.invite_for(asked.0, &sent_for, &address)
            }))
            .unwrap_or(InviteAnswer::Refused(InviteRefused::Failed));
            let _ = answers.send(Answer::Invited(sent_for, asked, answer));
            engine.wake_host();
        });
        if started.is_err() {
            log("shared: could not start a thread for an invitation");
            return false;
        }
        self.inviting.insert(share.to_string());
        self.dirty = true;
        true
    }

    fn invite_answer(&mut self) -> Option<(String, InviteResult)> {
        self.invite_answers.pop_front()
    }

    /// The account's top-level folders from this PC's copy of the list, by name (a read on the window's thread, as
    /// Files' own listing is); none when the list cannot be read yet.
    fn folders(&self) -> Vec<(String, String)> {
        use beebeeb_engine::drive_v2::model::{self, Sort};
        let read = |db: &beebeeb_engine::state_db::StateDb| {
            model::children_of(db, None, Sort::default(), 0, usize::MAX)
                .map(|(rows, _)| rows)
                .unwrap_or_default()
        };
        let rows = self
            .engine
            .local_state()
            .and_then(|state| state.with(read))
            .unwrap_or_default();
        rows.into_iter()
            .filter(|row| row.is_folder)
            .map(|row| (row.id, row.name))
            .collect()
    }

    fn create_request(&mut self, form: RequestForm) -> bool {
        self.shared_requests_create(form)
    }

    fn take_link(&mut self) -> Option<LinkText> {
        self.shared_links_take_link()
            .or_else(|| self.shared_requests_take_link())
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
        self.asked = true;
        let fresh = self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
        }
    }

    /// A finished check: lists someone looked at in this session are read again; otherwise the next opening reads.
    fn refresh(&mut self) {
        if !self.engine.has_session() {
            return;
        }
        if self.asked {
            self.read();
        } else {
            self.read_at = None;
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
        changed |= self.shared_safety_pump();
        let notice = self.notice.take();
        Pumped {
            changed: changed || notice.is_some() || !self.invite_answers.is_empty(),
            busy: self.reading.is_some()
                || !self.changing.is_empty()
                || !self.inviting.is_empty()
                || self.shared_safety_busy(),
            notice,
        }
    }

    fn forget(&mut self) {
        self.rev += 1;
        self.shared_requests_forget();
        self.shared_safety_forget();
        self.held = None;
        self.again = false;
        self.asked = false;
        self.changing.clear();
        self.inviting.clear();
        self.invite_answers.clear();
        self.unmade = false;
        self.read_at = None;
        self.notice = None;
        self.loaded = Loaded::Loading;
        self.dirty = true;
    }
}

/// A with-me row whose id the engine named as kept on this PC is drawn as available offline; every other row stays
/// "no claim". Nothing here reads anything: the set is the engine's answer.
pub(crate) fn mark_kept(held: &mut Held, kept: &HashSet<String>) {
    for row in &mut held.with_me {
        if kept.contains(&row.id) {
            row.pc = Some(PcState::AvailableOffline);
        }
    }
}

/// The engine's overview as the view's lists. Nothing here reads anything.
pub(crate) fn held_of(overview: SharesOverview, now: DateTime<Local>) -> Held {
    let kind = |is_folder: bool, name: &str| kind_of(is_folder, mime_for_name(name));
    let date = |stamp: &str| seconds_of(stamp).map(|at| when(at, now)).unwrap_or_default();
    let (mut invitations, mut with_me) = (Vec::new(), Vec::new());
    let mut invites = HashMap::new();
    for item in overview.incoming {
        if !item.waiting {
            invites.insert(item.item_id.clone(), item.invite_id.clone());
        }
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
            bytes: (!item.is_folder).then(|| item.size_bytes.max(0) as u64),
            at: seconds_of(&item.shared_at),
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
    let mut link_shares = HashMap::new();
    let mut link_items = HashMap::new();
    let links = overview
        .links
        .into_iter()
        .map(|link| {
            link_shares.insert(link.share_id.clone(), link.share_ids.clone());
            link_items.insert(link.share_id.clone(), link.item_id.clone());
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
                id: link.share_id,
                name: link.name,
                line,
                expired,
            }
        })
        .collect();
    let mut request_folders = HashMap::new();
    let requests = overview
        .requests
        .into_iter()
        .map(|r| {
            if let Some(folder) = &r.target_folder_id {
                request_folders.insert(r.id.clone(), folder.clone());
            }
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
        link_shares,
        link_items,
        requests,
        invites,
        request_folders,
    }
}

/// The engine's answer as the view's, with the words a refusal shows (no key, address or name in any of them).
fn invite_result(answer: &InviteAnswer) -> InviteResult {
    match answer {
        InviteAnswer::Shared { .. } => InviteResult::HasAccess,
        InviteAnswer::Invited { .. } => InviteResult::Sent,
        InviteAnswer::AwaitingApproval { .. } => InviteResult::WaitingForApproval,
        InviteAnswer::PeerKeyChanged { .. } => InviteResult::KeyChanged,
        // A claimed invite that stays (its key never went over, and it could not be withdrawn): never "nothing was
        // shared" (review of desktop#424).
        InviteAnswer::Unfinished {
            approval_sent: true, ..
        } => InviteResult::Refused(UNFINISHED_UNCONFIRMED),
        InviteAnswer::Unfinished { .. } => InviteResult::Refused(UNFINISHED),
        InviteAnswer::Refused(kind) => InviteResult::Refused(refusal_words(*kind)),
    }
}

/// What the invite dialog says when they accepted, but the key never went over and the invite could not be withdrawn.
pub(crate) const UNFINISHED: &str = "They accepted, but Beebeeb couldn\u{2019}t finish sharing and the invitation is still open. They can\u{2019}t open anything yet. Cancel it in Shared, then invite them again.";

/// What the invite dialog says when the approval was sent and its answer was lost, and the invite could not be
/// withdrawn either: Beebeeb cannot say whether they can open it now.
pub(crate) const UNFINISHED_UNCONFIRMED: &str = "Beebeeb couldn\u{2019}t confirm whether they can open it yet. They may already be able to. Check Shared, and cancel it there if you didn\u{2019}t mean to share.";

/// What the invite dialog says for a refusal. Every one ends in what was (not) done.
pub(crate) fn refusal_words(kind: InviteRefused) -> &'static str {
    match kind {
        InviteRefused::AlreadyOnIt => "They\u{2019}re on this share already.",
        InviteRefused::NotYours => "Only items in your own files can be shared with people. Nothing was shared.",
        InviteRefused::FolderNeedsUpgrade => {
            "This folder\u{2019}s sharing needs an update this version can\u{2019}t make yet. Nothing was shared."
        }
        InviteRefused::FolderKeyUnverified => {
            "Beebeeb couldn\u{2019}t confirm this folder\u{2019}s key, so nothing was shared."
        }
        InviteRefused::InvalidEmail => "That isn\u{2019}t an email address Beebeeb can send to. Nothing was shared.",
        InviteRefused::SharedFoldersFlagUnknown => {
            "Couldn\u{2019}t check whether folder sharing is on. Nothing was shared. Try again in a moment."
        }
        InviteRefused::Offline => "Can\u{2019}t reach Beebeeb. Nothing was shared.",
        InviteRefused::SessionEnded => "Your session ended. Sign in again to share. Nothing was shared.",
        InviteRefused::Failed => "Couldn\u{2019}t send the invitation. Nothing was shared.",
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
        // The engine's trust read is laid over it when the lists are handed out (`shared_safety.rs`).
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
