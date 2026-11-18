//! People's safety number, the "compared" marks and "Share anyway" on the real engine (w18-people-safety-number,
//! DW-0513, DW-0515). Part of [`EngineSharedSource`] (`shared.rs`), which hands the `SharedSource` safety calls here.
//!
//! - The number, the two marks and "Share anyway" are the engine's (`Engine::safety_number_for`,
//!   `mark_compared_for`, `share_anyway_for`, over core's `share_flow::safety`): each runs on a worker, for the session
//!   the lists were read for, one per person at a time. Nothing here computes or keeps a key: the state holds the
//!   twelve groups of digits and a word, no more.
//! - A mark is local engine state only. "They don't match" and "They match" are asked for only after the number is in
//!   (the state is `Number`), so a mark is always for the number the person saw.
//! - "Share anyway" runs only from [`SafetyAsk::Anyway`], i.e. the button's press; the engine seals only the key the
//!   changed-key warning carried. A folder's invite is withdrawn instead.
//! - `Member::compared` is the engine's trust read ([`Engine::share_trust_for`]) after every read of the lists: a share
//!   is compared only when its mark says so on the key it was sealed to.
//! - An answer for another session than the current one is dropped.
//!
//! Must not: log a number, a key, an id or an address. Holds no key.

use beebeeb_engine::app::bindings::share_safety::{MemberTrust, SafetyRefused, ShareAnyway, ShareTrust};

use super::*;
use crate::frames::shared::source::{SafetyAsk, SafetyState};

/// What a worker ends in.
pub(super) enum Done {
    Asked {
        key: (String, String),
        session: SessionGeneration,
        ask: SafetyAsk,
        number: Vec<String>,
        result: Result<Outcome, SafetyRefused>,
    },
    Trust(SessionGeneration, Vec<MemberTrust>),
}

pub(super) enum Outcome {
    Number(Vec<String>),
    Marked(bool),
    Shared,
    Withdrawn,
}

/// What the safety calls keep. The adapter holds one.
pub(super) struct SafetyCalls {
    tx: mpsc::Sender<Done>,
    rx: mpsc::Receiver<Done>,
    states: HashMap<(String, String), SafetyState>,
    /// The invites (people) whose share the engine says is compared, for the current session's lists.
    compared: HashSet<String>,
    trusting: bool,
    /// The person each share's last changed-key invite stopped on.
    key_changed: HashMap<String, String>,
}

impl Default for SafetyCalls {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            states: HashMap::new(),
            compared: HashSet::new(),
            trusting: false,
            key_changed: HashMap::new(),
        }
    }
}

/// The words a refusal is shown in. Fixed: the engine's `kind` never reaches the view.
pub(super) fn refusal_text(why: SafetyRefused) -> &'static str {
    match why {
        SafetyRefused::NotSignedIn => "Sign in to compare safety numbers.",
        SafetyRefused::Locked => "Unlock Beebeeb to compare safety numbers.",
        SafetyRefused::SessionEnded => "Beebeeb no longer accepts this session. Sign in again.",
        SafetyRefused::Offline => "Couldn\u{2019}t reach Beebeeb. Nothing was changed.",
        SafetyRefused::NotFound | SafetyRefused::NotYours => "This share isn\u{2019}t available any more.",
        SafetyRefused::NoPeerId => "Beebeeb doesn\u{2019}t list their account yet. Try again in a moment.",
        SafetyRefused::NoSealedKey => {
            "This was shared before safety numbers were kept, so there is nothing to compare."
        }
        SafetyRefused::ApprovalUnanswered => "Beebeeb didn\u{2019}t answer. They may already have access.",
        SafetyRefused::Failed => "Couldn\u{2019}t do that. Nothing was changed.",
    }
}

impl<C: CredentialStores> EngineSharedSource<C> {
    pub(super) fn shared_safety_state(&self, share: &str, person: &str) -> SafetyState {
        self.safety
            .states
            .get(&(share.to_string(), person.to_string()))
            .cloned()
            .unwrap_or(SafetyState::Idle)
    }

    pub(super) fn shared_safety_clear(&mut self, share: &str, person: &str) {
        let key = (share.to_string(), person.to_string());
        // A call on its way is not forgotten: its answer would have nowhere to land.
        if !matches!(self.safety.states.get(&key), Some(SafetyState::Asking(_))) {
            self.safety.states.remove(&key);
        }
    }

    /// Sent once per press for a person of the current session's lists; the marks only after the number is in.
    pub(super) fn shared_safety_start(&mut self, share: &str, person: &str, ask: SafetyAsk) -> bool {
        let Some(held) = self.current() else {
            return false;
        };
        let session = held.session;
        let Some(member) = held
            .members
            .get(share)
            .and_then(|m| m.iter().find(|m| m.id == person && m.access.is_some()))
        else {
            return false;
        };
        let email = member.email.clone();
        let key = (share.to_string(), person.to_string());
        let number = match (self.safety.states.get(&key), ask) {
            (Some(SafetyState::Asking(_)), _) => return false,
            (Some(SafetyState::Number(groups)), SafetyAsk::Matches | SafetyAsk::Differs) => groups.clone(),
            (_, SafetyAsk::Matches | SafetyAsk::Differs) => return false,
            _ => Vec::new(),
        };
        let (engine, tx) = (self.engine.clone(), self.safety.tx.clone());
        let (item, member_id, worker_key, worker_number) = (share.to_string(), person.to_string(), key.clone(), number);
        let started = std::thread::Builder::new().name("shared-safety".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match ask {
                SafetyAsk::Number => engine
                    .safety_number_for(session, &member_id)
                    .map(|n| Outcome::Number(n.groups)),
                SafetyAsk::Matches | SafetyAsk::Differs => engine
                    .mark_compared_for(session, &member_id, ask == SafetyAsk::Matches)
                    .map(|()| Outcome::Marked(ask == SafetyAsk::Matches)),
                SafetyAsk::Anyway => {
                    engine
                        .share_anyway_for(session, &member_id, &item, &email)
                        .map(|done| match done {
                            ShareAnyway::Shared { .. } => Outcome::Shared,
                            ShareAnyway::Withdrawn { .. } => Outcome::Withdrawn,
                        })
                }
            }))
            .unwrap_or(Err(SafetyRefused::Failed));
            let _ = tx.send(Done::Asked {
                key: worker_key,
                session,
                ask,
                number: worker_number,
                result,
            });
            engine.wake_host();
        });
        if started.is_err() {
            log("shared: could not start a thread for a safety call");
            return false;
        }
        self.safety.states.insert(key, SafetyState::Asking(ask));
        self.dirty = true;
        true
    }

    /// The trust of every person's share, read after the lists were: what `Member::compared` says.
    pub(super) fn shared_safety_after_read(&mut self) {
        if self.safety.trusting {
            return;
        }
        let Some(session) = self.current().map(|held| held.session) else {
            return;
        };
        let (engine, tx) = (self.engine.clone(), self.safety.tx.clone());
        let started = std::thread::Builder::new().name("shared-trust".into()).spawn(move || {
            let trust = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.share_trust_for(session)))
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            let _ = tx.send(Done::Trust(session, trust));
            engine.wake_host();
        });
        self.safety.trusting = started.is_ok();
    }

    /// A changed-key invite for `share` stopped on this invite: People warns over it.
    pub(super) fn shared_safety_key_changed(&mut self, share: &str, invite: &str) {
        self.safety.key_changed.insert(share.to_string(), invite.to_string());
    }

    pub(super) fn shared_safety_key_changed_person(&self, share: &str) -> Option<String> {
        self.safety.key_changed.get(share).cloned()
    }

    /// `Member::compared` for what the trust read says. Applied where the lists are handed out.
    pub(super) fn shared_safety_overlay(&self, members: &mut [Member]) {
        for m in members {
            m.compared = self.safety.compared.contains(&m.id);
        }
    }

    pub(super) fn shared_safety_forget(&mut self) {
        let calls = &mut self.safety;
        calls.states.clear();
        calls.compared.clear();
        calls.key_changed.clear();
        while calls.rx.try_recv().is_ok() {}
    }

    /// Take in the answers. `true`: something the view draws changed.
    pub(super) fn shared_safety_pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.safety.rx.try_recv() {
            let now = self.engine.session_turn();
            match done {
                Done::Trust(session, trust) => {
                    self.safety.trusting = false;
                    if session == now {
                        self.safety.compared = trust
                            .into_iter()
                            .filter(|t| t.trust == ShareTrust::Compared)
                            .map(|t| t.invite_id)
                            .collect();
                        changed = true;
                    }
                }
                Done::Asked {
                    key,
                    session,
                    ask,
                    number,
                    result,
                } => {
                    // An answer of another session is dropped, never drawn.
                    if session != now {
                        self.safety.states.remove(&key);
                        continue;
                    }
                    let state = match result {
                        Ok(Outcome::Number(groups)) => SafetyState::Number(groups),
                        Ok(Outcome::Marked(matched)) => SafetyState::Marked { matched, number },
                        Ok(Outcome::Shared) => SafetyState::Shared,
                        Ok(Outcome::Withdrawn) => SafetyState::Withdrawn,
                        Err(why) => {
                            log(format!("shared: a safety call was refused ({})", why.kind()));
                            // A refused mark leaves the number as it was, with the words under it.
                            if matches!(ask, SafetyAsk::Matches | SafetyAsk::Differs) && !number.is_empty() {
                                SafetyState::Number(number)
                            } else {
                                SafetyState::Refused(refusal_text(why))
                            }
                        }
                    };
                    // The marks and a share that went through change the lists: read them, and the trust, again.
                    let lists_change = matches!(
                        state,
                        SafetyState::Marked { .. } | SafetyState::Shared | SafetyState::Withdrawn
                    );
                    self.safety.states.insert(key, state);
                    if lists_change {
                        self.read();
                    }
                    changed = true;
                }
            }
        }
        changed
    }

    /// A person is being worked on.
    pub(super) fn shared_safety_busy(&self) -> bool {
        self.safety.trusting || self.safety.states.values().any(|s| matches!(s, SafetyState::Asking(_)))
    }
}
