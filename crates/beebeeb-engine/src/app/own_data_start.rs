//! Task 2267 in the engine ("Unowned local data on a Windows PC" and "Found while
//! paused"). A Windows start refused with `OtherAccountsData` over local data that has no owner record, or one recorded
//! only by an old email, proves in the background that the data is the signed-in account's own (`own_data_proof`: ONE
//! listing of this account's tree, every server-named My Files row opening under this session's key) and then starts
//! by itself. Another account's data is never adopted, a queued change that cannot be tied to proven data is held for
//! the person's review in the same transaction as the owner (`found_while_paused`), and nothing is deleted. The
//! behaviour is the Tauri shell's binding (`src-tauri/src/bound_engine.rs`), carried over:
//!
//! - One proof per account runtime and session turn. A refusal while one runs, or after one settled the turn, asks
//!   nothing new; a proof that got no answer frees the turn, so a later start asks again (`Outcome::Unavailable`).
//! - The proof is a task of the engine's runtime that a Lock and a sign-out stop first ([`Engine::stop_proofs`]), so
//!   its client (the session's token and key) never outlives them. A stopped proof that does not let go in time (it is
//!   in a write that does not yield) is kept: the Lock or the sign-out goes on, and no start runs until its client is
//!   gone (`StopFinishing`, [`Engine::local_data_for_start`]).
//! - Sync starts after an adoption only while the session is still the one proven. That start binds again under the
//!   step lock and now keeps everything.
//! - No answer at all (offline, a time-out) turns the refusal into `IdentityUnknown` ("connect and open Beebeeb
//!   again"). A token the server refused is not "offline", so that sentence is not shown for it.
//!
//! The upgrade from a release before the binding (The binding rule), on every OS: the first
//! startup of this build ([`Engine::fix_upgrade_candidate_at_startup`], from `restore`) fixes the adoption candidate,
//! the account whose vault key the credential store holds then, and shuts the adoption window on disk, even with no
//! key. That startup writes nothing but the window's one `user_version` mark into a `state.db` that exists, and never
//! makes a folder or a `state.db`. The start ([`Engine::local_data_for_start`], step 7, before the binding) records the
//! candidate as the owner of unowned local data when the session is that same account, so its first start keeps
//! everything; every other session adopts nothing there. A PC that held no key at the upgrade is left to the proof
//! above. An unknown session is refused before any of this (`IdentityUnknown`, step 3 and `decide`).
//!
//! A "Keep on this PC only" that a crash interrupted is finished or undone before an engine exists
//! ([`Engine::settle_an_interrupted_keep`], step 7 of the start, after the binding). The decisions the card needs
//!  are the `found_while_paused_*_for` calls, stamped with the session turn
//! ([`Engine::session_turn`]); the window binds them.
//!
//! The proof's hooks the start, the Lock and the sign-out call exist on every OS and do nothing where no start is
//! refused over local data a proof could settle (the same gate as `own_data_proof` in `lib.rs`).
//!
//! Must not: log a name, a path, an email, an id, a token or a key (counts and fixed words only); adopt, reset or
//! delete local data; take the step lock inside a task of the engine's runtime.

use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use crate::account::AccountRuntime;
use crate::account_binding::{self, Identity};
use crate::app::{CredentialStores, Engine, LocalState, SyncStartRefused};
use crate::runner::EngineRunner;
use crate::state_db::StateDb;
use crate::state_paths;

#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use std::path::PathBuf;
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use std::sync::Condvar;
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use std::time::Duration;

#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use crate::account::SessionGeneration;
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use crate::api_client::ApiClient;
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use crate::app::owned_state::OwnedState;
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use crate::found_while_paused::{self, KeepResult, Recovery, Review};
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use crate::own_data_proof::{self, Listing, Outcome, Tally};
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use crate::signin::start;
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
use crate::sync_view::Start;

/// dc-144b: what the window says once after a corrupt `state.db` was moved aside and the files listed again.
pub const LOCAL_LIST_REBUILT_NOTICE: &str = "Beebeeb rebuilt its local file list. Your files are listed again from your account; nothing in your account changed.";

#[cfg(test)]
#[path = "own_data_start_tests.rs"]
mod tests;

/// How long the proof waits for the whole listing of this account's tree (the snapshot, then the trash, with the
/// client's retries). A Lock or a sign-out does not wait for it: they stop the proof first.
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
pub const LIST_TIMEOUT: Duration = Duration::from_secs(600);

/// How long one request of the proof's listing may take. The snapshot is the whole tree in one response, so the app's
/// 30 s per request would refuse a large tree on a slow link every time. Nobody waits on the proof.
#[cfg(target_os = "windows")]
pub const PROOF_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// How long a Lock or a sign-out waits for its stopped proofs to let go of their client (one wait for them all). A
/// stopped task is dropped at its next await; a database write it is in finishes first. One still alive after it is
/// kept ([`ProofStop::Unconfirmed`]).
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
const STOP_WAIT: Duration = Duration::from_secs(5);

/// What [`Engine::stop_proofs`] came to. The step that stopped them (a Lock, a sign-out, the close of a session the
/// server ended, the strong unlock's replacement) goes on either way and tells the person nothing new: the seat itself
/// refuses every start while a proof that did not end is alive.
#[must_use = "a stopped proof that did not end still holds the session's token and key"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub(in crate::app) enum ProofStop {
    /// Every stopped proof let go of its client, or none ran.
    Ended,
    /// A stopped proof did not let go of its client within [`STOP_WAIT`]. Its seat keeps it: every start is refused
    /// ([`SyncStartRefused::StopFinishing`]) until its client is gone, and the next stop waits for it again.
    Unconfirmed,
}

/// What Upload, Keep, the review and "Show in Explorer" say when this PC's local data is not the signed-in account's
/// own, the session moved since the card was read, or nobody is signed in.
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
pub const FOUND_WHILE_PAUSED_NOT_YOURS: &str =
    "Beebeeb couldn’t confirm these files belong to the account you’re signed in to, so nothing was changed.";

/// What Keep says when it stopped part way (the database could not be written). Every item it moved is forgotten in
/// its own transaction, or settled at the next start ([`Engine::settle_an_interrupted_keep`]).
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
pub const FOUND_WHILE_PAUSED_KEEP_STOPPED: &str = "Beebeeb couldn’t finish moving these files. Anything already moved is in “Beebeeb - not uploaded”; the rest stays where it is and isn’t uploaded.";

/// What the card says when it cannot read the held changes. It reads them again at the next refresh; Upload waits for
/// that read, since it sends only what the card showed.
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
pub const FOUND_WHILE_PAUSED_REVIEW_FAILED: &str =
    "Beebeeb couldn’t read these changes, so nothing was uploaded or moved. It tries again in a moment.";

/// What Upload says when the held changes could not be released: nothing changed.
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
pub const FOUND_WHILE_PAUSED_UPLOAD_FAILED: &str =
    "Beebeeb couldn’t start uploading these files, so they’re still waiting. Try again.";

/// What "Show in Explorer" says when the folder the last keep recorded is not there.
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
pub const KEPT_FOLDER_GONE: &str = "Beebeeb can’t find the folder it moved them to. It may have been moved or renamed.";

/// Every handle's seat, by its account runtime. A `Weak`, so the address a dropped handle leaves is never matched by
/// the next one. The handle's own fields are `app.rs`'s, which has no line left for them.
static SEATS: Mutex<Vec<(Weak<AccountRuntime>, Arc<Seat>)>> = Mutex::new(Vec::new());

/// One handle's upgrade candidate and its proofs.
#[derive(Default)]
struct Seat {
    state: Mutex<SeatState>,
    /// Rung when the proof a refused start owes may run.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    wake: Condvar,
    /// [`Engine::serve_own_data_proofs`] ran for this handle.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    serving: AtomicBool,
}

#[derive(Default)]
struct SeatState {
    /// The upgrade's adoption, as this handle's first startup fixed it.
    startup: Startup,
    /// dc-144b: a corrupt `state.db` was moved aside and a fresh one is made; the window has not said so yet.
    rebuilt: bool,
    /// The proof the last refused start owes; `ready` once that refusal is in the view.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    owed: Option<Owed>,
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    ready: bool,
    /// The turn a proof was asked for: one per turn. Freed when the proof got no answer.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    asked: Option<SessionGeneration>,
    /// The proofs on the runtime, for a Lock or a sign-out to stop.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    running: Vec<Running>,
    /// Stopped proofs that did not let go of their client within [`STOP_WAIT`]: no start while one is alive
    /// ([`Engine::local_data_for_start`]), and the next stop aborts and waits for them again.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    unconfirmed: Vec<Running>,
    /// How many stops ([`Engine::stop_proofs`]) this handle has had. A proof owed before the last one never reads a
    /// key: it is compared under this lock, in the same hold that reads the keys and registers the proof's task.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    stops: u64,
    #[cfg(test)]
    test: TestSeams,
}

/// The binding rule: the adoption window as the first startup found it. `candidate` is set only when that startup found
/// the window open (or no `state.db` yet), and holds the account whose vault key the credential store held then.
#[derive(Default)]
struct Startup {
    fixed: bool,
    candidate: Option<Identity>,
}

/// A proof a refused start owes: the session turn its keys were read in, the folder it was asked to start in, and
/// the seat's stop count then (a stop since ends it before it reads a key).
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
struct Owed {
    turn: SessionGeneration,
    sync_root: PathBuf,
    stops: u64,
}

/// A proof's task, registered in the same hold of the seat lock that spawned it, so a stop never misses it.
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
struct Running {
    abort: tokio::task::AbortHandle,
    /// Closed when the proof's task is dropped (it ended, or it was stopped), after its client ([`EndsAfterRelease`]).
    ended: tokio::sync::oneshot::Receiver<()>,
}

/// A proof's task: the proof, then the sender a stop waits on. A struct's fields are dropped in their declaration
/// order, so the sender closes only once the proof, and with it every reference to the proof's client, is gone. (An
/// `async` block's locals and captures have no order the language promises.)
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
struct EndsAfterRelease<F> {
    proof: std::pin::Pin<Box<F>>,
    _ended: tokio::sync::oneshot::Sender<()>,
}

#[cfg(any(target_os = "windows", test, feature = "test-support"))]
impl<F: std::future::Future> std::future::Future for EndsAfterRelease<F> {
    type Output = F::Output;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<F::Output> {
        self.get_mut().proof.as_mut().poll(cx)
    }
}

/// Test builds only: what a test puts in place of the network and the OS, and what it reads back.
#[cfg(test)]
#[derive(Default)]
struct TestSeams {
    /// The listing, in place of the snapshot and the trash.
    listing: Option<TestListing>,
    /// The listing's time-out, in place of [`LIST_TIMEOUT`].
    list_timeout: Option<Duration>,
    /// How long a stop waits for a stopped proof, in place of [`STOP_WAIT`].
    stop_wait: Option<Duration>,
    /// The OS's answers for the start after an adoption, in place of the real ones.
    restart_port: Option<Arc<dyn crate::sync_root_port::SyncRootPort>>,
    /// Owed proofs the service took up (before the one-per-turn rule), and proofs it started.
    owed_seen: usize,
    proofs_run: usize,
    /// Every proof's client, to see that it was let go.
    clients: Vec<Weak<ApiClient>>,
    /// The reconcile asks counted at each release of held changes (Upload), to see their order.
    asks_at_release: Vec<u64>,
    /// Each finished proof's outcome, by kind, once the service acted on it.
    outcomes: Vec<&'static str>,
    /// Where the service thread waits for a test: just before the proof reads the session's keys, and right after it
    /// spawned the proof's task. A test's stop then comes at that moment.
    before_keys: Option<TestPause>,
    after_spawn: Option<TestPause>,
}

/// A test's pause for the service thread: it returns when the test lets the thread go on.
#[cfg(test)]
type TestPause = Arc<dyn Fn() + Send + Sync>;

#[cfg(test)]
type TestListing = Arc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Listing> + Send>> + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
impl Seat {
    /// Waits at the test's pause `at` picks, if one is set, with the seat unlocked.
    fn test_pause(&self, at: impl FnOnce(&TestSeams) -> Option<TestPause>) {
        let pause = at(&lock(&self.state).test);
        if let Some(pause) = pause {
            pause();
        }
    }
}

#[cfg(any(target_os = "windows", test, feature = "test-support"))]
impl Seat {
    /// The next proof that is owed and ready, waiting for one; `None` once `alive` says the handle is gone.
    fn next_owed(&self, alive: impl Fn() -> bool) -> Option<Owed> {
        let mut state = lock(&self.state);
        loop {
            if state.ready {
                state.ready = false;
                if let Some(owed) = state.owed.take() {
                    return Some(owed);
                }
            }
            if !alive() {
                return None;
            }
            state = self
                .wake
                .wait_timeout(state, Duration::from_secs(1))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Start the proof service: ONE thread (`own-data-proofs`) that runs the proof a refused start owes. It holds only
    /// a `Weak` to this handle and ends with it; a second call starts nothing. The native shell starts it once, beside
    /// [`Self::watch_session`]. Where no start is refused over local data a proof could settle, nothing is started.
    pub fn serve_own_data_proofs(self: &Arc<Self>) {
        #[cfg(any(target_os = "windows", test, feature = "test-support"))]
        {
            let seat = self.own_data_seat();
            if seat.serving.swap(true, Ordering::SeqCst) {
                return;
            }
            let engine = Arc::downgrade(self);
            let spawned = std::thread::Builder::new()
                .name("own-data-proofs".to_string())
                .spawn(move || {
                    while let Some(owed) = seat.next_owed(|| engine.strong_count() > 0) {
                        let Some(engine) = engine.upgrade() else { return };
                        engine.prove_owed(owed);
                    }
                });
            if spawned.is_err() {
                tracing::warn!("ownership proof: the service did not start");
            }
        }
    }

    /// Step 7 of the start, on the binding's refusal: one with `OtherAccountsData` owes a proof for the session the
    /// start read its keys in (the turn, read here under the step lock) and for its folder. The service runs it once
    /// the refusal is in the view ([`Self::wake_own_data_proofs`]). Returns the refusal unchanged.
    pub(in crate::app) fn proof_owed_on(&self, refused: SyncStartRefused, sync_root: &Path) -> SyncStartRefused {
        #[cfg(any(target_os = "windows", test, feature = "test-support"))]
        if refused == SyncStartRefused::OtherAccountsData {
            let seat = self.own_data_seat();
            let mut state = lock(&seat.state);
            state.owed = Some(Owed {
                turn: self.acct.session_generation(),
                sync_root: sync_root.to_path_buf(),
                stops: state.stops,
            });
            state.ready = false;
        }
        #[cfg(not(any(target_os = "windows", test, feature = "test-support")))]
        let _ = sync_root;
        refused
    }

    /// `start_on`, after it wrote a refusal into the view: the proof that start owes may run now. Its word (an
    /// `IdentityUnknown`) is then never overwritten by the refusal it answers.
    pub(in crate::app) fn wake_own_data_proofs(&self) {
        #[cfg(any(target_os = "windows", test, feature = "test-support"))]
        {
            let seat = self.own_data_seat();
            let mut state = lock(&seat.state);
            if state.owed.is_some() {
                state.ready = true;
                seat.wake.notify_all();
            }
        }
    }

    /// The Lock, the sign-out, the close of a session the server ended and the strong unlock's replacement of a session
    /// in memory, first: every proof of this handle stops, and they are waited for (at most [`STOP_WAIT`] in all) until
    /// each let go of its client, so the session's token and key are gone before the next engine waits for every
    /// credential to be released (`runner.rs`). Every caller holds the step lock, which a proof never
    /// takes (the start after an adoption takes it as any start does, so it is never cut half way). A proof stopped
    /// after it recorded the owner has recorded this same account's data; one stopped before it recorded nothing.
    ///
    /// A proof still alive when the wait ran out (in a write that does not yield) is kept in the seat before this
    /// returns, and the stop says so ([`ProofStop::Unconfirmed`]): every start is refused until its client is gone
    /// ([`Self::local_data_for_start`]), and the next stop aborts it and waits for it again. Never waits longer: the
    /// caller goes on and tells the person nothing new. No start sees the seat while a proof is out of it here, since
    /// every start takes the step lock the caller holds.
    ///
    /// No proof slips past: a proof reads the session's keys, makes its client and registers its task in one hold of
    /// the seat lock, after it compared the stop count this bumps in its own hold ([`Engine::run_proof`]). So a proof
    /// is either registered here (stopped and waited for) or reads no key at all. Never waits for the service thread
    /// itself, whose start after an adoption takes the step lock the caller holds.
    pub(in crate::app) async fn stop_proofs(&self) -> ProofStop {
        #[cfg(any(target_os = "windows", test, feature = "test-support"))]
        {
            let seat = self.own_data_seat();
            let (proofs, wait) = {
                let mut state = lock(&seat.state);
                state.owed = None;
                state.ready = false;
                state.stops = state.stops.wrapping_add(1);
                #[cfg(test)]
                let wait = state.test.stop_wait.unwrap_or(STOP_WAIT);
                #[cfg(not(test))]
                let wait = STOP_WAIT;
                let mut proofs = std::mem::take(&mut state.running);
                proofs.append(&mut state.unconfirmed);
                (proofs, wait)
            };
            let until = tokio::time::Instant::now() + wait;
            let mut alive = Vec::new();
            for mut proof in proofs {
                proof.abort.abort();
                // By reference: a proof still alive at the deadline keeps the receiver that says when it let go.
                if tokio::time::timeout_at(until, &mut proof.ended).await.is_err() {
                    alive.push(proof);
                }
            }
            if alive.is_empty() {
                return ProofStop::Ended;
            }
            tracing::warn!("ownership proof: a stopped proof did not end in time");
            lock(&seat.state).unconfirmed.append(&mut alive);
            ProofStop::Unconfirmed
        }
        #[cfg(not(any(target_os = "windows", test, feature = "test-support")))]
        ProofStop::Ended
    }

    /// A stopped proof that did not let go of its client in time ([`ProofStop::Unconfirmed`]) is still alive. Forgets
    /// each one that let go since. Polls only, never waits.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    fn a_stopped_proof_is_alive(&self) -> bool {
        let seat = self.own_data_seat();
        let mut state = lock(&seat.state);
        state.unconfirmed.retain_mut(|proof| {
            matches!(
                proof.ended.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            )
        });
        !state.unconfirmed.is_empty()
    }

    /// Step 7 of the start, right after the binding and before an engine exists: a "Keep on this PC only" that a crash
    /// interrupted is finished (the item had moved) or undone (it had not), on the binding's own handle (`data`, closed
    /// here), so it acts on this account's own data only. Never while an engine is in the slot (`runner`). A database
    /// that cannot be read or written leaves the journal for the next start; the start goes on either way, and the
    /// item's changes stay held.
    pub(in crate::app) fn settle_an_interrupted_keep(
        &self,
        data: LocalState,
        sync_root: &Path,
        runner: &Option<EngineRunner>,
    ) {
        #[cfg(any(target_os = "windows", test, feature = "test-support"))]
        if runner.is_some() {
            tracing::warn!("keep: a move a crash interrupted is not settled while sync runs");
        } else {
            match found_while_paused::recover(&data.db, sync_root) {
                Ok(Recovery::Nothing) => {}
                Ok(Recovery::Finished) => {
                    tracing::info!("keep: a move a crash interrupted was finished before sync started");
                }
                Ok(Recovery::Cleared) => {
                    tracing::info!("keep: a move a crash interrupted was undone before sync started");
                }
                Err(_) => tracing::warn!("keep: a move a crash interrupted is settled at the next start"),
            }
        }
        #[cfg(not(any(target_os = "windows", test, feature = "test-support")))]
        let _ = (sync_root, runner);
        drop(data);
    }

    /// dc-144b: a corrupt `state.db` is moved aside (`local_state::move_corrupt_state_db_aside`, never deleted), so
    /// the next open makes a fresh one and the files are listed again from the server. Called at the startup
    /// ([`Self::fix_upgrade_candidate_at_startup`]) and at every engine start, once no other live process holds this
    /// state. A move is remembered until the window says so once ([`Engine::take_local_list_rebuilt_notice`]). Any
    /// other failure to open is an `Err` and nothing is moved: it fails closed.
    pub(in crate::app) fn move_aside_a_corrupt_state_db(&self) -> Result<(), String> {
        match crate::local_state::move_corrupt_state_db_aside(&self.config.state_dir)? {
            crate::local_state::StateDbCheck::Sound => {}
            crate::local_state::StateDbCheck::MovedAside(_) => {
                lock(&self.own_data_seat().state).rebuilt = true;
                self.host.wake();
            }
        }
        Ok(())
    }

    /// The words the window shows once after a corrupt `state.db` was moved aside, or `None`. Taking them clears
    /// them: a second ask answers `None` until the next move.
    pub fn take_local_list_rebuilt_notice(&self) -> Option<&'static str> {
        std::mem::take(&mut lock(&self.own_data_seat().state).rebuilt).then_some(LOCAL_LIST_REBUILT_NOTICE)
    }

    /// The binding rule, from `restore` (under the step lock, after the stored key was or was not loaded): the first
    /// startup of this handle fixes the upgrade's adoption candidate and shuts the adoption window on disk. The window
    /// is open on a `state.db` that a release before the binding wrote, and nothing else: whatever this startup finds,
    /// it shuts it, even with no key, so a later startup (whatever key it finds) never adopts by this route. The
    /// candidate is the account whose vault key the credential store holds now ([`Self::vault_key_account`]); it is
    /// held in memory for this handle's starts ([`Self::local_data_for_start`]). With no `state.db` yet the candidate
    /// is fixed all the same: one an older version kept elsewhere is moved in by the first start and judged there.
    /// Writes nothing but the window's `user_version` mark, and makes no folder and no `state.db`.
    pub(in crate::app) fn fix_upgrade_candidate_at_startup(&self) {
        let seat = self.own_data_seat();
        {
            let mut state = lock(&seat.state);
            if state.startup.fixed {
                return;
            }
            state.startup.fixed = true;
        }
        if self.move_aside_a_corrupt_state_db().is_err() {
            tracing::warn!("state.db could not be checked at startup; the next start checks again");
        }
        let path = state_paths::state_db_path_from_state_dir(&self.config.state_dir);
        let open = match StateDb::shut_adoption_window_at_startup(&path) {
            Ok(None) => true,
            Ok(Some(was_open)) => was_open,
            Err(_) => {
                tracing::warn!("upgrade: the adoption window could not be read at startup");
                false
            }
        };
        if open {
            lock(&seat.state).startup.candidate = self.vault_key_account();
        }
    }

    /// The account whose vault key the credential store holds now: the session in memory when the startup loaded its
    /// key (the key is loaded only when it is this session's account's), else the owner the stored key is tagged with
    /// (read without reading the key itself, as a closed vault requires). Unknown: none.
    fn vault_key_account(&self) -> Option<Identity> {
        let identity = match self.session_identity() {
            Ok(identity) => identity,
            Err(_) => Identity::new(self.vault().vault_key_owner().ok().flatten().as_deref(), None),
        };
        identity.is_known().then_some(identity)
    }

    /// Step 7 of the start, before the binding: this PC's local data for `session`. The upgrade's adoption runs here,
    /// inside the start and before the window closes: when `session` is the candidate the first startup fixed
    /// (`account_binding::is_the_upgrade_candidate`), unowned local data is recorded as its own (one transaction, never
    /// over a recorded owner), so this first start keeps everything. Then the window is shut, as at every start.
    /// Any error refuses the start as `Other` (nothing was started; the window is shut at the next start that reads).
    ///
    /// First, before anything is opened: while a stopped ownership proof that did not let go of its client in time is
    /// alive ([`ProofStop::Unconfirmed`]), the start is refused as `StopFinishing` (unlike an engine whose stop was
    /// never confirmed, `StopUnconfirmed`, which only a restart ends). The refusal ends by itself once that client is gone.
    pub(in crate::app) fn local_data_for_start(
        &self,
        sync_root: &Path,
        session: &Identity,
    ) -> Result<LocalState, SyncStartRefused> {
        #[cfg(any(target_os = "windows", test, feature = "test-support"))]
        if self.a_stopped_proof_is_alive() {
            return Err(SyncStartRefused::StopFinishing);
        }
        let data =
            LocalState::for_engine_start(&self.config.state_dir, sync_root).map_err(|_| SyncStartRefused::Other)?;
        let seat = self.own_data_seat();
        let candidate = lock(&seat.state).startup.candidate.clone();
        if account_binding::is_the_upgrade_candidate(candidate.as_ref(), session)
            && data
                .db
                .adopt_owner_for_startup_candidate(session)
                .map_err(|_| SyncStartRefused::Other)?
        {
            lock(&seat.state).startup.candidate = None;
            tracing::info!("upgrade: this account is recorded as the owner of the local data at its first start");
        }
        data.db.close_adoption_window().map_err(|_| SyncStartRefused::Other)?;
        Ok(data)
    }

    /// This handle's seat (made at the first ask).
    fn own_data_seat(&self) -> Arc<Seat> {
        let mut registry = lock(&SEATS);
        registry.retain(|(acct, _)| acct.strong_count() > 0);
        let mine = Arc::as_ptr(&self.acct);
        if let Some((_, seat)) = registry.iter().find(|(acct, _)| std::ptr::eq(acct.as_ptr(), mine)) {
            return Arc::clone(seat);
        }
        let seat = Arc::new(Seat::default());
        registry.push((Arc::downgrade(&self.acct), Arc::clone(&seat)));
        seat
    }
}

#[cfg(any(target_os = "windows", test, feature = "test-support"))]
impl<C: CredentialStores> Engine<C> {
    /// The session read at `turn` is still the session, and this handle's account is still the active one.
    fn proof_still_current(&self, turn: SessionGeneration) -> bool {
        self.acct.session_unchanged_since(turn)
            && self
                .accounts
                .active_account()
                .is_ok_and(|active| Arc::ptr_eq(&active, &self.acct))
    }

    /// The service thread's part: claim the turn, run the proof and wait for it, then act on its outcome. Blocks:
    /// never on the window's thread or in a task of the runtime.
    fn prove_owed(&self, owed: Owed) {
        let seat = self.own_data_seat();
        {
            let mut state = lock(&seat.state);
            #[cfg(test)]
            {
                state.test.owed_seen += 1;
            }
            if state.asked == Some(owed.turn) {
                return;
            }
            state.asked = Some(owed.turn);
            #[cfg(test)]
            {
                state.test.proofs_run += 1;
            }
        }
        let outcome = self.run_proof(&owed, &seat);
        if matches!(outcome, Some(Ok(Outcome::Unavailable(_))) | Some(Err(_))) {
            // Nothing was settled: a later start asks again.
            let mut state = lock(&seat.state);
            if state.asked == Some(owed.turn) {
                state.asked = None;
            }
        }
        #[cfg(test)]
        let kind = match &outcome {
            None => "stopped",
            Some(Err(_)) => "failed",
            Some(Ok(Outcome::Adopted(_))) => "adopted",
            Some(Ok(Outcome::Unavailable(_))) => "unavailable",
            Some(Ok(Outcome::NotProven(_))) => "not_proven",
            Some(Ok(Outcome::Changed(_))) => "changed",
            Some(Ok(Outcome::NotEligible)) => "not_eligible",
        };
        match outcome {
            None => tracing::info!("ownership proof: stopped"),
            Some(Err(_)) => tracing::warn!("ownership proof: could not run"),
            Some(Ok(Outcome::Adopted(tally))) => {
                tracing::info!(
                    checked = tally.checked,
                    own = tally.own,
                    neutral = tally.neutral,
                    gone = tally.gone,
                    proven_changes = tally.proven_changes,
                    held_for_review = tally.held_for_review,
                    "ownership proof: adopted, starting sync"
                );
                self.start_after_proof(owed);
            }
            Some(Ok(Outcome::Unavailable(tally))) => {
                tracing::warn!(
                    signed_out = tally.signed_out,
                    unavailable = tally.unavailable,
                    "ownership proof: no answer, nothing adopted"
                );
                if tally.signed_out == 0 {
                    self.refusal_becomes_identity_unknown(owed.turn);
                }
            }
            Some(Ok(Outcome::NotProven(tally))) => tracing::warn!(
                checked = tally.checked,
                own = tally.own,
                neutral = tally.neutral,
                gone = tally.gone,
                unproven = tally.unproven,
                missing = tally.missing,
                "ownership proof: not proven, nothing adopted"
            ),
            Some(Ok(Outcome::Changed(_))) => tracing::info!("ownership proof: the session changed, nothing adopted"),
            Some(Ok(Outcome::NotEligible)) => tracing::info!("ownership proof: nothing to prove for this session"),
        }
        #[cfg(test)]
        lock(&seat.state).test.outcomes.push(kind);
    }

    /// The proof `owed` asks for (the session read at its turn), as a task of the runtime that a Lock or a sign-out
    /// stops, and the wait for it. `None`: it was stopped. Its keys and identity are read together, as a start reads
    /// them (no session: nothing to prove). Its client is dropped with the task, before this returns.
    ///
    /// The keys are read, the client is made and the task is spawned and registered in ONE hold of the seat lock,
    /// after the stop count is compared with the one `owed` was made at: a stop either finds the task registered (and
    /// waits until its client is gone) or came first, and then no key is read. Nothing in that hold waits on the
    /// network, the step lock or the database; the database is opened before it.
    fn run_proof(&self, owed: &Owed, seat: &Seat) -> Option<Result<Outcome, String>> {
        let turn = owed.turn;
        if !self.proof_still_current(turn) {
            return Some(Ok(Outcome::Changed(Tally::default())));
        }
        let path = state_paths::state_db_path_from_state_dir(&self.config.state_dir);
        let db = path.exists().then(|| StateDb::open(&path));
        #[cfg(test)]
        seat.test_pause(|seams| seams.before_keys.clone());
        let task = {
            let mut state = lock(&seat.state);
            if state.stops != owed.stops {
                return None;
            }
            let Ok(((token, key), identity)) = start::keys_for_engine_start(&self.acct) else {
                return Some(Ok(Outcome::NotEligible));
            };
            let identity = self.with_recorded_user_id(identity);
            let db = match db {
                None => return Some(Ok(Outcome::NotEligible)),
                Some(Err(_)) => return Some(Err("open the local state".to_string())),
                Some(Ok(db)) => db,
            };
            let api = Arc::new(proof_client(self.config.api_base.clone(), String::clone(&token), *key));
            drop(token);
            #[cfg(target_os = "windows")]
            crate::windows_cf::track_credentials(&api);
            let list_timeout = LIST_TIMEOUT;
            #[cfg(test)]
            let (list_timeout, seam) = {
                state.test.clients.push(Arc::downgrade(&api));
                (
                    state.test.list_timeout.unwrap_or(list_timeout),
                    state.test.listing.clone(),
                )
            };
            let acct = Arc::clone(&self.acct);
            let proof = async move {
                let list = || {
                    let api = Arc::clone(&api);
                    #[cfg(test)]
                    let seam = seam.clone();
                    async move {
                        #[cfg(test)]
                        if let Some(seam) = seam {
                            let _client = api;
                            return tokio::time::timeout(list_timeout, seam())
                                .await
                                .unwrap_or(Listing::Unavailable);
                        }
                        let listed = async {
                            let snapshot = api.sync_snapshot().await;
                            let trashed = match &snapshot {
                                Ok(_) => api.list_trashed_files_all().await,
                                Err(_) => Ok(Vec::new()),
                            };
                            own_data_proof::listing_of(snapshot, trashed)
                        };
                        tokio::time::timeout(list_timeout, listed)
                            .await
                            .unwrap_or(Listing::Unavailable)
                    }
                };
                let still_current = || acct.session_unchanged_since(turn);
                let outcome = own_data_proof::prove_and_adopt(&db, &identity, &key, list, still_current).await;
                drop(api);
                outcome
            };
            let (ended, ended_rx) = tokio::sync::oneshot::channel::<()>();
            let task = self.runtime.spawn(EndsAfterRelease {
                proof: Box::pin(proof),
                _ended: ended,
            });
            state.running.retain(|running| !running.abort.is_finished());
            state.running.push(Running {
                abort: task.abort_handle(),
                ended: ended_rx,
            });
            task
        };
        #[cfg(test)]
        seat.test_pause(|seams| seams.after_spawn.clone());
        match self.runtime.block_on(task) {
            Ok(outcome) => Some(outcome),
            Err(error) if error.is_cancelled() => None,
            Err(_) => Some(Err("the proof ended early".to_string())),
        }
    }

    /// After an adoption: the normal start again, for the folder the refused start was asked for, while the session is
    /// still the one proven. The start takes the step lock itself and binds again under it; step 3 reads the keys
    /// there, so a Lock or a sign-out that won the lock leaves it nothing to start with. Its own refusal is recorded
    /// as any start's is.
    fn start_after_proof(&self, owed: Owed) {
        if !self.proof_still_current(owed.turn) {
            tracing::info!("ownership proof: the session changed, nothing to start");
            return;
        }
        #[cfg(test)]
        let port = lock(&self.own_data_seat().state).test.restart_port.clone();
        #[cfg(test)]
        if let Some(port) = port {
            let _ = self.start_on(owed.sync_root, &*port);
            return;
        }
        let _ = self.start_sync(owed.sync_root);
    }

    /// No answer at all: the refusal says to connect and open Beebeeb again, while it is still the refusal this proof
    /// answers and the session is still the one it was asked for.
    fn refusal_becomes_identity_unknown(&self, turn: SessionGeneration) {
        if !self.proof_still_current(turn) {
            return;
        }
        self.view.write(|view| {
            if view.start == Start::Refused(SyncStartRefused::OtherAccountsData) {
                view.start = Start::Refused(SyncStartRefused::IdentityUnknown);
            }
        });
        self.publish();
    }

    /// Settings › Sync, the card (S2b draws it): the changes held as found while paused, by kind, the ids Upload sends
    /// back, and the service (if any) that syncs the folder a keep moves them next to. Counts and ids only, never a
    /// name. `turn`: the session the card is read for ([`Self::session_turn`]). BLOCKS (the step lock): a worker's.
    pub fn found_while_paused_review_for(&self, turn: SessionGeneration) -> Result<Review, String> {
        let sync_root = self.decision_root()?;
        self.holding_step(|| self.found_while_paused_review_in(turn, &sync_root))
    }

    /// "Upload to my Beebeeb": the changes the card showed (`op_ids`, from the review) that are still held are due
    /// again, so the running engine uploads them; one held after the review stays held. Task 1888: a released change
    /// may be a ghost folder's create the reconcile pass left to the person, so the release asks for a pass before it
    /// commits and again after (the queue sends nothing while an ask is unserved). Returns how many were released.
    /// BLOCKS (the step lock): a worker's.
    pub fn found_while_paused_upload_for(&self, turn: SessionGeneration, op_ids: &[String]) -> Result<usize, String> {
        self.holding_step(|| self.found_while_paused_upload_in(turn, op_ids, unix_now()))
    }

    /// "Keep on this PC only": every item made on this PC that a change the card showed acts on moves next to the
    /// Beebeeb folder, into `Beebeeb - not uploaded\YYYY-MM-DD\`, by rename only and never over a name, and is
    /// forgotten (`found_while_paused::keep`). Counts only in the log. BLOCKS (the step lock): a worker's.
    pub fn found_while_paused_keep_for(
        &self,
        turn: SessionGeneration,
        op_ids: &[String],
    ) -> Result<KeepResult, String> {
        let sync_root = self.decision_root()?;
        self.holding_step(|| self.found_while_paused_keep_in(turn, op_ids, &sync_root, &today()))
    }

    /// "Show in Explorer" after a keep: the folder the last keep recorded, only when it is a dated folder directly
    /// inside `Beebeeb - not uploaded` next to the Beebeeb folder, and is there. The caller never passes a path.
    pub fn kept_folder_to_show_for(&self, turn: SessionGeneration) -> Result<PathBuf, String> {
        let sync_root = self.decision_root()?;
        self.kept_folder_to_show_in(turn, &sync_root)
    }

    /// The signed-in account's own local data, for a decision the card read in `turn`: refused when nobody is signed
    /// in, the local data's owner is not this session's account, or the session moved since `turn`.
    fn decision_state(&self, turn: SessionGeneration) -> Result<OwnedState<'_>, String> {
        match self.local_state_for_changes() {
            Ok(state) if state.session() == turn => Ok(state),
            _ => Err(FOUND_WHILE_PAUSED_NOT_YOURS.to_string()),
        }
    }

    /// The Beebeeb folder the settings name (never a path from the caller); none decides nothing.
    fn decision_root(&self) -> Result<PathBuf, String> {
        crate::config::DesktopConfig::load_for(self.identity)
            .ok()
            .and_then(|config| config.sync_root)
            .ok_or_else(|| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())
    }

    /// `work` under the step lock, so no Lock or sign-out runs in between. Never on a task of the runtime.
    fn holding_step<T>(&self, work: impl FnOnce() -> T) -> T {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            work()
        })
    }

    fn found_while_paused_review_in(&self, turn: SessionGeneration, sync_root: &Path) -> Result<Review, String> {
        let changes = self
            .decision_state(turn)?
            .with(|db| db.found_while_paused_changes())
            .map_err(|_| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())?
            .map_err(|_| {
                tracing::warn!("found while paused: the held changes could not be read");
                FOUND_WHILE_PAUSED_REVIEW_FAILED.to_string()
            })?;
        let mut review = found_while_paused::review(&changes);
        review.synced_by = kept_folder_synced_by(sync_root);
        Ok(review)
    }

    fn found_while_paused_upload_in(
        &self,
        turn: SessionGeneration,
        op_ids: &[String],
        now: i64,
    ) -> Result<usize, String> {
        let pass_wanted = &self.acct.reconcile_pass_wanted;
        let released = self
            .decision_state(turn)?
            .with(|db| {
                pass_wanted.fetch_add(1, Ordering::SeqCst);
                let released = self.release_held(db, op_ids, now);
                pass_wanted.fetch_add(1, Ordering::SeqCst);
                released
            })
            .map_err(|_| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())?
            .map_err(|_| {
                tracing::warn!("found while paused: the held changes could not be released");
                FOUND_WHILE_PAUSED_UPLOAD_FAILED.to_string()
            })?;
        tracing::info!(released, "found while paused: released to upload");
        Ok(released)
    }

    /// The release itself: only the ids in `op_ids` that are still held as found while paused.
    fn release_held(&self, db: &StateDb, op_ids: &[String], now: i64) -> rusqlite::Result<usize> {
        #[cfg(test)]
        lock(&self.own_data_seat().state)
            .test
            .asks_at_release
            .push(self.acct.reconcile_pass_wanted.load(Ordering::SeqCst));
        db.release_found_while_paused(op_ids, now)
    }

    fn found_while_paused_keep_in(
        &self,
        turn: SessionGeneration,
        op_ids: &[String],
        sync_root: &Path,
        today: &str,
    ) -> Result<KeepResult, String> {
        let mut kept = self
            .decision_state(turn)?
            .with(|db| found_while_paused::keep(db, sync_root, today, &found_while_paused::rename_no_replace, op_ids))
            .map_err(|_| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())?
            .map_err(|_| {
                tracing::warn!("found while paused: the keep stopped");
                FOUND_WHILE_PAUSED_KEEP_STOPPED.to_string()
            })?;
        kept.synced_by = kept_folder_synced_by(sync_root);
        tracing::info!(
            moved_items = kept.moved_items,
            moved_files = kept.moved_files,
            not_moved = kept.not_moved,
            stays_held = kept.stays_held,
            "found while paused: kept on this pc"
        );
        Ok(kept)
    }

    fn kept_folder_to_show_in(&self, turn: SessionGeneration, sync_root: &Path) -> Result<PathBuf, String> {
        let recorded = self
            .decision_state(turn)?
            .with(|db| db.last_kept_dir())
            .map_err(|_| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())?;
        kept_folder_to_show(sync_root, recorded)
    }
}

/// The proof's client: this session's, with a request time-out for a whole tree (Windows; elsewhere only tests run a
/// proof, and their listing is their own).
#[cfg(target_os = "windows")]
fn proof_client(base: String, token: String, key: [u8; 32]) -> ApiClient {
    ApiClient::new(base, token, key).with_request_timeout(PROOF_REQUEST_TIMEOUT)
}

#[cfg(all(any(test, feature = "test-support"), not(target_os = "windows")))]
fn proof_client(base: String, token: String, key: [u8; 32]) -> ApiClient {
    ApiClient::new(base, token, key)
}

/// The service that syncs the folder a keep moves items next to (the Beebeeb folder's parent), if OneDrive or Dropbox
/// does.
#[cfg(target_os = "windows")]
fn kept_folder_synced_by(sync_root: &Path) -> Option<String> {
    let parent = sync_root.parent()?;
    found_while_paused::synced_by(parent, &found_while_paused::cloud_synced_roots()).map(str::to_string)
}

#[cfg(all(any(test, feature = "test-support"), not(target_os = "windows")))]
fn kept_folder_synced_by(_sync_root: &Path) -> Option<String> {
    None
}

/// Today's local date, as the dated folder is named.
#[cfg(target_os = "windows")]
fn today() -> String {
    found_while_paused::today()
}

#[cfg(all(any(test, feature = "test-support"), not(target_os = "windows")))]
fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[cfg(any(target_os = "windows", test, feature = "test-support"))]
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// The folder "Show in Explorer" opens: the dated folder the last keep recorded (`recorded`), only when it is a
/// `YYYY-MM-DD` folder directly inside `Beebeeb - not uploaded` next to the Beebeeb folder, and is there.
#[cfg(any(target_os = "windows", test, feature = "test-support"))]
fn kept_folder_to_show(sync_root: &Path, recorded: rusqlite::Result<Option<String>>) -> Result<PathBuf, String> {
    let folder = PathBuf::from(
        recorded
            .map_err(|_| KEPT_FOLDER_GONE.to_string())?
            .ok_or(KEPT_FOLDER_GONE)?,
    );
    let kept = sync_root
        .parent()
        .map(|parent| parent.join(found_while_paused::KEPT_FOLDER_NAME));
    let dated = folder.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
        name.len() == 10
            && name.bytes().enumerate().all(|(at, byte)| {
                if at == 4 || at == 7 {
                    byte == b'-'
                } else {
                    byte.is_ascii_digit()
                }
            })
    });
    if dated && kept.is_some_and(|kept| folder.parent() == Some(kept.as_path())) && folder.is_dir() {
        Ok(folder)
    } else {
        Err(KEPT_FOLDER_GONE.to_string())
    }
}
