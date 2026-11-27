//! Finder-setup ports, the part that needs no gate, no bridge and no engine slot: the session facts and the
//! merge-only settings writes. The gated bridge calls, the engine slot and `impl Ports` are not here yet (see the
//! dm-23 report: they need `LifecycleEvent` re-exported by the engine driver, the bridge's add/read/wait/remove
//! wrappers in `bridge.rs`, and tokio's `sync` feature in this crate). Reference: `desktop-v0.8.25`
//! `finder_setup/macos_ports.rs`. Nothing here reads an OS error message; `FailureRecord` is reason, domain, code,
//! time only.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::finder_setup::core::{self, DomainState, Observation, Op, OpResult, SessionFacts};
use beebeeb_engine::finder_setup::driver::{op_limit, within, FinderSetupView, LifecycleEvent, Ports};
use beebeeb_engine::finder_setup::error::{app_code, FailureRecord, FpError};
use beebeeb_engine::surfaces::phase::FinderFailureReason;
use tokio::sync::oneshot;

use super::bridge::{self, Gate};
use super::lifecycle_log::{self, WaitEnd, WaitSteps};
use super::removal::{self, DomainRemoval, KeptFolder, RemovalFailure};

/// How long the removal and the launch read wait for a busy gate: long enough for a call that is merely in flight
/// (a working-set signal, the sweep) to return, short enough that a stuck `fileproviderd` still ends in a `timeout`.
/// Inside the operation's own limit (`op_limit`: 10 s for `Observe`, 15 s for `RemoveDomain`).
pub(crate) const BRIDGE_GATE_WAIT: Duration = Duration::from_secs(3);

/// The File Provider calls the reconciler makes. The real one is [`SystemBridge`]; tests put a fake here, so no test
/// makes an FFI call. The caller holds the process-wide gate around each call.
pub(crate) trait FpBridge: Send + Sync + 'static {
    fn add_domain(&self) -> Result<(), FpError>;
    fn domain_state(&self) -> Result<DomainState, FpError>;
    fn wait_for_domain_ready(&self, timeout: Duration) -> Result<(), FpError>;
    /// Whether the File Provider extension has connected to the IPC socket.
    fn extension_connected(&self) -> bool {
        false
    }
    /// Dm-87: macOS gives the domain's root a place on disk (`bridge::root_in_place`). The caller holds the gate.
    fn root_in_place(&self) -> bool {
        false
    }
    /// Dm-87: one working-set signal, which makes the system ask the extension for changes and so launches it.
    /// Takes the gate itself; best effort.
    fn nudge(&self) {}
    /// Removes the domain, keeping what never reached the server (`removal::remove_keeping_unsynced`, never a plain
    /// remove).
    fn remove(&self) -> Result<DomainRemoval, RemovalFailure>;
}

/// The real bridge.
pub(crate) struct SystemBridge;

impl FpBridge for SystemBridge {
    fn add_domain(&self) -> Result<(), FpError> {
        bridge::add_domain()
    }
    fn domain_state(&self) -> Result<DomainState, FpError> {
        bridge::domain_state()
    }
    fn wait_for_domain_ready(&self, timeout: Duration) -> Result<(), FpError> {
        bridge::wait_for_domain_ready(timeout)
    }
    fn extension_connected(&self) -> bool {
        bridge::extension_connected()
    }
    fn root_in_place(&self) -> bool {
        bridge::root_in_place().unwrap_or(false)
    }
    fn nudge(&self) {
        if let Err(error) = bridge::signal_working_set() {
            tracing::info!(
                domain = %error.domain,
                status = error.code,
                "finder setup: the working-set signal did not go"
            );
        }
    }
    fn remove(&self) -> Result<DomainRemoval, RemovalFailure> {
        removal::remove_keeping_unsynced_gated()
    }
}

/// What the app gives the ports: the session, the engine slot, the window and the alert. The engine start, finish and
/// stop are NOT cut off by the ports (a future dropped after an engine spawned loses its `started` answer, and one
/// dropped after it left its slot detaches the engine with the session's keys): `limit` bounds the wait for the slot
/// inside the host.
pub(crate) trait FpHost: Send + Sync + 'static {
    fn vault_unlocked(&self) -> bool;
    fn auth_present(&self) -> bool;
    fn removal_owed(&self) -> bool;
    /// The settings file; `None` when it cannot be found.
    fn config_path(&self) -> Option<PathBuf>;
    /// `Ok(true)`: a new engine was started; `Ok(false)`: one was already running.
    fn start_engine(&self, limit: Duration) -> impl Future<Output = Result<bool, FpError>> + Send;
    /// Saves the sync folder and takes the engine slot (its one await, before anything is spawned).
    fn finish_ready(&self) -> impl Future<Output = Result<(), FpError>> + Send;
    /// `Err` when the slot was not taken in time or the stop was not confirmed.
    fn stop_engine(&self, limit: Duration) -> impl Future<Output = Result<(), FpError>> + Send;
    fn publish(&self, view: &FinderSetupView);
    fn window_visible(&self) -> bool;
    /// Shows a kept folder: saved for the Settings › Sync row, then the alert. Never logged.
    fn surface_kept_folder(&self, location: Option<String>);
    fn clear_removal_owed(&self);
    fn clear_domain_gone(&self);
}

/// The macOS `Ports`: runs each `Op` against the bridge (through the one process-wide gate), the engine slot, the
/// lifecycle log and the settings file. It decides nothing.
pub(crate) struct MacosPorts<H: FpHost, B: FpBridge> {
    host: Arc<H>,
    bridge: Arc<B>,
    gate_wait: Duration,
}

impl<H: FpHost> MacosPorts<H, SystemBridge> {
    pub(crate) fn new(host: Arc<H>) -> Self {
        Self::with_bridge(host, Arc::new(SystemBridge), BRIDGE_GATE_WAIT)
    }
}

impl<H: FpHost, B: FpBridge> MacosPorts<H, B> {
    pub(crate) fn with_bridge(host: Arc<H>, bridge: Arc<B>, gate_wait: Duration) -> Self {
        Self {
            host,
            bridge,
            gate_wait,
        }
    }
}

/// One bridge call at a time, on a blocking thread off the async executor. The gate is taken ON that thread and
/// dropped when the OS call returns, at no earlier point, whatever happens to the future waiting for it: a call that
/// `within` cut off still holds the gate until the OS answers. `wait` `None` fails at once with `OP_TIMEOUT` on a
/// busy gate; `Some` waits at most that long (the launch read and the removal, which the core never retries).
async fn gated<T: Send + 'static>(
    wait: Option<Duration>,
    call: impl FnOnce() -> Result<T, FpError> + Send + 'static,
) -> Result<T, FpError> {
    tokio::task::spawn_blocking(move || {
        let _gate = match wait {
            Some(wait) => Gate::enter_waiting(wait)?,
            None => Gate::try_enter()?,
        };
        call()
    })
    .await
    .unwrap_or_else(|_| Err(FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, "bridge task failed")))
}

/// How long one stabilization wait runs before readiness is looked at again.
const STABILIZE_SLICE: Duration = Duration::from_secs(2);

fn ms(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// One look at readiness (dm-87). Ready = `getDomains` lists the domain Enabled AND either the extension has connected
/// to the IPC socket, or macOS gives the domain's root a place on disk (an existing domain the system serves: RC13's
/// shell read worked while the old check, which wanted the extension's connection only, waited out 60 s three times).
/// An Enabled domain that is neither gets one working-set signal per wait, which launches a slow extension.
fn look<B: FpBridge>(bridge: &B, gate_wait: Duration, steps: &mut WaitSteps) -> Option<WaitEnd> {
    steps.looks += 1;
    let asked = Instant::now();
    let state = Gate::enter_waiting(gate_wait).and_then(|_gate| bridge.domain_state());
    steps.domain_ms = steps.domain_ms.max(ms(asked.elapsed()));
    steps.enabled = state == Ok(DomainState::Enabled);
    if !steps.enabled {
        return None;
    }
    steps.extension = bridge.extension_connected();
    if steps.extension {
        return Some(WaitEnd::Extension);
    }
    let asked = Instant::now();
    steps.root_in_place = Gate::enter_waiting(gate_wait).is_ok_and(|_gate| bridge.root_in_place());
    steps.root_ms = steps.root_ms.max(ms(asked.elapsed()));
    if steps.root_in_place {
        return Some(WaitEnd::RootInPlace);
    }
    if !steps.nudged {
        steps.nudged = true;
        bridge.nudge();
    }
    None
}

/// The wait for the domain (`WaitStable`): [`look`] first and between slices of `waitForStabilization`, which run off
/// the gate (under it every working-set signal met a busy gate, RC5). The wait ends ready at the first look that says
/// so, or when stabilization answers; a stabilization timeout is looked at once more at the deadline. Every wait
/// writes one `wait ...` line with each step's time to the lifecycle log ([`WaitSteps`]). Windows does not use this.
///
/// What "become available" waited for (RC16): `waitForStabilization` answers once the system has caught up with
/// every pending change of the domain, which on a loaded Mac with a big tree takes longer than one attempt's 60 s;
/// the extension's connection comes only with its first request. Neither is needed to serve a read.
async fn wait_ready<B: FpBridge>(
    bridge: Arc<B>,
    timeout: Duration,
    gate_wait: Duration,
    slice: Duration,
) -> Result<(), FpError> {
    tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        let deadline = started + timeout;
        let mut steps = WaitSteps::default();
        let answer = loop {
            if let Some(end) = look(&*bridge, gate_wait, &mut steps) {
                break Ok(end);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let asked = Instant::now();
            let stabilized = bridge.wait_for_domain_ready(left.min(slice));
            steps.slices += 1;
            steps.stabilize_ms += ms(asked.elapsed());
            match stabilized {
                Ok(()) => break Ok(WaitEnd::Stabilized),
                Err(error) => {
                    if beebeeb_engine::finder_setup::policy::classify(&error) != FinderFailureReason::Timeout {
                        break Err((WaitEnd::Failed, error));
                    }
                    if Instant::now() >= deadline {
                        break match look(&*bridge, gate_wait, &mut steps) {
                            Some(end) => Ok(end),
                            None => Err((WaitEnd::TimedOut, error)),
                        };
                    }
                }
            }
        };
        steps.total_ms = ms(started.elapsed());
        steps.end = Some(match &answer {
            Ok(end) | Err((end, _)) => *end,
        });
        lifecycle_log::wait(&steps);
        answer.map(|_| ()).map_err(|(_, error)| error)
    })
    .await
    .unwrap_or_else(|_| Err(FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, "bridge task failed")))
}

/// What a removal answers.
type Removal = Result<DomainRemoval, RemovalFailure>;

/// The removal's call for the gate: runs `remove` and sends the answer on `answer`. The reconciler stops waiting at its
/// op limit, but the OS call goes on, and macOS may have kept files when it returns. While the reconciler listens it
/// gets the answer; once it stopped, the send fails and this thread hands the folder to `late` itself, before the gate
/// is freed. Exactly one of the two gets it ([`removal_answer`] closes the channel before it reads it).
fn answering(
    answer: oneshot::Sender<Removal>,
    remove: impl FnOnce() -> Removal + Send + 'static,
    late: impl FnOnce(KeptFolder) + Send + 'static,
) -> impl FnOnce() -> Result<(), FpError> + Send + 'static {
    move || {
        if let Err(unheard) = answer.send(remove()) {
            late(match unheard {
                Ok(removal) => removal.kept,
                Err(failure) => failure.kept,
            });
        }
        Ok(())
    }
}

/// The engine's kept-folder data for the shell's.
fn engine_kept(kept: KeptFolder) -> core::KeptFolder {
    match kept {
        KeptFolder::Nothing(_) => core::KeptFolder::Nothing,
        KeptFolder::Kept {
            path,
            contents_checked,
            empty,
        } => core::KeptFolder::Kept {
            path,
            contents_checked,
            empty,
        },
        KeptFolder::Unknown => core::KeptFolder::Unknown,
    }
}

/// What the reconciler makes of a removal run with [`answering`], once `within` returned `cut`. The channel is closed
/// first, so a removal that finishes from then on goes to its late sink; an answer already there is read, also when
/// `within` cut the call off just after it sent.
fn removal_answer(cut: Result<(), FpError>, mut answered: oneshot::Receiver<Removal>) -> OpResult {
    answered.close();
    match (answered.try_recv(), cut) {
        (Ok(Ok(removal)), _) => OpResult::Removed(Ok(()), engine_kept(removal.kept)),
        (Ok(Err(failure)), _) => OpResult::Removed(
            // The OS's message is not carried: it may name a file. The kept folder rides along.
            Err(FpError::app(
                app_code::UNEXPECTED_BRIDGE_RETURN,
                "the Finder location could not be removed",
            )),
            engine_kept(failure.kept),
        ),
        (Err(_), Err(error)) => OpResult::Removed(Err(error), core::KeptFolder::default()),
        (Err(_), Ok(())) => OpResult::Removed(
            Err(FpError::app(
                app_code::UNEXPECTED_BRIDGE_RETURN,
                "the removal returned no answer",
            )),
            core::KeptFolder::default(),
        ),
    }
}

/// A folder a removal kept, shown the way the app-start sweep shows one. Logs never carry the folder.
fn surface_kept<H: FpHost>(host: &H, kept: KeptFolder, context: &'static str) {
    host.surface_kept_folder(DomainRemoval { kept }.kept_location(context));
}

/// §5.1's facts. Fail safe: an unreadable config never counts as "signed out by choice", so it never removes the
/// domain.
pub(super) fn facts_from(
    vault_unlocked: bool,
    auth_present: bool,
    removal_owed: bool,
    config: Result<DesktopConfig, String>,
) -> SessionFacts {
    let signed_out_by_choice = config.map(|cfg| cfg.finder_signed_out_by_choice).unwrap_or(false);
    SessionFacts {
        vault_unlocked,
        auth_present,
        signed_out_by_choice,
        removal_owed,
    }
}

/// Each setter returns whether `cfg` changed, so a repeat writes nothing.
pub(super) fn set_failure(cfg: &mut DesktopConfig, record: Option<FailureRecord>) -> bool {
    if cfg.finder_last_failure == record {
        return false;
    }
    cfg.finder_last_failure = record;
    true
}

pub(super) fn set_signed_out_by_choice(cfg: &mut DesktopConfig, value: bool) -> bool {
    if cfg.finder_signed_out_by_choice == value {
        return false;
    }
    cfg.finder_signed_out_by_choice = value;
    true
}

/// [`update_config_at`] on the host's settings file.
fn update_config<H: FpHost>(host: &H, change: impl FnOnce(&mut DesktopConfig) -> bool) {
    match host.config_path() {
        Some(path) => update_config_at(&path, change),
        None => tracing::warn!("finder setup: could not find the settings file"),
    }
}

/// One load-change-save on the settings file at `path`, under the config-write lock (`DesktopConfig::try_update_at`,
/// which is `try_update_at_then`). Merge-only: no copy loaded earlier is ever saved, so a kept folder another writer
/// saved meanwhile survives. A change that is not saved leaves the file untouched; a failed write is logged (no path,
/// no OS text) and ignored.
pub(super) fn update_config_at(path: &Path, change: impl FnOnce(&mut DesktopConfig) -> bool) {
    if DesktopConfig::try_update_at(path, |cfg| (change(cfg), ())).is_err() {
        tracing::warn!("finder setup: could not update the settings file");
    }
}

impl<H: FpHost, B: FpBridge> Ports for MacosPorts<H, B> {
    fn run(&mut self, op: Op) -> impl Future<Output = OpResult> + Send {
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let gate_wait = self.gate_wait;
        async move {
            let limit = op_limit(op);
            match op {
                Op::Observe => {
                    let facts = facts_from(
                        host.vault_unlocked(),
                        host.auth_present(),
                        host.removal_owed(),
                        host.config_path()
                            .ok_or_else(|| "no settings file".to_string())
                            .and_then(|path| DesktopConfig::load_from(&path).map_err(|error| error.to_string())),
                    );
                    // Waits a little for the gate: a launch read lost to a call merely in flight would leave a
                    // domain that should be removed until the next launch.
                    let domain = within(op, limit, gated(Some(gate_wait), move || bridge.domain_state())).await;
                    OpResult::Observed(Observation { facts, domain })
                }
                // Not `within`: `limit` bounds the wait for the engine slot inside the host.
                Op::StartEngine => OpResult::EngineStarted(host.start_engine(limit).await),
                Op::AddDomain => OpResult::Added(within(op, limit, gated(None, move || bridge.add_domain())).await),
                Op::ReadDomain => OpResult::Domain(within(op, limit, gated(None, move || bridge.domain_state())).await),
                // NOT gated: `waitForStabilization` can block for the whole timeout, and under the gate every
                // working-set signal met a busy gate (RC5). Readiness is the domain Enabled AND the extension
                // connected, see `wait_ready`.
                Op::WaitStable(timeout) => {
                    OpResult::Stable(within(op, limit, wait_ready(bridge, timeout, gate_wait, STABILIZE_SLICE)).await)
                }
                // Cut off as a whole: safe only because the host's finish has one await, the engine slot, before it
                // spawns.
                Op::FinishReady => OpResult::Finished(within(op, limit, host.finish_ready()).await),
                // Not `within`, like `StartEngine`: once the engine is out of its slot its stop must run to the end.
                Op::StopEngine => OpResult::EngineStopped(host.stop_engine(limit).await),
                // Waits a little for the gate (inside `limit`): the core never retries a removal. A removal that
                // finishes after `limit` still shows its folder, from the blocking thread (`answering`).
                Op::RemoveDomain => {
                    let late_host = host.clone();
                    let late = move |kept| surface_kept(&*late_host, kept, "finder reconciler, after its time limit");
                    let (answer, answered) = oneshot::channel();
                    let cut = within(
                        op,
                        limit,
                        gated(Some(gate_wait), answering(answer, move || bridge.remove(), late)),
                    )
                    .await;
                    removal_answer(cut, answered)
                }
            }
        }
    }

    fn publish(&mut self, view: &FinderSetupView) {
        self.host.publish(view);
    }

    fn log(&mut self, event: LifecycleEvent) {
        lifecycle_log::event(event);
    }

    fn persist_failure(&mut self, record: Option<FailureRecord>) {
        update_config(&*self.host, move |cfg| set_failure(cfg, record));
    }

    fn persist_signed_out_by_choice(&mut self, value: bool) {
        update_config(&*self.host, move |cfg| set_signed_out_by_choice(cfg, value));
    }

    fn domain_removal_confirmed(&mut self) {
        self.host.clear_removal_owed();
    }

    fn domain_added(&mut self) {
        self.host.clear_domain_gone();
    }

    fn kept_folder(&mut self, kept: core::KeptFolder) {
        let location = match kept {
            core::KeptFolder::Kept { path, .. } => Some(path),
            core::KeptFolder::Nothing | core::KeptFolder::Unknown => None,
        };
        self.host.surface_kept_folder(location);
    }

    fn window_visible(&self) -> bool {
        self.host.window_visible()
    }
}

#[cfg(test)]
#[path = "ports_tests/mod.rs"]
mod tests;
