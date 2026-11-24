//! The two-factor status on the real engine (DW-A71, and the backup-codes line of the Security page): when it was
//! added, when it was last used and how many backup codes are left, from `GET /api/v1/me/two-factor`
//! ([`Engine::two_factor_status`]).
//!
//! Read when a pane that shows it opens (and again when what is held is older than [`FRESH_FOR`]), never from a
//! draw; an answer for another session or account is dropped. A read that failed (or an answer that does not
//! read) draws NO value: the page keeps hiding the rows (a made-up value is never shown). After a change of
//! two-factor (on, off, new backup codes) what is held is stale and read again.
//!
//! Dates follow the Passkeys list's rule: the day as "28 Oct"; a recent use as "Today, 09:12" / "Yesterday, 09:12".
//!
//! Must not: log a date, a count or a server's words.

use std::sync::{Arc, mpsc};
use std::time::Instant;

use beebeeb_engine::api_client::TwoFactorStatus;
use beebeeb_engine::app::account_api::Stamped;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use chrono::{DateTime, Local};

use super::{FRESH_FOR, Stamp, account_of, stamp};
use crate::frames::account::Pane;
use crate::frames::account::source::TwoFactor;

struct Answer {
    number: u64,
    made_for: Stamp,
    result: Result<Stamped<TwoFactorStatus>, AccountCallError>,
}

struct Held {
    stamp: Stamp,
    status: TwoFactorStatus,
}

pub(super) struct StatusPart<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    held: Option<Held>,
    reads: u64,
    reading: Option<u64>,
    read_at: Option<Instant>,
    /// The session the last read failed for: not asked again until a pane opens again.
    failed_for: Option<u64>,
    showing: bool,
    tx: mpsc::Sender<Answer>,
    rx: mpsc::Receiver<Answer>,
}

/// "28 Oct": the day, in the person's time zone; empty when it does not read.
fn day_words(at: &str) -> String {
    DateTime::parse_from_rfc3339(at)
        .map(|at| at.with_timezone(&Local).format("%-d %b").to_string())
        .unwrap_or_default()
}

/// "Today, 09:12", "Yesterday, 09:12", else "28 Oct, 09:12"; empty when it does not read.
fn use_words(at: &str, now: DateTime<Local>) -> String {
    let Ok(at) = DateTime::parse_from_rfc3339(at) else {
        return String::new();
    };
    let at = at.with_timezone(&Local);
    let days = now.date_naive().signed_duration_since(at.date_naive()).num_days();
    let time = at.format("%H:%M");
    match days {
        0 => format!("Today, {time}"),
        1 => format!("Yesterday, {time}"),
        _ => format!("{}, {time}", at.format("%-d %b")),
    }
}

/// What the page draws for a status; a value Beebeeb did not give stays empty / 0 (the page hides that row).
pub(super) fn two_factor_of(status: &TwoFactorStatus, now: DateTime<Local>) -> TwoFactor {
    if !status.enabled {
        return TwoFactor {
            on: false,
            added: String::new(),
            last_used: String::new(),
            codes_left: 0,
            codes_total: 0,
        };
    }
    let total = status.backup_codes_total.unwrap_or(0) as usize;
    TwoFactor {
        on: true,
        added: status.added_at.as_deref().map(day_words).unwrap_or_default(),
        last_used: status
            .last_used_at
            .as_deref()
            .map(|at| use_words(at, now))
            .unwrap_or_default(),
        // A count that is not known, or larger than its total, is no count.
        codes_left: if total > 0 {
            (status.backup_codes_left.unwrap_or(0) as usize).min(total)
        } else {
            0
        },
        codes_total: if status.backup_codes_left.is_some() { total } else { 0 },
    }
}

impl<C: CredentialStores + 'static> StatusPart<C> {
    pub(super) fn new(engine: Arc<Engine<C>>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            engine,
            held: None,
            reads: 0,
            reading: None,
            read_at: None,
            failed_for: None,
            showing: false,
            tx,
            rx,
        }
    }

    fn open(&self) -> bool {
        self.engine.has_session() && !self.engine.account_session_refused()
    }

    fn is_now(&self, of: &Stamp) -> bool {
        self.open() && of.0 == self.engine.session_turn() && of.1 == account_of(&self.engine)
    }

    fn current(&self) -> Option<&TwoFactorStatus> {
        self.held
            .as_ref()
            .filter(|held| self.is_now(&held.stamp))
            .map(|held| &held.status)
    }

    /// The status as the page draws it, when one was read for this session and account.
    pub(super) fn known(&self) -> Option<TwoFactor> {
        self.current().map(|status| two_factor_of(status, Local::now()))
    }

    fn shows(pane: Pane) -> bool {
        matches!(pane, Pane::TwoFactor | Pane::Security)
    }

    /// A pane opened: read when nothing fresh is held.
    pub(super) fn opened(&mut self, pane: Pane) {
        self.showing = Self::shows(pane);
        if !self.showing || !self.open() {
            return;
        }
        self.failed_for = None;
        let fresh = self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
        }
    }

    pub(super) fn closed(&mut self) {
        self.showing = false;
    }

    /// What is held is out of date (two-factor was changed): read again when a pane shows it.
    pub(super) fn stale(&mut self) {
        self.read_at = None;
        if self.showing {
            self.read();
        }
    }

    fn read(&mut self) {
        if !self.open() || self.reading.is_some() {
            return;
        }
        self.reads += 1;
        let number = self.reads;
        let made_for = stamp(&self.engine);
        let (engine, tx, mine) = (self.engine.clone(), self.tx.clone(), made_for.clone());
        let started = std::thread::Builder::new()
            .name("account-two-factor".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.two_factor_status()))
                    .unwrap_or(Err(AccountCallError::ServerError));
                let _ = tx.send(Answer {
                    number,
                    made_for: mine,
                    result,
                });
                engine.wake_host();
            })
            .is_ok();
        if started {
            self.reading = Some(number);
            self.failed_for = None;
        } else {
            self.failed_for = Some(number);
        }
    }

    /// The window's poll: `(changed, busy)`.
    pub(super) fn pump(&mut self) -> (bool, bool) {
        let mut changed = false;
        // The session or the account moved on: nothing of the status stays.
        if self.held.as_ref().is_some_and(|held| !self.is_now(&held.stamp)) {
            self.held = None;
            self.read_at = None;
            changed = true;
        }
        while let Ok(answer) = self.rx.try_recv() {
            if self.reading == Some(answer.number) {
                self.reading = None;
            }
            // Dropped: made for a session or an account that is not the one here now.
            if !self.is_now(&answer.made_for) {
                continue;
            }
            changed = true;
            match answer.result {
                Ok(stamped) if stamped.session == answer.made_for.0 => {
                    self.held = Some(Held {
                        stamp: answer.made_for,
                        status: stamped.value,
                    });
                    self.read_at = Some(Instant::now());
                    self.failed_for = None;
                }
                _ => {
                    // Not read: no value is drawn, and it is never fresh.
                    self.held = None;
                    self.read_at = None;
                    self.failed_for = Some(answer.number);
                }
            }
        }
        // A new session while the pane shows: read once for it.
        if self.showing
            && self.open()
            && self.reading.is_none()
            && self.current().is_none()
            && self.failed_for.is_none()
        {
            self.read();
            changed = true;
        }
        (changed, self.reading.is_some())
    }
}

#[cfg(test)]
#[path = "account_twofactor_status_tests.rs"]
mod tests;
