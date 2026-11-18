//! Photos routes: SaveToPc, SendCopy (stub).

use std::sync::Arc;

use rune::Tx;

use super::{Intent, PhotoId};
use crate::State;
use crate::engine_host::save_copy::CopySaver;
use crate::shell::platform::Platform;

#[cfg(test)]
#[path = "route_save_send_tests.rs"]
mod tests;

pub fn handle(_s: &mut State, _tx: &mut Tx, intent: Intent) -> Result<(), Intent> {
    Err(intent)
}

const NEEDS_SYNC_ELSEWHERE: &str = "x";
const SHEET_FAILED: &str = "y";

pub(super) fn send_copy(_s: &mut State, _platform: &dyn Platform, _id: PhotoId) {}

pub(super) fn save_to_pc(_s: &mut State, _tx: &mut Tx, _platform: Arc<dyn Platform>, _saver: Arc<dyn CopySaver>, _id: PhotoId) {}
