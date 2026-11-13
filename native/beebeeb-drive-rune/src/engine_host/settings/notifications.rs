//! Settings › Notifications (DW-0750) and Recent notifications (DW-0791) on the real engine: the part of
//! [`super::EngineSettingsSource`] that answers the five notification methods of `SettingsSource`.
//!
//! What each value is, and where it comes from:
//! - "Your account" (Shared with you, File updates, Storage warnings): the account's own alert switches on Beebeeb
//!   ([`Engine::notification_preferences`]; `share_received`, `file_updated`, `storage_warning`). A change is sent
//!   once ([`Engine::set_notification_preferences`], only the one switch) and shown only after Beebeeb took it. The
//!   group is always drawn in its place (so no row moves under the pointer when an answer lands); until the switches
//!   are read (or when the read failed) it has no rows: never a value that was not read. A press finds its row by
//!   its fixed place in the group table, never by what is drawn at that position now.
//!   `new_device_login` is never sent from here: the pane says security alerts cannot be turned off.
//! - "Allow notifications", "Sync on this PC" and "Backups on this PC": this PC's settings file (`DesktopConfig`'s
//!   `notify_*` switches), read when the window starts and written merge-only by one writer, the switch alone. A
//!   write that fails puts back what the file holds, and the toast says so.
//! - Recent notifications and the "N new" on the Notifications row: the account's notifications from Beebeeb
//!   ([`Engine::notifications`]), newest first, grouped by day on this computer's clock. A type the pane has no
//!   place for (a welcome, a storage-location notice, a backup pass's notices) is left out, so the count is the
//!   count of the "New" alerts the page shows. Leaving the page marks those seen ([`Engine::mark_notifications_read`],
//!   only the ones shown as new, one call each); a refusal says so and reads again, so "New" comes back on exactly
//!   the ones Beebeeb still holds unread (the engine stops at the first that fails; the ones before it are marked).
//! - What Windows allows for this app is not read here: `allowed_in_windows` is always `true` (see the PR record).
//!
//! When it reads: when Notifications or Recent notifications opens and what it holds is missing or older than
//! [`FRESH_FOR`]. Never from a draw call: every accessor only reads what is here.
//!
//! Session rule: every answer carries the session ([`Engine::session_turn`]) and the account it was made for; one
//! that is not the current session's and account's is dropped, every accessor checks before it hands anything out,
//! and the window's poll ([`NotificationsPart::pump`]) drops what it holds the moment the session moves on (a lock,
//! a sign-out, another sign-in).
//!
//! Thread rule: each call runs on a worker of its own and sends its answer down one channel, taken in by `pump`
//! (the window's poll); the worker wakes the window ([`Engine::wake_host`]).
//!
//! Must not: log a title, a body (an email, an address), an id or a server's words; only a call's kind.

use std::collections::HashSet;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::account_dto::{
    Notification, NotificationList, NotificationPreferenceValues, NotificationPreferencesUpdate,
};
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::identity::EngineIdentity;
use chrono::{DateTime, Datelike, Local, TimeZone};

use super::super::log;
use super::super::trash::seconds_of;
use crate::frames::settings::Pane;
use crate::frames::settings::source::{Alert, AlertKind, AlertTarget, Notifications, RecentAlert};

#[cfg(test)]
mod tests;

/// What this part holds is not read again, when its pane opens, for this long.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// What the toasts say.
pub const ALERT_SAVED: &str = "Notification setting updated";
/// Only after an HTTP refusal (a status Beebeeb answered): the switch stands as it was.
pub const ALERT_NOT_SAVED: &str = "Couldn't change the notification setting. Nothing changed.";
/// A timeout, a transport error mid-flight, or an unreadable 2xx: the change may have landed.
pub const ALERT_UNKNOWN: &str = "Couldn't confirm the change. Checking what the server has.";
pub const SWITCH_NOT_SAVED: &str = "Couldn't save the notification setting on this PC.";
pub const NOT_LOADED: &str = "Couldn't load your notifications. Try again in a moment.";
pub const NOT_MARKED: &str = "Couldn't mark your notifications as seen.";
pub const STILL_SAVING: &str = "Still saving the last change. Try again in a moment.";

/// One of the account's switches on Beebeeb.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AccountSwitch {
    Shared,
    FileUpdates,
    StorageWarnings,
}

impl AccountSwitch {
    fn of(self, values: &NotificationPreferenceValues) -> bool {
        match self {
            Self::Shared => values.share_received,
            Self::FileUpdates => values.file_updated,
            Self::StorageWarnings => values.storage_warning,
        }
    }

    /// The update that changes this switch alone.
    fn update(self, on: bool) -> NotificationPreferencesUpdate {
        let mut update = NotificationPreferencesUpdate::default();
        match self {
            Self::Shared => update.share_received = Some(on),
            Self::FileUpdates => update.file_updated = Some(on),
            Self::StorageWarnings => update.storage_warning = Some(on),
        }
        update
    }
}

/// One of this PC's switches in its settings file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PcSwitch {
    Allow,
    SyncStopped,
    NeedsDecision,
    SyncFinished,
    LocalSpace,
    BackupFinished,
    BackupAttention,
}

impl PcSwitch {
    const ALL: [PcSwitch; 7] = [
        Self::Allow,
        Self::SyncStopped,
        Self::NeedsDecision,
        Self::SyncFinished,
        Self::LocalSpace,
        Self::BackupFinished,
        Self::BackupAttention,
    ];

    fn slot(self) -> usize {
        self as usize
    }

    fn of(self, config: &DesktopConfig) -> bool {
        match self {
            Self::Allow => config.notify_allow,
            Self::SyncStopped => config.notify_sync_stopped,
            Self::NeedsDecision => config.notify_needs_decision,
            Self::SyncFinished => config.notify_sync_complete,
            Self::LocalSpace => config.notify_local_space,
            Self::BackupFinished => config.notify_backup_finished,
            Self::BackupAttention => config.notify_backup_attention,
        }
    }

    fn set(self, config: &mut DesktopConfig, on: bool) {
        let field = match self {
            Self::Allow => &mut config.notify_allow,
            Self::SyncStopped => &mut config.notify_sync_stopped,
            Self::NeedsDecision => &mut config.notify_needs_decision,
            Self::SyncFinished => &mut config.notify_sync_complete,
            Self::LocalSpace => &mut config.notify_local_space,
            Self::BackupFinished => &mut config.notify_backup_finished,
            Self::BackupAttention => &mut config.notify_backup_attention,
        };
        *field = on;
    }
}

/// Where a row's switch is stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stored {
    Account(AccountSwitch),
    Pc(PcSwitch),
}

/// The groups as the board draws them (DW-0750): a title, and each row's words and store.
const GROUPS: [(&str, &[(&str, &str, Stored)]); 3] = [
    (
        "Your account",
        &[
            (
                "Shared with you",
                "When someone shares a file with you",
                Stored::Account(AccountSwitch::Shared),
            ),
            (
                "File updates",
                "When a file changes on another device",
                Stored::Account(AccountSwitch::FileUpdates),
            ),
            (
                "Storage warnings",
                "When you're close to your limit",
                Stored::Account(AccountSwitch::StorageWarnings),
            ),
        ],
    ),
    (
        "Sync on this PC",
        &[
            (
                "Sync stopped",
                "When files couldn't upload",
                Stored::Pc(PcSwitch::SyncStopped),
            ),
            (
                "Files need a decision",
                "When a file changed in two places",
                Stored::Pc(PcSwitch::NeedsDecision),
            ),
            (
                "Sync finished",
                "When everything on this PC has synced. Off by default",
                Stored::Pc(PcSwitch::SyncFinished),
            ),
            (
                "Space on this PC",
                "When downloaded files near their limit",
                Stored::Pc(PcSwitch::LocalSpace),
            ),
        ],
    ),
    (
        "Backups on this PC",
        &[
            (
                "Backup finished",
                "When a backup completes",
                Stored::Pc(PcSwitch::BackupFinished),
            ),
            (
                "Backup needs attention",
                "Such as a folder that can't be read",
                Stored::Pc(PcSwitch::BackupAttention),
            ),
        ],
    ),
];

/// Where a notification of this type leads, and its icon. `None`: the pane has no place for it, so it is left out.
fn place_of(kind: &str) -> Option<(AlertKind, AlertTarget)> {
    match kind {
        "new_device_login" => Some((AlertKind::SignIn, AlertTarget::DevicesAndSessions)),
        "security" => Some((AlertKind::Security, AlertTarget::SecurityLog)),
        "share_received" | "share_approved" => Some((AlertKind::Share, AlertTarget::Shared)),
        "share_claimed" => Some((AlertKind::Request, AlertTarget::SharedRequests)),
        _ => None,
    }
}

/// One notification the pane can show.
#[derive(Clone, Debug, PartialEq)]
struct Listed {
    id: String,
    kind: AlertKind,
    target: AlertTarget,
    title: String,
    says: String,
    /// Unix seconds.
    at: i64,
    /// Not read on Beebeeb.
    unread: bool,
}

fn listed(list: NotificationList) -> Vec<Listed> {
    list.notifications.into_iter().filter_map(listed_one).collect()
}

fn listed_one(n: Notification) -> Option<Listed> {
    let (kind, target) = place_of(&n.notification_type)?;
    // A time that does not read cannot be placed under a day.
    let at = seconds_of(&n.created_at)?;
    Some(Listed {
        id: n.id,
        kind,
        target,
        title: n.title,
        says: n.body.unwrap_or_default(),
        at,
        unread: !n.read,
    })
}

/// A day's heading on this computer's clock: "Today", "Yesterday", "3 Nov", or "3 Nov 2025" in another year.
pub(crate) fn day_of(at: DateTime<Local>, now: DateTime<Local>) -> String {
    let (day, today) = (at.date_naive(), now.date_naive());
    if day == today {
        "Today".into()
    } else if today.pred_opt() == Some(day) {
        "Yesterday".into()
    } else if day.year() == today.year() {
        at.format("%-d %b").to_string()
    } else {
        at.format("%-d %b %Y").to_string()
    }
}

/// What was read, for one session of one account.
struct Held<T> {
    session: SessionGeneration,
    account: Option<String>,
    value: T,
}

enum Answer {
    /// The read with this number: the switches and the list, each with the session and account it was made for
    /// (`None`: the session moved on during the call).
    Read {
        number: u64,
        /// The count of account switch changes Beebeeb had taken when the read began.
        changes: u64,
        made_for: Option<(SessionGeneration, Option<String>)>,
        switches: Result<NotificationPreferenceValues, AccountCallError>,
        list: Result<Vec<Listed>, AccountCallError>,
    },
    /// A change of one account switch, for this session, and what Beebeeb holds after it, or why not.
    Switched {
        made_for: Option<(SessionGeneration, Option<String>)>,
        result: Result<NotificationPreferenceValues, AccountCallError>,
    },
    /// These notifications were marked seen for this session, or why not.
    Marked {
        made_for: Option<(SessionGeneration, Option<String>)>,
        ids: Vec<String>,
        result: Result<(), AccountCallError>,
    },
    /// A write of this PC's switch: the value it wrote, and whether the file took it.
    Pc { switch: PcSwitch, on: bool, saved: bool },
}

/// Whose account a call is for: the stored email, else the user id (as `super::account_of`).
fn account_of<C: CredentialStores>(engine: &Engine<C>) -> Option<String> {
    engine.account_email().or_else(|| engine.user_id())
}

/// The session and account a call on `engine` is made for, taken before it.
fn stamp<C: CredentialStores>(engine: &Engine<C>) -> (SessionGeneration, Option<String>) {
    (engine.session_turn(), account_of(engine))
}

/// The stamp, when it is still the engine's after the call: else `None` (the answer is dropped).
fn still<C: CredentialStores>(
    engine: &Engine<C>,
    before: (SessionGeneration, Option<String>),
) -> Option<(SessionGeneration, Option<String>)> {
    (stamp(engine) == before).then_some(before)
}

/// Run `work` on a worker named `name`; its answer (or `fallback` if it panicked) goes down `answers` and wakes the
/// window. `false`: no worker could start.
fn spawn<C: CredentialStores + 'static>(
    name: &str,
    engine: &Arc<Engine<C>>,
    answers: &mpsc::Sender<Answer>,
    work: impl FnOnce(&Engine<C>) -> Answer + Send + 'static,
    fallback: impl FnOnce() -> Answer + Send + 'static,
) -> bool {
    let (engine, answers) = (engine.clone(), answers.clone());
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let answer =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&engine))).unwrap_or_else(|_| fallback());
            let _ = answers.send(answer);
            engine.wake_host();
        })
        .is_ok()
}

/// The notification methods of the settings source. See the module's doc.
pub(super) struct NotificationsPart<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    identity: &'static EngineIdentity,
    /// This PC's switches as the pane shows them, and as the file holds them (by `PcSwitch::slot`).
    pc: [bool; 7],
    pc_saved: [bool; 7],
    /// Writes of each switch on their way.
    pc_saving: [usize; 7],
    /// The one writer of this PC's switches (started on the first change).
    pc_writer: Option<mpsc::Sender<(PcSwitch, bool)>>,
    switches: Option<Held<NotificationPreferenceValues>>,
    recent: Option<Held<Vec<Listed>>>,
    /// Notifications shown as new that the page has been left on, for the current session (a read that began
    /// before Beebeeb marked them may still say unread).
    seen: HashSet<String>,
    read_at: Option<Instant>,
    /// The pane opened last is Notifications or Recent notifications.
    showing: bool,
    /// The session the last read was asked for: a new session while the pane is showing is read once.
    asked_for: Option<SessionGeneration>,
    /// A read was asked for while one was on its way: start it when that one lands.
    again: bool,
    /// The read on its way, by number.
    reading: Option<u64>,
    reads: u64,
    /// An account switch change is on its way: no second one starts until it is answered.
    switching: bool,
    /// Counts the account switch changes Beebeeb took: switches read before one of them are stale when they land.
    changes: u64,
    /// Marks on their way.
    marking: usize,
    toast_next: Option<&'static str>,
    dirty: bool,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
}

impl<C: CredentialStores + 'static> NotificationsPart<C> {
    /// `config`: this PC's settings file as read when the window started (`None`: it could not be read; the
    /// switches are then the file's defaults, what a file without them reads as). Nothing is asked of Beebeeb
    /// until the pane opens.
    pub(super) fn new(
        engine: Arc<Engine<C>>,
        identity: &'static EngineIdentity,
        config: Option<&DesktopConfig>,
    ) -> Self {
        let defaults = DesktopConfig::default();
        let config = config.unwrap_or(&defaults);
        let pc = PcSwitch::ALL.map(|switch| switch.of(config));
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            identity,
            pc,
            pc_saved: pc,
            pc_saving: [0; 7],
            pc_writer: None,
            switches: None,
            recent: None,
            seen: HashSet::new(),
            read_at: None,
            showing: false,
            again: false,
            asked_for: None,
            reading: None,
            reads: 0,
            switching: false,
            changes: 0,
            marking: 0,
            toast_next: None,
            dirty: false,
            answers,
            inbox,
        }
    }

    /// Whether a stamp is the engine's session and account now.
    fn is_now(&self, session: SessionGeneration, account: &Option<String>) -> bool {
        self.engine.has_session() && session == self.engine.session_turn() && *account == account_of(&self.engine)
    }

    fn current<'a, T>(&self, held: &'a Option<Held<T>>) -> Option<&'a T> {
        held.as_ref()
            .filter(|held| self.is_now(held.session, &held.account))
            .map(|held| &held.value)
    }

    /// The account's notifications the pane can show, while they are the current session's, with "new" as the
    /// page shows it.
    fn shown(&self) -> Vec<(Listed, bool)> {
        self.current(&self.recent)
            .map(|list| {
                list.iter()
                    .map(|listed| (listed.clone(), listed.unread && !self.seen.contains(&listed.id)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A row's switch now. `None`: an account switch that is not read (or not the current session's).
    fn value_of(&self, stored: Stored) -> Option<bool> {
        match stored {
            Stored::Account(switch) => self.current(&self.switches).map(|values| switch.of(values)),
            Stored::Pc(switch) => Some(self.pc[switch.slot()]),
        }
    }

    /// The groups the pane draws now: every group of [`GROUPS`], always in its place, so a row never moves under
    /// the pointer when an answer lands. The account's group has no rows until its switches are read (never a
    /// guessed switch), and loses them again when a read fails or the session moves on.
    fn rows(&self) -> Vec<(&'static str, Vec<Alert>)> {
        GROUPS
            .iter()
            .map(|(title, rows)| {
                let rows = rows
                    .iter()
                    .filter_map(|&(name, says, stored)| {
                        self.value_of(stored).map(|on| Alert {
                            title: name.into(),
                            says: says.into(),
                            on,
                        })
                    })
                    .collect();
                (*title, rows)
            })
            .collect()
    }

    /// Ask Beebeeb for the switches and the list on a worker, unless a read is on its way.
    fn read(&mut self) {
        if !self.engine.has_session() {
            return;
        }
        if self.reading.is_some() {
            // What is on its way may have been answered before what asks now: read again when it lands.
            self.again = true;
            return;
        }
        self.reads += 1;
        self.asked_for = Some(self.engine.session_turn());
        let (number, changes) = (self.reads, self.changes);
        let started = spawn(
            "settings-notifications",
            &self.engine,
            &self.answers,
            move |engine| {
                let before = stamp(engine);
                let switches = engine.notification_preferences().map(|answer| answer.preferences);
                let list = engine.notifications().map(listed);
                Answer::Read {
                    number,
                    changes,
                    made_for: still(engine, before),
                    switches,
                    list,
                }
            },
            move || Answer::Read {
                number,
                changes,
                made_for: None,
                switches: Err(AccountCallError::Offline),
                list: Err(AccountCallError::Offline),
            },
        );
        if started {
            self.reading = Some(number);
        } else {
            log("settings: could not start a thread for the notifications");
            self.toast_next = Some(NOT_LOADED);
        }
    }

    /// The one writer of this PC's switches: each change in the order made, merge-only (the switch alone).
    fn pc_writer(&mut self) -> Option<&mpsc::Sender<(PcSwitch, bool)>> {
        if self.pc_writer.is_none() {
            let (wants, wanted) = mpsc::channel::<(PcSwitch, bool)>();
            let (identity, answers, engine) = (self.identity, self.answers.clone(), self.engine.clone());
            let started = std::thread::Builder::new()
                .name("settings-notify-switches".into())
                .spawn(move || {
                    // Ends when the part is dropped (its sender with it).
                    for (switch, on) in wanted {
                        let saved = DesktopConfig::update_for(identity, |config| {
                            let changed = switch.of(config) != on;
                            switch.set(config, on);
                            (changed, ())
                        })
                        .is_ok();
                        let _ = answers.send(Answer::Pc { switch, on, saved });
                        engine.wake_host();
                    }
                });
            if started.is_ok() {
                self.pc_writer = Some(wants);
            }
        }
        self.pc_writer.as_ref()
    }

    fn set_pc(&mut self, switch: PcSwitch, on: bool) {
        let slot = switch.slot();
        if self.pc[slot] == on {
            return;
        }
        let sent = self.pc_writer().is_some_and(|writer| writer.send((switch, on)).is_ok());
        if sent {
            self.pc[slot] = on;
            self.pc_saving[slot] += 1;
        } else {
            log("settings: a notification switch could not be saved (no writer)");
            self.toast_next = Some(SWITCH_NOT_SAVED);
        }
        self.dirty = true;
    }

    /// Send a change of one account switch, on a worker. One at a time: a change while one is on its way waits, and
    /// the toast says so.
    fn set_account(&mut self, switch: AccountSwitch, on: bool) {
        if self.switching {
            self.toast_next = Some(STILL_SAVING);
            return;
        }
        let update = switch.update(on);
        let started = spawn(
            "settings-notifications-set",
            &self.engine,
            &self.answers,
            move |engine| {
                let before = stamp(engine);
                let result = engine
                    .set_notification_preferences(&update)
                    .map(|answer| answer.preferences);
                Answer::Switched {
                    made_for: still(engine, before),
                    result,
                }
            },
            || Answer::Switched {
                made_for: None,
                result: Err(AccountCallError::Offline),
            },
        );
        self.switching = started;
        if !started {
            log("settings: could not start a thread for a notification setting");
            self.toast_next = Some(ALERT_NOT_SAVED);
        }
    }

    /// Drop everything of the account: its switches, its notifications, what was seen.
    fn forget(&mut self) {
        self.again = false;
        self.switches = None;
        self.recent = None;
        self.seen.clear();
        self.read_at = None;
        self.dirty = true;
    }

    fn take_in(&mut self, answer: Answer, toast: &mut Option<&'static str>) {
        match answer {
            Answer::Read {
                number,
                changes,
                made_for,
                switches,
                list,
            } => {
                if self.reading == Some(number) {
                    self.reading = None;
                }
                // Made for a session or an account that is not the current one: never drawn.
                let Some((session, account)) = made_for.filter(|(s, a)| self.is_now(*s, a)) else {
                    return;
                };
                self.read_at = Some(Instant::now());
                let mut failed = None;
                match switches {
                    // A change Beebeeb took after this read began is newer than what the read says.
                    Ok(_) if changes != self.changes => {}
                    Ok(value) => {
                        self.switches = Some(Held {
                            session,
                            account: account.clone(),
                            value,
                        })
                    }
                    Err(kind) => {
                        self.switches = None;
                        failed = Some(kind);
                    }
                }
                match list {
                    Ok(value) => {
                        // A notification Beebeeb now says is read needs no "seen" of its own any more.
                        self.seen
                            .retain(|id| value.iter().any(|listed| &listed.id == id && listed.unread));
                        self.recent = Some(Held {
                            session,
                            account,
                            value,
                        });
                    }
                    Err(kind) => {
                        self.recent = None;
                        failed = failed.or(Some(kind));
                    }
                }
                if let Some(kind) = failed {
                    self.read_at = None;
                    if !matches!(kind, AccountCallError::NotSignedIn | AccountCallError::SessionEnded) {
                        log(format!("settings: the notifications did not load ({kind})"));
                        *toast = Some(NOT_LOADED);
                    }
                }
            }
            Answer::Switched { made_for, result } => {
                self.switching = false;
                let mine = made_for.filter(|(s, a)| self.is_now(*s, a));
                match result {
                    Ok(value) => {
                        self.changes += 1;
                        if let Some((session, account)) = mine {
                            self.switches = Some(Held {
                                session,
                                account,
                                value,
                            });
                            *toast = Some(ALERT_SAVED);
                        }
                    }
                    Err(kind) => {
                        log(format!("settings: a notification setting was not changed ({kind})"));
                        if mine.is_some() {
                            match kind {
                                AccountCallError::ServerError | AccountCallError::SessionEnded => {
                                    *toast = Some(ALERT_NOT_SAVED);
                                }
                                AccountCallError::NotSignedIn => {}
                                _ => {
                                    // It may have landed: say the result is unknown and read what Beebeeb has.
                                    *toast = Some(ALERT_UNKNOWN);
                                    self.read();
                                }
                            }
                        }
                    }
                }
            }
            Answer::Marked { made_for, ids, result } => {
                self.marking = self.marking.saturating_sub(1);
                if !made_for.is_some_and(|(s, a)| self.is_now(s, &a)) {
                    return;
                }
                if let Err(kind) = result {
                    log(format!("settings: notifications were not marked seen ({kind})"));
                    // The engine stops at the first that fails, so the ones before it ARE marked on Beebeeb. "New"
                    // comes back on all of them until a read (asked now) says which ones Beebeeb still holds unread.
                    for id in &ids {
                        self.seen.remove(id);
                    }
                    self.read();
                    if !matches!(kind, AccountCallError::NotSignedIn | AccountCallError::SessionEnded) {
                        *toast = Some(NOT_MARKED);
                    }
                }
            }
            Answer::Pc { switch, on, saved } => {
                let slot = switch.slot();
                self.pc_saving[slot] = self.pc_saving[slot].saturating_sub(1);
                if saved {
                    self.pc_saved[slot] = on;
                } else if self.pc_saving[slot] == 0 {
                    // The last write of this switch failed: back to what the file holds.
                    self.pc[slot] = self.pc_saved[slot];
                    *toast = Some(SWITCH_NOT_SAVED);
                }
            }
        }
    }

    /// Whether the panes this part answers for draw only what it has read (they do).
    pub(super) fn answers_for(pane: Pane) -> bool {
        matches!(pane, Pane::Notifications | Pane::Recent)
    }

    /// The window shows `pane` now (any pane: another one means this part's panes are not showing).
    pub(super) fn opened(&mut self, pane: Pane) {
        self.showing = Self::answers_for(pane);
        if !self.showing {
            return;
        }
        let held = self.current(&self.switches).is_some() && self.current(&self.recent).is_some();
        let fresh = held && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
        }
    }

    /// The window's poll: drop what belongs to a session that ended, take in what the workers answered.
    /// `(changed, toast, busy)`.
    pub(super) fn pump(&mut self) -> (bool, Option<&'static str>, bool) {
        let mut changed = std::mem::take(&mut self.dirty);
        let mut toast = self.toast_next.take();
        let stale = |held: Option<(SessionGeneration, &Option<String>)>| {
            held.is_some_and(|(session, account)| !self.is_now(session, account))
        };
        if stale(self.switches.as_ref().map(|h| (h.session, &h.account)))
            || stale(self.recent.as_ref().map(|h| (h.session, &h.account)))
        {
            self.forget();
            changed = true;
        }
        while let Ok(answer) = self.inbox.try_recv() {
            self.take_in(answer, &mut toast);
            changed = true;
        }
        if self.again && self.reading.is_none() {
            self.again = false;
            self.read();
        }
        // Another sign-in while the pane shows: its own values are read once, without a reopening.
        if self.showing
            && self.reading.is_none()
            && self.engine.has_session()
            && self.asked_for != Some(self.engine.session_turn())
        {
            self.read();
        }
        changed |= toast.is_some();
        let busy =
            self.reading.is_some() || self.switching || self.marking > 0 || self.pc_saving.iter().any(|&n| n > 0);
        (changed, toast, busy)
    }

    pub(super) fn recent_alerts(&self) -> Vec<(String, Vec<RecentAlert>)> {
        let now = Local::now();
        let mut days: Vec<(String, Vec<RecentAlert>)> = Vec::new();
        for (listed, new) in self.shown() {
            let Some(at) = Local.timestamp_opt(listed.at, 0).single() else {
                continue;
            };
            let day = day_of(at, now);
            let alert = RecentAlert {
                kind: listed.kind,
                title: listed.title,
                says: listed.says,
                time: at.format("%H:%M").to_string(),
                new,
                leads_to: listed.target,
            };
            match days.last_mut() {
                Some((last, alerts)) if *last == day => alerts.push(alert),
                _ => days.push((day, vec![alert])),
            }
        }
        days
    }

    pub(super) fn notifications(&self) -> Notifications {
        Notifications {
            // Not read: see the module's doc.
            allowed_in_windows: true,
            allow: self.pc[PcSwitch::Allow.slot()],
            new_recent: self.shown().iter().filter(|(_, new)| *new).count(),
            groups: self
                .rows()
                .into_iter()
                .map(|(title, rows)| (title.to_string(), rows))
                .collect(),
        }
    }

    pub(super) fn set_allow_notifications(&mut self, on: bool) {
        self.set_pc(PcSwitch::Allow, on);
    }

    /// `group`, `index` as in `notifications().groups`. The row is found by its fixed place in [`GROUPS`] (every
    /// group is drawn in its place, and a group's rows are all there or none), never by what sits at that position
    /// now: a press can only change the switch that was drawn there. An account switch that is not read is not
    /// changed.
    pub(super) fn set_alert(&mut self, group: usize, index: usize, on: bool) {
        let Some(&(_, _, stored)) = GROUPS.get(group).and_then(|(_, rows)| rows.get(index)) else {
            return;
        };
        if self.value_of(stored).is_none_or(|now| now == on) {
            return;
        }
        match stored {
            Stored::Account(switch) => self.set_account(switch, on),
            Stored::Pc(switch) => self.set_pc(switch, on),
        }
    }

    /// The page was left: what it showed as new is seen. Sent once, on a worker; the page stops saying "New" at
    /// once; a refusal is a toast and a read, after which only what Beebeeb still holds unread says "New" again.
    pub(super) fn mark_recent_seen(&mut self) {
        let ids: Vec<String> = self
            .shown()
            .into_iter()
            .filter(|(_, new)| *new)
            .map(|(listed, _)| listed.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        let sent = ids.clone();
        let started = spawn(
            "settings-notifications-seen",
            &self.engine,
            &self.answers,
            move |engine| {
                let before = stamp(engine);
                let result = engine.mark_notifications_read(&sent);
                Answer::Marked {
                    made_for: still(engine, before),
                    ids: sent,
                    result,
                }
            },
            || Answer::Marked {
                made_for: None,
                ids: Vec::new(),
                result: Err(AccountCallError::Offline),
            },
        );
        if started {
            self.marking += 1;
            self.seen.extend(ids);
            self.dirty = true;
        } else {
            log("settings: could not start a thread to mark notifications seen");
            self.toast_next = Some(NOT_MARKED);
        }
    }
}

#[cfg(test)]
impl<C: CredentialStores + 'static> NotificationsPart<C> {
    /// What it holds is old: the next opening reads again.
    pub(super) fn expire(&mut self) {
        self.read_at = None;
    }
}
