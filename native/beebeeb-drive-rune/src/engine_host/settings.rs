//! Settings on the real engine (task 2473): [`EngineSettingsSource`], the `SettingsSource` of the running app.
//!
//! What it answers, and from where:
//! - General (DW-0701) and its Theme menu (DW-0787): the theme from this PC's settings file, read when the window
//!   starts (the first paint is local) and written merge-only; where new files are stored from Beebeeb
//!   ([`Engine::storage_region`]); the version this binary was built as. No updater runs in this app yet, so
//!   General never says a newer version exists.
//! - General › Storage location (DW-0788, b, c, d): the account's choice and the list, by country, from Beebeeb. A
//!   choice and "only store in this country" are sent once ([`Engine::set_storage_region_for`], the one call here
//!   that writes account state) and shown only after Beebeeb took them; a refusal leaves the old choice and says so.
//! - Notifications (DW-0750) and Recent notifications (DW-0791): [`notifications`], its own part (the account's
//!   alert switches and notifications from Beebeeb, this PC's switches from its settings file).
//! - Not wired here (the shell's "not built yet" frame in their place): About and the release notes (no updater
//!   yet), Report a problem (no support bundle is built here yet).
//!
//! Thread rule (task 2470 W5): nothing here waits for Beebeeb or for the settings file's write lock on the window's
//! thread. Each call and each write runs on a worker thread of its own and sends its answer down one channel, which
//! [`SettingsSource::pump`] drains (`frames::settings::poll`); a read is not started while one is under way, and a
//! write is not started while one is.
//!
//! Session rule: a read and a change each carry the session ([`Engine::session_turn`]) and the account they were
//! asked under, taken on the window's thread at the press (never on the worker). The engine sends a change only
//! under that very session; an answer that lands under another session or account is never drawn.
//!
//! Must not: log a name, a path, a token or a server's words; show a city; draw a value it has not read.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::account_dto::UserRegionResponse;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::config::{DesktopConfig, DesktopTheme};
use beebeeb_engine::identity::EngineIdentity;

use super::log;
use crate::frames::settings::Pane;
use crate::frames::settings::source::{
    About, Channel, General, Location, Locations, Notifications, Pumped, ReadState, RecentAlert, SettingsSource,
    ThemeChoice, Unwired, Update,
};

mod notifications;
#[cfg(test)]
mod tests;

use notifications::NotificationsPart;

/// A list read this recently is not asked for again when its pane opens.
const FRESH_FOR: Duration = Duration::from_secs(30);

/// The id the account's default carries in Beebeeb's list (`account_dto::region`).
const DEFAULT_ID: &str = "europe";

/// What the toasts say. `NOT_CHOSEN` may be said ONLY after an HTTP refusal (a status code the
/// server returned): the old choice stands. `UNKNOWN_CHANGE` is for a timeout, a transport error
/// mid-flight, or a 2xx with an unreadable body: the change may have landed.
pub const CHOSEN: &str = "Storage location updated";
pub const NOT_CHOSEN: &str = "Couldn't change the storage location. Nothing changed.";
pub const UNKNOWN_CHANGE: &str = "Couldn't confirm the change. Checking what the server has.";
pub const THEME_NOT_SAVED: &str = "Couldn't save the theme on this PC.";

/// What the "still saving" toast says: a change waits for the one before it.
pub const STILL_SAVING: &str = "Still saving the last change. Try again in a moment.";

/// Settings on the real engine. See the module's doc for what each pane reads.
pub struct EngineSettingsSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    identity: &'static EngineIdentity,
    /// The theme the windows draw in, and the one this PC's settings file holds.
    theme: ThemeChoice,
    saved_theme: ThemeChoice,
    /// The one writer of the theme, in the order the person chose (started on the first change).
    theme_writer: Option<mpsc::Sender<ThemeChoice>>,
    /// Theme writes on their way.
    saving: usize,
    region: Region,
    /// Whose account `region` is (`account_of`): another account never shows it.
    region_of: Option<String>,
    /// The list as the pane draws it and each row's id, worked out once per answer.
    listing: Option<(Locations, Vec<String>)>,
    /// When the region was last answered.
    read_at: Option<Instant>,
    reading: bool,
    /// A read is wanted once the one under way has landed (what it says may be older than a change).
    read_again: bool,
    /// A change of the storage location is on its way: no second one starts until it is answered.
    writing: bool,
    /// Counts the changes Beebeeb took: a read that began before one of them is stale when it lands.
    generation: u64,
    /// A toast for the next poll (a change that has to wait).
    toast_next: Option<&'static str>,
    /// Tests only: the next change waits here, on its worker, before it calls the engine.
    #[cfg(test)]
    hold_next_write: Option<mpsc::Receiver<()>>,
    version: String,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// Notifications and Recent notifications.
    alerts: NotificationsPart<C>,
    /// The panes this source does not answer for.
    rest: Unwired,
}

/// What Beebeeb last said about where new files go.
enum Region {
    /// Not answered yet, or the last read failed (its kind is logged when it arrives).
    Unknown,
    Known(UserRegionResponse),
}

/// The session and the account something was asked under.
type Stamp = (SessionGeneration, Option<String>);

/// What a worker sends back.
enum Answer {
    /// A read of the storage location.
    Region {
        /// The session and account it was asked under.
        asked: Stamp,
        /// Whether they were still the engine's when the call ended.
        held: bool,
        /// The generation the read began in.
        generation: u64,
        result: Result<UserRegionResponse, AccountCallError>,
    },
    /// A change of the storage location.
    Chosen {
        /// The session and account of the press.
        pressed: Stamp,
        /// Whether they were still the engine's when the call ended.
        held: bool,
        /// What Beebeeb stored, or why not.
        result: Result<(Option<String>, bool), AccountCallError>,
    },
    /// A theme write: the theme it wrote, and whether the file took it.
    Theme { wanted: ThemeChoice, saved: bool },
}

fn theme_of(theme: DesktopTheme) -> ThemeChoice {
    match theme {
        DesktopTheme::System => ThemeChoice::System,
        DesktopTheme::Light => ThemeChoice::Light,
        DesktopTheme::Dark => ThemeChoice::Dark,
    }
}

fn desktop_theme(theme: ThemeChoice) -> DesktopTheme {
    match theme {
        ThemeChoice::System => DesktopTheme::System,
        ThemeChoice::Light => DesktopTheme::Light,
        ThemeChoice::Dark => DesktopTheme::Dark,
    }
}

/// The version this binary was built as: a release's number, else a test build named by its commit.
pub fn version_words() -> String {
    match option_env!("BEEBEEB_RELEASE_VERSION").filter(|v| !v.is_empty()) {
        Some(version) => version.to_string(),
        None => match option_env!("BEEBEEB_BUILD_COMMIT") {
            Some(commit) if !commit.is_empty() => format!("test build {}", commit.get(..7).unwrap_or(commit)),
            _ => "test build".to_string(),
        },
    }
}

/// One row of Beebeeb's list as a place a pane may show: the default, or a country. Anything else (an older
/// server's row per data center) is left out, so a city can never reach a pane.
fn place_of(id: &str, kind: &str, name: &str, status: &str, selectable: bool) -> Option<Location> {
    let live = status == "live" && selectable;
    match kind {
        "region" if id == DEFAULT_ID => Some(Location {
            name: "Default".into(),
            says: Some("Europe: Beebeeb places each new file in the EU".into()),
            live,
            default: true,
        }),
        "country" => Some(Location {
            name: name.to_string(),
            says: None,
            live,
            default: false,
        }),
        _ => None,
    }
}

/// The list as the pane draws it, and each row's id. `None`: no answer, or an answer without a place to choose.
fn listed(region: &UserRegionResponse) -> Option<(Locations, Vec<String>)> {
    let mut choices = Vec::new();
    let mut ids = Vec::new();
    for choice in &region.choices {
        if let Some(place) = place_of(
            &choice.id,
            &choice.kind,
            &choice.name,
            &choice.status,
            choice.selectable,
        ) {
            choices.push(place);
            ids.push(choice.id.clone());
        }
    }
    let wanted = region.preferred_region.as_deref().unwrap_or(DEFAULT_ID);
    // A choice this list does not have: never a guess at what it was.
    let chosen = ids.iter().position(|id| id == wanted)?;
    let unavailable = !choices[chosen].default && !choices[chosen].live;
    Some((
        Locations {
            choices,
            chosen,
            only_here: region.region_strict && wanted != DEFAULT_ID,
            unavailable,
        },
        ids,
    ))
}

/// Whose account a read is for: the stored email, else the user id the server named (one of them, the same one for
/// one account in one run, as `LiveEngine::session` names it). In memory only: never waits.
fn account_of<C: CredentialStores>(engine: &Engine<C>) -> Option<String> {
    engine.account_email().or_else(|| engine.user_id())
}

/// The session and account a call on `engine` is made under. Taken on the window's thread, before a worker starts.
fn stamp<C: CredentialStores>(engine: &Engine<C>) -> Stamp {
    (engine.session_turn(), account_of(engine))
}

/// Whether a stamp is the engine's session and account now, with a session in memory.
fn is_now<C: CredentialStores>(engine: &Engine<C>, stamp: &Stamp) -> bool {
    engine.has_session() && stamp.1.is_some() && *stamp == self::stamp(engine)
}

impl<C: CredentialStores + 'static> EngineSettingsSource<C> {
    /// The source of the running app. Reads this PC's settings file now (local and small: the first paint is
    /// local) and, when someone is signed in, asks Beebeeb for the storage location on a worker.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self::for_identity(engine, crate::shell::identity::engine_identity())
    }

    pub(crate) fn for_identity(engine: Arc<Engine<C>>, identity: &'static EngineIdentity) -> Self {
        let config = DesktopConfig::load_for(identity).ok();
        let theme = match &config {
            Some(config) => theme_of(config.theme),
            None => {
                log("settings: the settings file could not be read; the theme follows Windows");
                ThemeChoice::System
            }
        };
        let alerts = NotificationsPart::new(engine.clone(), identity, config.as_ref());
        let (answers, inbox) = mpsc::channel();
        let mut source = Self {
            engine,
            identity,
            theme,
            saved_theme: theme,
            theme_writer: None,
            saving: 0,
            region: Region::Unknown,
            region_of: None,
            listing: None,
            read_at: None,
            reading: false,
            read_again: false,
            writing: false,
            generation: 0,
            toast_next: None,
            #[cfg(test)]
            hold_next_write: None,
            version: version_words(),
            answers,
            inbox,
            alerts,
            rest: Unwired,
        };
        if account_of(&source.engine).is_some() {
            source.read_region();
        }
        source
    }

    /// Ask Beebeeb for the storage location, on a worker, unless a read is under way. The answer is taken only
    /// while the session is the one of this moment (no session change between here and the answer, so the token
    /// the engine used was this session's).
    fn read_region(&mut self) {
        if self.reading {
            return;
        }
        let asked = stamp(&self.engine);
        let (engine, answers, generation) = (self.engine.clone(), self.answers.clone(), self.generation);
        let started = std::thread::Builder::new()
            .name("settings-region".into())
            .spawn(move || {
                // A panic in this worker must not leave `reading` set for good: send an error.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let answer = engine.storage_region();
                    (is_now(&engine, &asked), answer)
                }));
                let (held, result) = result.unwrap_or((false, Err(AccountCallError::Offline)));
                let _ = answers.send(Answer::Region {
                    asked,
                    held,
                    generation,
                    result,
                });
            });
        self.reading = started.is_ok();
        if started.is_err() {
            log("settings: could not start a thread for the storage location");
        }
    }

    /// Send a change of the storage location, on a worker. One at a time: a change while one is on its way waits,
    /// and the toast says so. `pressed` is the session and account of the press, read on the window's thread
    /// BEFORE the list the choice came from was looked at: the engine sends nothing under another session.
    fn send_choice(&mut self, pressed: Stamp, id: String, strict: bool) {
        if self.writing {
            self.toast_next = Some(STILL_SAVING);
            return;
        }
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        #[cfg(test)]
        let hold = self.hold_next_write.take();
        let started = std::thread::Builder::new()
            .name("settings-region-set".into())
            .spawn(move || {
                // A panic in this worker must not leave `writing` set for good: send an error.
                #[cfg(test)]
                if let Some(hold) = hold {
                    let _ = hold.recv();
                }
                let (session, at_press) = (pressed.0, pressed.clone());
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let stored = engine
                        .set_storage_region_for(session, Some(&id), Some(strict))
                        .map(|answer| (answer.preferred_region, answer.region_strict));
                    (is_now(&engine, &at_press), stored)
                }));
                let (held, result) = result.unwrap_or((false, Err(AccountCallError::Offline)));
                let _ = answers.send(Answer::Chosen { pressed, held, result });
            });
        self.writing = started.is_ok();
        if started.is_err() {
            log("settings: could not start a thread for a change of the storage location");
            self.toast_next = Some(NOT_CHOSEN);
        }
    }

    /// The list and its ids, while what was read is the signed-in account's and its session is in memory (a locked
    /// vault shows no list, so nothing can be chosen from one).
    fn listing(&self) -> Option<&(Locations, Vec<String>)> {
        let mine = self.engine.has_session() && self.region_of.is_some() && self.region_of == account_of(&self.engine);
        self.listing.as_ref().filter(|_| mine)
    }

    fn relist(&mut self) {
        self.listing = match &self.region {
            Region::Known(region) => listed(region),
            Region::Unknown => None,
        };
    }

    /// The panes of this source that draw only what it has read.
    fn answers_for(pane: Pane) -> bool {
        matches!(pane, Pane::General | Pane::StorageLocation) || NotificationsPart::<C>::answers_for(pane)
    }

    /// The one writer of the theme: each change in the order chosen, merge-only (the theme alone).
    fn theme_writer(&mut self) -> Option<&mpsc::Sender<ThemeChoice>> {
        if self.theme_writer.is_none() {
            let (wants, wanted) = mpsc::channel::<ThemeChoice>();
            let (identity, answers) = (self.identity, self.answers.clone());
            let started = std::thread::Builder::new()
                .name("settings-theme".into())
                .spawn(move || {
                    // Ends when the source is dropped (its sender with it).
                    for theme in wanted {
                        let saved = DesktopConfig::update_for(identity, |config| {
                            let wanted = desktop_theme(theme);
                            let changed = config.theme != wanted;
                            config.theme = wanted;
                            (changed, ())
                        })
                        .is_ok();
                        let _ = answers.send(Answer::Theme { wanted: theme, saved });
                    }
                });
            if started.is_ok() {
                self.theme_writer = Some(wants);
            }
        }
        self.theme_writer.as_ref()
    }
}

impl<C: CredentialStores + 'static> SettingsSource for EngineSettingsSource<C> {
    fn wired(&self, pane: Pane) -> bool {
        Self::answers_for(pane)
    }

    /// Report a problem (DW-0782) is a dialog over General with no support bundle behind it yet.
    fn opens(&self, frame: &str) -> bool {
        frame != "DW-0782"
    }

    fn opened(&mut self, pane: Pane) {
        self.alerts.opened(pane);
        if NotificationsPart::<C>::answers_for(pane) {
            return;
        }
        let fresh = self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR) && self.listing().is_some();
        if Self::answers_for(pane) && !fresh {
            self.read_region();
        }
    }

    fn pump(&mut self) -> Pumped {
        let mut pumped = Pumped {
            toast: self.toast_next.take(),
            ..Pumped::default()
        };
        pumped.changed = pumped.toast.is_some();
        while let Ok(answer) = self.inbox.try_recv() {
            pumped.changed = true;
            match answer {
                Answer::Region {
                    asked,
                    held,
                    generation,
                    result,
                } => {
                    self.reading = false;
                    let again = std::mem::take(&mut self.read_again);
                    // A change Beebeeb took after this read began is newer than what the read says.
                    if generation != self.generation {
                        if again {
                            self.read_region();
                        }
                        continue;
                    }
                    // Asked under a session or an account that is not the current one: never drawn, and not
                    // counted as a read. Whoever is here now is asked for afresh.
                    if !(held && is_now(&self.engine, &asked)) {
                        self.region = Region::Unknown;
                        self.region_of = None;
                        self.read_at = None;
                        self.relist();
                        // Only when the session really moved on since (so at most once per session change).
                        if self.engine.has_session() && stamp(&self.engine) != asked {
                            self.read_region();
                        }
                        continue;
                    }
                    let account = asked.1;
                    match result {
                        Ok(region) => {
                            self.read_at = Some(Instant::now());
                            self.region = Region::Known(region);
                            self.region_of = account;
                        }
                        Err(kind) => {
                            if kind != AccountCallError::NotSignedIn {
                                log(format!("settings: the storage location did not load ({kind})"));
                            }
                            // A read that failed is not a read: the next opening asks again.
                            self.read_at = None;
                            self.region = Region::Unknown;
                            self.region_of = None;
                        }
                    }
                    self.relist();
                }
                Answer::Chosen { pressed, held, result } => {
                    self.writing = false;
                    // Drawn only for the session AND the account of the press, and only onto that account's list.
                    let mine = held && is_now(&self.engine, &pressed) && pressed.1 == self.region_of;
                    match (result, &mut self.region) {
                        (Ok((preferred, strict)), Region::Known(region)) if mine => {
                            region.preferred_region = preferred;
                            region.region_strict = strict;
                            self.generation += 1;
                            self.relist();
                            pumped.toast = Some(CHOSEN);
                        }
                        // Taken, and the answer is not for the session shown now: it is never drawn. When the account
                        // that pressed is the one here again (a lock and an unlock in between), what Beebeeb now
                        // holds is read afresh, so the pane does not keep an older answer.
                        (Ok(_), _) => {
                            self.generation += 1;
                            if self.engine.has_session() && pressed.1.is_some() && pressed.1 == account_of(&self.engine)
                            {
                                self.read_again = self.reading;
                                self.read_region();
                            }
                        }
                        // The engine sent nothing: the session of the press is gone (a lock, a sign-out, another
                        // sign-in). Said only to the account that pressed, when it is the one here again; under
                        // another account, or signed out, nothing is said and nothing changes.
                        (Err(AccountCallError::NotSignedIn), _) => {
                            log("settings: the storage location was not changed (the session of the press is gone)");
                            let same_person = self.engine.has_session()
                                && pressed.1.is_some()
                                && pressed.1 == account_of(&self.engine);
                            if same_person {
                                pumped.toast = Some(NOT_CHOSEN);
                            }
                        }
                        (Err(kind), _) => {
                            log(format!("settings: the storage location was not changed ({kind})"));
                            if mine {
                                // "Nothing changed" only after an HTTP refusal (the server answered
                                // and said no). A timeout, a transport error mid-flight, or an
                                // unreadable 2xx: the change may have landed, so say the result is
                                // unknown and re-read what the server has (P2-1, task 2473).
                                match kind {
                                    AccountCallError::ServerError | AccountCallError::SessionEnded => {
                                        pumped.toast = Some(NOT_CHOSEN);
                                    }
                                    _ => {
                                        pumped.toast = Some(UNKNOWN_CHANGE);
                                        self.read_region();
                                    }
                                }
                            }
                        }
                    }
                }
                Answer::Theme { wanted, saved } => {
                    self.saving = self.saving.saturating_sub(1);
                    if saved {
                        self.saved_theme = wanted;
                    } else if self.saving == 0 {
                        // The last write failed: back to what the file holds.
                        self.theme = self.saved_theme;
                        pumped.toast = Some(THEME_NOT_SAVED);
                    }
                }
            }
        }
        let (changed, toast, busy) = self.alerts.pump();
        pumped.changed |= changed;
        pumped.toast = pumped.toast.or(toast);
        pumped.busy = self.reading || self.writing || self.saving > 0 || busy;
        pumped
    }

    fn locations_loading(&self) -> bool {
        self.reading && self.listing().is_none()
    }

    fn theme(&self) -> ThemeChoice {
        self.theme
    }

    fn mark_recent_seen(&mut self) {
        self.alerts.mark_recent_seen();
    }
    fn check_for_updates(&mut self) {
        self.rest.check_for_updates();
    }
    fn recent_alerts(&self) -> Vec<(String, Vec<RecentAlert>)> {
        self.alerts.recent_alerts()
    }
    fn notifications(&self) -> Notifications {
        self.alerts.notifications()
    }
    fn set_allow_notifications(&mut self, on: bool) {
        self.alerts.set_allow_notifications(on);
    }
    fn set_alert(&mut self, group: usize, index: usize, on: bool) {
        self.alerts.set_alert(group, index, on);
    }
    fn recent_state(&self) -> ReadState {
        self.alerts.recent_state()
    }
    fn alerts_state(&self, group: usize) -> ReadState {
        self.alerts.alerts_state(group)
    }
    fn reload_notifications(&mut self) {
        self.alerts.reload();
    }

    fn locations(&self) -> Option<Locations> {
        self.listing().map(|(locations, _)| locations.clone())
    }

    /// Sent once; the pane shows the new choice, and the toast says so, only after Beebeeb took it. Never a place
    /// that is not live. `false` always: nothing is taken yet when this returns.
    fn choose_location(&mut self, index: usize) -> bool {
        // The session of the press, before the list is looked at (see `send_choice`).
        let pressed = stamp(&self.engine);
        let Some((locations, ids)) = self.listing() else {
            return false;
        };
        let Some(place) = locations.choices.get(index).filter(|place| place.live) else {
            return false;
        };
        if index == locations.chosen {
            return false;
        }
        let (id, strict) = (ids[index].clone(), !place.default && locations.only_here);
        self.send_choice(pressed, id, strict);
        false
    }

    fn set_only_here(&mut self, on: bool) {
        let pressed = stamp(&self.engine);
        let Some((locations, ids)) = self.listing() else {
            return;
        };
        if locations.choices[locations.chosen].default || locations.only_here == on {
            return;
        }
        let id = ids[locations.chosen].clone();
        self.send_choice(pressed, id, on);
    }

    fn reload_locations(&mut self) {
        self.read_region();
    }

    fn about(&self) -> About {
        self.rest.about()
    }
    fn set_channel(&mut self, channel: Channel) {
        self.rest.set_channel(channel);
    }

    fn general(&self) -> General {
        let storage_location = self
            .listing()
            .and_then(|(locations, _)| locations.chosen())
            .map(|chosen| {
                if chosen.default {
                    "Europe".to_string()
                } else {
                    chosen.name.clone()
                }
            })
            .unwrap_or_default();
        General {
            theme: self.theme,
            storage_location,
            version: self.version.clone(),
            update: Update::Current,
        }
    }

    /// Applies at once (every window reads it on its next frame); written to this PC's settings file by the one
    /// theme writer, in the order chosen, merge-only. A last write that fails puts back what the file holds, and
    /// the toast says so.
    fn set_theme(&mut self, theme: ThemeChoice) {
        if theme == self.theme {
            return;
        }
        let sent = self.theme_writer().is_some_and(|writer| writer.send(theme).is_ok());
        if sent {
            self.theme = theme;
            self.saving += 1;
        } else {
            log("settings: the theme could not be saved (no writer)");
            self.toast_next = Some(THEME_NOT_SAVED);
        }
    }
}

#[cfg(test)]
impl<C: CredentialStores + 'static> EngineSettingsSource<C> {
    /// The next change of the storage location waits on its worker, before it calls the engine, until the returned
    /// sender is sent to or dropped.
    pub(super) fn hold_next_write(&mut self) -> mpsc::Sender<()> {
        let (release, held) = mpsc::channel();
        self.hold_next_write = Some(held);
        release
    }
}
