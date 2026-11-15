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
            // Stored here, not sent yet: a newer version noticed while the save waited stays offered (its banner is
            // drawn once the editor closes, below).
            // Labelled with the version it was saved from: the text on screen is based on that one.
            Settled::Queued { body, version } => {
                let file = OpenFile {
                    body,
                    version,
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
            // The version on screen stays until Show it; a second one replaces the first banner's time.
            Settled::Newer(newer) => self.file.newer = Some(newer),
            // A change of an earlier opening: said, nothing on screen changes.
            Settled::Said(words) => self.toast = Some(words),
            Settled::Failed => {
                self.dialog = None;
                self.toast = Some("Not saved".to_string());
            }
            // Not saved, and why: the edit stays as it is.
            Settled::Refused(words) => {
                self.dialog = None;
                self.toast = Some(words);
            }
        }
    }
}
