//! Slice B3 (plan40 batch 1): "Show sync status on files" (DW-0746), honoured by File Explorer on Windows. Part of
//! [`EngineDriveSettings`] (`drive_settings.rs`), which hands `set_status_on_files` here through `not_wired(…)`.
//!
//! Not bound yet: the adapter's body of before the scaffold, moved unchanged. The engine stores the setting and
//! nothing honours it yet. Session-bound like its neighbours once bound.

use super::*;

/// What slice B3 keeps between a press and its answer: its worker's inbox, what it holds for one session and
/// account. The adapter holds one (`EngineDriveSettings::status_on_files`). Empty until B3 binds.
#[derive(Default)]
pub(super) struct StatusOnFilesCalls {}

impl<E: DriveEngine> EngineDriveSettings<E> {
    pub(super) fn status_on_files_set_status_on_files(&mut self, _on: bool) {}
}
