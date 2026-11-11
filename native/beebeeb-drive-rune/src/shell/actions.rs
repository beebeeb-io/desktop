//! What the shell and the frames may ask for, as traits, so nothing here links the engine or
//! the session yet.
//!
//! - [`EngineActions`]: the engine's and the session's side. The engine lane implements it;
//!   until then [`FixtureEngine`] stands in (in memory, nothing leaves the process).
//! - [`ShellActions`]: what a frame, the tray menu and the flyout call. [`HostActions`]
//!   implements it from an [`EngineActions`] and the [`Platform`].

use std::sync::{Arc, Mutex};

use super::app::GateState;
use super::instance::ProfileHolder;
use super::platform::{Platform, PlatformError};
use super::tray::{EngineState, TrayInputs};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionError {
    /// The gate is closed, or the state does not allow it (Explorer before Beebeeb is in it).
    NotAvailable(&'static str),
    /// This platform's half is not built yet.
    NotBuilt(&'static str),
    Failed(String),
}

impl From<PlatformError> for ActionError {
    fn from(error: PlatformError) -> ActionError {
        match error {
            PlatformError::NotBuilt(what) => ActionError::NotBuilt(what),
            PlatformError::Os(message) => ActionError::Failed(message),
        }
    }
}

/// The engine and the session, as the shell needs them.
pub trait EngineActions: Send + Sync {
    fn gate_state(&self) -> GateState;
    /// What the b shows. Cheap: called on every refresh.
    fn tray_inputs(&self) -> TrayInputs;
    fn pause_sync(&self) -> Result<(), ActionError>;
    fn resume_sync(&self) -> Result<(), ActionError>;
    /// Opens File Explorer at the Beebeeb location (DW-1180).
    fn open_in_file_explorer(&self) -> Result<(), ActionError>;
    /// Another agent holding this profile's sync lock (`lockfile.rs`), if one is alive.
    fn profile_holder(&self) -> Option<ProfileHolder>;
}

/// Everything a frame, the tray menu or the flyout may ask the shell to do beyond navigation.
pub trait ShellActions: Send + Sync {
    fn gate_state(&self) -> GateState;
    fn tray_inputs(&self) -> TrayInputs;
    fn pause_sync(&self) -> Result<(), ActionError>;
    fn resume_sync(&self) -> Result<(), ActionError>;
    fn open_in_file_explorer(&self) -> Result<(), ActionError>;
    /// "Open Beebeeb when you log in" (DW-0281, Settings).
    fn autostart_enabled(&self) -> bool;
    fn set_autostart(&self, enabled: bool) -> Result<(), ActionError>;
}

/// The real [`ShellActions`]: the engine for what is the engine's, the OS for the rest.
pub struct HostActions {
    engine: Arc<dyn EngineActions>,
    platform: Arc<dyn Platform>,
}

impl HostActions {
    pub fn new(engine: Arc<dyn EngineActions>, platform: Arc<dyn Platform>) -> HostActions {
        HostActions { engine, platform }
    }
}

impl ShellActions for HostActions {
    fn gate_state(&self) -> GateState {
        self.engine.gate_state()
    }
    fn tray_inputs(&self) -> TrayInputs {
        self.engine.tray_inputs()
    }
    fn pause_sync(&self) -> Result<(), ActionError> {
        self.engine.pause_sync()
    }
    fn resume_sync(&self) -> Result<(), ActionError> {
        self.engine.resume_sync()
    }
    fn open_in_file_explorer(&self) -> Result<(), ActionError> {
        self.engine.open_in_file_explorer()
    }
    fn autostart_enabled(&self) -> bool {
        // Unreadable reads as off: the switch then shows what a toggle would change.
        self.platform.autostart_enabled().unwrap_or(false)
    }
    fn set_autostart(&self, enabled: bool) -> Result<(), ActionError> {
        Ok(self.platform.set_autostart(enabled)?)
    }
}

/// What a [`FixtureEngine`] was asked to do, for tests.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixtureCall {
    Pause,
    Resume,
    OpenInFileExplorer,
}

struct Fixture {
    gate: GateState,
    inputs: TrayInputs,
    holder: Option<ProfileHolder>,
    calls: Vec<FixtureCall>,
}

/// An engine that is not there: a signed-in, up-to-date account in memory. Pause and resume
/// flip its own flag; nothing syncs, nothing is read from disk, nothing leaves the process.
pub struct FixtureEngine {
    state: Mutex<Fixture>,
}

impl FixtureEngine {
    /// Signed in, unlocked, in File Explorer, up to date.
    pub fn ready() -> FixtureEngine {
        FixtureEngine::with(
            GateState::Ready,
            TrayInputs {
                signed_in: true,
                explorer_ready: true,
                engine: EngineState::Idle,
                ..TrayInputs::default()
            },
        )
    }

    pub fn signed_out() -> FixtureEngine {
        FixtureEngine::with(GateState::SignedOut, TrayInputs::default())
    }

    pub fn with(gate: GateState, inputs: TrayInputs) -> FixtureEngine {
        FixtureEngine {
            state: Mutex::new(Fixture {
                gate,
                inputs,
                holder: None,
                calls: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Fixture> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn set_gate(&self, gate: GateState) {
        let mut state = self.lock();
        state.gate = gate;
        state.inputs.signed_in = gate != GateState::SignedOut;
        state.inputs.locked = gate == GateState::Locked;
    }

    pub fn set_inputs(&self, inputs: TrayInputs) {
        self.lock().inputs = inputs;
    }

    pub fn set_profile_holder(&self, holder: Option<ProfileHolder>) {
        self.lock().holder = holder;
    }

    pub fn calls(&self) -> Vec<FixtureCall> {
        self.lock().calls.clone()
    }
}

impl EngineActions for FixtureEngine {
    fn gate_state(&self) -> GateState {
        self.lock().gate
    }
    fn tray_inputs(&self) -> TrayInputs {
        self.lock().inputs.clone()
    }
    fn pause_sync(&self) -> Result<(), ActionError> {
        let mut state = self.lock();
        if !state.inputs.signed_in {
            return Err(ActionError::NotAvailable("signed out"));
        }
        state.inputs.paused = true;
        state.calls.push(FixtureCall::Pause);
        Ok(())
    }
    fn resume_sync(&self) -> Result<(), ActionError> {
        let mut state = self.lock();
        if !state.inputs.signed_in {
            return Err(ActionError::NotAvailable("signed out"));
        }
        state.inputs.paused = false;
        state.calls.push(FixtureCall::Resume);
        Ok(())
    }
    fn open_in_file_explorer(&self) -> Result<(), ActionError> {
        let mut state = self.lock();
        if !(state.inputs.signed_in && state.inputs.explorer_ready) {
            return Err(ActionError::NotAvailable("Beebeeb is not in File Explorer"));
        }
        // The fixture has no folder to open; it records that it was asked.
        state.calls.push(FixtureCall::OpenInFileExplorer);
        Ok(())
    }
    fn profile_holder(&self) -> Option<ProfileHolder> {
        self.lock().holder.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::super::platform::fake::FakePlatform;
    use super::*;

    fn host() -> (Arc<FixtureEngine>, Arc<FakePlatform>, HostActions) {
        let engine = Arc::new(FixtureEngine::ready());
        let platform = Arc::new(FakePlatform::default());
        let host = HostActions::new(engine.clone(), platform.clone());
        (engine, platform, host)
    }

    #[test]
    fn pause_and_resume_reach_the_engine_and_show_in_the_tray_inputs() {
        let (engine, _, host) = host();
        assert!(!host.tray_inputs().paused);
        host.pause_sync().unwrap();
        assert!(host.tray_inputs().paused);
        host.resume_sync().unwrap();
        assert!(!host.tray_inputs().paused);
        assert_eq!(engine.calls(), vec![FixtureCall::Pause, FixtureCall::Resume]);
    }

    #[test]
    fn engine_actions_are_refused_while_signed_out() {
        let engine = Arc::new(FixtureEngine::signed_out());
        let host = HostActions::new(engine.clone(), Arc::new(FakePlatform::default()));
        assert_eq!(host.gate_state(), GateState::SignedOut);
        assert!(matches!(host.pause_sync(), Err(ActionError::NotAvailable(_))));
        assert!(matches!(host.resume_sync(), Err(ActionError::NotAvailable(_))));
        assert!(matches!(
            host.open_in_file_explorer(),
            Err(ActionError::NotAvailable(_))
        ));
        assert!(engine.calls().is_empty());
    }

    #[test]
    fn explorer_needs_beebeeb_to_be_in_file_explorer() {
        let (engine, _, host) = host();
        host.open_in_file_explorer().unwrap();
        engine.set_inputs(TrayInputs {
            signed_in: true,
            explorer_ready: false,
            ..TrayInputs::default()
        });
        assert!(host.open_in_file_explorer().is_err());
        assert_eq!(engine.calls(), vec![FixtureCall::OpenInFileExplorer]);
    }

    #[test]
    fn the_autostart_toggle_goes_to_the_platform_not_the_engine() {
        let (engine, platform, host) = host();
        assert!(!host.autostart_enabled());
        host.set_autostart(true).unwrap();
        assert!(host.autostart_enabled());
        host.set_autostart(false).unwrap();
        assert_eq!(platform.autostart_writes(), vec![true, false]);
        assert!(engine.calls().is_empty());
    }

    #[test]
    fn a_platform_without_autostart_says_not_built() {
        let host = HostActions::new(Arc::new(FixtureEngine::ready()), Arc::new(FakePlatform::unbuilt()));
        assert!(matches!(host.set_autostart(true), Err(ActionError::NotBuilt(_))));
        assert!(!host.autostart_enabled());
    }

    #[test]
    fn the_gate_follows_the_session() {
        let (engine, _, host) = host();
        engine.set_gate(GateState::Locked);
        assert_eq!(host.gate_state(), GateState::Locked);
        assert!(host.tray_inputs().locked);
        engine.set_gate(GateState::SignedOut);
        assert!(!host.tray_inputs().signed_in);
        assert_eq!(host.gate_state(), GateState::SignedOut);
    }
}
