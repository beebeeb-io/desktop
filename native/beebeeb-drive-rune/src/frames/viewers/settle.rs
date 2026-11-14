//! What a change that left the window's thread became, applied to the open file.

use super::Ui;
use super::source::{OpenFile, SaveOutcome, Settled};

impl Ui {
    /// What a change that left the window's thread became.
    pub(super) fn settle(&mut self, settled: Settled) {
        match settled {
            Settled::Offline { on, done } => {
                if done {
                    self.file.details.offline = on;
                } else {
                    self.toast = Some("Could not change Available offline".to_string());
                }
            }
            Settled::Saved {
                id,
                name,
                body,
                version,
                at,
            } => {
                let file = OpenFile {
                    id,
                    name,
                    version,
                    body,
                    newer: None,
                    ..self.file.clone()
                };
                self.outcome(SaveOutcome::Saved { file, at });
            }
            Settled::Queued { body } => {
                let file = OpenFile {
                    body,
                    newer: None,
                    ..self.file.clone()
                };
                self.show(file);
                self.toast = Some("Saved on this PC \u{00B7} sends when Beebeeb can be reached".to_string());
            }
            Settled::Opened { body, version } => {
                let file = OpenFile {
                    body,
                    version,
                    newer: None,
                    ..self.file.clone()
                };
                self.outcome(SaveOutcome::Opened(file));
            }
            Settled::Conflict(other) => self.outcome(SaveOutcome::Conflict(other)),
            Settled::Failed => {
                self.dialog = None;
                self.toast = Some("Not saved".to_string());
            }
        }
    }
}
