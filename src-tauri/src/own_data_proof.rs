//! Task 2267 (P1): a Windows PC whose local data had no recorded owner, or an owner recorded only by an email the
//! account no longer uses, refused every sync start with `OTHER_ACCOUNT_ON_WINDOWS`, although the data was the
//! signed-in account's own. Signing out did not help: the Windows sign-out keeps unsent changes, and those cannot send
//! while sync is refused.
//!
//! The one-shot startup adoption (`account_binding::adopt_unbound`) names its candidate by the stored session's email,
//! and a session stored before the email was kept has none, so the window closed with no owner recorded. From then on
//! every start saw unowned data and decided `Reset`, which Windows refuses (R10).
//!
//! This module settles such a refusal with evidence instead of a guess: it asks the server for a sample of the local
//! My Files rows and adopts the data only when they decrypt under THIS session's master key. Another account's My
//! Files rows never decrypt under this key (each name is sealed with a key derived from its owner's master key and the
//! file id), so another account's data is never adopted (R10 holds), and nothing is deleted here in any case.

use crate::account_binding::{Identity, same_account};
use crate::state_db::StateDb;
use std::future::Future;

/// How many local file rows a proof asks the server about.
pub const SAMPLE_LIMIT: usize = 8;
/// How many of them must prove to be this account's own (all of them when fewer were sampled).
pub const REQUIRED_PROOFS: usize = 2;

/// What the server answered for one sampled file.
#[derive(Debug, Clone)]
pub enum Fetched {
    /// The file's metadata, as `GET /api/v1/files/{id}` returned it.
    Metadata(serde_json::Value),
    /// Not this account's to read, or gone (403, 404, 410).
    Missing,
    /// No answer that says anything about ownership: offline, a timeout, 401, 429, a server error.
    Unavailable,
}

/// One sampled file, judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Its name decrypts under this session's master key: this account's own file.
    Own,
    /// This account can read it, but its name does not decrypt under this account's key.
    Unproven,
    Missing,
    Unavailable,
}

/// The counts of one proof. Logged and reported; never a name or a path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub sampled: usize,
    pub own: usize,
    pub unproven: usize,
    pub missing: usize,
    pub unavailable: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Adopt,
    Unavailable,
    NotProven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing a proof can settle: the data is owned by this session already, there is none, or the owner record
    /// names another account by its user id (R10's strongest signal, never overridden).
    NotEligible,
    /// Proven, and this session is now the recorded owner: the next start proceeds and keeps everything.
    Adopted(Tally),
    NotProven(Tally),
    /// Too few answers to decide; nothing recorded. A later start asks again.
    Unavailable(Tally),
    /// The session or the owner record changed while the proof ran; nothing recorded.
    Changed(Tally),
}

/// Whether a refused start is one a proof may settle: this session is known, local data exists, and the data has no
/// recorded owner or one that differs from this session only by email (an email-only record, or a session whose user
/// id is not known yet).
pub fn eligible(owner: Option<&Identity>, session: &Identity, has_local_data: bool) -> bool {
    if !session.is_known() || !has_local_data {
        return false;
    }
    match owner.filter(|owner| owner.is_known()) {
        None => true,
        // Two user ids that differ are two accounts: R10's strongest signal, never questioned.
        Some(owner) => {
            let both_ids = owner.user_id.is_some() && session.user_id.is_some();
            !both_ids && same_account(owner, session) == Some(false)
        }
    }
}

/// Judge one server answer for `file_id` against this session's master key.
pub fn probe(master_key: &[u8; 32], file_id: &str, fetched: &Fetched) -> Probe {
    let metadata = match fetched {
        Fetched::Metadata(metadata) => metadata,
        Fetched::Missing => return Probe::Missing,
        Fetched::Unavailable => return Probe::Unavailable,
    };
    // The key is derived from THIS session's master key and the id asked about, so metadata of another file, or a
    // name sealed under another account's key, never opens (AES-GCM authenticates it). Both name envelopes are read,
    // as the engine reads them (`engine_bridge::decrypt_shared_name_with_key`).
    let file_key = beebeeb_core::kdf::derive_file_key(
        &beebeeb_core::kdf::MasterKey::from_bytes(*master_key),
        file_id.as_bytes(),
    );
    let own = metadata["name_encrypted"]
        .as_str()
        .and_then(|sealed| crate::engine_bridge::decrypt_shared_name_with_key(&file_key, sealed))
        .is_some_and(|name| !name.trim().is_empty());
    if own { Probe::Own } else { Probe::Unproven }
}

/// The decision over a whole sample (see the module docs and the PR's Decisions).
pub fn verdict(tally: &Tally) -> Verdict {
    // Anything this account can read but cannot open is another account's: the data is not adopted, whatever else
    // the sample shows.
    if tally.sampled == 0 || tally.unproven > 0 {
        return Verdict::NotProven;
    }
    // Enough of this account's own files, and more of them than files it cannot see (another account's private files
    // look exactly like deleted ones, so they may not outnumber the proof).
    if tally.own >= REQUIRED_PROOFS.min(tally.sampled) && tally.own > tally.missing {
        return Verdict::Adopt;
    }
    if tally.unavailable > 0 {
        Verdict::Unavailable
    } else {
        Verdict::NotProven
    }
}

/// Whether an HTTP status says the file is not this account's to read, or gone.
pub fn status_means_missing(status: u16) -> bool {
    matches!(status, 403 | 404 | 410)
}

/// A `get_file` result as the proof reads it: an HTTP status that says "not yours, or gone" is `Missing`, any other
/// failure says nothing (`Unavailable`).
#[cfg(target_os = "windows")]
pub fn fetched(result: anyhow::Result<serde_json::Value>) -> Fetched {
    match result {
        Ok(metadata) => Fetched::Metadata(metadata),
        Err(error) => match error.downcast_ref::<reqwest::Error>().and_then(reqwest::Error::status) {
            Some(status) if status_means_missing(status.as_u16()) => Fetched::Missing,
            _ => Fetched::Unavailable,
        },
    }
}

/// Run the proof for `session` over `db` and, when it holds, record `session` as the owner. `fetch` asks the server for
/// one file id; `still_current` is checked right before the record is written, so a session that changed meanwhile
/// (a sign-out, another sign-in) records nothing. The owner is replaced only if it is still the one the proof began
/// with. Nothing is ever deleted.
pub async fn prove_and_adopt<F, Fut>(
    db: &StateDb,
    session: &Identity,
    master_key: &[u8; 32],
    fetch: F,
    still_current: impl Fn() -> bool,
) -> Result<Outcome, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Fetched>,
{
    let owner = db.owner().map_err(|e| format!("read the local data owner: {e}"))?;
    let has_local_data = db
        .has_account_data()
        .map_err(|e| format!("inspect the local data: {e}"))?;
    if !eligible(owner.as_ref(), session, has_local_data) {
        return Ok(Outcome::NotEligible);
    }
    let sample = db
        .own_data_proof_sample(SAMPLE_LIMIT)
        .map_err(|e| format!("sample the local data: {e}"))?;
    let mut tally = Tally {
        sampled: sample.len(),
        ..Tally::default()
    };
    for file_id in sample {
        let fetched = fetch(file_id.clone()).await;
        match probe(master_key, &file_id, &fetched) {
            Probe::Own => tally.own += 1,
            Probe::Unproven => tally.unproven += 1,
            Probe::Missing => tally.missing += 1,
            Probe::Unavailable => tally.unavailable += 1,
        }
    }
    match verdict(&tally) {
        Verdict::NotProven => Ok(Outcome::NotProven(tally)),
        Verdict::Unavailable => Ok(Outcome::Unavailable(tally)),
        Verdict::Adopt if !still_current() => Ok(Outcome::Changed(tally)),
        Verdict::Adopt => {
            let recorded = db
                .replace_owner_if(owner.as_ref(), session)
                .map_err(|e| format!("record the local data owner: {e}"))?;
            Ok(if recorded {
                Outcome::Adopted(tally)
            } else {
                Outcome::Changed(tally)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_binding::{
        Bound, IDENTITY_UNKNOWN, LocalData, OTHER_ACCOUNT_ON_WINDOWS, adopt_unbound, bind_before_engine_start,
    };
    use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation};
    use std::cell::RefCell;

    const WINDOWS: bool = false; // `reset_allowed` on Windows

    const MINE: [u8; 32] = [7u8; 32];
    const THEIRS: [u8; 32] = [9u8; 32];

    fn session() -> Identity {
        Identity::new(Some("user-david"), Some("david@example.eu"))
    }

    /// The real `state.db`, through the binding's own trait, as the engine start reads it.
    struct Db(StateDb);

    impl LocalData for Db {
        fn owner(&self) -> Result<Option<Identity>, String> {
            self.0.owner().map_err(|e| e.to_string())
        }
        fn has_account_data(&self) -> Result<bool, String> {
            self.0.has_account_data().map_err(|e| e.to_string())
        }
        fn reset(&self) -> Result<(), String> {
            panic!("Windows never resets in these tests")
        }
        fn record_owner(&self, owner: &Identity) -> Result<(), String> {
            self.0.set_owner(owner).map_err(|e| e.to_string())
        }
        fn adopt_once(&self, candidate: Option<&Identity>) -> Result<bool, String> {
            self.0.adopt_owner_once(candidate).map_err(|e| e.to_string())
        }
    }

    struct Pc {
        db: Db,
        _dir: tempfile::TempDir,
    }

    /// A PC that synced `rows` My Files rows from the server and holds one change waiting to upload on the first
    /// of them (a held write), the shape of the reported PCs.
    fn pc_with(rows: &[&str]) -> Pc {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (n, id) in rows.iter().enumerate() {
            db.upsert_file(&FileEntry {
                file_id: (*id).to_string(),
                path: format!("folder/file-{n}.txt"),
                status: FileStatus::Local,
                size_bytes: 10,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000 + n as i64,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        }
        if let Some(first) = rows.first() {
            db.enqueue_operation(&held_write("held-1", first)).unwrap();
        }
        Pc { db: Db(db), _dir: dir }
    }

    fn held_write(op_id: &str, file_id: &str) -> PendingOperation {
        PendingOperation {
            op_id: op_id.to_string(),
            kind: OperationKind::UploadVersion,
            file_id: Some(file_id.to_string()),
            parent_id: None,
            target_path: Some("folder/file-0.txt".to_string()),
            metadata_json: None,
            payload_path: None,
            base_version: Some(1),
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    /// The server's metadata for `file_id`, its name sealed under `owner_key` exactly as every client seals it.
    fn sealed(owner_key: [u8; 32], file_id: &str) -> Fetched {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(owner_key);
        let name = beebeeb_core::encrypt::encrypt_name(&mk, file_id, "notes.txt", Some("text/plain")).unwrap();
        Fetched::Metadata(serde_json::json!({ "id": file_id, "name_encrypted": name, "size_bytes": 10 }))
    }

    /// A server that answers from a table, and counts what it was asked.
    struct Server {
        answers: Vec<(String, Fetched)>,
        asked: RefCell<Vec<String>>,
    }

    impl Server {
        fn new(answers: Vec<(&str, Fetched)>) -> Self {
            Self {
                answers: answers.into_iter().map(|(id, f)| (id.to_string(), f)).collect(),
                asked: RefCell::new(Vec::new()),
            }
        }
        fn get(&self, id: String) -> std::future::Ready<Fetched> {
            self.asked.borrow_mut().push(id.clone());
            let answer = self
                .answers
                .iter()
                .find(|(known, _)| *known == id)
                .map(|(_, f)| f.clone())
                .unwrap_or(Fetched::Missing);
            std::future::ready(answer)
        }
    }

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn prove(pc: &Pc, session: &Identity, key: [u8; 32], server: &Server) -> Outcome {
        block_on(prove_and_adopt(&pc.db.0, session, &key, |id| server.get(id), || true)).expect("the proof runs")
    }

    fn held_writes(pc: &Pc) -> usize {
        pc.db.0.list_due_operations(i64::MAX).unwrap().len()
    }

    const IDS: [&str; 3] = [
        "0a000000-0000-4000-8000-000000000001",
        "0a000000-0000-4000-8000-000000000002",
        "0a000000-0000-4000-8000-000000000003",
    ];

    fn own_server() -> Server {
        Server::new(IDS.iter().map(|id| (*id, sealed(MINE, id))).collect())
    }

    // ---- the stranding paths, reproduced against the real state.db ----

    /// Path 1 (the report): a session stored before its email was kept names no adoption candidate at the first
    /// startup, so the one-shot window closes with no owner, and every later start refuses with the
    /// "another account" sentence although the data is this account's own. The proof adopts it; the held write is
    /// kept and the next start proceeds.
    #[test]
    fn a_closed_window_with_no_candidate_no_longer_strands_the_owner() {
        let pc = pc_with(&IDS);
        assert_eq!(adopt_unbound(&pc.db, None), Ok(false), "the startup had no candidate");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string()),
            "stranded before the proof"
        );

        let outcome = prove(&pc, &session(), MINE, &own_server());

        assert!(
            matches!(outcome, Outcome::Adopted(t) if t.own == 3 && t.sampled == 3),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), Some(session()));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
        assert_eq!(held_writes(&pc), 1, "the held write is kept, to upload");
    }

    /// Path 2: the startup had a candidate but the window was already shut (an engine start or a sign-in opened the
    /// database first: `StateDbLocalData::for_engine_start` closes it). The late candidate is refused, as designed;
    /// the proof still adopts.
    #[test]
    fn a_window_shut_before_the_candidate_arrived_no_longer_strands_the_owner() {
        let pc = pc_with(&IDS);
        pc.db.0.close_adoption_window().unwrap();
        assert_eq!(
            adopt_unbound(&pc.db, Some(&session())),
            Ok(false),
            "the window was already shut"
        );
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );

        assert!(matches!(
            prove(&pc, &session(), MINE, &own_server()),
            Outcome::Adopted(_)
        ));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
        assert_eq!(held_writes(&pc), 1);
    }

    /// Path 3: signing out cannot break the loop. The Windows sign-out refuses while a change waits to upload, and
    /// the change cannot upload while sync is refused, so sign-out and sign-in go round. After the proof the held
    /// write is still there, and the start proceeds, so it can upload.
    #[test]
    fn the_sign_out_loop_is_broken_by_the_proof_not_by_deleting_the_held_write() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        assert!(
            pc.db.0.windows_signout_preflight().is_err(),
            "sign-out refuses: a change waits to upload"
        );
        assert!(pc.db.0.finish_windows_signout().is_err(), "and the purge refuses too");
        assert_eq!(held_writes(&pc), 1);

        assert!(matches!(
            prove(&pc, &session(), MINE, &own_server()),
            Outcome::Adopted(_)
        ));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
        assert_eq!(held_writes(&pc), 1, "nothing was deleted to get here");
    }

    /// Path 4: the owner was recorded by email only, and the account's email changed since. The emails differ, so the
    /// binding decided "another account". The proof adopts and records the full identity.
    #[test]
    fn an_email_only_owner_whose_email_changed_is_proven_and_rerecorded() {
        let pc = pc_with(&IDS);
        pc.db
            .0
            .set_owner(&Identity::new(None, Some("old-address@example.eu")))
            .unwrap();
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );

        assert!(matches!(
            prove(&pc, &session(), MINE, &own_server()),
            Outcome::Adopted(_)
        ));
        assert_eq!(pc.db.0.owner().unwrap(), Some(session()));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
    }

    /// A session whose user id is not known yet (its profile was not fetched) against an owner recorded with an old
    /// email: the same proof applies, and the email-only record it writes is completed by the next start.
    #[test]
    fn a_session_without_its_user_id_yet_is_proven_by_its_email() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let by_email = Identity::new(None, Some("david@example.eu"));
        assert!(matches!(
            prove(&pc, &by_email, MINE, &own_server()),
            Outcome::Adopted(_)
        ));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
        assert_eq!(pc.db.0.owner().unwrap(), Some(session()), "the id completes the record");
    }

    // ---- R10 holds: another account's data is never adopted, and nothing is deleted ----

    /// Another account's private files: this session cannot read them (404). Nothing is adopted, the start still
    /// refuses, and every row and the held write are still there.
    #[test]
    fn another_accounts_private_data_is_never_adopted() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(Vec::new()); // every id: Missing

        let outcome = prove(&pc, &session(), MINE, &server);

        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.missing == 3 && t.own == 0),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );
        assert_eq!(held_writes(&pc), 1);
        assert_eq!(pc.db.0.list_files().unwrap().len(), 3);
    }

    /// Another account's files that it shared with this one: this session can read them, but their names are sealed
    /// under the other account's key. Not adopted.
    #[test]
    fn another_accounts_shared_data_is_never_adopted() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(IDS.iter().map(|id| (*id, sealed(THEIRS, id))).collect());

        let outcome = prove(&pc, &session(), MINE, &server);

        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.unproven == 3),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(held_writes(&pc), 1);
    }

    /// A mix: some rows this account's own, one readable but sealed under another key. A database that holds anything
    /// of another account is not adopted.
    #[test]
    fn a_mixed_database_is_never_adopted() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![
            (IDS[0], sealed(MINE, IDS[0])),
            (IDS[1], sealed(MINE, IDS[1])),
            (IDS[2], sealed(THEIRS, IDS[2])),
        ]);
        assert!(matches!(prove(&pc, &session(), MINE, &server), Outcome::NotProven(_)));
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// An owner recorded WITH a user id that differs from this session's is another account by R10's strongest
    /// signal. No proof is attempted: the server is not even asked.
    #[test]
    fn an_owner_recorded_by_another_user_id_is_never_questioned() {
        let pc = pc_with(&IDS);
        let alice = Identity::new(Some("user-alice"), Some("alice@example.eu"));
        pc.db.0.set_owner(&alice).unwrap();
        let server = own_server();

        assert_eq!(prove(&pc, &session(), MINE, &server), Outcome::NotEligible);
        assert!(server.asked.borrow().is_empty(), "nothing asked");
        assert_eq!(pc.db.0.owner().unwrap(), Some(alice));
    }

    /// Offline, or the server failing: nothing is decided and nothing recorded; a later start asks again.
    #[test]
    fn no_answer_decides_nothing() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(IDS.iter().map(|id| (*id, Fetched::Unavailable)).collect());

        assert!(matches!(prove(&pc, &session(), MINE, &server), Outcome::Unavailable(t) if t.unavailable == 3));
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(held_writes(&pc), 1);
    }

    /// A session that changed while the proof ran (a sign-out, another sign-in) records nothing.
    #[test]
    fn a_session_that_changed_during_the_proof_records_nothing() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            |id| server.get(id),
            || false,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// An owner recorded by someone else while the proof ran is never replaced.
    #[test]
    fn an_owner_recorded_during_the_proof_is_never_replaced() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let racer = Identity::new(Some("user-racer"), None);
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            |id| {
                pc.db.0.set_owner(&racer).unwrap();
                server.get(id)
            },
            || true,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), Some(racer));
    }

    /// A Windows sign-out that emptied the database while the proof ran (its purge also removes the owner record):
    /// nothing is recorded, so the next account to sign in on this PC finds no owner and no data.
    #[test]
    fn an_owner_is_never_recorded_on_a_database_emptied_during_the_proof() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            |id| {
                if pc.db.0.has_account_data().unwrap() {
                    pc.db.0.remove_operation("held-1").unwrap();
                    pc.db.0.finish_windows_signout().unwrap();
                }
                server.get(id)
            },
            || true,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert!(!pc.db.0.has_account_data().unwrap());
    }

    /// Nothing to sample (no row the server knows): nothing can be proven, so nothing is adopted.
    #[test]
    fn data_without_any_server_row_is_never_adopted() {
        let pc = pc_with(&[]);
        pc.db
            .0
            .enqueue_operation(&held_write("held-new", "provisional-1"))
            .unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let outcome = prove(&pc, &session(), MINE, &own_server());
        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.sampled == 0),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(held_writes(&pc), 1);
    }

    // ---- the sample ----

    /// The sample asks first about the files the held writes change, then the most recently changed rows, at most
    /// `SAMPLE_LIMIT`; shared-folder rows and rows the server never named are not asked about.
    #[test]
    fn the_sample_prefers_held_writes_and_skips_rows_that_cannot_prove_anything() {
        let ids: Vec<String> = (0..12).map(|n| format!("0b000000-0000-4000-8000-{n:012}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let pc = pc_with(&refs);
        // A held write on the OLDEST row (index 0) is already queued by `pc_with`.
        pc.db
            .0
            .upsert_file(&FileEntry {
                file_id: "never-on-the-server".to_string(),
                path: "new.txt".to_string(),
                status: FileStatus::Local,
                size_bytes: 1,
                modified_at: 1,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();

        let sample = pc.db.0.own_data_proof_sample(SAMPLE_LIMIT).unwrap();

        assert_eq!(sample.len(), SAMPLE_LIMIT);
        assert_eq!(sample[0], ids[0], "the held write's file first");
        assert_eq!(sample[1], ids[11], "then the newest");
        assert!(!sample.iter().any(|id| id == "never-on-the-server"));
    }

    // ---- the pure parts ----

    #[test]
    fn the_verdict_table() {
        let t = |sampled, own, unproven, missing, unavailable| Tally {
            sampled,
            own,
            unproven,
            missing,
            unavailable,
        };
        let rows = [
            (t(0, 0, 0, 0, 0), Verdict::NotProven, "nothing sampled"),
            (t(1, 1, 0, 0, 0), Verdict::Adopt, "one row, its own"),
            (t(2, 1, 0, 1, 0), Verdict::NotProven, "one of two is not enough"),
            (t(3, 2, 0, 1, 0), Verdict::Adopt, "two own, one gone"),
            (t(8, 2, 0, 6, 0), Verdict::NotProven, "more gone than own"),
            (t(8, 5, 0, 3, 0), Verdict::Adopt, "more own than gone"),
            (
                t(8, 7, 1, 0, 0),
                Verdict::NotProven,
                "anything sealed under another key vetoes",
            ),
            (t(3, 0, 0, 0, 3), Verdict::Unavailable, "no answers"),
            (t(3, 1, 0, 0, 2), Verdict::Unavailable, "too few answers"),
            (t(3, 2, 0, 0, 1), Verdict::Adopt, "enough answers"),
            (t(3, 0, 0, 3, 0), Verdict::NotProven, "all gone or not readable"),
        ];
        for (tally, expected, why) in rows {
            assert_eq!(verdict(&tally), expected, "{why}: {tally:?}");
        }
    }

    #[test]
    fn a_probe_is_own_only_when_the_name_decrypts_under_this_key_for_this_id() {
        let id = IDS[0];
        assert_eq!(probe(&MINE, id, &sealed(MINE, id)), Probe::Own);
        assert_eq!(probe(&MINE, id, &sealed(THEIRS, id)), Probe::Unproven, "another key");
        assert_eq!(
            probe(&MINE, id, &sealed(MINE, IDS[1])),
            Probe::Unproven,
            "another file's metadata"
        );
        let unnamed = Fetched::Metadata(serde_json::json!({ "id": id }));
        assert_eq!(probe(&MINE, id, &unnamed), Probe::Unproven, "no name");
        assert_eq!(probe(&MINE, id, &Fetched::Missing), Probe::Missing);
        assert_eq!(probe(&MINE, id, &Fetched::Unavailable), Probe::Unavailable);
    }

    #[test]
    fn eligibility() {
        let me = session();
        let me_by_email = Identity::new(None, Some("david@example.eu"));
        let old_email = Identity::new(None, Some("old@example.eu"));
        let alice = Identity::new(Some("user-alice"), Some("alice@example.eu"));
        let alice_old_email = Identity::new(Some("user-alice"), Some("old@example.eu"));
        assert!(eligible(None, &me, true), "unowned data");
        assert!(!eligible(None, &me, false), "no data");
        assert!(!eligible(Some(&me), &me, true), "already this session's");
        assert!(!eligible(Some(&me_by_email), &me, true), "the same account by email");
        assert!(
            eligible(Some(&old_email), &me, true),
            "an email-only owner that differs"
        );
        assert!(
            eligible(Some(&alice_old_email), &me_by_email, true),
            "a session without its id yet"
        );
        assert!(!eligible(Some(&alice), &me, true), "another user id: never");
        assert!(!eligible(None, &Identity::default(), true), "an unknown session: never");
    }

    #[test]
    fn missing_statuses() {
        for status in [403, 404, 410] {
            assert!(status_means_missing(status), "{status}");
        }
        for status in [200, 400, 401, 408, 429, 500, 502, 503] {
            assert!(!status_means_missing(status), "{status}");
        }
    }

    /// The copy a refused Windows start shows never tells a person they have another account.
    #[test]
    fn the_refusal_never_claims_another_account() {
        for sentence in [OTHER_ACCOUNT_ON_WINDOWS, IDENTITY_UNKNOWN] {
            let lower = sentence.to_lowercase();
            assert!(!lower.contains("another"), "{sentence}");
            assert!(!lower.contains("other account"), "{sentence}");
        }
        assert!(OTHER_ACCOUNT_ON_WINDOWS.contains("Nothing was uploaded or deleted"));
    }
}
