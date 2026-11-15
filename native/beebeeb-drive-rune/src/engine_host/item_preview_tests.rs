//! An item's read-only Preview on the engine seam (`Readers`, behind `EngineItemPreview`): the two doors. A file in
//! Trash is read from Beebeeb only (`BeebeebOnly`): this PC's copy, the local state's facts, a save, keeping it and
//! another app are never asked of the engine. A file shared with the person reads this PC's copy first. Every name
//! and byte is invented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::{LocalCopy, OpenFileError, OpenFileFacts, OpenedFile};
use beebeeb_engine::app::viewer_edit_api::{EditFacts, EditRefused, NewCopy, SaveEnd};
use zeroize::Zeroizing;

use super::*;
use crate::frames::viewers::source::Elsewhere;

/// How often each call reached the engine.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
struct Calls {
    fetch: usize,
    local: usize,
    facts: usize,
    is_syncing: usize,
    open_elsewhere: usize,
    edit_facts: usize,
    set_kept: usize,
    save_text: usize,
    begin_opening: usize,
    save_copy: usize,
}

/// The engine, counting every call; it answers a download with `content`.
struct Counting {
    calls: Mutex<Calls>,
    content: Vec<u8>,
    wakes: AtomicBool,
}

impl Counting {
    fn new(content: &[u8]) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Calls::default()),
            content: content.to_vec(),
            wakes: AtomicBool::new(false),
        })
    }
    fn calls(&self) -> Calls {
        self.calls.lock().unwrap().clone()
    }
    fn count(&self, which: impl FnOnce(&mut Calls) -> &mut usize) {
        *which(&mut self.calls.lock().unwrap()) += 1;
    }
}

impl ViewerEngine for Counting {
    fn session_turn(&self) -> SessionGeneration {
        SessionGeneration::for_test(1)
    }
    fn has_session(&self) -> bool {
        true
    }
    fn account(&self) -> Option<String> {
        Some("a@example.eu".into())
    }
    fn facts(&self, _: &str) -> Result<OpenFileFacts, OpenFileError> {
        self.count(|c| &mut c.facts);
        Err(OpenFileError::Failed)
    }
    fn fetch(
        &self,
        _: &str,
        _: u64,
        _: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        _: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        self.count(|c| &mut c.fetch);
        Ok(OpenedFile {
            session: SessionGeneration::for_test(1),
            bytes: Zeroizing::new(self.content.clone()),
        })
    }
    fn local(&self, _: &str, _: u64, _: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        self.count(|c| &mut c.local);
        Ok(LocalCopy::NotHere)
    }
    fn is_syncing(&self) -> bool {
        self.count(|c| &mut c.is_syncing);
        true
    }
    fn open_elsewhere(&self, _: &str) -> Elsewhere {
        self.count(|c| &mut c.open_elsewhere);
        Elsewhere::Opened
    }
    fn wake(&self) {
        self.wakes.store(true, Ordering::SeqCst);
    }
    fn edit_facts(&self, _: SessionGeneration, _: &str, _: u64) -> Option<EditFacts> {
        self.count(|c| &mut c.edit_facts);
        None
    }
    fn set_kept(&self, _: SessionGeneration, _: &str, _: bool) -> Result<(), ()> {
        self.count(|c| &mut c.set_kept);
        Ok(())
    }
    fn save_text(&self, _: SessionGeneration, _: &str, _: &str, _: i64, _: u64) -> Result<SaveEnd, EditRefused> {
        self.count(|c| &mut c.save_text);
        Err(EditRefused::Failed)
    }
    fn begin_opening(&self, _: SessionGeneration, _: &str) -> u64 {
        self.count(|c| &mut c.begin_opening);
        7
    }
    fn save_copy(&self, _: SessionGeneration, _: &str, _: &str, _: &str, _: &str) -> Result<NewCopy, EditRefused> {
        self.count(|c| &mut c.save_copy);
        Err(EditRefused::Failed)
    }
}

/// Ask `later` until the preview is no longer on its way.
fn settled(readers: &mut Readers<Counting>, id: &str) -> Previewed {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let now = readers.later(id);
        if now != Previewed::Loading || Instant::now() > until {
            return now;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ask(from: ReadFrom) -> PreviewAsk {
    PreviewAsk {
        id: "t1".into(),
        name: "invented trip notes.txt".into(),
        from,
    }
}

#[test]
fn trash_preview_reads_from_beebeeb_only() {
    let engine = Counting::new(b"train at eight\npack the charger");
    let mut readers = Readers::new(engine.clone());
    readers.start(&ask(ReadFrom::Beebeeb));
    let Previewed::Shown(shown) = settled(&mut readers, "t1") else {
        panic!("not shown")
    };
    assert_eq!(shown.page.sections[0].1, "train at eight\npack the charger");
    assert_eq!(
        engine.calls(),
        Calls {
            fetch: 1,
            ..Calls::default()
        },
        "downloaded once; nothing of this PC, its local state, a save, keeping or another app was asked"
    );
    // The wrapper on its own: what it answers, and that it asks the engine for none of it.
    let only = BeebeebOnly(engine.clone());
    let cancel = AtomicBool::new(false);
    assert!(matches!(only.local("t1", 10, &cancel), Ok(LocalCopy::NotHere)));
    assert_eq!(only.facts("t1").map(|_| ()), Err(OpenFileError::Failed));
    assert!(!only.is_syncing());
    assert_eq!(only.open_elsewhere("t1"), Elsewhere::Refused);
    let session = SessionGeneration::for_test(1);
    assert!(only.edit_facts(session, "t1", 7).is_none());
    assert!(only.set_kept(session, "t1", true).is_err());
    assert!(only.save_text(session, "t1", "words", 1, 7).is_err());
    assert_eq!(only.begin_opening(session, "t1"), 0);
    assert!(only.save_copy(session, "t1", "words", "PC", "10:00").is_err());
    assert_eq!(engine.calls().fetch, 1);
    assert_eq!(
        Calls {
            fetch: 0,
            ..engine.calls()
        },
        Calls::default(),
        "the wrapper asked the engine for nothing of this PC"
    );

    // A file shared with the person: this PC's copy is asked first.
    let engine = Counting::new(b"shared words");
    let mut readers = Readers::new(engine.clone());
    readers.start(&ask(ReadFrom::ThisPcFirst));
    assert!(matches!(settled(&mut readers, "t1"), Previewed::Shown(_)));
    assert_eq!((engine.calls().local, engine.calls().fetch), (1, 1));
}

#[test]
fn another_id_than_the_one_asked_is_unavailable_and_cancel_forgets_the_ask() {
    let engine = Counting::new(b"words");
    let mut readers = Readers::new(engine);
    assert_eq!(readers.later("t1"), Previewed::Failed(ActionError::Unavailable));
    readers.start(&ask(ReadFrom::Beebeeb));
    assert_eq!(readers.later("t2"), Previewed::Failed(ActionError::Unavailable));
    readers.cancel();
    assert_eq!(readers.later("t1"), Previewed::Failed(ActionError::Unavailable));
    // A cancel with nothing asked is a no-op.
    readers.cancel();
}
