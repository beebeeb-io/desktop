//! `probe` items split out of `own_data_proof.rs`.

use super::*;

/// Whether a refused start is one a proof may settle: this session is known, local data exists, and the data has no
/// recorded owner, or one recorded by email only that differs from this session's.
pub fn eligible(owner: Option<&Identity>, session: &Identity, has_local_data: bool) -> bool {
    if !session.is_known() || !has_local_data {
        return false;
    }
    match owner.filter(|owner| owner.is_known()) {
        None => true,
        // An owner recorded WITH a user id is compared by that id only (review P3-2): another id is another account,
        // R10's strongest signal, never questioned, and a session whose id is not known yet cannot be compared with it.
        Some(owner) => owner.user_id.is_none() && same_account(owner, session) == Some(false),
    }
}

/// Judge one node of this account's tree against this session's master key.
pub fn probe(master_key: &[u8; 32], node: &serde_json::Value) -> Probe {
    let set = |field: &str| {
        node.get(field)
            .is_some_and(|value| !value.is_null() && value.as_str() != Some(""))
    };
    // Sealed under someone else's key by design (review P2-3): a file received through a file request, or one a
    // collaborator uploaded into this account's folder (the server names `uploaded_by` only when it is not the owner,
    // and wraps a file key for the owner only on such uploads).
    if set("file_request_id") || set("uploaded_by") || set("owner_wrapped_file_key") {
        return Probe::Neutral;
    }
    let Some(file_id) = node.get("id").and_then(serde_json::Value::as_str) else {
        return Probe::Unproven;
    };
    // Lead review of round 2, P3-6: a node that does not say who sealed its name is no evidence of a file request, so a
    // name that does not open is unproven there too (the production API says it, since 2026-10-27).
    match node
        .get("name_encrypted")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|sealed| sealed.starts_with('{'))
    {
        // A plaintext (legacy) name, or none: proves nothing either way.
        None => Probe::Neutral,
        Some(sealed) if name_opens(master_key, file_id, sealed) => Probe::Own,
        Some(_) => Probe::Unproven,
    }
}

/// The decision over the whole tree (see the module docs and the PR's Decisions).
pub fn verdict(tally: &Tally) -> Verdict {
    // Anything this account cannot explain blocks: a name that does not open under its key, or a row the server named
    // that is not in its tree and not gone from one of its folders.
    if tally.unproven > 0 || tally.missing > 0 {
        return Verdict::NotProven;
    }
    // Review P3-1: rows that prove nothing never outnumber the rows that prove ownership, and at least
    // `REQUIRED_PROOFS` prove it (every row, when fewer were checked).
    if tally.neutral > tally.own {
        return Verdict::NotProven;
    }
    if tally.own > 0 && tally.own >= REQUIRED_PROOFS.min(tally.checked) {
        Verdict::Adopt
    } else {
        Verdict::NotProven
    }
}

/// An HTTP failure of the listing as the proof reads it (`None`: no status at all, e.g. offline).
pub fn listing_from_status(status: Option<u16>) -> Listing {
    match status {
        Some(401) => Listing::SignedOut,
        _ => Listing::Unavailable,
    }
}

/// This account's tree as the proof reads it (self-review F2): the snapshot, which lists only what is not in the trash,
/// and the trash listing, so a row the person trashed on the server while sync was refused is still found in the tree.
/// Either one unread is a listing that did not answer.
#[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
pub fn listing_of(
    snapshot: anyhow::Result<crate::api_client::SyncSnapshot>,
    trashed: anyhow::Result<Vec<serde_json::Value>>,
) -> Listing {
    match (snapshot, trashed) {
        (Ok(snapshot), Ok(trashed)) => Listing::Nodes(snapshot.nodes.into_iter().chain(trashed).collect()),
        (Err(error), _) | (_, Err(error)) => listing_from_status(
            error
                .downcast_ref::<reqwest::Error>()
                .and_then(reqwest::Error::status)
                .map(|status| status.as_u16()),
        ),
    }
}
