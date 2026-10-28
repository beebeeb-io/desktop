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
//!
//! Fix round 1 (the PR's self-review): a name is read in every format the engine reads (both key derivations, both
//! envelopes); a name that proves nothing either way (a plaintext legacy name, none, a file received through a file
//! request) is set aside instead of vetoing; a file this account cannot see blocks adoption unless it sat in a folder
//! proven this account's own (a file deleted since); no answer, for any sampled file, means "ask again later"; and the
//! sample reaches past the newest rows (held writes, the newest, then a random spread of the rest).

use crate::account_binding::{Identity, same_account};
use crate::state_db::StateDb;
use futures_util::stream::{self, StreamExt};
use std::collections::HashMap;
use std::future::Future;

/// How many local file rows a proof asks the server about.
pub const SAMPLE_LIMIT: usize = 16;
/// How many of them must prove to be this account's own (all of them when fewer were sampled).
pub const REQUIRED_PROOFS: usize = 2;
/// How many of them are asked about at once.
pub const CONCURRENT_ASKS: usize = 4;

/// What the server answered for one sampled file.
#[derive(Debug, Clone)]
pub enum Fetched {
    /// The file's metadata, as `GET /api/v1/files/{id}` returned it.
    Metadata(serde_json::Value),
    /// Not this account's to read, or gone (403, 404, 410).
    Missing,
    /// The server refused this session's token (401): nothing about ownership, and not a connection problem either.
    SignedOut,
    /// No answer that says anything about ownership: offline, a timeout, 429, a server error.
    Unavailable,
}

/// One sampled file, judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Its name decrypts under this session's master key: this account's own file.
    Own,
    /// This account can read it, but its name does not decrypt under this account's key.
    Unproven,
    /// Proves nothing either way: a plaintext (legacy) name, no name, or a file received through a file request (its
    /// name is sealed under the sender's content key). Set aside, never counted for or against.
    Neutral,
    /// Missing, but it sat in a folder proven this account's own: deleted since. Set by the proof, never by `probe`.
    Gone,
    Missing,
    SignedOut,
    Unavailable,
}

/// The counts of one proof. Logged and reported; never a name or a path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub sampled: usize,
    pub own: usize,
    pub neutral: usize,
    /// Missing files whose folder proved this account's own: deleted since, not another account's.
    pub gone: usize,
    pub unproven: usize,
    pub missing: usize,
    pub signed_out: usize,
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
        Fetched::SignedOut => return Probe::SignedOut,
        Fetched::Unavailable => return Probe::Unavailable,
    };
    // A file received through a file request has its name sealed under the sender's content key, and a plaintext
    // (legacy) name or none is sealed under no key: neither says whose the file is.
    if !metadata["file_request_id"].is_null() {
        return Probe::Neutral;
    }
    let Some(sealed) = metadata["name_encrypted"]
        .as_str()
        .filter(|name| name.trim_start().starts_with('{'))
    else {
        return Probe::Neutral;
    };
    // The keys are derived from THIS session's master key and the id asked about, so metadata of another file, or a
    // name sealed under another account's key, never opens (AES-GCM authenticates it). Every format the engine reads
    // is read: the string id (as every client seals today) and the 16-byte binary id (the old CLI), each in both
    // envelopes (`engine_bridge::decrypt_shared_name_with_key`).
    let master = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
    let by_string = beebeeb_core::kdf::derive_file_key(&master, file_id.as_bytes());
    let by_binary = uuid::Uuid::parse_str(file_id)
        .ok()
        .map(|id| beebeeb_core::kdf::derive_file_key(&master, id.as_bytes()));
    let own = std::iter::once(by_string).chain(by_binary).any(|file_key| {
        crate::engine_bridge::decrypt_shared_name_with_key(&file_key, sealed)
            .is_some_and(|name| !name.trim().is_empty())
    });
    if own { Probe::Own } else { Probe::Unproven }
}

/// The decision over a whole sample (see the module docs and the PR's Decisions).
pub fn verdict(tally: &Tally) -> Verdict {
    // Evidence against comes first: a file this account can read but cannot open, or one it cannot see that no folder
    // of its own explains (another account's private file looks exactly like that). Either one: not adopted.
    if tally.unproven > 0 || tally.missing > 0 {
        return Verdict::NotProven;
    }
    // A file that did not answer is not evidence for this account: ask again later.
    if tally.unavailable > 0 || tally.signed_out > 0 {
        return Verdict::Unavailable;
    }
    // Enough of this account's own files: two, or the one when only one could answer.
    if tally.own > 0 && tally.own >= REQUIRED_PROOFS.min(tally.own + tally.gone) {
        Verdict::Adopt
    } else {
        Verdict::NotProven
    }
}

/// Whether an HTTP status says the file is not this account's to read, or gone.
pub fn status_means_missing(status: u16) -> bool {
    matches!(status, 403 | 404 | 410)
}

/// An HTTP failure as the proof reads it (`None`: no status at all, e.g. offline).
pub fn fetched_from_status(status: Option<u16>) -> Fetched {
    match status {
        Some(status) if status_means_missing(status) => Fetched::Missing,
        Some(401) => Fetched::SignedOut,
        _ => Fetched::Unavailable,
    }
}

/// A `get_file` result as the proof reads it: an HTTP status that says "not yours, or gone" is `Missing`, any other
/// failure says nothing (`Unavailable`).
#[cfg(target_os = "windows")]
pub fn fetched(result: anyhow::Result<serde_json::Value>) -> Fetched {
    match result {
        Ok(metadata) => Fetched::Metadata(metadata),
        Err(error) => fetched_from_status(
            error
                .downcast_ref::<reqwest::Error>()
                .and_then(reqwest::Error::status)
                .map(|status| status.as_u16()),
        ),
    }
}

/// Run the proof for `session` over `db` and, when it holds, record `session` as the owner. `fetch` asks the server for
/// one file id, at most `CONCURRENT_ASKS` at once. `still_current` is checked before every question and right before
/// the record is written, so a session that changed meanwhile (a Lock, a sign-out, another sign-in) stops the asking
/// and records nothing. The owner is replaced only if it is still the one the proof began with. Nothing is ever
/// deleted.
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
    let (fetch, still_current) = (&fetch, &still_current);
    let answers: Vec<(String, Option<Probe>)> = stream::iter(sample)
        .map(|file_id| async move {
            let probe = ask(fetch, still_current, master_key, &file_id).await;
            (file_id, probe)
        })
        .buffered(CONCURRENT_ASKS)
        .collect()
        .await;
    let mut judged: HashMap<String, Probe> = HashMap::new();
    for (file_id, probe) in &answers {
        match probe {
            Some(probe) => judged.insert(file_id.clone(), *probe),
            None => return Ok(Outcome::Changed(tally)),
        };
    }
    for (file_id, probe) in answers {
        let mut probe = probe.unwrap_or(Probe::Unavailable);
        // A file this account cannot see is explained only when it sat in a folder proven this account's own: it was
        // deleted since. Another account's private file sits in that account's folders, which this account cannot
        // see either, or at the top level, which explains nothing.
        if probe == Probe::Missing
            && let Some(parent) = db
                .get_file(&file_id)
                .map_err(|e| format!("read a sampled row: {e}"))?
                .and_then(|row| row.parent_id)
                .filter(|parent| !parent.is_empty())
        {
            let folder = match judged.get(&parent) {
                Some(folder) => *folder,
                None => {
                    let Some(folder) = ask(fetch, still_current, master_key, &parent).await else {
                        return Ok(Outcome::Changed(tally));
                    };
                    judged.insert(parent, folder);
                    folder
                }
            };
            probe = match folder {
                Probe::Own => Probe::Gone,
                Probe::SignedOut | Probe::Unavailable => folder,
                _ => Probe::Missing,
            };
        }
        match probe {
            Probe::Own => tally.own += 1,
            Probe::Neutral => tally.neutral += 1,
            Probe::Gone => tally.gone += 1,
            Probe::Unproven => tally.unproven += 1,
            Probe::Missing => tally.missing += 1,
            Probe::SignedOut => tally.signed_out += 1,
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

/// Ask about one file while the session is still the one the proof began with (`None` once it is not). A file that
/// did not answer is asked once more.
async fn ask<F, Fut>(
    fetch: &F,
    still_current: &impl Fn() -> bool,
    master_key: &[u8; 32],
    file_id: &str,
) -> Option<Probe>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Fetched>,
{
    let mut judged = Probe::Unavailable;
    for _ in 0..2 {
        if !still_current() {
            return None;
        }
        judged = probe(master_key, file_id, &fetch(file_id.to_string()).await);
        if judged != Probe::Unavailable {
            break;
        }
    }
    Some(judged)
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
        pc_with_held(rows, rows.first().copied())
    }

    /// `rows` oldest first, with the held write on `held_on`.
    fn pc_with_held(rows: &[&str], held_on: Option<&str>) -> Pc {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let pc = Pc { db: Db(db), _dir: dir };
        for (n, id) in rows.iter().enumerate() {
            add_row(&pc, id, None, n);
        }
        if let Some(id) = held_on {
            pc.db.0.enqueue_operation(&held_write("held-1", id)).unwrap();
        }
        pc
    }

    /// A My Files row the server named, `n` ordering it by when the server last changed it. Its parent is written
    /// through the row's contract, as the pull writes every server row.
    fn add_row(pc: &Pc, id: &str, parent: Option<&str>, n: usize) {
        pc.db
            .0
            .upsert_file_with_contract(
                &FileEntry {
                    file_id: id.to_string(),
                    path: format!("folder/file-{id}.txt"),
                    status: FileStatus::Local,
                    size_bytes: 10,
                    modified_at: 1_700_000_000,
                    content_hash: None,
                    remote_updated_at: 1_700_000_000 + n as i64,
                    parent_id: parent.map(str::to_string),
                    item_kind: ItemKind::File,
                },
                |contract| contract.parent_id = parent.map(str::to_string),
            )
            .unwrap();
    }

    fn ids(prefix: &str, count: usize) -> Vec<String> {
        (0..count).map(|n| format!("{prefix}-0000-4000-8000-{n:012}")).collect()
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

    fn sealed_name(owner_key: [u8; 32], file_id: &str) -> String {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(owner_key);
        beebeeb_core::encrypt::encrypt_name(&mk, file_id, "received.pdf", None).unwrap()
    }

    /// A name sealed under the LEGACY key derivation (the 16-byte binary UUID, as the old CLI sealed names).
    fn sealed_binary(owner_key: [u8; 32], file_id: &str) -> Fetched {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(owner_key);
        let uuid = uuid::Uuid::parse_str(file_id).unwrap();
        let key = beebeeb_core::kdf::derive_file_key(&mk, uuid.as_bytes());
        let plaintext = serde_json::json!({ "name": "old.txt", "mime_type": null }).to_string();
        let blob = beebeeb_core::encrypt::encrypt_metadata(&key, &plaintext).unwrap();
        Fetched::Metadata(serde_json::json!({ "id": file_id, "name_encrypted": serde_json::to_string(&blob).unwrap() }))
    }

    /// A server that answers from a table, and counts what it was asked.
    struct Server {
        answers: Vec<(String, Fetched)>,
        asked: RefCell<Vec<String>>,
        /// Ids that do not answer the first time they are asked.
        flaky_once: RefCell<Vec<String>>,
    }

    impl Server {
        fn new(answers: Vec<(&str, Fetched)>) -> Self {
            Self {
                answers: answers.into_iter().map(|(id, f)| (id.to_string(), f)).collect(),
                asked: RefCell::new(Vec::new()),
                flaky_once: RefCell::new(Vec::new()),
            }
        }
        fn get(&self, id: String) -> std::future::Ready<Fetched> {
            self.asked.borrow_mut().push(id.clone());
            let mut flaky = self.flaky_once.borrow_mut();
            if let Some(at) = flaky.iter().position(|f| *f == id) {
                flaky.remove(at);
                return std::future::ready(Fetched::Unavailable);
            }
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

    // ---- fix round 1 (the PR's self-review) ----

    /// Finding 1: a name this account sealed under the legacy key derivation (the old CLI's binary UUID) is this
    /// account's own, as the engine reads it; it is not a veto.
    #[test]
    fn a_name_sealed_under_the_legacy_key_derivation_proves_ownership() {
        let id = IDS[0];
        assert_eq!(probe(&MINE, id, &sealed_binary(MINE, id)), Probe::Own);
        assert_eq!(
            probe(&MINE, id, &sealed_binary(THEIRS, id)),
            Probe::Unproven,
            "another account's, legacy"
        );

        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![
            (IDS[0], sealed_binary(MINE, IDS[0])),
            (IDS[1], sealed(MINE, IDS[1])),
            (IDS[2], sealed(MINE, IDS[2])),
        ]);
        assert!(
            matches!(prove(&pc, &session(), MINE, &server), Outcome::Adopted(t) if t.own == 3),
            "legacy rows adopt"
        );
    }

    /// Finding 1: a name that proves nothing either way (a plaintext legacy name, no name, a file received through a
    /// file request, sealed under the sender's key) is set aside: it neither proves nor vetoes.
    #[test]
    fn a_name_that_proves_nothing_is_set_aside_not_a_veto() {
        let id = IDS[0];
        let plaintext = Fetched::Metadata(serde_json::json!({ "id": id, "name_encrypted": "Photos" }));
        let unnamed = Fetched::Metadata(serde_json::json!({ "id": id }));
        let received = Fetched::Metadata(serde_json::json!({
            "id": id,
            "name_encrypted": sealed_name(THEIRS, id),
            "file_request_id": "0f000000-0000-4000-8000-000000000001",
        }));
        for (fetched, why) in [
            (&plaintext, "plaintext"),
            (&unnamed, "no name"),
            (&received, "a file request upload"),
        ] {
            assert_eq!(probe(&MINE, id, fetched), Probe::Neutral, "{why}");
        }

        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![
            (IDS[0], plaintext),
            (IDS[1], sealed(MINE, IDS[1])),
            (IDS[2], received),
        ]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(
            matches!(outcome, Outcome::Adopted(t) if t.own == 1 && t.neutral == 2),
            "{outcome:?}"
        );
    }

    /// Finding 2: a file that did not answer is not evidence for this account. Two own files and six that timed out
    /// (they may be another account's private files, which would have said "not found") is no adoption; a later start
    /// asks again.
    #[test]
    fn files_that_did_not_answer_never_count_as_proof() {
        let rows = ids("0d000000", 8);
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let pc = pc_with(&refs);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(
            refs.iter()
                .enumerate()
                .map(|(n, id)| (*id, if n < 2 { sealed(MINE, id) } else { Fetched::Unavailable }))
                .collect(),
        );
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(
            matches!(outcome, Outcome::Unavailable(t) if t.own == 2 && t.unavailable == 6),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// Finding 3: another account's rows that the newest rows would hide are still reached: the sample spreads past
    /// the held writes and the newest, and one row this account cannot see blocks adoption.
    #[test]
    fn another_accounts_older_rows_are_reached_and_block_adoption() {
        let theirs = ids("0e000000", 20);
        let mine = ids("0e000001", 10);
        let pc = pc_with_held(&[], None);
        for (n, id) in theirs.iter().chain(mine.iter()).enumerate() {
            add_row(&pc, id, None, n); // another account's rows first (older), then this account's (newer)
        }
        pc.db.0.enqueue_operation(&held_write("held-1", &mine[0])).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(mine.iter().map(|id| (id.as_str(), sealed(MINE, id))).collect()); // theirs: 404

        let outcome = prove(&pc, &session(), MINE, &server);

        assert!(matches!(outcome, Outcome::NotProven(t) if t.missing > 0), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(pc.db.0.list_files().unwrap().len(), 30, "nothing deleted");
    }

    /// Finding 3: a file this account cannot see, and that no proven folder of this account explains, blocks adoption
    /// however many own files outnumber it (another account's private file looks exactly like that).
    #[test]
    fn a_file_this_account_cannot_see_blocks_adoption() {
        let pc = pc_with(&IDS);
        add_row(&pc, "0a000000-0000-4000-8000-000000000009", None, 9);
        adopt_unbound(&pc.db, None).unwrap();
        let outcome = prove(&pc, &session(), MINE, &own_server()); // the fourth row: 404
        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.own == 3 && t.missing == 1),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// A file deleted since from one of this account's own folders (404, its folder proven this account's) does not
    /// block adoption: it is counted as gone.
    #[test]
    fn a_file_deleted_from_one_of_this_accounts_folders_does_not_block_adoption() {
        let folder = "0a000000-0000-4000-8000-0000000000f0";
        let pc = pc_with(&IDS);
        add_row(&pc, folder, None, 5);
        add_row(&pc, "0a000000-0000-4000-8000-0000000000f1", Some(folder), 6); // deleted since: 404
        adopt_unbound(&pc.db, None).unwrap();
        let mut answers: Vec<(&str, Fetched)> = IDS.iter().map(|id| (*id, sealed(MINE, id))).collect();
        answers.push((folder, sealed(MINE, folder)));
        let outcome = prove(&pc, &session(), MINE, &Server::new(answers));
        assert!(
            matches!(outcome, Outcome::Adopted(t) if t.gone == 1 && t.missing == 0),
            "{outcome:?}"
        );
    }

    /// A file this account cannot see whose folder is not this account's either is not explained: it blocks.
    #[test]
    fn a_missing_file_in_a_folder_that_is_not_this_accounts_blocks_adoption() {
        let pc = pc_with(&IDS);
        add_row(
            &pc,
            "0a000000-0000-4000-8000-0000000000f1",
            Some("0a000000-0000-4000-8000-0000000000e0"),
            6,
        );
        adopt_unbound(&pc.db, None).unwrap();
        let outcome = prove(&pc, &session(), MINE, &own_server()); // the file and its folder: 404
        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.missing == 1 && t.gone == 0),
            "{outcome:?}"
        );
    }

    /// Finding 5: a token the server refuses (401) is not "offline": it is told apart, so the refusal does not tell
    /// the person to connect to the internet.
    #[test]
    fn a_refused_token_is_told_apart_from_no_connection() {
        assert!(matches!(fetched_from_status(Some(401)), Fetched::SignedOut));
        assert!(matches!(fetched_from_status(None), Fetched::Unavailable));
        assert!(matches!(fetched_from_status(Some(429)), Fetched::Unavailable));
        assert!(matches!(fetched_from_status(Some(404)), Fetched::Missing));
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(IDS.iter().map(|id| (*id, Fetched::SignedOut)).collect());
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(
            matches!(outcome, Outcome::Unavailable(t) if t.signed_out == 3),
            "{outcome:?}"
        );
    }

    /// Finding 4: a session that changes while the proof runs (a Lock, a sign-out) stops the asking, so the session's
    /// key and token are not used for the rest of the sample.
    #[test]
    fn a_session_that_changes_mid_proof_stops_asking() {
        let rows = ids("0d000001", 16);
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let pc = pc_with(&refs);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(refs.iter().map(|id| (*id, sealed(MINE, id))).collect());
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            |id| server.get(id),
            || server.asked.borrow().is_empty(),
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert!(
            server.asked.borrow().len() <= CONCURRENT_ASKS,
            "asked {}",
            server.asked.borrow().len()
        );
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// One file that does not answer the first time is asked once more before the proof gives up on it.
    #[test]
    fn a_file_that_does_not_answer_is_asked_once_more() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        server.flaky_once.borrow_mut().push(IDS[1].to_string());
        assert!(matches!(prove(&pc, &session(), MINE, &server), Outcome::Adopted(t) if t.own == 3));
    }

    // ---- the sample ----

    /// The sample asks first about the files the held writes change, then the most recently changed rows, then a
    /// random spread of the rest, at most `SAMPLE_LIMIT` and each row once; rows the server never named are not asked
    /// about.
    #[test]
    fn the_sample_takes_held_writes_then_the_newest_then_a_spread() {
        let rows = ids("0b000000", 40);
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let pc = pc_with(&refs); // the held write is on the OLDEST row (index 0)
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
        assert_eq!(sample[0], rows[0], "the held write's file first");
        assert_eq!(
            &sample[1..5],
            &[rows[39].clone(), rows[38].clone(), rows[37].clone(), rows[36].clone()],
            "then the newest"
        );
        let distinct: std::collections::HashSet<&String> = sample.iter().collect();
        assert_eq!(distinct.len(), sample.len(), "each row once");
        assert!(!sample.iter().any(|id| id == "never-on-the-server"));
        assert!(
            sample.iter().any(|id| rows[1..24].contains(id)),
            "the spread reaches rows older than the newest {SAMPLE_LIMIT}: {sample:?}"
        );
    }

    // ---- the pure parts ----

    #[test]
    fn the_verdict_table() {
        let t = |own, gone, unproven, missing, signed_out, unavailable| Tally {
            sampled: own + gone + unproven + missing + signed_out + unavailable,
            own,
            neutral: 0,
            gone,
            unproven,
            missing,
            signed_out,
            unavailable,
        };
        let rows = [
            (t(0, 0, 0, 0, 0, 0), Verdict::NotProven, "nothing sampled"),
            (t(1, 0, 0, 0, 0, 0), Verdict::Adopt, "one row, its own"),
            (t(2, 0, 0, 0, 0, 0), Verdict::Adopt, "two own"),
            (t(1, 1, 0, 0, 0, 0), Verdict::NotProven, "one own of two is not enough"),
            (
                t(3, 1, 0, 0, 0, 0),
                Verdict::Adopt,
                "a file gone from an own folder does not block",
            ),
            (t(0, 2, 0, 0, 0, 0), Verdict::NotProven, "only gone files prove nothing"),
            (
                t(7, 0, 0, 1, 0, 0),
                Verdict::NotProven,
                "a file this account cannot see blocks",
            ),
            (
                t(7, 0, 1, 0, 0, 0),
                Verdict::NotProven,
                "a name sealed under another key blocks",
            ),
            (
                t(2, 0, 0, 0, 0, 6),
                Verdict::Unavailable,
                "files that did not answer are not proof",
            ),
            (
                t(7, 0, 0, 0, 0, 1),
                Verdict::Unavailable,
                "one unanswered file: ask again later",
            ),
            (
                t(3, 0, 0, 0, 3, 0),
                Verdict::Unavailable,
                "a refused token: ask again later",
            ),
            (t(0, 0, 0, 0, 0, 3), Verdict::Unavailable, "no answers"),
            (
                t(5, 0, 0, 1, 0, 2),
                Verdict::NotProven,
                "evidence against beats no answer",
            ),
            (t(0, 0, 0, 3, 0, 0), Verdict::NotProven, "all not readable"),
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
        assert_eq!(
            probe(&MINE, id, &unnamed),
            Probe::Neutral,
            "no name proves nothing either way"
        );
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
