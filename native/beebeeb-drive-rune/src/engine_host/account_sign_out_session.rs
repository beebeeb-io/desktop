//! Sending one sign-out of ANOTHER session of the account (Devices and sessions) on a worker: the one function
//! `EngineAccountSource::sign_out_device` and `sign_out_everywhere_else` stand on. Moved out of `account.rs`
//! unchanged (its size limit); it works on that struct's own fields.

use super::*;

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    /// Send one sign-out on a worker: `key` is a session's id of the list that shows, or [`ELSEWHERE`]. Once per
    /// press; a second press while it is on its way sends nothing and says so, and so does a press that is
    /// refused before anything is sent. `true`: sent.
    ///
    /// `again`: the toast's button, with what its toast was about. "Everywhere else" is then sent one by one, for
    /// the sessions that were listed at the confirm and that the list still has: never for one made since (the
    /// one request for all of them would end those too, and nobody confirmed that).
    pub(super) fn sign_out(&mut self, key: &str, again: Option<About>) -> bool {
        // The person did something else: what worked is no longer said, nor anything about this sign-out. What
        // was not done for ANOTHER sign-out stays in the line.
        self.notices
            .retain(|(_, said)| said.failed && said.about.as_ref().is_some_and(|about| about.key != key));
        self.dirty = true;
        if !self.open() {
            return false;
        }
        let one_by_one = key == ELSEWHERE && again.is_some();
        let retry = again.is_some();
        let held = self
            .sessions
            .as_ref()
            .filter(|held| self.is_now(held.session, &held.account));
        let Some(held) = held else {
            // No list to send against. From the toast's button the toast keeps its button: it works again
            // once the list has been read.
            let said = match again {
                Some(about) => Said::failed(NOT_SENT, Some(about), true),
                None => Said::failed(NOT_SENT, None, false),
            };
            self.say(said);
            return false;
        };
        let session = held.session;
        let others = || held.value.iter().filter(|row| !row.current);
        // Only rows of the list that shows, never this session, and for the button only the confirmed ones.
        let confirmed = match again {
            Some(about) => about.ids,
            None if key == ELSEWHERE => others().map(|row| row.id.clone()).collect(),
            None => vec![key.to_string()],
        };
        let targets: Vec<String> = others()
            .filter(|row| confirmed.contains(&row.id))
            .map(|row| row.id.clone())
            .collect();
        if targets.is_empty() {
            let said = if !retry {
                Said::failed(NOT_SENT, None, false)
            } else if key != ELSEWHERE {
                // The list no longer has what the toast was about: there is nothing left to send.
                Said::done(IS_SIGNED_OUT)
            } else if others().next().is_some() {
                Said::done(SIGNED_OUT_THOSE)
            } else {
                Said::done(SIGNED_OUT_ELSEWHERE)
            };
            self.say(said);
            return false;
        }
        if self.changing.iter().any(|(s, k)| *s == session && k == key) {
            self.say(Said::busy());
            return false;
        }
        let about = About {
            key: key.to_string(),
            ids: targets,
        };
        let (sent, fallback) = (about.clone(), about.clone());
        // As for a read: a worker that panics answers for this session, as a sign-out that is not known.
        let asked_as = stamp(&self.engine);
        let started = spawn(
            "account-sign-out",
            &self.engine,
            &self.answers,
            self.work(move |engine| {
                let before = stamp(engine);
                let mut about = sent;
                let (result, ended, gone) = if !about.elsewhere() {
                    let result = engine.end_account_session(session, &about.key);
                    (result, usize::from(result == Ok(SessionEnd::Ended)), false)
                } else if !one_by_one {
                    let result = engine.revoke_other_account_sessions(session).map(|_| SessionEnd::Ended);
                    (result, 0, false)
                } else {
                    // Each is checked against the session by the engine as it is sent; each has its own
                    // outcome. Only after an error Beebeeb ANSWERED with is the next one sent: a refusal that
                    // holds for the rest (no session, a 401) or a call that got no answer (each waits its full
                    // time: five would be minutes without a word) ends it, and the rest count as not done.
                    let (mut ended, mut gone, mut left, mut first) = (0, false, Vec::new(), None);
                    let mut ids = about.ids.iter();
                    for id in ids.by_ref() {
                        match engine.end_account_session(session, id) {
                            Ok(SessionEnd::Ended) => ended += 1,
                            Ok(SessionEnd::AlreadyGone) => gone = true,
                            Err(error) => {
                                left.push(id.clone());
                                match error {
                                    AccountCallError::ServerError => {
                                        first.get_or_insert(error);
                                    }
                                    AccountCallError::NotSignedIn | AccountCallError::SessionEnded => {
                                        first.get_or_insert(error);
                                        break;
                                    }
                                    // No answer: that sign-out may have landed, whatever an earlier one was
                                    // answered. It is the outcome: not known, and the list is read to find out.
                                    _ => {
                                        first = Some(error);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    left.extend(ids.cloned());
                    match first {
                        Some(error) => {
                            about.ids = left;
                            (Err(error), ended, gone)
                        }
                        // A 404 for one of them: as for one session, the list has the last word.
                        None if gone => (Ok(SessionEnd::AlreadyGone), ended, gone),
                        None => (Ok(SessionEnd::Ended), ended, gone),
                    }
                };
                Answer::SignedOut {
                    about,
                    sent_for: session,
                    made_for: still(engine, before),
                    ended,
                    gone,
                    one_by_one,
                    result,
                }
            }),
            move || Answer::SignedOut {
                about: fallback,
                sent_for: session,
                made_for: Some(asked_as),
                ended: 0,
                gone: false,
                one_by_one,
                result: Err(AccountCallError::Offline),
            },
        );
        if started {
            self.changing.push((session, key.to_string()));
        } else {
            log("account: could not start a thread for a sign-out");
            self.say(Said::failed(NOT_SENT, Some(about), true));
        }
        started
    }
}
