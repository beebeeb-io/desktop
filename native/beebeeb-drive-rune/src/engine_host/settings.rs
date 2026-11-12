//! Settings on the real engine (task 2473): [`EngineSettingsSource`], the `SettingsSource` of the running app.
//!
//! What it answers, and from where:
//! - General (DW-0701) and its Theme menu (DW-0787): the theme from this PC's settings file, read when the window
//!   starts (the first paint is local) and written merge-only; where new files are stored from Beebeeb
//!   ([`Engine::storage_region`]); the version this binary was built as. No updater runs in this app yet, so
//!   General never says a newer version exists.
//! - General › Storage location (DW-0788, b, c, d): the account's choice and the list, by country, from Beebeeb. A
//!   choice and "only store in this country" are sent once ([`Engine::set_storage_region`], the one call here that
//!   writes account state) and shown only after Beebeeb took them; a refusal leaves the old choice and says so.
//! - Not wired here (the shell's "not built yet" frame in their place): About and the release notes (no updater
//!   yet), Notifications and Recent notifications (what Windows allows for this app is not read yet, so the pane
//!   could not say when nothing arrives), Report a problem (no support bundle is built here yet).
//!
//! Thread rule (task 2470 W5): nothing here waits for Beebeeb or for the settings file's write lock on the window's
//! thread. Each call and each write runs on a worker thread of its own and sends its answer down one channel, which
//! [`SettingsSource::pump`] drains (`frames::settings::poll`); a read is not started while one is under way, and a
//! write is not started while one is.
//!
//! Must not: log a name, a path, a token or a server's words; show a city; draw a value it has not read.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account_dto::{StorageChoice, UserRegionResponse};
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::config::{DesktopConfig, DesktopTheme};
use beebeeb_engine::identity::EngineIdentity;

use super::log;
use crate::frames::settings::Pane;
use crate::frames::settings::source::{
    About, Channel, General, Location, Locations, Notifications, Pumped, RecentAlert, SettingsSource, ThemeChoice,
    Unwired, Update,
};

#[cfg(test)]
mod tests;

/// A list read this recently is not asked for again when its pane opens.
const FRESH_FOR: Duration = Duration::from_secs(30);

/// The id the account's default carries in Beebeeb's list (`account_dto::region`).
const DEFAULT_ID: &str = "europe";

/// What the toasts say. A refusal says that nothing changed: the old choice stands.
pub const CHOSEN: &str = "Storage location updated";
pub const NOT_CHOSEN: &str = "Couldn't change the storage location. Nothing changed.";
pub const THEME_NOT_SAVED: &str = "Couldn't save the theme on this PC.";

/// Settings on the real engine. See the module's doc for what each pane reads.
pub struct EngineSettingsSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    identity: &'static EngineIdentity,
    theme: ThemeChoice,
    region: Region,
    /// When the region was last answered.
    read_at: Option<Instant>,
    reading: bool,
    /// A change of the storage location is on its way: no second one starts until it is answered.
    writing: bool,
    /// Theme writes on their way (each writes only the theme).
    saving: usize,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The panes this source does not answer for.
    rest: Unwired,
}

/// What Beebeeb last said about where new files go.
enum Region {
    /// Not answered yet.
    Unknown,
    Failed(AccountCallError),
    Known(UserRegionResponse),
}

/// What a worker sends back.
enum Answer {
    Region(Result<UserRegionResponse, AccountCallError>),
    /// A change of the storage location: what Beebeeb stored, or why not.
    Chosen(Result<(Option<String>, bool), AccountCallError>),
    /// A theme write: the theme it wrote, the one before it, and whether the file took it.
    Theme {
        wanted: ThemeChoice,
        before: ThemeChoice,
        saved: bool,
    },
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
fn place_of(choice: &StorageChoice) -> Option<Location> {
    let live = choice.status == "live" && choice.selectable;
    match choice.kind.as_str() {
        "region" if choice.id == DEFAULT_ID => Some(Location {
            name: "Default".into(),
            says: Some("Europe: Beebeeb places each new file in the EU".into()),
            live,
            default: true,
        }),
        "country" => Some(Location {
            name: choice.name.clone(),
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
        if let Some(place) = place_of(choice) {
            choices.push(place);
            ids.push(choice.id.clone());
        }
    }
    let wanted = region.preferred_region.as_deref().unwrap_or(DEFAULT_ID);
    let chosen = match ids.iter().position(|id| id == wanted) {
        Some(index) => index,
        // No choice stored: placed as the default.
        None if region.preferred_region.is_none() => choices.iter().position(|c| c.default)?,
        // A choice this list does not have: never a guess at what it was.
        None => return None,
    };
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

impl<C: CredentialStores> EngineSettingsSource<C> {
    /// The source of the running app. Reads this PC's settings file now (local and small: the first paint is
    /// local) and asks Beebeeb for the storage location on a worker.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self::for_identity(engine, crate::shell::identity::engine_identity())
    }

    pub(crate) fn for_identity(engine: Arc<Engine<C>>, identity: &'static EngineIdentity) -> Self {
        let theme = match DesktopConfig::load_for(identity) {
            Ok(config) => theme_of(config.theme),
            Err(_) => {
                log("settings: the settings file could not be read; the theme follows Windows");
                ThemeChoice::System
            }
        };
        let (answers, inbox) = mpsc::channel();
        let mut source = Self {
            engine,
            identity,
            theme,
            region: Region::Unknown,
            read_at: None,
            reading: false,
            writing: false,
            saving: 0,
            answers,
            inbox,
            rest: Unwired,
        };
        source.read_region();
        source
    }

    /// Ask Beebeeb for the storage location, on a worker, unless a read is under way.
    fn read_region(&mut self) {
        if self.reading {
            return;
        }
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let started = std::thread::Builder::new()
            .name("settings-region".into())
            .spawn(move || {
                let _ = answers.send(Answer::Region(engine.storage_region()));
            });
        self.reading = started.is_ok();
        if started.is_err() {
            log("settings: could not start a thread for the storage location");
        }
    }

    /// Send a change of the storage location, on a worker. `false`: one is on its way already, or no thread.
    fn send_choice(&mut self, id: String, strict: bool) -> bool {
        if self.writing {
            return false;
        }
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let started = std::thread::Builder::new()
            .name("settings-region-set".into())
            .spawn(move || {
                let stored = engine
                    .set_storage_region(Some(&id), Some(strict))
                    .map(|answer| (answer.preferred_region, answer.region_strict));
                let _ = answers.send(Answer::Chosen(stored));
            });
        self.writing = started.is_ok();
        self.writing
    }

    fn known(&self) -> Option<&UserRegionResponse> {
        match &self.region {
            Region::Known(region) => Some(region),
            _ => None,
        }
    }

    /// The panes of this source that draw only what it has read.
    fn answers_for(pane: Pane) -> bool {
        matches!(pane, Pane::General | Pane::StorageLocation)
    }
}

impl<C: CredentialStores> SettingsSource for EngineSettingsSource<C> {
    fn wired(&self, pane: Pane) -> bool {
        Self::answers_for(pane)
    }

    /// Report a problem (DW-0782) is a dialog over General with no support bundle behind it yet.
    fn opens(&self, frame: &str) -> bool {
        frame != "DW-0782"
    }

    fn opened(&mut self, pane: Pane) {
        let fresh = self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR) && self.known().is_some();
        if Self::answers_for(pane) && !fresh {
            self.read_region();
        }
    }

    fn pump(&mut self) -> Pumped {
        let mut pumped = Pumped::default();
        while let Ok(answer) = self.inbox.try_recv() {
            pumped.changed = true;
            match answer {
                Answer::Region(result) => {
                    self.reading = false;
                    self.read_at = Some(Instant::now());
                    self.region = match result {
                        Ok(region) => Region::Known(region),
                        Err(kind) => {
                            log(format!("settings: the storage location did not load ({kind})"));
                            Region::Failed(kind)
                        }
                    };
                }
                Answer::Chosen(result) => {
                    self.writing = false;
                    match (result, &mut self.region) {
                        (Ok((preferred, strict)), Region::Known(region)) => {
                            region.preferred_region = preferred;
                            region.region_strict = strict;
                            pumped.toast = Some(CHOSEN);
                        }
                        (Ok(_), _) => pumped.toast = Some(CHOSEN),
                        (Err(kind), _) => {
                            log(format!("settings: the storage location was not changed ({kind})"));
                            pumped.toast = Some(NOT_CHOSEN);
                        }
                    }
                }
                Answer::Theme { wanted, before, saved } => {
                    self.saving = self.saving.saturating_sub(1);
                    // Put back only when nothing newer was chosen meanwhile.
                    if !saved && self.theme == wanted {
                        self.theme = before;
                        pumped.toast = Some(THEME_NOT_SAVED);
                    }
                }
            }
        }
        pumped.busy = self.reading || self.writing || self.saving > 0;
        pumped
    }

    fn locations_loading(&self) -> bool {
        self.reading && self.known().is_none()
    }

    fn mark_recent_seen(&mut self) {
        self.rest.mark_recent_seen();
    }
    fn check_for_updates(&mut self) {
        self.rest.check_for_updates();
    }
    fn recent_alerts(&self) -> Vec<(String, Vec<RecentAlert>)> {
        self.rest.recent_alerts()
    }
    fn notifications(&self) -> Notifications {
        self.rest.notifications()
    }
    fn set_allow_notifications(&mut self, on: bool) {
        self.rest.set_allow_notifications(on);
    }
    fn set_alert(&mut self, group: usize, index: usize, on: bool) {
        self.rest.set_alert(group, index, on);
    }

    fn locations(&self) -> Option<Locations> {
        self.known().and_then(listed).map(|(locations, _)| locations)
    }

    /// Sent once; the pane shows the new choice, and the toast says so, only after Beebeeb took it. Never a place
    /// that is not live. `false` always: nothing is taken yet when this returns.
    fn choose_location(&mut self, index: usize) -> bool {
        let Some((locations, ids)) = self.known().and_then(listed) else {
            return false;
        };
        let Some(place) = locations.choices.get(index).filter(|place| place.live) else {
            return false;
        };
        if index == locations.chosen && !locations.unavailable {
            return false;
        }
        let strict = !place.default && locations.only_here;
        self.send_choice(ids[index].clone(), strict);
        false
    }

    fn set_only_here(&mut self, on: bool) {
        let Some((locations, ids)) = self.known().and_then(listed) else {
            return;
        };
        if locations.choices[locations.chosen].default || locations.only_here == on {
            return;
        }
        let id = ids[locations.chosen].clone();
        self.send_choice(id, on);
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
            .known()
            .and_then(listed)
            .and_then(|(locations, _)| {
                let chosen = locations.chosen()?;
                Some(if chosen.default {
                    "Europe".to_string()
                } else {
                    chosen.name.clone()
                })
            })
            .unwrap_or_default();
        General {
            theme: self.theme,
            storage_location,
            version: version_words(),
            update: Update::Current,
        }
    }

    /// Applies at once (every window reads it on its next frame); written to this PC's settings file on a worker,
    /// merge-only. A write that fails puts the theme back and says so.
    fn set_theme(&mut self, theme: ThemeChoice) {
        if theme == self.theme {
            return;
        }
        let before = std::mem::replace(&mut self.theme, theme);
        let (identity, answers) = (self.identity, self.answers.clone());
        let started = std::thread::Builder::new()
            .name("settings-theme".into())
            .spawn(move || {
                let saved = DesktopConfig::update_for(identity, |config| {
                    let wanted = desktop_theme(theme);
                    let changed = config.theme != wanted;
                    config.theme = wanted;
                    (changed, ())
                })
                .is_ok();
                let _ = answers.send(Answer::Theme {
                    wanted: theme,
                    before,
                    saved,
                });
            });
        match started {
            Ok(_) => self.saving += 1,
            Err(_) => self.theme = before,
        }
    }
}
