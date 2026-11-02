//! `adopt` items split out of `own_data_proof.rs`.

use super::*;

/// Run the proof for `session` over `db` and, when it holds, record `session` as the owner and hold for review what
/// could not be tied to its data, in one transaction. `list` asks the server for this account's tree; a listing that
/// does not answer is asked once more. `still_current` is checked before every question and right before the record
/// is written, so a session that changed meanwhile (a Lock, a sign-out, another sign-in) stops the asking and records
/// nothing. The owner is replaced only if it is still the one the proof began with. Nothing is ever deleted.
pub async fn prove_and_adopt<L, Fut>(
    db: &StateDb,
    session: &Identity,
    master_key: &[u8; 32],
    list: L,
    still_current: impl Fn() -> bool,
) -> Result<Outcome, String>
where
    L: Fn() -> Fut,
    Fut: Future<Output = Listing>,
{
    let owner = db.owner().map_err(|e| format!("read the local data owner: {e}"))?;
    let has_local_data = db
        .has_account_data()
        .map_err(|e| format!("inspect the local data: {e}"))?;
    if !eligible(owner.as_ref(), session, has_local_data) {
        return Ok(Outcome::NotEligible);
    }
    let mut tally = Tally::default();
    let mut nodes = None;
    for _ in 0..2 {
        if !still_current() {
            return Ok(Outcome::Changed(tally));
        }
        match list().await {
            Listing::Nodes(listed) => {
                nodes = Some(listed);
                break;
            }
            Listing::SignedOut => {
                tally.signed_out = 1;
                return Ok(Outcome::Unavailable(tally));
            }
            Listing::Unavailable => {}
        }
    }
    let Some(nodes) = nodes else {
        tally.unavailable = 1;
        return Ok(Outcome::Unavailable(tally));
    };
    // Read after the listing: whatever was written while it ran is judged too, or held (it is not in `proven`).
    let rows = db.own_data_proof_rows().map_err(|e| format!("read the local rows: {e}"))?;
    let ops = db.own_data_proof_ops().map_err(|e| format!("read the queue: {e}"))?;
    let Judgement { tally, proven } = judge(master_key, &rows, &ops, &nodes);
    if verdict(&tally) != Verdict::Adopt {
        return Ok(Outcome::NotProven(tally));
    }
    if !still_current() {
        return Ok(Outcome::Changed(tally));
    }
    match db
        .adopt_owner_holding(owner.as_ref(), session, &proven, now_secs())
        .map_err(|e| format!("record the local data owner: {e}"))?
    {
        Some(held) => Ok(Outcome::Adopted(Tally { held_for_review: held, ..tally })),
        None => Ok(Outcome::Changed(tally)),
    }
}

pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
