//! An item's read-only Preview on the real engine ([`EngineItemPreview`], the `ItemPreviewSource` of the running
//! app): the panel over Trash and Shared (`frames::item`). It reads by the Files panel's own reader
//! (`files_preview::Previews`: one preview at a time, stamped with the session and the account, the first lines only,
//! in `Zeroizing` buffers, wiped on cancel), through one of two doors:
//!
//! - **A file in Trash** (`ReadFrom::Beebeeb`): through [`BeebeebOnly`], which answers that no copy is on this PC
//!   and never asks the engine for one. The file is downloaded from Beebeeb and decrypted in memory by the engine's
//!   download path (`Engine::open_file_in_memory`); nothing about it is written to disk or to the local state.
//!   Nothing here reads the file's facts from the local state, keeps it on this PC, saves into it, or hands it to
//!   another app.
//! - **A file shared with the person** (`ReadFrom::ThisPcFirst`): the copy wholly on this PC first, as Files' panel.
//!
//! What it shows honestly: a text or Markdown file's first lines; a PDF or a picture "not available in this version
//! yet"; any other kind "no preview". A file this account's key does not open (another person's, in its folder)
//! answers "couldn't show a preview", never someone else's words.
//!
//! Must not: log a name, an id or a line.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::{LocalCopy, OpenFileError, OpenFileFacts, OpenedFile};
use beebeeb_engine::app::{CredentialStores, Engine};

use super::files_preview::Previews;
use super::viewer::{LiveViewerEngine, ViewerEngine};
use crate::frames::files::actions_data::Previewed;
use crate::frames::files::source::ActionError;
use crate::frames::item::source::{ItemPreviewSource, PreviewAsk, ReadFrom};
use crate::frames::viewers::source::Elsewhere;
use crate::source::DriveItem;

#[cfg(test)]
#[path = "item_preview_tests.rs"]
mod tests;

/// The engine as a preview of a file in Trash may ask it: the session, the account, and the download from Beebeeb.
/// Nothing on this PC: `local` answers that no copy is here (and never asks the engine), `facts` is refused (they are
/// the local state's), sync is not running for it, and no other app is asked to open it. Every edit keeps the
/// trait's refusing default; a method the trait gains later that reads this PC or writes refuses here too.
pub struct BeebeebOnly<E: ViewerEngine>(pub Arc<E>);

impl<E: ViewerEngine> ViewerEngine for BeebeebOnly<E> {
    fn session_turn(&self) -> SessionGeneration {
        self.0.session_turn()
    }

    fn has_session(&self) -> bool {
        self.0.has_session()
    }

    fn account(&self) -> Option<String> {
        self.0.account()
    }

    fn facts(&self, _file_id: &str) -> Result<OpenFileFacts, OpenFileError> {
        Err(OpenFileError::Failed)
    }

    fn fetch(
        &self,
        file_id: &str,
        max_bytes: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        self.0.fetch(file_id, max_bytes, progress, cancel)
    }

    fn local(&self, _file_id: &str, _max_bytes: u64, _cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        Ok(LocalCopy::NotHere)
    }

    fn is_syncing(&self) -> bool {
        false
    }

    fn open_elsewhere(&self, _file_id: &str) -> Elsewhere {
        Elsewhere::Refused
    }

    fn wake(&self) {
        self.0.wake();
    }
}

/// The two readers, one per door, and which one holds the preview asked last.
pub struct Readers<E: ViewerEngine> {
    from_beebeeb: Previews<BeebeebOnly<E>>,
    this_pc_first: Previews<E>,
    asked: Option<(String, ReadFrom)>,
}

/// A display item for the reader: the id and the name (its kind decides what is read); nothing else is known.
fn asked_item(id: &str, name: &str) -> DriveItem {
    crate::frames::item::display_item(id, name, false, None)
}

impl<E: ViewerEngine> Readers<E> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<E>) -> Self {
        Self {
            from_beebeeb: Previews::new(Arc::new(BeebeebOnly(engine.clone()))),
            this_pc_first: Previews::new(engine),
            asked: None,
        }
    }

    /// Start the preview of `ask` by its door. The one before it stops first, and what it read is wiped.
    pub fn start(&mut self, ask: &PreviewAsk) {
        self.cancel();
        let item = asked_item(&ask.id, &ask.name);
        match ask.from {
            ReadFrom::Beebeeb => self.from_beebeeb.start(&item),
            ReadFrom::ThisPcFirst => self.this_pc_first.start(&item),
        }
        self.asked = Some((ask.id.clone(), ask.from));
    }

    /// Where the preview of `id` stands; any other id than the one asked is unavailable.
    pub fn later(&mut self, id: &str) -> Previewed {
        let Some((asked, from)) = self.asked.as_ref().filter(|(asked, _)| asked == id) else {
            return Previewed::Failed(ActionError::Unavailable);
        };
        let item = asked_item(asked, "");
        match from {
            ReadFrom::Beebeeb => self.from_beebeeb.later(&item),
            ReadFrom::ThisPcFirst => self.this_pc_first.later(&item),
        }
    }

    /// Stop both readers and wipe what they read. Nothing asked: nothing happens.
    pub fn cancel(&mut self) {
        if self.asked.take().is_some() {
            self.from_beebeeb.cancel();
            self.this_pc_first.cancel();
        }
    }
}

/// See the module's doc.
pub struct EngineItemPreview<C: CredentialStores + 'static> {
    readers: Readers<LiveViewerEngine<C>>,
}

impl<C: CredentialStores + 'static> EngineItemPreview<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self {
            readers: Readers::new(Arc::new(LiveViewerEngine::new(engine))),
        }
    }
}

impl<C: CredentialStores + 'static> ItemPreviewSource for EngineItemPreview<C> {
    /// With no session in memory nothing is downloaded: the reader answers by the kind first (a PDF "not available in
    /// this version yet", as Files' panel says it), else that it could not show a preview.
    fn start(&mut self, ask: &PreviewAsk) {
        self.readers.start(ask);
    }

    fn later(&mut self, id: &str) -> Previewed {
        self.readers.later(id)
    }

    fn cancel(&mut self) {
        self.readers.cancel();
    }
}
