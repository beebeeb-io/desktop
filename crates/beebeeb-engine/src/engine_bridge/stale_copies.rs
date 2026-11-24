//! Task 2523 (F1): another device's version reaches the copy on this PC.
//!
//! A pull that raises a file's version writes the row only (`meta_2.rs`), so a copy in the sync folder kept the older
//! version's bytes, marked in sync, with no time limit (a kept file until it was unpinned). Once per tick, after the
//! pull, this pass takes the copies the engine's rule names (`StateDb::list_stale_agreed_copies`: agreed content
//! behind the current version, nothing queued, no held edit), checks each one's bytes against the agreed content,
//! and brings only those to the current version:
//! - **Windows:** the placeholder is emptied under ONE exclusive handle, in [`REFRESH_ORDER`], and given the current
//!   version's size and time; the next open fetches the current version. A kept file is fetched again right after.
//! - **Linux:** a re-download of the current version into its place is queued (`HydrateFile`); the hydrate refuses
//!   over a change, at its start and again right before it replaces the file.
//! - **macOS:** nothing (the File Provider fetches on the working-set signal).
//!
//! Bytes that are not the agreed content are a change: they are left exactly as they are, and an edit sent on the
//! older version is parked as the two-versions question, as before.

use super::*;

/// How many copies one pass brings to the current version; the rest wait for the next tick.
pub const STALE_COPIES_PER_PASS: usize = 32;

/// How many of the rule's copies one pass reads. More than it acts on: a copy with a change stays in the rule's set
/// (on Linux nothing ever sees the change) and is skipped at the price of a cached hash, so it never holds back the
/// copies behind it.
pub const STALE_COPIES_READ_PER_PASS: usize = 1024;

/// The op's reason, so a hydrate this pass queued that meets a change is dropped, never retried.
pub(super) const REFRESH_REASON: &str = "newer_version";

/// One step of the Windows refresh of one placeholder, all under ONE exclusive handle (LD-0105's measured order for
/// emptying a placeholder, `windows_cf/poison.rs`, plus the checks before it and the size and row after it).
/// Platform-neutral data, so its unit test runs everywhere; the Windows executor walks [`REFRESH_ORDER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefreshStep {
    /// The placeholder carries this row's file id (its FileIdentity): a stale path names another file.
    CheckIdentity,
    /// Its bytes, read through the handle, are the row's agreed content. Anything else is a change: left alone.
    CheckAgreedBytes,
    /// The rule still names the row with that same agreed content.
    CheckRow,
    /// The pin, read to be put back. (The in-sync state is not read: a failed step marks the copy NOT_IN_SYNC, and
    /// the stamp pass marks agreed bytes in sync again at the next tick.)
    RecordState,
    /// A pinned placeholder refuses to dehydrate (`0x80070188`).
    Unpin,
    /// A placeholder not in sync refuses to dehydrate (`0x80070179`). Only the connected provider may set it.
    MarkInSync,
    Dehydrate,
    /// Nothing of the older version is left on disk.
    VerifyEmpty,
    /// The current version's size and time on the empty placeholder, in sync, so the first fetch asks for the new
    /// length (an old length is a failed fetch first, `callbacks.rs`).
    SetCurrentMetadata,
    /// The row: cloud-only, its older agreed content kept as the fetched candidate (as "Free up space" records an
    /// emptied placeholder); only while it still reads as checked. Before the pin goes back, so a fetch the pin
    /// starts never meets the row as `local`.
    RecordRow,
    /// The pin as it was.
    RestorePin,
}

/// The order. Nothing changes before every check passed; on a failure from `Unpin` to `VerifyEmpty` the copy is
/// marked NOT_IN_SYNC and its pin put back before the handle is released.
pub const REFRESH_ORDER: [RefreshStep; 11] = [
    RefreshStep::CheckIdentity,
    RefreshStep::CheckAgreedBytes,
    RefreshStep::CheckRow,
    RefreshStep::RecordState,
    RefreshStep::Unpin,
    RefreshStep::MarkInSync,
    RefreshStep::Dehydrate,
    RefreshStep::VerifyEmpty,
    RefreshStep::SetCurrentMetadata,
    RefreshStep::RecordRow,
    RefreshStep::RestorePin,
];

/// What one pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StaleCopyPass {
    /// Linux: a re-download of the current version queued into the file's place.
    pub queued: Vec<String>,
    /// Windows: emptied and given the current version's size; the next open fetches it.
    pub emptied: Vec<String>,
    /// Windows: kept files among them, to fetch again now, off the tick: they join the tick's kept fetch (S6).
    pub refetch: Vec<KeptPlace>,
    /// Bytes that are not the agreed content: a change, left as it is.
    pub changed: usize,
    /// Not done this pass (open elsewhere, not wholly on disk, nothing at its place, the row moved): next tick.
    pub later: usize,
}

impl EngineBridge {
    /// The pass (see the module doc). Blocking: it may hash files and, on Windows, holds a handle per file; the runner
    /// calls it off the async workers, after the pull, before the queue step.
    pub fn refresh_stale_copies(&self, sync_root: &Path) -> Result<StaleCopyPass, EngineError> {
        let mut pass = StaleCopyPass::default();
        if cfg!(target_os = "macos") {
            return Ok(pass);
        }
        for copy in self.db.list_stale_agreed_copies(STALE_COPIES_READ_PER_PASS)? {
            if pass.queued.len() + pass.emptied.len() >= STALE_COPIES_PER_PASS {
                pass.later += 1;
                continue;
            }
            let Ok(place) = local_file_path_under_sync_root(sync_root, &copy.path) else {
                pass.later += 1;
                continue;
            };
            self.refresh_stale_copy(&copy, &place, &mut pass)?;
        }
        #[cfg(target_os = "windows")]
        self.wipe_stale_ranges(sync_root, &mut pass)?;
        if pass != StaleCopyPass::default() {
            tracing::info!(
                queued = pass.queued.len(),
                emptied = pass.emptied.len(),
                refetch = pass.refetch.len(),
                changed = pass.changed,
                later = pass.later,
                "stale copies: brought to the current version"
            );
        }
        Ok(pass)
    }

    /// dc-103c (design 1640 "Not done" (b)): placeholders holding only RANGES of a version the pull moved past have no
    /// agreed content, so the rule above never names them, and their next session continued the older version. They
    /// are emptied (`windows_cf::stale_ranges`) so the next open fetches the current version; within the same budget.
    #[cfg(target_os = "windows")]
    fn wipe_stale_ranges(&self, sync_root: &Path, pass: &mut StaleCopyPass) -> Result<(), EngineError> {
        use crate::windows_cf::stale_ranges::Wiped;
        for stale in self.db.list_stale_ranges(STALE_COPIES_READ_PER_PASS)? {
            if pass.queued.len() + pass.emptied.len() >= STALE_COPIES_PER_PASS {
                pass.later += 1;
                continue;
            }
            let Ok(place) = local_file_path_under_sync_root(sync_root, &stale.path) else {
                pass.later += 1;
                continue;
            };
            match crate::windows_cf::stale_ranges::wipe(&self.db, &stale, &place) {
                Wiped::Emptied => pass.emptied.push(stale.file_id),
                // A placeholder that holds nothing needs no wipe (populate gives it the current identity).
                Wiped::Later("empty") => {}
                Wiped::Later(_) => pass.later += 1,
            }
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn refresh_stale_copy(
        &self,
        copy: &crate::state_db::StaleCopy,
        place: &Path,
        pass: &mut StaleCopyPass,
    ) -> Result<(), EngineError> {
        use crate::windows_cf::stale_copy::Refreshed;
        match crate::windows_cf::stale_copy::refresh(&self.db, copy, place) {
            // Review P2-2: kept when the handle read a pin, or the database names one.
            Refreshed::Emptied { kept } => {
                pass.emptied.push(copy.file_id.clone());
                if kept || copy.pinned {
                    pass.refetch.push(KeptPlace {
                        file_id: copy.file_id.clone(),
                        place: place.to_path_buf(),
                    });
                }
            }
            Refreshed::Changed => pass.changed += 1,
            Refreshed::Later(_) => pass.later += 1,
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn refresh_stale_copy(
        &self,
        copy: &crate::state_db::StaleCopy,
        place: &Path,
        pass: &mut StaleCopyPass,
    ) -> Result<(), EngineError> {
        if !place.is_file() {
            pass.later += 1;
            return Ok(());
        }
        match cached_sha256_file_hex(&copy.file_id, place) {
            Ok(hash) if hash == copy.local_hash => {}
            Ok(_) => {
                pass.changed += 1;
                return Ok(());
            }
            Err(_) => {
                pass.later += 1;
                return Ok(());
            }
        }
        // The row read again, with the bytes just checked: still behind, nothing queued since.
        let still_behind = self.db.stale_copy_still_behind(copy)?;
        let Some(entry) = self.db.get_file(&copy.file_id)?.filter(|_| still_behind) else {
            pass.later += 1;
            return Ok(());
        };
        self.enqueue_hydrate_operation(&entry, now_secs(), REFRESH_REASON)?;
        pass.queued.push(copy.file_id.clone());
        Ok(())
    }
}

/// `true` when `error` is a hydrate a tick queued (this pass's re-download, or a kept fetch) meeting a change
/// (`UnsentLocalChange`): the place holds a change made since it was checked, so the fetch is dropped, never retried
/// over it (the change keeps its bytes; neither step queues it again while the place holds it).
pub(super) fn tick_fetch_met_a_change(op: &PendingOperation, error: &anyhow::Error) -> bool {
    engine_cause::<UnsentLocalChange>(error).is_some() && is_tick_fetch(op)
}

/// `true` when `op` is a hydrate a tick queued: a re-download this pass queued (reason `newer_version`) or a kept
/// fetch (S6, reason `kept_refetch`).
pub(super) fn is_tick_fetch(op: &PendingOperation) -> bool {
    operation_metadata(op).is_ok_and(|metadata| {
        metadata["reason"] == REFRESH_REASON || metadata["reason"] == super::kept_refetch::KEPT_REASON
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use RefreshStep::*;

    /// What the order must hold, as a refusal word: every check before the first change (the identity before the bytes
    /// are read); LD-0105's measured order for emptying a placeholder; the new size and the row only after the bytes
    /// are verified gone, and before the pin goes back; the pin back last; each step once.
    fn order_refusal(order: &[RefreshStep]) -> Option<&'static str> {
        let at = |step: RefreshStep| order.iter().position(|s| *s == step);
        let mut seen = std::collections::HashSet::new();
        if !order.iter().all(|step| seen.insert(*step)) || order.len() != 11 {
            return Some("each step once");
        }
        let at = |step| at(step).expect("every step is there");
        let changes = [Unpin, MarkInSync, Dehydrate, SetCurrentMetadata, RecordRow, RestorePin];
        if [CheckIdentity, CheckAgreedBytes, CheckRow]
            .into_iter()
            .any(|check| changes.iter().any(|change| at(check) > at(*change)))
        {
            return Some("a change before a check");
        }
        if at(CheckIdentity) > at(CheckAgreedBytes) {
            return Some("bytes read before the identity");
        }
        let measured = [RecordState, Unpin, MarkInSync, Dehydrate, VerifyEmpty, RestorePin];
        if measured.windows(2).any(|pair| at(pair[0]) > at(pair[1])) {
            return Some("not the measured order");
        }
        if at(SetCurrentMetadata) < at(VerifyEmpty) || at(RecordRow) < at(VerifyEmpty) {
            return Some("size or row before the bytes are gone");
        }
        if order.last() != Some(&RestorePin) {
            return Some("the pin goes back last");
        }
        None
    }

    /// Task 2523: the Windows refresh order holds, and the check refuses an order that breaks it (each mutant below
    /// is one swap or one missing step).
    #[test]
    fn the_windows_refresh_order_checks_first_then_empties_in_the_measured_order() {
        assert_eq!(order_refusal(&REFRESH_ORDER), None, "{REFRESH_ORDER:?}");
        let swapped = |a: RefreshStep, b: RefreshStep| {
            let mut order = REFRESH_ORDER.to_vec();
            let (i, j) = (
                order.iter().position(|s| *s == a).unwrap(),
                order.iter().position(|s| *s == b).unwrap(),
            );
            order.swap(i, j);
            order
        };
        let mutants = [
            (swapped(CheckRow, Unpin), "a change before a check"),
            (
                swapped(CheckIdentity, CheckAgreedBytes),
                "bytes read before the identity",
            ),
            (swapped(MarkInSync, Dehydrate), "not the measured order"),
            (swapped(Unpin, Dehydrate), "not the measured order"),
            (
                swapped(VerifyEmpty, SetCurrentMetadata),
                "size or row before the bytes are gone",
            ),
            (swapped(RecordRow, RestorePin), "the pin goes back last"),
            (REFRESH_ORDER[..10].to_vec(), "each step once"),
        ];
        let refused = mutants
            .iter()
            .filter(|(order, word)| order_refusal(order) == Some(*word))
            .count();
        println!("stale-2523 order: {refused} of {} mutants refused", mutants.len());
        assert_eq!(refused, mutants.len());
    }
}
