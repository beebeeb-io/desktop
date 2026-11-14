//! Explorer's "Free up space" command and the engine handle (task 2490). The command asks the handle served here
//! ([`Engine::serve_explorer_command`]) whether the local state is the signed-in account's, and frees through the
//! handle's own reclaim ([`Engine::free_up_space`], the account-bound handle). Every stop, lock and sign-out removes
//! the binding Explorer finds the command by, for the folder an engine started in and for the configured one; a later
//! engine start binds it again when it connects the folder.
//!
//! Neither answer takes the transition lock or waits on the handle's runtime, so Explorer is never held by a sign-in
//! step or a sign-out. Must not: log a name or a path.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::explorer_command::{self, ExplorerBinding, OsExplorerBinding, ReclaimOwner};
use crate::identity::EngineIdentity;
use crate::sync_view::Explorer;

/// Why Repair or Turn off changed nothing. No variant carries a path, a name or an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplorerChangeRefused {
    /// The session of the press is not the one in memory.
    NotSignedIn,
    /// No sync folder is set.
    NoFolder,
    /// Changes made on this PC have not uploaded yet: turning off would lose them.
    ChangesWait,
    /// The local state could not say whether changes wait.
    CannotTellChanges,
    /// Windows or the start refused; nothing needs undoing.
    Failed,
}

impl ExplorerChangeRefused {
    /// A fixed sentence for the window.
    pub fn words(self) -> &'static str {
        match self {
            Self::NotSignedIn => "Unlock Beebeeb to change this.",
            Self::NoFolder => "Beebeeb has no sync folder yet.",
            Self::ChangesWait => "Changes on this PC have not uploaded yet. Wait for them, then turn it off.",
            Self::CannotTellChanges => "Couldn't check for changes that have not uploaded yet. Try again.",
            Self::Failed => "Couldn't change Beebeeb in File Explorer. Nothing was changed that you need to undo.",
        }
    }
}

/// What the handle keeps for Explorer's command: the binding it removes, and the folder an engine started in.
pub(super) struct ExplorerSeat {
    binding: Arc<dyn ExplorerBinding>,
    started_in: Option<PathBuf>,
    #[cfg(any(test, feature = "test-support"))]
    sync_root_for_test: Option<PathBuf>,
}

impl ExplorerSeat {
    pub(super) fn new(identity: &'static EngineIdentity) -> Self {
        ExplorerSeat {
            binding: Arc::new(OsExplorerBinding::new(identity)),
            started_in: None,
            #[cfg(any(test, feature = "test-support"))]
            sync_root_for_test: None,
        }
    }
}

impl<C: CredentialStores> ReclaimOwner for Engine<C> {
    fn refusal(&self) -> Option<LocalStateRefused> {
        self.local_state_refusal()
    }

    fn free_up_space(&self) -> Result<u64, String> {
        Engine::free_up_space(self)
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Explorer's "Free up space" command asks this handle from now on (Windows; elsewhere nothing asks). A host
    /// calls it once, after [`Engine::open`]. The command keeps no engine alive: once this handle is dropped, it is
    /// Disabled. Without it, the command is Disabled.
    pub fn serve_explorer_command(self: &Arc<Self>) {
        let owner: Weak<dyn ReclaimOwner> = Arc::downgrade(self) as Weak<dyn ReclaimOwner>;
        explorer_command::serve(owner);
    }

    /// The folder an engine was started in: its binding is removed at the stop.
    pub(super) fn note_explorer_root(&self, sync_root: &Path) {
        self.seat().started_in = Some(sync_root.to_path_buf());
    }

    /// Remove the command's binding for the folder an engine started in and for the configured folder. The files,
    /// the placeholders and the folder's registration stay.
    pub(super) fn remove_explorer_command(&self) {
        let (binding, mut roots) = {
            let mut seat = self.seat();
            (
                seat.binding.clone(),
                seat.started_in.take().into_iter().collect::<Vec<_>>(),
            )
        };
        if let Ok(Some(root)) = self.configured_sync_root()
            && !roots.contains(&root)
        {
            roots.push(root);
        }
        for root in &roots {
            binding.remove(root);
        }
    }

    /// Ask Windows again for the File Explorer entry of the folder an engine runs in, and say what it answered.
    /// Never stops, starts, registers the Cloud Files root or touches a file: it asks nothing unless an engine runs
    /// in a known folder (`false`). BLOCKS (the transition lock): a worker's, never the UI thread's.
    pub fn add_to_file_explorer_again(&self) -> bool {
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
        #[cfg(target_os = "windows")]
        {
            let explorer = self.runtime.block_on(async {
                let _step = self.transition.lock().await;
                let running = self
                    .acct
                    .engine
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(crate::runner::EngineRunner::is_running);
                let started_in = self.seat().started_in.clone();
                entry_after(running, started_in.as_deref(), |root| {
                    crate::windows_cf::register_shell_sync_root(self.identity, root)
                        .map_err(|error| crate::sync_root_port::hresult_of(&error))
                })
            });
            let Some(explorer) = explorer else {
                return false;
            };
            self.view.write(|view| {
                if view.start == crate::sync_view::Start::Running {
                    view.explorer = explorer;
                }
            });
            self.publish();
            true
        }
    }

    /// The sync folder the settings name (`None`: none is set).
    pub(super) fn configured_sync_root(&self) -> Result<Option<PathBuf>, String> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(root) = self.seat().sync_root_for_test.clone() {
            return Ok(Some(root));
        }
        Ok(crate::config::DesktopConfig::load_for(self.identity)?.sync_root)
    }

    /// Test builds only: the binding removed at a stop is `binding`, not the OS's.
    #[cfg(any(test, feature = "test-support"))]
    pub fn use_explorer_binding_for_test(&self, binding: Arc<dyn ExplorerBinding>) {
        self.seat().binding = binding;
    }

    /// Test builds only: `root` is the configured sync folder, so a test does not write the shared settings.
    #[cfg(any(test, feature = "test-support"))]
    pub fn use_sync_root_for_test(&self, root: PathBuf) {
        self.seat().sync_root_for_test = Some(root);
    }

    /// Settings > Sync > "Repair" (T2-E1): sync is stopped and started again into the configured folder, which is the
    /// registration path Setup's "Try again" asks too (only a start asks Windows). Nothing is changed unless the
    /// session in memory is still `session`, the one of the press.
    pub fn repair_explorer_for(&self, session: SessionGeneration) -> Result<(), ExplorerChangeRefused> {
        let root = self.explorer_change_root(session)?;
        self.stop();
        match self.start_sync(root) {
            Ok(super::SyncStart::Started | super::SyncStart::AlreadyRunning) => Ok(()),
            Ok(super::SyncStart::NoSession) => Err(ExplorerChangeRefused::NotSignedIn),
            Err(_) => Err(ExplorerChangeRefused::Failed),
        }
    }

    /// Settings > Sync > "Turn off Beebeeb in File Explorer" (T2-E1): refused while changes wait to upload; else sync
    /// is stopped and the folder's registration removed (Windows; the placeholders go with it, the files that are
    /// downloaded stay). Nothing is changed unless the session in memory is still `session`.
    pub fn turn_off_explorer_for(&self, session: SessionGeneration) -> Result<(), ExplorerChangeRefused> {
        self.turn_off_explorer_with(session, &|root| self.unregister_in_os(root))
    }

    /// [`Self::turn_off_explorer_for`] with the registration removal given, so a test can make each answer on any OS.
    pub(crate) fn turn_off_explorer_with(
        &self,
        session: SessionGeneration,
        unregister: &dyn Fn(&Path) -> Result<(), ()>,
    ) -> Result<(), ExplorerChangeRefused> {
        let root = self.explorer_change_root(session)?;
        // Changes that have not left this PC are lost with the registration's placeholders: refused, not asked.
        let waiting = self
            .local_state()
            .ok()
            .and_then(|state| state.with(|db| db.queue_hold_summary().ok()).ok().flatten())
            .ok_or(ExplorerChangeRefused::CannotTellChanges)?;
        if waiting.waiting + waiting.gave_up + waiting.held.iter().map(|g| g.count).sum::<i64>() > 0 {
            return Err(ExplorerChangeRefused::ChangesWait);
        }
        self.stop();
        // The session may have moved on while sync stopped.
        if !self.acct.session_unchanged_since(session) {
            return Err(ExplorerChangeRefused::NotSignedIn);
        }
        if unregister(&root).is_err() {
            // Nothing was removed: sync starts again, as it was.
            let _ = self.start_sync(root);
            return Err(ExplorerChangeRefused::Failed);
        }
        Ok(())
    }

    /// The configured folder, once the session of the press is the one in memory.
    fn explorer_change_root(&self, session: SessionGeneration) -> Result<PathBuf, ExplorerChangeRefused> {
        if !self.acct.session_unchanged_since(session) {
            return Err(ExplorerChangeRefused::NotSignedIn);
        }
        match self.configured_sync_root() {
            Ok(Some(root)) => Ok(root),
            _ => Err(ExplorerChangeRefused::NoFolder),
        }
    }

    #[cfg(target_os = "windows")]
    fn unregister_in_os(&self, root: &Path) -> Result<(), ()> {
        crate::windows_cf::unregister_sync_root(self.identity, root).map_err(|_| ())
    }

    /// Only Windows has a registration to remove.
    #[cfg(not(target_os = "windows"))]
    fn unregister_in_os(&self, _root: &Path) -> Result<(), ()> {
        Err(())
    }

    fn seat(&self) -> std::sync::MutexGuard<'_, ExplorerSeat> {
        self.explorer.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// What Windows answered when asked again: nothing is asked unless an engine `running` in a known folder.
/// A function of its inputs, so the rule is tested on every OS.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn entry_after(
    running: bool,
    started_in: Option<&Path>,
    register: impl FnOnce(&Path) -> Result<(), Option<i32>>,
) -> Option<Explorer> {
    if !running {
        return None;
    }
    let root = started_in?;
    Some(match register(root) {
        Ok(()) => Explorer::Added,
        Err(code) => Explorer::Failed { code },
    })
}

#[cfg(test)]
#[path = "explorer_api_tests.rs"]
mod tests;
