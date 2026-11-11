//! `bound_engine` items split out of `lib.rs`.

use super::*;

/// R10: who a session is, from the session's OWN email and, when the cached profile is that account's, its
/// user id. The profile counts only when its email is exactly the session's, so a profile left over from
/// another account is never taken for this one; a session with no email is unidentified (and an owner that
/// cannot be compared with it refuses the start).
pub(super) fn identity_of_session(
    email: Option<&str>,
    profile: Option<&account_dto::AccountProfile>,
) -> account_binding::Identity {
    let email = email.map(str::trim).filter(|email| !email.is_empty());
    let profile = profile.filter(|profile| email.is_some_and(|email| profile.email.trim() == email));
    account_binding::Identity::new(profile.map(|profile| profile.user_id.as_str()), email)
}

/// R10: bind `data` to `session` before an engine starts. Another account's data is reset first
/// (refused on Windows). An owner that cannot be compared stops the start and deletes nothing. Any
/// error means: do not start the engine.
pub(super) fn bind_data_to_session(
    state: &AppState,
    data: &StateDbLocalData,
    session: &account_binding::Identity,
) -> Result<(), String> {
    let bound = account_binding::bind_before_engine_start(data, session, !cfg!(target_os = "windows"));
    // W12: a reset that kept files hands its folder on whatever the binding answered. A reset whose purge failed after
    // a completed keep returns an error, and the folder it recorded is on disk either way.
    #[cfg(target_os = "macos")]
    hand_on_reset_kept(state, data);
    match bound? {
        account_binding::Bound::Kept => {}
        account_binding::Bound::Reset => {
            tracing::warn!("R10: local data of another account was reset before sync started");
            // Finder may still list what the reset removed. A Repair removes Beebeeb and adds it back.
            // Never awaited: the reconciler itself starts engines through this function.
            #[cfg(target_os = "macos")]
            notify_finder(state, finder_setup::core::Trigger::Repair);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = state;
    Ok(())
}

/// W12: an R10 reset that kept files hands its folder to the state, to be surfaced once the start returns (the row,
/// then one alert). Taken out, so it is surfaced once.
#[cfg(target_os = "macos")]
pub(super) fn hand_on_reset_kept(state: &AppState, data: &StateDbLocalData) {
    if let Some(kept) = data.kept.lock().unwrap_or_else(|p| p.into_inner()).take()
        && let Some(folder) = kept.folder
    {
        *state.sign_out.reset_kept.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((folder.to_string_lossy().into_owned(), kept.files));
    }
}

/// Whether the engine may start once the local data is bound.
pub(super) enum BoundData {
    Ready,
    /// A Finder domain removal is owed (see [`EngineStart::FinderRemovalOwed`]).
    FinderRemovalOwed,
}

/// R10: bind this computer's local data to the session's account before an engine starts. A start refuses while a
/// Finder domain removal is owed, before it binds anything; and a reset on a Mac, which leaves the previous
/// account's domain registered, records that debt itself, so the engine waits for the removal Repair asks for.
pub(super) fn bind_local_data_to_session(
    state: &AppState,
    identity: &account_binding::Identity,
    paths: &LocalDataPaths,
) -> Result<BoundData, String> {
    let data = StateDbLocalData::for_engine_start(paths)?;
    if data
        .db
        .finder_removal_owed()
        .map_err(|e| format!("read the Finder removal debt: {e}"))?
    {
        return Ok(BoundData::FinderRemovalOwed);
    }
    // A first start onto an UNOWNED, EMPTY database (R3, fix round 2) owes the removal too, as a reset does: the
    // database may have been deleted, or never existed, while the previous account's Finder domain stayed
    // registered. The reconciler's next check settles it (an Observe that finds no domain releases the debt).
    // Not owed when a sign-out whose Finder removal was confirmed left the domain known gone.
    let first_owner_here = cfg!(target_os = "macos")
        && data
            .db
            .owner()
            .map_err(|e| format!("read the local data owner: {e}"))?
            .is_none()
        && !data
            .db
            .has_account_data()
            .map_err(|e| format!("inspect the local data: {e}"))?
        && !data
            .db
            .finder_domain_gone()
            .map_err(|e| format!("read the Finder domain mark: {e}"))?;
    if first_owner_here {
        data.db
            .set_finder_removal_owed(true)
            .map_err(|e| format!("record the Finder removal debt: {e}"))?;
        // Ask for a check so the reconciler looks (it may have no reason to otherwise).
        #[cfg(target_os = "macos")]
        notify_finder(state, finder_setup::core::Trigger::KeysArrived);
    }
    bind_data_to_session(state, &data, identity)?;
    if data
        .db
        .finder_removal_owed()
        .map_err(|e| format!("read the Finder removal debt: {e}"))?
    {
        return Ok(BoundData::FinderRemovalOwed);
    }
    Ok(BoundData::Ready)
}

/// Everything an engine start needs decided before the engine exists, with the caller holding the
/// engine slot (the guard is the proof, and its emptiness is checked). In this order: the keys and the
/// identity of their session, read together under the slot; then the binding of the local data to that
/// identity. `NoSession`: the vault is locked, so there is nothing to start. Any `Err` means: do not start
/// the engine. A binding that failed is returned with `?` and nothing after it runs
/// (`every_engine_start_is_bound_first`).
pub(super) fn authorize_engine_start(
    state: &AppState,
    acct: &account::AccountRuntime,
    engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    paths: &LocalDataPaths,
) -> Result<StartPermit, String> {
    if engine_slot.is_some() {
        return Err(ENGINE_SLOT_OCCUPIED_ERROR.to_string());
    }
    let (keys, identity) = match keys_for_engine_start(acct) {
        Ok(read) => read,
        Err(EngineStartBlocked::NoSession) => return Ok(StartPermit::NoSession),
        Err(refused) => return Err(refused.to_string()),
    };
    // A session that does not know who it is (its sign-in could not fetch the account record) starts nothing and
    // touches nothing: no database opened, no adoption, no purge, no Finder debt. The same message as an owner
    // that cannot be compared: it tells the person to connect and open Beebeeb again.
    if !identity.is_known() {
        return Err(account_binding::IDENTITY_UNKNOWN.to_string());
    }
    let bound = bind_local_data_to_session(state, &identity, paths)?;
    match bound {
        BoundData::Ready => Ok(StartPermit::Go(keys)),
        BoundData::FinderRemovalOwed => Ok(StartPermit::FinderRemovalOwed),
    }
}

/// The start itself, with the engine's constructor passed in: authorize (keys and identity under the slot,
/// then the binding), and only then call `spawn` and put what it returns in the slot. Production passes the
/// one real constructor (see [`spawn_bound_engine`]); a test passes one that looks at the world at the exact
/// moment an engine would start.
pub(super) fn start_engine_bound(
    state: &AppState,
    acct: &account::AccountRuntime,
    engine_slot: &mut tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    paths: &LocalDataPaths,
    spawn: impl FnOnce(PathBuf, zeroize::Zeroizing<String>, zeroize::Zeroizing<[u8; 32]>) -> EngineRunner,
) -> Result<EngineStart, String> {
    let turn = acct.session_generation();
    // Ruling F9: the ask is read before the keys are, so it is served only by a start that holds the session of the
    // sign-in that asked, or a later one. A Mac's only: off macOS the engine serves it after its reconcile pass (spec
    // D11, task 1888).
    #[cfg(target_os = "macos")]
    let resume_asked = acct.auth_resume_asked.load(Ordering::SeqCst);
    let (token, key) = match authorize_engine_start(state, acct, &*engine_slot, paths)
        .inspect_err(|error| record_engine_refusal(acct, turn, error))?
    {
        StartPermit::Go(keys) => keys,
        StartPermit::NoSession => return Ok(EngineStart::NoSession),
        StartPermit::FinderRemovalOwed => return Ok(EngineStart::FinderRemovalOwed),
    };
    #[cfg(target_os = "macos")]
    resume_operations_paused_for_auth(acct, paths, resume_asked);
    // Task 2267 (fix round 2): a "Keep on this PC only" a crash interrupted is settled before the engine exists.
    #[cfg(target_os = "windows")]
    settle_an_interrupted_keep(paths);
    **engine_slot = Some(spawn(paths.sync_root.clone(), token, key));
    *acct
        .engine_refusal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    Ok(EngineStart::Started)
}

/// Lead ruling F9 (spec 2026-10-06 R8: "sync resumes"): a sign-in that has put a session in memory asks for the
/// operations paused for `auth` to be due again, and so does the startup check that confirmed the stored token (F9
/// review I-2). Only inside the writer's checked turn (`_write`) and after the session is in memory. A Mac's engine
/// start serves the ask (see [`resume_operations_paused_for_auth`]); since spec D11 (task 1888) Windows and Linux ask
/// too, and their engine serves it after its reconcile pass and the key check
/// ([`engine_bridge::EngineBridge::reconcile_then_resume`]).
pub(super) fn ask_to_resume_auth_paused(_write: &SessionWrite, acct: &AccountRuntime) {
    acct.auth_resume_asked.fetch_add(1, Ordering::SeqCst);
}

/// Lead ruling, F9 review I-1 (spec 2026-10-06 R8, §5.6): the vault key this Mac kept is no longer the account's. Every
/// operation paused for `auth` was queued with its names encrypted under that key, so it is re-marked `key_replaced`:
/// kept, never sent, and never made due by any resume (a sign-in's or the startup check's). Only under the engine slot
/// (`_engine_slot`), after the caller took the engine out of it, so no engine pauses another one for `auth` afterwards and
/// no start resumes them in between. No `state.db` yet: nothing is queued. macOS only, like the resume it guards: Windows
/// and Linux keep today's flow (spec R8).
#[cfg(target_os = "macos")]
pub(super) fn hold_auth_paused_for_the_replaced_key(
    sources: &LocalSources,
    _engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
) -> Result<usize, String> {
    let held = match sources.state_db()? {
        Some(db) => db
            .hold_operations_paused_for_auth_as_key_replaced(now_unix_seconds())
            .map_err(|e| format!("hold the operations queued under the replaced key: {e}"))?,
        None => 0,
    };
    tracing::info!(
        held,
        "the vault key was replaced: the operations paused for auth are kept and not sent"
    );
    Ok(held)
}

/// F9 review M2, with lead ruling I-1 (spec §5.6): a browser re-sign-in whose handed-over key is not the one this Mac
/// kept. The caller swapped the token while holding the engine slot and still holds it (`engine_slot`), so no start has
/// served the swap's ask. The outgoing engine stops first (its token was revoked, so it pauses every edit it tries for
/// `auth`); only then is every `auth` pause held as `key_replaced` ([`hold_auth_paused_for_the_replaced_key`]). A start
/// after this, with the outgoing key or the handed-over one, finds nothing queued under the outgoing key to resume.
#[cfg(target_os = "macos")]
pub(super) async fn stop_the_outgoing_engine_and_hold_auth_paused(
    acct: &AccountRuntime,
    sources: &LocalSources,
    engine_slot: &mut tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
) -> Result<usize, String> {
    if let Some(engine) = engine_slot.take()
        && !stop_engine_in_slot(acct, engine).await.is_stopped()
    {
        tracing::error!("the sync engine did not confirm it stopped before the edits under the kept key were held");
    }
    if let Ok(mut engine_state) = acct.engine_state.lock() {
        *engine_state = "stopped".to_string();
    }
    hold_auth_paused_for_the_replaced_key(sources, engine_slot)
}

/// Lead ruling F9 (spec 2026-10-06 R8 and §5.6): when a sign-in asked for it (`asked`, read before this start read its
/// keys) and no start has served that ask yet, every operation paused for `auth` becomes due now, its attempts kept. It
/// runs under the engine slot, after the binding (so another account's operations were already reset) and before the
/// engine exists (so its first tick sends them). Operations paused for any other reason stay paused, those held as
/// `key_replaced` included (F9 review I-1). A database that cannot be written leaves the ask open, so the next engine
/// start tries again (F9 review M3); the engine starts either way. macOS only (spec D11, task 1888): off macOS the
/// engine resumes them after its reconcile pass.
#[cfg(target_os = "macos")]
pub(super) fn resume_operations_paused_for_auth(acct: &AccountRuntime, paths: &LocalDataPaths, asked: u64) {
    if asked <= acct.auth_resume_done.load(Ordering::SeqCst) {
        return;
    }
    let path = state_paths::state_db_path_from_state_dir(&paths.state_dir);
    match state_db::StateDb::open(&path).and_then(|db| db.resume_operations_paused_for_auth(now_unix_seconds())) {
        Ok(resumed) => {
            acct.auth_resume_done.store(asked, Ordering::SeqCst);
            tracing::info!(resumed, "signed in: the operations paused for auth are due again");
        }
        Err(error) => tracing::warn!(
            %error,
            "signed in, but the operations paused for auth could not be made due; the next engine start tries again"
        ),
    }
}

/// Ruling P (Task 12 fix round 2): a start refused by the binding leaves its closed code on the account, so
/// `sync_status` can show the sentence instead of a stopped engine with no reason. Only the two binding refusals
/// count ([`account_binding::Refusal::of`]); any other error records nothing. Recorded only while the session is still
/// the one this start read (`turn`): a transition since then cleared the field and wins.
pub(super) fn record_engine_refusal(acct: &account::AccountRuntime, turn: account::SessionGeneration, error: &str) {
    let Some(refusal) = account_binding::Refusal::of(error) else {
        return;
    };
    let mut recorded = acct
        .engine_refusal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if acct.session_unchanged_since(turn) {
        *recorded = Some(refusal);
    }
}

/// `sync_status.engine_refusal`: `{code, sentence}` for the last binding refusal of the current session, else null.
pub(super) fn engine_refusal_view(acct: &account::AccountRuntime) -> serde_json::Value {
    match *acct
        .engine_refusal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
    {
        Some(refusal) => serde_json::json!({ "code": refusal.code(), "sentence": refusal.sentence() }),
        None => serde_json::Value::Null,
    }
}

/// R10 (spec 2026-10-06 §5.6; lead ruling T9-starts): the ONLY place a sync engine starts. The caller
/// holds the engine slot (it passes the guard) and has stopped any earlier engine in it. This reads the
/// session and its keys under that slot, binds the local data to the session's account, and only then
/// spawns, so:
///  - there is no key or token parameter: a Lock that cleared the session before this start got the
///    slot finds nothing to start with, and no engine exists after the Lock returned;
///  - another account's local data is reset first, and any error (a failed reset, an owner that cannot
///    be compared, Windows refusing) means no engine starts.
///
/// The keys reach the engine as `Zeroizing` values. `every_engine_start_is_bound_first` pins that this is
/// the one start in production code and that the binding comes first.
pub(super) fn spawn_bound_engine(
    app: tauri::AppHandle,
    state: &AppState,
    acct: &account::AccountRuntime,
    engine_slot: &mut tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    sync_root: PathBuf,
) -> Result<EngineStart, String> {
    let paths = LocalDataPaths::production(&sync_root)?;
    let surface_app = app.clone();
    let started = start_engine_bound(state, acct, engine_slot, &paths, |sync_root, token, key| {
        EngineRunner::spawn(
            crate::engine_host::host(&app),
            sync_root,
            token,
            key,
            acct.sync_paused.clone(),
            acct.auth_health.clone(),
            runner::ResumeTicket {
                asked: acct.auth_resume_asked.clone(),
                done: acct.auth_resume_done.clone(),
                pass_wanted: acct.reconcile_pass_wanted.clone(),
                pass_served: acct.reconcile_pass_served.clone(),
            },
        )
    });
    // W12: the R10 reset's kept folder is surfaced as the app-start sweep surfaces macOS's: the row, then one alert.
    let reset_kept = state
        .sign_out
        .reset_kept
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    if let Some((folder, files)) = reset_kept {
        surface_reset_folder(&surface_app, &folder, files);
    }
    // Task 2267: a Windows start refused over local data that no owner record matches asks, in the background, whether
    // that data is this account's own, and sync starts once it is proven (`prove_own_data_in_background`).
    // The proof runs for the session this start read: the same account runtime, the turn read under the engine slot.
    #[cfg(target_os = "windows")]
    if matches!(&started, Err(error) if error == account_binding::OTHER_ACCOUNT_ON_WINDOWS)
        && let Ok(active) = state.active_account()
        && std::ptr::eq(Arc::as_ptr(&active), acct)
    {
        prove_own_data_in_background(surface_app, paths.sync_root.clone(), active, acct.session_generation());
    }
    started
}

/// Task 2267: the background half of a Windows start refused with `OTHER_ACCOUNT_ON_WINDOWS`. It proves the local
/// data is the signed-in account's own (`own_data_proof`: its My Files rows decrypt under this session's key), records
/// the session as the owner, and starts sync through the usual start, which binds again and now keeps everything.
/// Another account's data is never adopted and nothing is deleted.
///
/// One proof per session (account runtime and turn) at a time: a start refused again while it runs asks nothing new,
/// a proof for a session that has since changed does not hold the next session's back, and the slot is freed however
/// the proof ends, a panic included. Sync starts only for the session that was proven, checked under the
/// session-transition lock, so a Lock or a sign-out since leaves nothing to start.
#[cfg(target_os = "windows")]
pub(super) fn prove_own_data_in_background(
    app: tauri::AppHandle,
    sync_root: PathBuf,
    acct: Arc<account::AccountRuntime>,
    turn: account::SessionGeneration,
) {
    type Key = (usize, account::SessionGeneration);
    static IN_FLIGHT: std::sync::Mutex<Option<Key>> = std::sync::Mutex::new(None);
    struct Flight(Key);
    impl Drop for Flight {
        fn drop(&mut self) {
            let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
            if *in_flight == Some(self.0) {
                *in_flight = None;
            }
        }
    }
    let key = (Arc::as_ptr(&acct) as usize, turn);
    {
        let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
        if *in_flight == Some(key) {
            return;
        }
        *in_flight = Some(key);
    }
    let flight = Flight(key);
    let proof = tauri::async_runtime::spawn(async move {
        let _flight = flight;
        match prove_own_data_of_session(&app, &sync_root, &acct, turn).await {
            Ok(own_data_proof::Outcome::Adopted(tally)) => {
                tracing::info!(
                    ?tally,
                    "R10: this PC's local data is proven this account's own; starting sync"
                );
                let transition = SESSION_TRANSITION.lock().await;
                if !session_still_active(&app, &acct, turn) {
                    tracing::info!("R10: the session changed after the proof; nothing to start");
                    return;
                }
                let state = app.state::<AppState>();
                if let Err(error) = start_engine_if_possible(app.clone(), &state, &transition).await {
                    tracing::warn!(%error, "R10: the start after the proof was refused");
                }
            }
            Ok(outcome) => tracing::warn!(?outcome, "R10: this PC's local data was not adopted"),
            Err(error) => tracing::warn!(%error, "R10: the ownership proof could not run"),
        }
    });
    let mut proofs = OWN_DATA_PROOFS.lock().unwrap_or_else(|p| p.into_inner());
    proofs.retain(|running| !running.inner().is_finished());
    proofs.push(proof);
}

/// Task 2267 (review P2-2): the ownership proofs that may still be running. Each holds the session's key and token
/// (its client is registered with `windows_cf::track_credentials`), so a Lock and a sign-out stop them
/// (`stop_own_data_proofs`) before they wait for every credential to be released.
#[cfg(target_os = "windows")]
pub(super) static OWN_DATA_PROOFS: std::sync::Mutex<Vec<tauri::async_runtime::JoinHandle<()>>> =
    std::sync::Mutex::new(Vec::new());

/// Stop every running ownership proof. Both callers hold the session-transition lock, which a proof takes only to
/// start sync after an adoption, so a proof is never stopped half way through a start. A proof stopped after it
/// recorded the owner has recorded this same account's own data; one stopped before records nothing.
#[cfg(target_os = "windows")]
pub(super) fn stop_own_data_proofs() {
    let proofs = std::mem::take(&mut *OWN_DATA_PROOFS.lock().unwrap_or_else(|p| p.into_inner()));
    for proof in proofs {
        proof.abort();
    }
}

/// Whether `acct` is still the active account and its session is still the one read at `turn`.
#[cfg(target_os = "windows")]
pub(super) fn session_still_active(
    app: &tauri::AppHandle,
    acct: &Arc<account::AccountRuntime>,
    turn: account::SessionGeneration,
) -> bool {
    acct.session_unchanged_since(turn)
        && app
            .state::<AppState>()
            .active_account()
            .is_ok_and(|active| Arc::ptr_eq(&active, acct))
}

/// How long the proof waits for the whole listing of this account's tree (the snapshot, then the trash, with the
/// client's retries). A Lock or a sign-out does not wait for it: they stop the proof first.
#[cfg(target_os = "windows")]
pub(super) const LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Self-review F5: how long one request of the proof's listing may take. The snapshot is the whole tree in one response,
/// so the app's 30 s per request would refuse a large tree on a slow link every time, and say "connect to the internet"
/// to someone who is online. Nobody waits on the proof (it runs in the background, and a Lock or a sign-out stops it).
#[cfg(target_os = "windows")]
pub(super) const PROOF_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The proof for the session read at `turn`, with its keys and identity read together as an engine start reads them.
/// A session that changes stops the asking and records nothing (`session_still_active`). With no answer at all the
/// refusal becomes the one that says to connect and open Beebeeb again; a token the server refused is not "offline",
/// so that sentence is not shown for it. The next start asks again.
#[cfg(target_os = "windows")]
pub(super) async fn prove_own_data_of_session(
    app: &tauri::AppHandle,
    sync_root: &Path,
    acct: &Arc<account::AccountRuntime>,
    turn: account::SessionGeneration,
) -> Result<own_data_proof::Outcome, String> {
    if !session_still_active(app, acct, turn) {
        return Ok(own_data_proof::Outcome::Changed(own_data_proof::Tally::default()));
    }
    let Ok(((token, key), identity)) = keys_for_engine_start(acct) else {
        return Ok(own_data_proof::Outcome::NotEligible);
    };
    let paths = LocalDataPaths::production(sync_root)?;
    let Some(db) = state_db_from_state_dir(&paths.state_dir)? else {
        return Ok(own_data_proof::Outcome::NotEligible);
    };
    let api = Arc::new(
        api_client::ApiClient::new(runner::api_base_url(), String::clone(&token), *key)
            .with_request_timeout(PROOF_REQUEST_TIMEOUT),
    );
    windows_cf::track_credentials(&api);
    let outcome = own_data_proof::prove_and_adopt(
        &db,
        &identity,
        &key,
        || {
            let api = Arc::clone(&api);
            async move {
                let listed = tokio::time::timeout(LIST_TIMEOUT, async {
                    let snapshot = api.sync_snapshot().await;
                    let trashed = match &snapshot {
                        Ok(_) => api.list_trashed_files_all().await,
                        Err(_) => Ok(Vec::new()),
                    };
                    own_data_proof::listing_of(snapshot, trashed)
                })
                .await;
                listed.unwrap_or(own_data_proof::Listing::Unavailable)
            }
        },
        || session_still_active(app, acct, turn),
    )
    .await?;
    drop(api);
    if matches!(outcome, own_data_proof::Outcome::Unavailable(tally) if tally.signed_out == 0) {
        record_engine_refusal(acct, turn, account_binding::IDENTITY_UNKNOWN);
    }
    Ok(outcome)
}

/// Task 2267 (fix round 2): what Upload and Keep say when this PC's local data is not the signed-in account's own.
#[cfg(any(target_os = "windows", test))]
pub(super) const FOUND_WHILE_PAUSED_NOT_YOURS: &str =
    "Beebeeb couldn’t confirm these files belong to the account you’re signed in to, so nothing was changed.";

/// Task 2267 (fix round 2): what Keep says when it stopped part way (the database could not be written). Every item it
/// moved is forgotten in its own transaction, or settled at the next start (`settle_an_interrupted_keep`).
#[cfg(target_os = "windows")]
pub(super) const FOUND_WHILE_PAUSED_KEEP_STOPPED: &str = "Beebeeb couldn’t finish moving these files. Anything already moved is in “Beebeeb - not uploaded”; the rest stays where it is and isn’t uploaded.";

/// Task 2267 (lead review round 3, P2-2): what the card says when it cannot read the held changes. It reads them again
/// at the next refresh; Upload waits for that read, since it sends only what the card showed.
#[cfg(target_os = "windows")]
pub(super) const FOUND_WHILE_PAUSED_REVIEW_FAILED: &str =
    "Beebeeb couldn’t read these changes, so nothing was uploaded or moved. It tries again in a moment.";

/// Task 2267 (fix round 2): what Upload says when the held changes could not be released: nothing changed.
#[cfg(target_os = "windows")]
pub(super) const FOUND_WHILE_PAUSED_UPLOAD_FAILED: &str =
    "Beebeeb couldn’t start uploading these files, so they’re still waiting. Try again.";

/// Task 2267 (fix round 2): what "Show in Explorer" says when the folder the last keep recorded is not there.
#[cfg(any(target_os = "windows", test))]
pub(super) const KEPT_FOLDER_GONE: &str =
    "Beebeeb can’t find the folder it moved them to. It may have been moved or renamed.";

/// Task 2267 (fix round 2): who is signed in, for a decision about the changes found while sync was paused: the
/// session's own email and, when the cached profile is that account's, its user id ([`identity_of_session`]). No key is
/// read. No session, or one that does not know who it is, decides nothing.
#[cfg(any(target_os = "windows", test))]
pub(super) fn decision_identity(state: &AppState) -> Result<account_binding::Identity, String> {
    let not_yours = || FOUND_WHILE_PAUSED_NOT_YOURS.to_string();
    let acct = state.active_account().map_err(|_| not_yours())?;
    let email = acct
        .session
        .lock()
        .map_err(|_| not_yours())?
        .as_ref()
        .map(|session| session.email.clone())
        .ok_or_else(not_yours)?;
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
    Some(identity_of_session(email.as_deref(), profile.as_ref()))
        .filter(|identity| identity.is_known())
        .ok_or_else(not_yours)
}

/// Task 2267 (fix round 2): this PC's `state.db` under `state_dir`, only when its recorded owner is `identity`'s
/// account. Another account's data, data nobody owns yet and no database decide nothing.
#[cfg(any(target_os = "windows", test))]
pub(super) fn own_state_db(
    identity: &account_binding::Identity,
    state_dir: &Path,
) -> Result<state_db::StateDb, String> {
    let db = state_db_from_state_dir(state_dir)?.ok_or_else(|| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())?;
    let owner = db.owner().map_err(|e| format!("read the local data owner: {e}"))?;
    match owner {
        Some(owner) if account_binding::same_account(&owner, identity) == Some(true) => Ok(db),
        _ => Err(FOUND_WHILE_PAUSED_NOT_YOURS.to_string()),
    }
}

/// Task 2267 (fix round 2): the signed-in account's own local data and the Beebeeb folder, for Upload and Keep.
#[cfg(target_os = "windows")]
pub(super) fn own_local_data_for_decision(
    identity: &account_binding::Identity,
) -> Result<(state_db::StateDb, PathBuf), String> {
    let sync_root = DesktopConfig::load()?
        .sync_root
        .ok_or_else(|| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())?;
    Ok((own_state_db(identity, &state_paths::beebeeb_state_dir()?)?, sync_root))
}

/// Task 2267 (fix round 2): the folder "Show in Explorer" opens: the dated folder the last keep recorded, only when it
/// is a `YYYY-MM-DD` folder directly inside `Beebeeb - not uploaded` next to the Beebeeb folder, and is there.
#[cfg(any(target_os = "windows", test))]
pub(super) fn kept_folder_to_show(
    sync_root: &Path,
    recorded: rusqlite::Result<Option<String>>,
) -> Result<PathBuf, String> {
    let folder = PathBuf::from(
        recorded
            .map_err(|e| format!("read the kept folder: {e}"))?
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

/// Task 2267 (fix round 2, lead ruling (a) 4): a "Keep on this PC only" that a crash interrupted is settled before the
/// engine exists: finished when the item had moved, undone when it had not (`found_while_paused::recover`). The engine
/// start calls it after the binding, so it acts on this account's own data only. A database that cannot be read or
/// written leaves the journal for the next start; the engine starts either way, and the item's changes stay held.
#[cfg(target_os = "windows")]
pub(super) fn settle_an_interrupted_keep(paths: &LocalDataPaths) {
    let path = state_paths::state_db_path_from_state_dir(&paths.state_dir);
    let settled = state_db::StateDb::open(&path)
        .map_err(|e| format!("open state.db: {e}"))
        .and_then(|db| found_while_paused::recover(&db, &paths.sync_root));
    match settled {
        Ok(found_while_paused::Recovery::Nothing) => {}
        Ok(recovery) => tracing::info!(
            ?recovery,
            "keep: a move a crash interrupted was settled before sync started"
        ),
        Err(error) => tracing::warn!(%error, "keep: a move a crash interrupted is settled at the next start"),
    }
}

/// Task 2267 (fix round 2), Settings › Sync: how many changes "Upload to my Beebeeb" released.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub(super) struct FoundWhilePausedReleased {
    pub(super) released: usize,
}

/// Task 2267 (lead review round 3, P2-2), Settings › Sync: the held changes by kind, the ids "Upload to my Beebeeb"
/// sends back, and which service (if any) syncs the folder a keep moves them next to. Only on the signed-in account's
/// own data, under the session-transition lock. Counts and ids only, never a name.
#[cfg(target_os = "windows")]
#[tauri::command]
pub(super) async fn found_while_paused_review(
    state: State<'_, AppState>,
) -> Result<found_while_paused::Review, String> {
    let _transition = SESSION_TRANSITION.lock().await;
    let identity = decision_identity(&state)?;
    tokio::task::spawn_blocking(move || -> Result<found_while_paused::Review, String> {
        let (db, sync_root) = own_local_data_for_decision(&identity)?;
        let mut review = found_while_paused::review(&db.found_while_paused_changes().map_err(|error| {
            tracing::warn!(%error, "found while paused: the held changes could not be read");
            FOUND_WHILE_PAUSED_REVIEW_FAILED.to_string()
        })?);
        review.synced_by = kept_folder_synced_by(&sync_root);
        Ok(review)
    })
    .await
    .map_err(|e| format!("review task failed: {e}"))?
}

/// Task 2267 (lead review round 3, P3-4): the service that syncs the folder the kept folder is made in (next to the
/// Beebeeb folder), if OneDrive or Dropbox does.
#[cfg(target_os = "windows")]
pub(super) fn kept_folder_synced_by(sync_root: &Path) -> Option<String> {
    let parent = sync_root.parent()?;
    found_while_paused::synced_by(parent, &found_while_paused::cloud_synced_roots()).map(str::to_string)
}

/// Task 2267 (fix round 2), Settings › Sync, "Upload to my Beebeeb": the changes the card showed (`op_ids`, from
/// [`found_while_paused_review`]; lead review round 3, P3-1) that are still held are due again, so the running engine
/// uploads them. A change held after the card read them stays held. Only on the signed-in account's own data, and
/// under the session-transition lock, so no sign-out or Lock runs in between.
#[cfg(target_os = "windows")]
#[tauri::command]
pub(super) async fn found_while_paused_upload(
    state: State<'_, AppState>,
    op_ids: Vec<String>,
) -> Result<FoundWhilePausedReleased, String> {
    let _transition = SESSION_TRANSITION.lock().await;
    let identity = decision_identity(&state)?;
    // Task 1888 (Task 1's ruling on 2267's held changes; review P2): a released change may be a ghost folder's create
    // the reconcile pass left to the person; sent before a pass sees it, it would make a folder the server already has
    // a second time. So the release asks for a pass, before it commits and again after: the engine's queue sends
    // nothing while an ask is unserved, and its next successful tick runs the pass (no resume: a release is not a
    // sign-in). Without the account there is no ask, so nothing is released.
    let pass_wanted = state
        .active_account()
        .map_err(|_| FOUND_WHILE_PAUSED_UPLOAD_FAILED.to_string())?
        .reconcile_pass_wanted
        .clone();
    let released = tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let (db, _) = own_local_data_for_decision(&identity)?;
        pass_wanted.fetch_add(1, Ordering::SeqCst);
        let released = db
            .release_found_while_paused(&op_ids, now_unix_seconds())
            .map_err(|error| {
                tracing::warn!(%error, "found while paused: the held changes could not be released");
                FOUND_WHILE_PAUSED_UPLOAD_FAILED.to_string()
            })?;
        pass_wanted.fetch_add(1, Ordering::SeqCst);
        Ok(released)
    })
    .await
    .map_err(|e| format!("upload task failed: {e}"))??;
    tracing::info!(released, "found while paused: released to upload");
    Ok(FoundWhilePausedReleased { released })
}

/// Task 2267 (fix round 2), Settings › Sync, "Keep on this PC only" (lead ruling (a)): every item made on this PC that
/// a held change the card showed (`op_ids`, self-review of round 3, F7) acts on moves next to the Beebeeb folder and is
/// forgotten (`found_while_paused::keep`). Only on the
/// signed-in account's own data, and under the session-transition lock. Counts only in the log.
#[cfg(target_os = "windows")]
#[tauri::command]
pub(super) async fn found_while_paused_keep(
    state: State<'_, AppState>,
    op_ids: Vec<String>,
) -> Result<found_while_paused::KeepResult, String> {
    let _transition = SESSION_TRANSITION.lock().await;
    let identity = decision_identity(&state)?;
    let kept = tokio::task::spawn_blocking(move || -> Result<found_while_paused::KeepResult, String> {
        let (db, sync_root) = own_local_data_for_decision(&identity)?;
        let today = found_while_paused::today();
        let mut kept =
            found_while_paused::keep(&db, &sync_root, &today, &found_while_paused::rename_no_replace, &op_ids)
                .map_err(|error| {
                    tracing::warn!(%error, "found while paused: the keep stopped");
                    FOUND_WHILE_PAUSED_KEEP_STOPPED.to_string()
                })?;
        kept.synced_by = kept_folder_synced_by(&sync_root);
        Ok(kept)
    })
    .await
    .map_err(|e| format!("keep task failed: {e}"))??;
    tracing::info!(
        moved_items = kept.moved_items,
        moved_files = kept.moved_files,
        not_moved = kept.not_moved,
        stays_held = kept.stays_held,
        "found while paused: kept on this PC"
    );
    Ok(kept)
}

/// Task 2267 (fix round 2), "Show in Explorer" after a keep: opens the folder the last keep recorded
/// ([`kept_folder_to_show`]). The frontend never passes a path.
#[cfg(target_os = "windows")]
#[tauri::command]
pub(super) async fn show_kept_folder() -> Result<(), String> {
    tokio::task::spawn_blocking(|| -> Result<(), String> {
        let cfg = DesktopConfig::load()?;
        let sync_root = cfg.sync_root.clone().ok_or(KEPT_FOLDER_GONE)?;
        let db = state_db_for_config(&cfg)?.ok_or(KEPT_FOLDER_GONE)?;
        let folder = kept_folder_to_show(&sync_root, db.last_kept_dir())?;
        std::process::Command::new("explorer")
            .arg(&folder)
            .spawn()
            .map_err(|e| format!("open Explorer: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("show task failed: {e}"))?
}

/// Task 2267: the changes found while sync was paused exist on Windows only (the ownership proof holds them). The
/// commands are registered everywhere, because `generate_handler!` takes no `cfg`, and say so elsewhere.
#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub(super) async fn found_while_paused_review() -> Result<(), String> {
    Err("This works on Windows only.".into())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub(super) async fn found_while_paused_upload() -> Result<(), String> {
    Err("This works on Windows only.".into())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub(super) async fn found_while_paused_keep() -> Result<(), String> {
    Err("This works on Windows only.".into())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub(super) async fn show_kept_folder() -> Result<(), String> {
    Err("This works on Windows only.".into())
}

/// W12 and §9: the folder an R10 reset kept reaches the person as the app-start sweep's does. The reset recorded it
/// already (W9), so the windows are told (an open Settings › Sync reads its row again), then one alert names it. The
/// path is never logged.
pub(super) fn surface_reset_folder(app: &tauri::AppHandle, folder: &str, files: usize) {
    emit_kept_folder_changed(app);
    if let Some((title, message)) = sign_out::copy::sign_out_alert(Some((folder, files)), None) {
        app.dialog()
            .message(message)
            .title(title)
            .kind(tauri_plugin_dialog::MessageDialogKind::Info)
            .show(|_| {});
    }
}

/// Is a Finder domain removal owed before any engine may start? Read by the reconciler's facts. An unreadable
/// database is not a debt it could act on (the engine start fails to open it too).
#[cfg(target_os = "macos")]
pub(crate) fn finder_removal_owed() -> bool {
    state_paths::beebeeb_state_dir()
        .ok()
        .is_some_and(|dir| finder_removal_owed_in(&dir))
}

#[cfg(target_os = "macos")]
pub(super) fn finder_removal_owed_in(state_dir: &Path) -> bool {
    matches!(state_db_from_state_dir(state_dir), Ok(Some(db)) if db.finder_removal_owed().unwrap_or(false))
}

/// The reconciler confirmed the domain gone (a removal worked, or an Observe found none): release the debt, which
/// lets the next engine start through.
#[cfg(target_os = "macos")]
pub(crate) fn clear_finder_removal_owed() {
    if let Ok(dir) = state_paths::beebeeb_state_dir() {
        clear_finder_removal_owed_in(&dir);
    }
}

/// The reconciler registered the domain (`addDomain` succeeded): it is no longer known gone, so a later start onto
/// an unowned, empty database owes a check again instead of trusting the mark a confirmed sign-out left.
#[cfg(target_os = "macos")]
pub(crate) fn clear_finder_domain_gone() {
    if let Ok(dir) = state_paths::beebeeb_state_dir() {
        clear_finder_domain_gone_in(&dir);
    }
}

#[cfg(target_os = "macos")]
pub(super) fn clear_finder_domain_gone_in(state_dir: &Path) {
    if let Ok(Some(db)) = state_db_from_state_dir(state_dir)
        && let Err(error) = db.clear_finder_domain_gone()
    {
        tracing::warn!(%error, "could not clear the Finder domain mark; a later first start may skip its removal check");
    }
}

#[cfg(target_os = "macos")]
pub(super) fn clear_finder_removal_owed_in(state_dir: &Path) {
    if let Ok(Some(db)) = state_db_from_state_dir(state_dir)
        && let Err(error) = db.set_finder_removal_owed(false)
    {
        tracing::warn!(%error, "could not release the Finder removal debt; engine starts keep waiting for a removal");
    }
}

/// R10, the upgrade path (fix round 1 of Task 10): local data from before the binding has no owner. On the first
/// startup after the upgrade, and only then, the account whose VAULT KEY the Keychain holds (`candidate`; `None`
/// when it holds a token without a key, or nothing) becomes the owner. Any other unowned data is an unknown
/// owner and is reset (or refused on Windows) at the next engine start. The window shuts whatever this finds.
/// Best-effort: an error here leaves the data unowned, which fails closed.
pub(super) fn adopt_unbound_local_data_at_startup(candidate: Option<&account_binding::Identity>) {
    let Ok(state_dir) = state_paths::beebeeb_state_dir() else {
        return;
    };
    adopt_unbound_local_data_in(&state_dir, candidate);
}

/// [`adopt_unbound_local_data_at_startup`] for a given state dir (a test points it at a throwaway one).
pub(super) fn adopt_unbound_local_data_in(state_dir: &Path, candidate: Option<&account_binding::Identity>) {
    let Ok(Some(db)) = state_db_from_state_dir(state_dir) else {
        return;
    };
    let data = StateDbLocalData {
        db,
        sync_root: None,
        staging_dirs: Vec::new(),
        kept: Default::default(),
    };
    match account_binding::adopt_unbound(&data, candidate) {
        Ok(true) => tracing::info!("R10: existing local data recorded as this computer's signed-in account's"),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "R10: could not record the owner of existing local data"),
    }
}

/// The account to adopt unowned local data into at startup: only a session the Keychain gave back COMPLETE
/// (token and wrapped vault key), and only by its email. A Keychain with a token but no key (a sign-in that
/// stopped before the recovery-phrase step, or any failure to read) names nobody.
pub(super) fn startup_adoption_candidate(
    loaded: &Result<Option<Session>, String>,
) -> Option<account_binding::Identity> {
    let session = loaded.as_ref().ok()?.as_ref()?;
    let candidate = account_binding::Identity::new(None, session.email.as_deref());
    candidate.is_known().then_some(candidate)
}

/// Sign-out (§5.3 (5)) and Repair (§5.3 (6)) on macOS: the reconciler cancels any check and removes
/// the domain; this waits for that removal (bounded), so the caller's next step cannot race a check.
/// `Err` when the reconciler is not running, did not answer in time, or the removal failed: the text
/// is the OS error's domain and code at most (lead ruling T1-4), and callers that report it use a fixed
/// sentence. There is no direct removal fallback: every bridge call goes through the gate (ruling 4).
/// Task 1882: the removal keeps the files that never reached the server, and both answers carry what
/// macOS kept (`kept_location` names the folder), so the caller can show it.
#[cfg(target_os = "macos")]
pub(super) async fn finder_remove_for(
    state: &AppState,
    trigger: finder_setup::core::Trigger,
) -> Result<finder_removal::DomainRemoval, finder_removal::RemovalFailure> {
    match state.finder_setup.get() {
        Some(handle) => handle.remove(trigger, finder_wait(FINDER_REMOVE_TIMEOUT)).await,
        None => Err(finder_setup::driver::NOT_RUNNING.to_string().into()),
    }
}

/// Lock (§5.3 (3)): the reconciler cancels any check and holds, and this waits for that
/// acknowledgement (bounded), so it can stop the engine without a check starting another.
#[cfg(target_os = "macos")]
pub(super) async fn finder_lock_for(state: &AppState) -> Result<(), String> {
    match state.finder_setup.get() {
        Some(handle) => handle.lock(finder_wait(FINDER_LOCK_TIMEOUT)).await,
        None => Err(finder_setup::driver::NOT_RUNNING.to_string()),
    }
}

/// Lead ruling T3-⚠2: a sign-out has told the reconciler to remove Finder. If it then stops short
/// (any return before the keys are gone) and the keys are still in memory, the person is still
/// signed in, so the reconciler is told keys are here and re-adds Finder. If the keys are gone the
/// reconciler stays held. Runs on every way out of `clear_session_impl`, including `?`.
#[cfg(target_os = "macos")]
pub(super) struct FinderRestoreOnAbort<'a> {
    pub(super) state: &'a AppState,
    pub(super) acct: &'a account::AccountRuntime,
}

#[cfg(target_os = "macos")]
impl Drop for FinderRestoreOnAbort<'_> {
    fn drop(&mut self) {
        let keys_in_memory = self.acct.session.lock().map(|guard| guard.is_some()).unwrap_or(false);
        // Not while an engine stop is unconfirmed (Task 9 fix round 1, item E): every start refuses until a
        // restart, so the check this would run could only fail, and the restart's `Launch` re-adds Finder.
        let stop_unconfirmed = self.acct.engine_stop_unconfirmed.load(Ordering::SeqCst);
        if keys_in_memory && !stop_unconfirmed {
            notify_finder(self.state, finder_setup::core::Trigger::KeysArrived);
        }
    }
}

pub(super) fn macos_version_string() -> String {
    sysinfo::System::os_version().unwrap_or_else(|| "unknown".to_string())
}

/// The names the lifecycle log scrubs from an NSError message: the same state.db source as the
/// support bundle (task 1685). Empty when there is no state.db yet; the redaction's allow-list
/// still removes every unknown word. It opens its own `StateDb`, takes no app lock, never
/// panics, and never calls `lifecycle_log::event` or `tail` (lead ruling T5-a): the log calls it
/// with no lock of its own held, but the caller of `event` may hold others.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn lifecycle_known_names() -> diagnostic_redaction::KnownNames {
    let sync_root = DesktopConfig::load()
        .ok()
        .and_then(|cfg| cfg.sync_root)
        .map(|p| p.to_string_lossy().into_owned());
    match state_db_from_app_local_state_dir() {
        Ok(Some(db)) => db
            .known_names(&sync_root.into_iter().collect::<Vec<_>>())
            .unwrap_or_default(),
        _ => diagnostic_redaction::KnownNames::new(),
    }
}

/// Spec §5.5 step 5 (macOS): on `Ready`, save the sync folder and make sure the engine runs.
/// Unlike `persist_sync_root_and_start_engine` it never restarts an engine that is already
/// running: the reconciler started it before `addDomain` (plan "Spec issues" 7), or the
/// relaunch's restore did.
#[cfg(target_os = "macos")]
pub(super) async fn ensure_sync_root_and_engine(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    root: PathBuf,
) -> Result<(), String> {
    // Task 1889: one load-change-save under the config-write lock; the pause state it read is the fallback for the
    // rehydration below, which stores it under the same lock.
    let pause_sync = DesktopConfig::update(|cfg| {
        let changed = cfg.sync_root.as_deref() != Some(root.as_path());
        if changed {
            cfg.sync_root = Some(root.clone());
        }
        (changed, cfg.pause_sync)
    })?;
    let acct = state.active_account()?;
    // The slot first, then the keys under it (lead rulings T8-⚠lock 2 and 3): Lock and Sign-out clear
    // the session before they release the slot, so a check that waited for it finds no keys, and
    // nothing starts beside an engine whose stop was never confirmed.
    let mut engine_slot = acct.engine.lock().await;
    engine_start_refusal(&acct)?;
    if engine_slot.is_none() {
        rehydrate_sync_paused(&acct.sync_paused, pause_sync);
        // The keys are read under the slot, and the local data is bound to the account, inside the start.
        match spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? {
            EngineStart::Started => {}
            EngineStart::NoSession => return Err(ENGINE_START_NO_SESSION_FOR_FINDER.to_string()),
            EngineStart::FinderRemovalOwed => return Err(FINDER_REMOVAL_OWED_ERROR.to_string()),
        }
    }
    Ok(())
}

/// The one wait a time limit may cut off in a check's engine start or stop: the wait for the engine
/// slot (lead ruling T7-1; Task 8 fix round 1). A cut-off here changes nothing: the engine is not yet
/// spawned or taken, so it stays exactly where sign-out and lock look for it. A limit around the work
/// that follows would orphan the engine (see `stop_check_engine`).
#[cfg(target_os = "macos")]
pub(super) async fn lock_engine_slot(
    acct: &account::AccountRuntime,
    op: finder_setup::core::Op,
    limit: std::time::Duration,
) -> Result<tokio::sync::MutexGuard<'_, Option<EngineRunner>>, finder_setup::error::FpError> {
    finder_setup::driver::within(op, limit, async { Ok(acct.engine.lock().await) }).await
}
