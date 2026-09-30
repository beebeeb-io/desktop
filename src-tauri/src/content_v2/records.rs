//! Version-1 owner body field order. Fixed arity rejects unknown authority fields.
//! Large inventories are fragment references, never inline unbounded lists.
use super::*;

pub(super) fn names(kind: &str) -> Result<&'static [&'static str]> {
    Ok(match kind {
        "FileControl" => &[
            "account",
            "root",
            "file",
            "aliases",
            "binding",
            "witness",
            "state",
            "latest_sequence",
            "anchor",
            "active_owner",
        ],
        "Snapshot" => &[
            "account",
            "root",
            "file",
            "artifact",
            "sequence",
            "predecessor",
            "base",
            "digest",
            "eof",
            "witness",
        ],
        "Upload" => &[
            "account",
            "root",
            "file",
            "snapshot",
            "deny_token",
            "intent",
            "phase",
            "session_lookup",
            "predecessor_receipt",
        ],
        "Finalization" => &[
            "account",
            "root",
            "file",
            "operation",
            "snapshot",
            "intent",
            "server_receipt",
            "digest",
            "eof",
            "binding_revision",
            "witness",
            "disposition",
        ],
        "Resolution" => &[
            "account",
            "root",
            "file",
            "choice",
            "snapshot",
            "cutoff",
            "remote",
            "witness",
            "phase",
            "covered_group",
            "catchup",
        ],
        "RemoteCatchUp" => &[
            "account",
            "root",
            "file",
            "resolution",
            "selected_remote",
            "highest_remote",
            "phase",
            "binding_revision",
            "terminal_receipt",
        ],
        "InstallIntent" => &[
            "account",
            "root",
            "file",
            "purpose",
            "witness",
            "preimage",
            "replacement",
            "remote",
            "phase",
            "return_state",
        ],
        "RecoveryCase" => &[
            "account",
            "root",
            "file",
            "reason",
            "related_group",
            "format",
            "captured_at",
            "digest",
            "eof",
            "manifest",
            "disposition",
        ],
        "Migration" => &[
            "account",
            "root",
            "file",
            "old_policy",
            "new_policy",
            "inventory_group",
            "cursor",
            "phase",
            "legacy_journal",
            "blocker_group",
        ],
        "RootRetirement" => &[
            "account",
            "root",
            "file",
            "witness",
            "registrations",
            "dacl_group",
            "inventory_group",
            "cursor",
            "phase",
            "blocker_group",
        ],
        _ => bail!("unknown owner kind"),
    })
}
fn phases(kind: &str) -> &'static [&'static str] {
    match kind {
        "Upload" => &["Pending", "Uploading", "CompletingUnknown", "Receipt", "Detached"],
        "Finalization" => &["AwaitingBinding", "AwaitingCleanliness", "Recovery", "Done"],
        "Resolution" => &[
            "ChoiceCommitted",
            "Downloading",
            "Uploading",
            "Installing",
            "Committed",
            "Recovery",
        ],
        "RemoteCatchUp" => &[
            "CheckRequired",
            "NewerPending",
            "CaughtUp",
            "SupersededByNamespaceOperation",
            "RootRetired",
        ],
        "InstallIntent" => &[
            "Prepared",
            "Writing",
            "BytesVerified",
            "Committed",
            "CleanupPending",
            "Done",
        ],
        "Migration" => &["Inventory", "Converting", "Ready", "Recovery"],
        "RootRetirement" => &[
            "SealIntent",
            "Sealing",
            "Sealed",
            "Cleanup",
            "UnsealIntent",
            "Unsealing",
            "Active",
            "DetachIntent",
            "Detaching",
            "Unregistered",
            "RemoveIntent",
            "Removed",
            "Retired",
        ],
        _ => &[],
    }
}
pub(super) fn decode(kind: &str, body: &[u8]) -> Result<Vec<Vec<u8>>> {
    let names = names(kind)?;
    let fields = wire::fields(body, kind, names.len())?;
    for (name, value) in names.iter().zip(&fields) {
        match *name {
            "account" | "root" | "file" | "artifact" | "snapshot" | "deny_token" | "operation" | "resolution"
            | "digest" | "preimage" | "replacement" | "catchup" | "covered_group" | "related_group"
            | "inventory_group" | "blocker_group" | "dacl_group" => {
                ensure!(value.len() == 32, "invalid {kind}.{name} ID/digest")
            }
            "predecessor" | "active_owner" => ensure!(value.is_empty() || value.len() == 32, "invalid optional ID"),
            "sequence" | "eof" | "latest_sequence" | "binding_revision" | "cutoff" | "captured_at" | "cursor" => {
                ensure!(value.len() == 8, "invalid {name} integer")
            }
            "state" | "return_state" => ensure!(
                ["S0", "S1", "S2", "S3", "S5", "S6", "S7", "S8", "S9", "S10", "S11"]
                    .iter()
                    .any(|s| s.as_bytes() == value),
                "invalid file state"
            ),
            "purpose" => ensure!(
                ["Hydration", "KeepTheirs", "KeepBothRemote", "RecoveryRestore"]
                    .iter()
                    .any(|s| s.as_bytes() == value),
                "invalid install purpose"
            ),
            "format" => ensure!(
                ["Full", "LegacyPartial", "Unknown"]
                    .iter()
                    .any(|s| s.as_bytes() == value),
                "invalid recovery format"
            ),
            "choice" => ensure!(
                ["KeepMine", "KeepTheirs", "KeepBoth"]
                    .iter()
                    .any(|s| s.as_bytes() == value),
                "invalid choice"
            ),
            "phase" => ensure!(phases(kind).iter().any(|s| s.as_bytes() == value), "unknown phase"),
            "intent" => {
                let ordinary = wire::fields(value, "OrdinaryLineage", 1).is_ok();
                let resolution = wire::fields(value, "ResolutionUpload", 4).is_ok();
                ensure!(ordinary || resolution, "unknown immutable upload intent");
            }
            "legacy_journal" => {
                ensure!(kind == "Migration", "legacy journal outside migration");
                if !value.is_empty() {
                    wire::fields(value, "LegacyMaterialization", 4)?;
                }
            }
            _ => ensure!(value.len() <= 32768, "oversized nested owner field"),
        }
    }
    Ok(fields)
}
pub(super) fn encode(kind: &str, fields: &[Vec<u8>]) -> Result<Vec<u8>> {
    let refs: Vec<&[u8]> = fields.iter().map(Vec::as_slice).collect();
    let body = wire::record(kind, &refs)?;
    decode(kind, &body)?;
    Ok(body)
}
/// Canonical fixture records supply every field. Behavioral slices replace these
/// fixture values with native/version witnesses, not a second storage format.
pub(super) fn fixture(kind: &str, account: Id, root: Id) -> Result<Vec<u8>> {
    let fields = names(kind)?
        .iter()
        .map(|name| match *name {
            "account" => account.to_vec(),
            "root" => root.to_vec(),
            "file" | "artifact" | "snapshot" | "deny_token" | "operation" | "resolution" | "digest" | "preimage"
            | "replacement" | "catchup" | "covered_group" | "related_group" | "inventory_group" | "blocker_group"
            | "dacl_group" => id().to_vec(),
            "predecessor" | "active_owner" | "legacy_journal" | "disposition" => Vec::new(),
            "sequence" | "eof" | "latest_sequence" | "binding_revision" | "cutoff" | "captured_at" | "cursor" => {
                0u64.to_be_bytes().to_vec()
            }
            "state" | "return_state" => b"S11".to_vec(),
            "purpose" => b"RecoveryRestore".to_vec(),
            "format" => b"Unknown".to_vec(),
            "choice" => b"KeepMine".to_vec(),
            "phase" => phases(kind)[0].as_bytes().to_vec(),
            "intent" => wire::record("OrdinaryLineage", &[b"immutable-fixture-base"]).unwrap(),
            "witness" => wire::record("Witness", &[b"volume", b"file", b"exclusion"]).unwrap(),
            _ => b"fixture-only".to_vec(),
        })
        .collect::<Vec<_>>();
    encode(kind, &fields)
}
