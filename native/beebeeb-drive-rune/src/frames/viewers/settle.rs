//! What a change that left the window's thread became, applied to the open file.

use super::source::{OpenFile, SaveOutcome, Settled};
use super::office_over::Over;
use super::{Dialog, Ui};

impl Ui {
    /// What a save, a keep or its answer came to, applied.
    pub(super) fn outcome(&mut self, outcome: SaveOutcome) {
        match outcome {
            SaveOutcome::Saved { file, at } => {
                self.toast = Some(format!("Saved \u{00B7} encrypted \u{00B7} version {}", file.version));
                self.saved_at = Some(at);
                self.show(file);
            }
            SaveOutcome::Opened(file) => self.show(file),
            SaveOutcome::Conflict(other) => self.ask(Dialog::Conflict(other)),
            // Not saved: the edit stays as it is, and so does what the status line says.
            SaveOutcome::Failed => self.dialog = None,
            // On its way: the edit stays as it is until the engine answers. An earlier refusal was the last press's.
            SaveOutcome::Working => {
                self.dialog = None;
                self.toast = Some("Saving\u{2026}".to_string());
                if let Some(editor) = self.editor.as_mut() {
                    editor.refused = None;
                }
            }
        }
    }

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
                self.toast = Some(crate::frames::office::edit::save::QUEUED_TOAST.to_string());
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
            // A live office editor asked: its question is DW-1554's, over the document, not the text editor's dialog.
            Settled::Conflict(other) if self.office.edit.is_some() => {
                self.dialog = None;
                self.office.save = None;
                if let Some(edit) = self.office.edit.as_mut() {
                    edit.saving = false;
                }
                self.office.over = Some(Over::Conflict(other));
            }
            Settled::Conflict(other) => self.outcome(SaveOutcome::Conflict(other)),
            // The save on its way was made from this revision: that is what it lands as.
            Settled::SavedRevision(revision) => {
                if let Some(sent) = self.office.save.as_mut() {
                    sent.revision = revision;
                }
            }
            // The version on screen stays until Show it; a second one replaces the first banner's time.
            Settled::Newer(newer) => self.file.newer = Some(newer),
            // A change of an earlier opening: said, nothing on screen changes.
            Settled::Said(words) => self.toast = Some(words),
            Settled::Failed => {
                self.dialog = None;
                self.toast = Some("Not saved".to_string());
            }
            // Not saved, and why: the edit stays as it is. The words go in the editor's banner, which offers Save a
            // copy (task 2530, item 1); said once, never also as a toast.
            Settled::Refused(words) => {
                self.dialog = None;
                match self.editor.as_mut() {
                    Some(editor) => {
                        editor.refused = Some(words);
                        self.toast = None;
                    }
                    None => self.toast = Some(words),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::office::edit::EditUi;
    use crate::frames::viewers::office_over::Over;
    use crate::frames::viewers::source::{Body, Conflict, FixtureViewerSource, OpenError, ViewerSource, board};
    use beebeeb_office_protocol::office::EditKind;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    type Answers = Rc<RefCell<VecDeque<Settled>>>;

    /// The fixture whose office save is handed to the engine and answers as the test queues it.
    struct Later {
        inner: FixtureViewerSource,
        settled: Answers,
    }

    impl ViewerSource for Later {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.inner.open(id)
        }
        fn poll(&mut self, id: &str) -> Option<Body> {
            self.inner.poll(id)
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.inner.retry(id)
        }
        fn save_document(&mut self, _: &str, _: u32) -> SaveOutcome {
            SaveOutcome::Working
        }
        fn settled(&mut self, _: &str) -> Option<Settled> {
            self.settled.borrow_mut().pop_front()
        }
    }

    /// A live Word editor at revision 1, saved up to 0.
    fn live() -> (Ui, Answers) {
        let settled = Answers::default();
        let source = Later {
            inner: FixtureViewerSource::new(),
            settled: settled.clone(),
        };
        let mut ui = Ui::open(Box::new(source), board::DOCX).expect("the fixture opens it");
        let mut edit = EditUi::new(EditKind::Docx);
        edit.revision = 1;
        ui.office.edit = Some(edit);
        (ui, settled)
    }

    #[test]
    fn the_saved_revision_is_what_the_save_lands_as() {
        let (mut ui, settled) = live();
        ui.office_save();
        // Typed during the save, and the helper had serialised revision 1.
        ui.office.edit.as_mut().expect("edit").revision = 3;
        settled.borrow_mut().push_back(Settled::SavedRevision(1));
        settled.borrow_mut().push_back(Settled::Saved {
            id: ui.file.id.clone(),
            name: ui.file.name.clone(),
            body: ui.file.body.clone(),
            version: 8,
            at: "09:41".into(),
        });
        assert!(ui.pump());
        assert!(ui.pump());
        assert!(ui.office_settle_save());
        let edit = ui.office.edit.as_ref().expect("edit");
        assert_eq!(edit.saved_revision, 1);
        assert!(edit.unsaved(), "revision 3 is still unsaved");
    }

    #[test]
    fn a_conflict_from_the_engine_goes_to_the_documents_question() {
        let (mut ui, settled) = live();
        ui.office_save();
        assert!(ui.office.edit.as_ref().is_some_and(|e| e.saving));
        settled.borrow_mut().push_back(Settled::Conflict(Conflict {
            from: "Anna's iPhone".into(),
            at: "09:14".into(),
            version: 8,
        }));
        assert!(ui.pump());
        assert!(matches!(ui.office.over, Some(Over::Conflict(_))));
        assert!(ui.dialog.is_none(), "the text editor's question is not asked");
        assert!(ui.office.edit.as_ref().is_some_and(|e| !e.saving && e.unsaved()));
    }
}
