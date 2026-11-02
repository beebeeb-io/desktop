//! `working_set_signal` items split out of `runner.rs`.

use super::*;

/// Pure decision core for the materialized-set signal filter (task 1697,
/// audit P0 item 4): a batch is worth a working-set signal when at least one
/// changed item's old-or-new parent is in the materialized set, or when the
/// materialized set is unknown/empty — Apple's documented fallback ("if the
/// extension doesn't keep track of the materialized set… the working set is
/// the entire dataset"), which fails OPEN so changes can never be lost to a
/// missing filter.
///
/// `changed_parent_ids`: the parent folder id of each changed item (None =
/// unknown — vault-root items, deletions whose row is already gone). Task
/// 1701: Finder shows ONE root ("Beebeeb"), the synthetic namespace roots
/// are gone, and the root container is always materialized (it is presented
/// without ever entering the system's materialized-set enumeration), so
/// `None` parents — root-child items under the single root — always signal.
/// A `namespace:` id can no longer occur (`normalize_parent_id` strips it
/// from writes) and no longer signals.
#[cfg(any(target_os = "macos", test))]
pub fn working_set_signal_needed(changed_parent_ids: &[Option<String>], materialized: &[String]) -> bool {
    if materialized.is_empty() {
        return !changed_parent_ids.is_empty();
    }
    let materialized: std::collections::HashSet<&str> = materialized.iter().map(String::as_str).collect();
    changed_parent_ids.iter().any(|parent| match parent {
        None => true,
        Some(parent_id) => {
            parent_id == "__fp_root__"
                || parent_id == "NSFileProviderRootContainerItemIdentifier"
                || parent_id == "rootContainer"
                // Task 1698: the trash container is a SYSTEM container —
                // always materialized, symmetric with the Swift
                // WorkingSetStore.alwaysMaterializedIdentifiers().
                || parent_id == "NSFileProviderTrashContainerItemIdentifier"
                || parent_id == "trashContainer"
                || materialized.contains(parent_id.as_str())
        }
    })
}

/// Task 1697: the daemon's ONLY replica-refresh channel. Under
/// `NSFileProviderReplicatedExtension` the system honors signals for the
/// WORKING SET alone (`.rootContainer` is ignored by design); the extension's
/// enumerator then pulls the daemon's change log via `ListChanges`. This
/// replaces the dead `file-provider-invalidate` Tauri event (zero consumers)
/// with a real signal through the ObjC FFI bridge. macOS-only and
/// best-effort: a failed signal is logged and never fails the tick.
///
/// Returns `true` when the signal was DROPPED because the shared bridge gate was busy (Task 9 fix round 1,
/// item B): the caller owes those ids to the next tick (`OwedWorkingSetSignal`), because nothing else
/// re-sends them.
#[cfg(target_os = "macos")]
pub(super) fn signal_file_provider_working_set(db: &StateDb, reason: &str, item_ids: &[String]) -> bool {
    if !crate::macos_file_provider::should_signal_working_set(item_ids) {
        return false;
    }
    // Audit P0 item 4: filter the signal to changes whose old-or-new parent
    // is materialized (the set the extension publishes via ReportMaterialized).
    // Fail-open on empty/unknown (see working_set_signal_needed).
    let parents: Vec<Option<String>> = item_ids
        .iter()
        .map(|id| db.get_file(id).ok().flatten().map(|entry| entry.parent_id).flatten())
        .collect();
    let materialized = db.materialized_containers().unwrap_or_default();
    if !working_set_signal_needed(&parents, &materialized) {
        tracing::debug!(
            reason,
            items = item_ids.len(),
            "no materialized parent changed; skipping the working-set signal"
        );
        return false;
    }
    // Through the one bridge gate, like every other File Provider call (lead ruling T8-gate-all): a busy
    // gate fails at once, and the signal is reported as dropped so the caller can owe it to the next tick.
    let mut dropped = false;
    #[cfg(test)]
    SIGNALS_ASKED.with(|asked| asked.set(asked.get() + 1));
    match crate::finder_setup::macos_ports::signal_working_set() {
        Ok(outcome) => {
            tracing::debug!(
                reason,
                items = item_ids.len(),
                ?outcome,
                "signaled the File Provider working set"
            );
        }
        Err(e) if crate::finder_setup::macos_ports::is_gate_busy(&e) => {
            dropped = true;
            tracing::warn!(
                reason,
                "the working-set signal met a busy bridge gate; it is owed to the next tick"
            );
        }
        Err(e) => {
            tracing::warn!(reason, error = %e, "signaling the File Provider working set failed (best-effort)");
        }
    }
    // Best-effort log hygiene: the replica can only have consumed changes
    // from the last ~week (it polls on every signal), so anything older is
    // safe to drop. NEVER touches fp_sync_anchor (the replica's cursor).
    if let Err(e) = db.sweep_file_changes(now_secs() - 7 * 24 * 3600) {
        tracing::debug!(error = %e, "change-log sweep failed (best-effort)");
    }
    dropped
}

/// Non-macOS stub: Windows CFAPI refreshes placeholders natively and Linux
/// FUSE is an unmounted prototype.
#[cfg(not(target_os = "macos"))]
pub(super) fn signal_file_provider_working_set(db: &StateDb, reason: &str, item_ids: &[String]) -> bool {
    let _ = (db, reason, item_ids);
    false
}

/// The most ids a dropped signal's debt keeps. A gate that stays busy for hours must not make the debt
/// grow without bound.
pub(super) const OWED_SIGNAL_MAX_IDS: usize = 2000;

/// What the debt becomes past the cap: one id no row has. `working_set_signal_needed` treats the
/// unknown parent as a change that always signals, so the signal still goes out, unfiltered.
pub(super) const OWED_SIGNAL_UNKNOWN_ID: &str = "__owed-working-set-signal__";

/// Working-set signals that met a busy bridge gate and are owed to the next tick (Task 9 fix round 1,
/// item B). The signal is the daemon's ONLY replica-refresh channel; the gate never queues, so a signal
/// that finds it busy (a reconciler `addDomain`, a window-focus read, the startup sweep) is dropped, and
/// before this nothing re-sent it: on a quiet account Finder stayed stale until some later tick happened
/// to change a materialized parent. Now the dropped ids are added to the next tick's signal, so the
/// staleness is at most one tick (`TICK_INTERVAL`, 30 s).
#[derive(Debug, Default)]
pub(super) struct OwedWorkingSetSignal {
    pub(super) ids: Vec<String>,
}

/// Task 2190: working-set signals a command could not deliver (a decision on a parked conflict met a
/// busy bridge gate, or the signal failed). The next tick takes them into its debt
/// ([`OwedWorkingSetSignal::take_command_debt`]), so they are re-sent with its signal, and stay owed
/// while the gate is busy, by the same rule as the tick's own.
pub(super) static COMMAND_OWED_SIGNAL: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Task 2190: owe `ids` to the next tick's working-set signal.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn owe_working_set_signal(ids: &[String]) {
    let mut owed = COMMAND_OWED_SIGNAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for id in ids {
        if !owed.contains(id) {
            owed.push(id.clone());
        }
    }
}

impl OwedWorkingSetSignal {
    /// Task 2190: the commands' undelivered signals join this debt (once: the queue is emptied).
    pub(super) fn take_command_debt(&mut self) {
        let taken = std::mem::take(&mut *COMMAND_OWED_SIGNAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
        if !taken.is_empty() {
            let merged = self.merged_with(&taken);
            self.record(true, merged);
        }
    }

    /// A debt that opens with `ids`: changes recorded before the tick loop started, which the first
    /// tick's signal must carry. Bounded like any other debt.
    pub(super) fn owing(ids: Vec<String>) -> Self {
        let mut owed = Self::default();
        owed.record(!ids.is_empty(), ids);
        owed
    }

    /// What this tick should signal: the debt, then this tick's ids, first-seen order, no duplicates. Not
    /// empty whenever something is owed, which keeps `should_signal_working_set` from skipping a quiet tick.
    pub(super) fn merged_with(&self, ids: &[String]) -> Vec<String> {
        let mut merged: Vec<String> = Vec::with_capacity(self.ids.len() + ids.len());
        for id in self.ids.iter().chain(ids) {
            if !merged.contains(id) {
                merged.push(id.clone());
            }
        }
        merged
    }

    /// Something is owed to the next signal.
    pub(super) fn is_owing(&self) -> bool {
        !self.ids.is_empty()
    }

    /// After a signal attempt for `sent` (what `merged_with` returned): a drop makes it the new debt,
    /// anything else pays the debt.
    pub(super) fn record(&mut self, dropped: bool, sent: Vec<String>) {
        if !dropped {
            self.ids.clear();
        } else if sent.len() > OWED_SIGNAL_MAX_IDS {
            self.ids = vec![OWED_SIGNAL_UNKNOWN_ID.to_string()];
        } else {
            self.ids = sent;
        }
    }
}

/// Task 1697 review fix (T3): ONE working-set signal per tick. Merges the
/// local operation queue's invalidations with the remote ingestion's applied
/// ids (server-side creates/modifies/moves/deletes reached state.db without
/// any signal before) into a single id list, and picks the reason from
/// whichever sides contributed — a mixed tick signals ONCE, not twice. An
/// all-empty tick keeps today's shape: an `operations_applied` call with an
/// empty batch (the signal path no-ops on it but still runs its log sweep).
pub(super) fn tick_working_set_signal(local_invalidated: &[String], remote_applied: &[String]) -> (&'static str, Vec<String>) {
    if local_invalidated.is_empty() && remote_applied.is_empty() {
        return ("operations_applied", Vec::new());
    }
    let mut ids: Vec<String> = Vec::with_capacity(local_invalidated.len() + remote_applied.len());
    for id in local_invalidated.iter().chain(remote_applied.iter()) {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let reason = match (local_invalidated.is_empty(), remote_applied.is_empty()) {
        (false, false) => "operations_applied+remote_changes_applied",
        (false, true) => "operations_applied",
        (true, false) => "remote_changes_applied",
        (true, true) => unreachable!("handled above"),
    };
    (reason, ids)
}
