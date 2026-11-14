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
use crate::explorer_command::{self, ExplorerBinding, OsExplorerBinding, ReclaimOwner};
use crate::identity::EngineIdentity;
use crate::sync_view::Explorer;

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
