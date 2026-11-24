//! Whether a Mac may ask for an item's Finder location: only for one of the person's own rows.
//! Must not call the File Provider.

use crate::state_db::Namespace;

/// "Show in Finder" for an id this Mac's state database does not know. Nothing to resolve.
pub const ITEM_NOT_KNOWN: &str = "That item is not known on this Mac yet.";

/// The File Provider-style id of the "shared with me" pseudo folder (`ipc_socket::NAMESPACE_SHARED_WITH_ME`). It is not a
/// row in the state database, and it is never shown in Finder.
pub const SHARED_WITH_ME_PSEUDO_FOLDER: &str = "namespace:shared_with_me";

/// "Show in Finder" for a row that is shared with the person (task 1885 fix round 1, I3). Shared content is webapp-only
/// by ruling 1701: it is not in Finder. A Mac never asks the File Provider for its location: macOS may fail the lookup
/// (a toast for a result that was offered) or place it under "Beebeeb", the surfacing the change feed filters out.
pub const SHARED_NOT_IN_FINDER: &str = "Files shared with you are not in Finder; they open in the web app.";

/// Whether a Mac may ask for an item's Finder location (task 1885 fix round 1, I3). `namespace_of` reads the item's
/// namespace from the state database (`Ok(None)`: no such row). Anything that is not plainly one of the person's own
/// rows is refused, and a lookup that fails is refused too, so the File Provider is asked about nothing it should not
/// be. The id is checked here, before any bridge call.
pub fn finder_may_show(
    item_id: &str,
    namespace_of: impl FnOnce(&str) -> Result<Option<Namespace>, String>,
) -> Result<(), String> {
    if item_id == SHARED_WITH_ME_PSEUDO_FOLDER {
        return Err(SHARED_NOT_IN_FINDER.to_string());
    }
    match namespace_of(item_id)? {
        Some(Namespace::SharedWithMe) => Err(SHARED_NOT_IN_FINDER.to_string()),
        Some(_) => Ok(()),
        None => Err(ITEM_NOT_KNOWN.to_string()),
    }
}

/// The namespace of the row `item_id` names, for [`finder_may_show`]. A provisional id whose
/// create has landed (an old quick-search result, a request after the system applied the swap)
/// is followed to the server file it became (spec 2026-10-09 §7.1), and that row decides.
/// `Ok(None)`: neither a row nor an alias.
pub fn item_namespace(db: &crate::state_db::StateDb, item_id: &str) -> Result<Option<Namespace>, String> {
    let landed_as = db
        .resolve_alias(item_id)
        .map_err(|error| format!("read item: {error}"))?;
    db.get_file_contract_state(landed_as.as_deref().unwrap_or(item_id))
        .map(|state| state.map(|state| state.namespace))
        .map_err(|error| format!("read item: {error}"))
}
