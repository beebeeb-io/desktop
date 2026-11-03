//! A19 moves: the source side comes from the op, never the moved row (P1-2),
//! and the subtree walk is bounded and explicit (P2-7).

use super::*;

impl crate::engine_bridge::EngineBridge {
    /// `id` and every descendant in the local mirror, or `None` when the
    /// mirror's bound is reached: a silent stop would under-adopt a move.
    fn subtree_ids(&self, id: &str) -> Option<Vec<String>> {
        const MAX_IDS: usize = 20_000;
        let entries = self.db().list_files().ok()?;
        let mut by_parent: HashMap<&str, Vec<&str>> = HashMap::new();
        for entry in &entries {
            if let Some(parent) = entry.parent_id.as_deref() {
                by_parent.entry(parent).or_default().push(entry.file_id.as_str());
            }
        }
        let mut seen: HashSet<String> = HashSet::from([id.to_string()]);
        let mut stack = vec![id.to_string()];
        while let Some(parent) = stack.pop() {
            for child in by_parent.get(parent.as_str()).into_iter().flatten() {
                if seen.insert((*child).to_string()) {
                    stack.push((*child).to_string());
                }
            }
            if seen.len() > MAX_IDS {
                return None;
            }
        }
        Some(seen.into_iter().collect())
    }

    /// A19: an owner move that takes an item (or a subtree) out of the shared
    /// folder whose custody key opens its foreign descendants must adopt every
    /// foreign item involved first, confirmed by read-back. A move that stays
    /// in the same shared root, or touches no foreign item, needs nothing.
    /// Anything else is refused with the reason. The source side comes from the
    /// op's `vacated_parent_id` (a local rename already moved the row).
    pub(crate) async fn adopt_foreign_subtree(
        &self,
        item_id: &str,
        from_parent: Option<&str>,
        new_parent: Option<&str>,
    ) -> anyhow::Result<()> {
        let scope = self.owner_key_scope();
        scope.check()?;
        // Never the moved row: its `parent_id` is already the destination here.
        let from_root = self.shared_folder_root_of(from_parent).or_else(|| {
            self.db()
                .has_seen_folder_custody(item_id)
                .ok()
                .filter(|seen| *seen)
                .map(|_| item_id.to_string())
        });
        let Some(from_root) = from_root else {
            // P2-4: the source root is unknown (pruned, created only on the server,
            // or a rehomed kept row). If the subtree holds foreign ids, allowing the
            // move would strand their custody key: refuse rather than adopt blind.
            let subtree = self.subtree_ids(item_id).ok_or_else(|| {
                anyhow::anyhow!("refusing a move whose local subtree exceeds the inspection bound")
            })?;
            let foreign = subtree
                .iter()
                .filter(|id| beebeeb_core::item_id::is_foreign_item_id(id).unwrap_or(false))
                .count();
            anyhow::ensure!(
                foreign == 0,
                "move needs adoption: an unknown source for a subtree of {foreign} foreign items"
            );
            return Ok(());
        };
        if self.shared_folder_root_of(new_parent).as_deref() == Some(from_root.as_str()) {
            return Ok(());
        }
        let subtree = self.subtree_ids(item_id).ok_or_else(|| {
            anyhow::anyhow!("refusing a move whose local subtree exceeds the inspection bound")
        })?;
        let foreign: Vec<String> = subtree
            .into_iter()
            .filter(|id| beebeeb_core::item_id::is_foreign_item_id(id).unwrap_or(false))
            .collect();
        // §5: above ten thousand foreign items the move is refused, not judged
        // from a truncated list.
        ensure_adoption_bound(foreign.len())?;
        if foreign.is_empty() {
            return Ok(());
        }
        let mut rows = Vec::with_capacity(foreign.len());
        for id in &foreign {
            let row = self.api().get_file(id).await?;
            scope.check()?;
            rows.push(row);
        }
        let prepared = self.prepare_owned_rows(&rows).await?;
        for item in &prepared {
            anyhow::ensure!(
                item.readable
                    && item.record.owner_wrap.is_some()
                    && item.record.pending_owner_wrap.is_none()
                    && item.record.blocked_fingerprint.is_none(),
                "move needs adoption: {}",
                item.record.file_id
            );
        }
        Ok(())
    }

}

/// §5: the adoption bound. Exactly 10,000 is allowed; 10,001 is refused, never
/// judged from a truncated list.
pub(crate) fn ensure_adoption_bound(count: usize) -> anyhow::Result<()> {
    anyhow::ensure!(
        count <= 10_000,
        "refusing a move of {count} foreign items: above the 10,000 items this device can adopt at once"
    );
    Ok(())
}

