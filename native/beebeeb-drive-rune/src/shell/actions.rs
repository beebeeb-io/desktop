//! What the shell and the frames may ask for, as traits, so nothing here links the engine or
//! the session yet.
//!
//! - [`EngineActions`]: the engine's and the session's side. The engine lane implements it;
//!   until then [`FixtureEngine`] stands in (in memory, nothing leaves the process).
//! - [`ShellActions`]: what a frame, the tray menu and the flyout call. [`HostActions`]
//!   implements it from an [`EngineActions`] and the [`Platform`].

use std::sync::{Arc, Mutex};

use super::app::{AccountId, GateState, Session};
use super::instance::ProfileHolder;
use super::platform::{Platform, PlatformError};
use super::tray::{EngineState, TrayInputs};
use crate::frames::global::sync_off::SyncOff;

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
            PlatformError::NotOurs(what) => ActionError::NotAvailable(what),
            PlatformError::Os(message) => ActionError::Failed(message),
        }
    }
}

/// The engine and the session, as the shell needs them.
pub trait EngineActions: Send + Sync {
    /// The gate AND whose session it is (the user id), read together. The shell samples this;
    /// the account is what lets it see a change of account it never saw a sign-out for.
    fn session(&self) -> Session;
    /// What the b shows. Cheap: called on every refresh.
    fn tray_inputs(&self) -> TrayInputs;
    fn pause_sync(&self) -> Result<(), ActionError>;
    fn resume_sync(&self) -> Result<(), ActionError>;
    /// Opens File Explorer at the Beebeeb location (DW-1180).
    fn open_in_file_explorer(&self) -> Result<(), ActionError>;
    /// Another agent holding this profile's sync lock (`lockfile.rs`), if one is alive: a
    /// Tauri 0.8.x app of the same identity, or `bb watch`.
    fn profile_holder(&self) -> Option<ProfileHolder>;
    /// Starts syncing. Only ever called through [`ShellActions::start_sync`], which refuses
    /// while [`EngineActions::profile_holder`] names another agent.
    fn start_sync(&self) -> Result<(), ActionError>;
    /// The app is quitting (the b's menu, the installer's `quit-for-update`, a Windows
    /// shutdown): stop the engine and park uploads. Must return promptly and be safe to call
    /// twice.
    fn stop_for_quit(&self);
    /// A Windows shutdown that ran [`EngineActions::stop_for_quit`] was cancelled: carry on.
    fn resume_after_cancelled_quit(&self);
    /// A scene function sets the gate on the engine so that `refresh()` reads it back. Only the
    /// fixture engine acts on this; the real engine is the authority and ignores it (the default).
    fn set_gate_for_scene(&self, _gate: GateState) {}
    /// The last start's refusal while it stands, with the attempt it was (the refusal modal, Q3). Cheap: asked on
    /// every frame.
    fn sync_refusal(&self) -> Option<SyncOff> {
        None
    }
}

/// Everything a frame, the tray menu or the flyout may ask the shell to do beyond navigation.
pub trait ShellActions: Send + Sync {
    fn session(&self) -> Session;
    /// Starts syncing, unless another agent (a Tauri 0.8.x app of the same identity, `bb
    /// watch`) holds the profile's sync lock: then it refuses and the engine is not asked.
    fn start_sync(&self) -> Result<(), ActionError>;
    /// The normal quit path's engine hook; see [`EngineActions::stop_for_quit`].
    fn stop_for_quit(&self);
    fn resume_after_cancelled_quit(&self);
    fn tray_inputs(&self) -> TrayInputs;
    fn pause_sync(&self) -> Result<(), ActionError>;
    fn resume_sync(&self) -> Result<(), ActionError>;
    fn open_in_file_explorer(&self) -> Result<(), ActionError>;
    /// "Open Beebeeb when you log in" (DW-0281, Settings).
    fn autostart_enabled(&self) -> bool;
    fn set_autostart(&self, enabled: bool) -> Result<(), ActionError>;
    /// A scene function sets the gate on the engine; see [`EngineActions::set_gate_for_scene`].
    fn set_gate_for_scene(&self, gate: GateState);
    /// See [`EngineActions::sync_refusal`].
    fn sync_refusal(&self) -> Option<SyncOff>;
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
    fn session(&self) -> Session {
        self.engine.session()
    }
    fn start_sync(&self) -> Result<(), ActionError> {
        if self.engine.profile_holder().is_some() {
            return Err(ActionError::NotAvailable(
                "another Beebeeb program holds this profile's sync lock",
            ));
        }
        self.engine.start_sync()
    }
    fn stop_for_quit(&self) {
        self.engine.stop_for_quit();
    }
    fn resume_after_cancelled_quit(&self) {
        self.engine.resume_after_cancelled_quit();
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
    fn set_gate_for_scene(&self, gate: GateState) {
        self.engine.set_gate_for_scene(gate);
    }
    fn sync_refusal(&self) -> Option<SyncOff> {
        self.engine.sync_refusal()
    }
}

/// What a [`FixtureEngine`] was asked to do, for tests.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixtureCall {
    Pause,
    Resume,
    OpenInFileExplorer,
    StartSync,
    StopForQuit,
    ResumeAfterCancelledQuit,
}

struct Fixture {
    gate: GateState,
    account: AccountId,
    inputs: TrayInputs,
    holder: Option<ProfileHolder>,
    calls: Vec<FixtureCall>,
    /// Whether `set_gate_for_scene` updates this engine's gate. True in fixture mode (scenes
    /// set the gate); false simulates a real engine that is the authority and ignores scene gates.
    accepts_scene_gate: bool,
    refusal: Option<SyncOff>,
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
                account: "fixture-account".into(),
                inputs,
                holder: None,
                calls: Vec::new(),
                accepts_scene_gate: true,
                refusal: None,
            }),
        }
    }

    /// A fixture engine that ignores `set_gate_for_scene`: simulates a real engine where the
    /// engine is the authority on the gate and scene functions cannot override it.
    pub fn live_stand_in(gate: GateState) -> FixtureEngine {
        let engine = FixtureEngine::with(gate, TrayInputs::default());
        engine.lock().accepts_scene_gate = false;
        engine
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

    /// Another account is signed in now (the gate stays as it is).
    pub fn set_account(&self, account: &str) {
        self.lock().account = account.to_owned();
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

    /// A start's refusal stands (or none does), as the engine's view would say.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_sync_refusal(&self, refusal: Option<SyncOff>) {
        self.lock().refusal = refusal;
    }
}

impl EngineActions for FixtureEngine {
    fn session(&self) -> Session {
        let state = self.lock();
        Session::of(state.gate, state.account.clone())
    }
    fn start_sync(&self) -> Result<(), ActionError> {
        self.lock().calls.push(FixtureCall::StartSync);
        Ok(())
    }
    fn stop_for_quit(&self) {
        self.lock().calls.push(FixtureCall::StopForQuit);
    }
    fn resume_after_cancelled_quit(&self) {
        self.lock().calls.push(FixtureCall::ResumeAfterCancelledQuit);
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
            return Err(ActionError::NotAvailable("the app is not in File Explorer"));
        }
        // The fixture has no folder to open; it records that it was asked.
        state.calls.push(FixtureCall::OpenInFileExplorer);
        Ok(())
    }
    fn profile_holder(&self) -> Option<ProfileHolder> {
        self.lock().holder.clone()
    }
    fn set_gate_for_scene(&self, gate: GateState) {
        let state = self.lock();
        if state.accepts_scene_gate {
            drop(state);
            self.set_gate(gate);
        }
    }
    fn sync_refusal(&self) -> Option<SyncOff> {
        self.lock().refusal.clone()
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
        assert_eq!(host.session(), Session::signed_out());
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
        assert_eq!(host.session(), Session::of(GateState::Locked, "fixture-account"));
        assert!(host.tray_inputs().locked);
        engine.set_gate(GateState::SignedOut);
        assert!(!host.tray_inputs().signed_in);
        assert_eq!(host.session(), Session::signed_out());
    }

    /// P2-9: the native app does not start syncing while a Tauri 0.8.x app of the same
    /// identity holds the sync lock; the engine is not even asked.
    #[test]
    fn sync_does_not_start_while_another_agent_holds_the_profile() {
        let (engine, _, host) = host();
        engine.set_profile_holder(Some(ProfileHolder {
            agent: "desktop".into(),
            pid: 4242,
        }));
        assert!(matches!(host.start_sync(), Err(ActionError::NotAvailable(_))));
        assert!(engine.calls().is_empty(), "the engine was not asked");
        engine.set_profile_holder(None);
        host.start_sync().unwrap();
        assert_eq!(engine.calls(), vec![FixtureCall::StartSync]);
    }

    #[test]
    fn the_session_names_its_account_and_signed_out_names_none() {
        let (engine, _, host) = host();
        assert_eq!(host.session().account.as_deref(), Some("fixture-account"));
        engine.set_account("user-b");
        assert_eq!(host.session(), Session::of(GateState::Ready, "user-b"));
        engine.set_gate(GateState::SignedOut);
        assert_eq!(host.session().account, None);
    }

    #[test]
    fn the_quit_hooks_reach_the_engine() {
        let (engine, _, host) = host();
        host.stop_for_quit();
        host.resume_after_cancelled_quit();
        assert_eq!(
            engine.calls(),
            vec![FixtureCall::StopForQuit, FixtureCall::ResumeAfterCancelledQuit]
        );
    }
}
