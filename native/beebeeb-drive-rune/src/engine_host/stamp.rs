//! The session and the account something is asked under: taken on the window's thread at the press, before a worker
//! starts, handed to the engine with the call, and compared again when the answer lands. The Files source's moves to and
//! from Trash use it; the older adapters still carry their own copies of the same three items.

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::{CredentialStores, Engine};

/// The session and the account of a press.
pub(super) type Stamp = (SessionGeneration, Option<String>);

/// Whose account is here: the stored email, else the user id the server named (one of them, the same one for one
/// account in one run). In memory only: never waits.
pub(super) fn account_of<C: CredentialStores>(engine: &Engine<C>) -> Option<String> {
    engine.account_email().or_else(|| engine.user_id())
}

/// The session and account a press on `engine` is made under.
pub(super) fn stamp<C: CredentialStores>(engine: &Engine<C>) -> Stamp {
    (engine.session_turn(), account_of(engine))
}
