//! The first run's sync folder (the sync-start contract's ruling Q2, §5.1 and slice S2): the question the File
//! Explorer step asks while the settings name no folder, and the start worker's part ([`start_now`]).
//!
//! The rule is the engine's ([`SetupEngine::check_folder`]): this file asks it for the folder the person kept or
//! picked, and only a folder it lets through is handed to a start. Nothing is created, registered or written
//! before that start. The folder is written to the settings by the start that worked, once, and only when the
//! settings name none; a refused start writes nothing, so another folder can still be chosen.
//!
//! Order, for a new PC: no start while the first run is due and no folder was chosen; the choice asks for one
//! start; the start worker uses the settings' folder, else the folder chosen under the session that starts, else
//! (no first run due) the suggestion, as before.
//!
//! Must not: log a path. The folder is shown to the person who chose it, in the step, and nowhere else.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use beebeeb_engine::app::SyncStart;
use beebeeb_engine::sync_folder_rule::SyncFolderRefused;

use super::{Answer, EngineSetupSource, FirstRun, SetupEngine, log};
use crate::frames::signin::setup::source::{FolderAsk, FolderSaid};

/// The picker's title.
pub const PICKER_TITLE: &str = "Choose Beebeeb's folder";

/// What a worker found out about the sync folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The settings name none: ask, suggesting this folder.
    Ask(String),
    /// The settings name the folder, or could not be read (the start says so itself): nothing to ask.
    Settled,
    /// A folder was chosen and one start was asked for.
    Chosen,
    /// The folder was not taken.
    Refused(FolderSaid),
    /// The picker was closed without a folder.
    Closed,
}

/// The engine's refusal, as the step says it.
pub fn said_of(refused: SyncFolderRefused) -> FolderSaid {
    match refused {
        SyncFolderRefused::NotAFullPath => FolderSaid::NotAFullPath,
        SyncFolderRefused::Network => FolderSaid::Network,
        SyncFolderRefused::System => FolderSaid::System,
        SyncFolderRefused::Home => FolderSaid::Home,
        SyncFolderRefused::OwnData => FolderSaid::OwnData,
        SyncFolderRefused::NotAFolder => FolderSaid::NotAFolder,
        SyncFolderRefused::Unreadable => FolderSaid::Unreadable,
        SyncFolderRefused::AnotherApps => FolderSaid::AnotherApps,
    }
}

/// Start sync, as the shell's one start worker does it (contract §4.3, §5.1). The folder: the one the settings
/// name; else the one chosen in the first run under the session that is starting; else, when no first run is
/// due, the identity's suggestion. While a first run is due and no folder was chosen, nothing is started. The
/// folder of a start that worked is written to the settings when they name none. `name` keeps the folder's name
/// for the refusal modal. Blocks: a `start-sync` worker's. The log names a refusal's kind only.
pub fn start_now<E: SetupEngine>(engine: &E, name: &Mutex<Option<String>>, first_run: &FirstRun) {
    let (folder, named) = match engine.settings_folder() {
        Err(_) => return log("sync was not started: the settings could not be read"),
        Ok(Some(named)) => (named, true),
        Ok(None) => match first_run.chosen_for(&engine.stamp()) {
            Some(chosen) => (chosen, false),
            None if first_run.due() => return log("sync was not started: the first run has no folder yet"),
            None => (engine.suggested_folder(), false),
        },
    };
    *name.lock().unwrap_or_else(PoisonError::into_inner) =
        folder.file_name().map(|name| name.to_string_lossy().into_owned());
    match engine.start(folder.clone()) {
        Ok(SyncStart::Started) => {
            log("sync started");
            if !named && engine.keep_folder(&folder).is_err() {
                log("the sync folder could not be written to the settings");
            }
        }
        Ok(SyncStart::AlreadyRunning) => {}
        Ok(SyncStart::NoSession) => log("sync was not started: no session"),
        Err(refused) => log(format!("sync was not started: {}", refused.kind())),
    }
}

/// On a worker: `folder` is the sync folder of this first run if the engine's rule lets it be; then one start
/// is asked for.
fn decide<E: SetupEngine>(
    engine: &E,
    first_run: &FirstRun,
    stamp: super::Stamp,
    folder: PathBuf,
    start: &(dyn Fn() -> bool + Send + Sync),
) -> Found {
    if let Err(refused) = engine.check_folder(&folder) {
        log(format!("first run: the folder was not taken ({})", refused.kind()));
        return Found::Refused(said_of(refused));
    }
    first_run.choose(stamp, folder);
    if !start() {
        log("first run: a sync start could not be asked for");
    }
    Found::Chosen
}

impl<E: SetupEngine> EngineSetupSource<E> {
    /// As the first run begins: whether the folder has to be asked for. Until the settings were read the step
    /// holds its question back (`busy`), so nothing reads as "adding" before it is known.
    pub(super) fn read_folder(&mut self) {
        if !self.hooks.explorer_step {
            return;
        }
        self.folder = FolderAsk::Ask {
            folder: String::new(),
            said: None,
            busy: true,
        };
        let first_run = self.hooks.first_run.clone();
        let stamp = self.made_for.clone();
        self.ask(
            "first-run-folder",
            move |engine, _| {
                Answer::Folder(match engine.settings_folder() {
                    Ok(None) if first_run.chosen_for(&stamp).is_none() => {
                        Found::Ask(engine.suggested_folder().display().to_string())
                    }
                    Ok(_) => Found::Settled,
                    Err(refused) => {
                        log(format!("first run: the settings were not read ({})", refused.kind()));
                        Found::Settled
                    }
                })
            },
            Some(Answer::Folder(Found::Settled)),
        );
    }

    /// Whether a choice can be made now: the question is up and no other choice is on its way.
    fn asking(&self) -> bool {
        matches!(self.folder, FolderAsk::Ask { busy: false, .. })
    }

    fn set_busy(&mut self, now: bool) {
        if let FolderAsk::Ask { busy, .. } = &mut self.folder {
            *busy = now;
            self.changed = true;
        }
    }

    /// "Use this folder": the suggestion.
    pub(super) fn keep_suggested(&mut self) {
        if !self.asking() || !self.mine() {
            return;
        }
        self.set_busy(true);
        let (first_run, start, stamp) = (
            self.hooks.first_run.clone(),
            self.hooks.start_sync.clone(),
            self.made_for.clone(),
        );
        self.ask(
            "first-run-folder",
            move |engine, _| {
                let folder = engine.suggested_folder();
                Answer::Folder(decide(engine, &first_run, stamp, folder, &*start))
            },
            Some(Answer::Folder(Found::Refused(FolderSaid::Unreadable))),
        );
    }

    /// "Choose another folder": Windows' picker, then the rule. Also after a refused start, while no folder
    /// was kept.
    pub(super) fn pick_another(&mut self) {
        use crate::frames::signin::setup::source::SetupSource;
        if !(self.asking() || self.can_choose_again()) || !self.mine() {
            return;
        }
        self.set_busy(true);
        let (first_run, start, stamp, platform) = (
            self.hooks.first_run.clone(),
            self.hooks.start_sync.clone(),
            self.made_for.clone(),
            self.hooks.platform.clone(),
        );
        self.ask(
            "first-run-folder",
            move |engine, _| {
                let shown = first_run
                    .chosen_for(&stamp)
                    .unwrap_or_else(|| engine.suggested_folder());
                Answer::Folder(match platform.pick_folder(PICKER_TITLE, &shown) {
                    Ok(Some(folder)) => decide(engine, &first_run, stamp, folder, &*start),
                    Ok(None) => Found::Closed,
                    Err(error) => {
                        // The platform's own words: no path is in them.
                        log(format!("first run: the folder picker did not open ({error})"));
                        Found::Refused(FolderSaid::NoPicker)
                    }
                })
            },
            Some(Answer::Folder(Found::Refused(FolderSaid::NoPicker))),
        );
    }

    /// Take in what a worker found out (the poll's, only for this first run's session and account).
    pub(super) fn take_in_folder(&mut self, found: Found) {
        match found {
            Found::Ask(folder) => {
                self.asked_folder = true;
                self.suggested = folder.clone();
                self.folder = FolderAsk::Ask {
                    folder,
                    said: None,
                    busy: false,
                };
            }
            Found::Settled => self.folder = FolderAsk::Settled,
            Found::Chosen => {
                self.folder = FolderAsk::Settled;
                // A start was asked for now: the view may read "not asked" for a moment longer.
                self.look_after_the_grace();
            }
            // The question comes back with the reason, whether it was up or the step showed a refused start.
            Found::Refused(said) => {
                self.folder = FolderAsk::Ask {
                    folder: self.suggested.clone(),
                    said: Some(said),
                    busy: false,
                };
            }
            Found::Closed => self.set_busy(false),
        }
    }
}
