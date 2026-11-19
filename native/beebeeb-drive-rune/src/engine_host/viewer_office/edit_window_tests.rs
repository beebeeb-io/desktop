//! The office editor in the window, end to end on the real engine: an Office file opened from the engine's fake API
//! (downloaded and decrypted by the engine), the editor started by the shell's poll, a composition and Ctrl+S through
//! the window's own hooks, the save stored by the engine and sent by the uploader's pass to the fake server as the
//! file's next version; New document from Files' New menu made as a new file there; and an editing this PC kept,
//! said after an unlock and restored from the offer. Only the helper is a fake (the confined renderer is
//! Windows-only): `viewer_office_tests.rs`'s `Fake` host. Every name, id and byte is invented.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::api_client::ApiClient;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::engine_bridge::EngineBridge;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, PERMISSION_READ, PERMISSION_WRITE, StateDb};
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, start_fake};
use beebeeb_office_protocol::office::{Dirty, EditKind, Purpose};
use rune::{AppSpec, Env, Ime, Key, KeyEvent, Mods, Runtime};
use zeroize::Zeroizing;

use super::super::super::office_tests::{Fake, edited, pages};
use super::super::super::EngineViewerSource;
use super::super::{EditShips, HostEdit};
use crate::State;
use crate::engine_host::LiveEngine;
use crate::engine_host::open::Opened;
use crate::frames::files::Toast;
use crate::frames::office::edit::{new_document, recovery};
use crate::frames::viewers;
use crate::frames::viewers::kinds::OfficeShips;
use crate::frames::viewers::new_file::Step;
use crate::frames::viewers::office_over::Over;
use crate::shell::actions::{ActionError, EngineActions, HostActions};
use crate::shell::app::Session;
use crate::shell::host::{Shell, Wiring};
use crate::shell::identity::engine_identity;
use crate::shell::instance::ProfileHolder;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::tray::TrayInputs;
use crate::shell::{host, registry, state};

const PASSWORD: &str = "an invented password for the office window";
const DOC: &str = "b0000000-0000-4000-8000-000000000e01";
const DOC_NAME: &str = "invented-letter-of-the-office-window.docx";

/// A package the engine takes as a Word document: a zip whose directory names the parts, nothing compressed.
fn package(extra: &[&str]) -> Vec<u8> {
    let mut names = vec!["[Content_Types].xml", "word/document.xml"];
    names.extend_from_slice(extra);
    let (mut out, mut directory) = (Vec::new(), Vec::new());
    for name in &names {
        let (offset, data) = (out.len() as u32, b"<invented/>");
        // The local header, then the part.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // Its entry in the directory.
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0; 12]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let at = out.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(names.len() as u16).to_le_bytes());
    out.extend_from_slice(&(names.len() as u16).to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The document as the server holds it, and as the editor saves it (one part more, so the sizes differ).
fn first_version() -> Vec<u8> {
    package(&[])
}

fn edited_version() -> Vec<u8> {
    package(&["word/an-invented-edit.xml"])
}

/// The engine behind the shell, with a sync start that is not carried out (the process's one state folder is shared
/// by every test of this binary). What a save stores is sent by [`Window::upload`], the uploader's own pass.
struct NoStart {
    live: Arc<LiveEngine<MemoryCredentials>>,
}

impl EngineActions for NoStart {
    fn session(&self) -> Session {
        self.live.session()
    }
    fn tray_inputs(&self) -> TrayInputs {
        self.live.tray_inputs()
    }
    fn pause_sync(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn resume_sync(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn open_in_file_explorer(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn profile_holder(&self) -> Option<ProfileHolder> {
        None
    }
    fn start_sync(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn stop_for_quit(&self) {}
    fn resume_after_cancelled_quit(&self) {}
    fn sync_refusal(&self) -> Option<crate::frames::global::sync_off::SyncOff> {
        self.live.sync_refusal()
    }
}

/// What the editing opener was handed, in order: the kind and how many bytes.
type Edits = Arc<Mutex<Vec<(EditKind, usize)>>>;

struct Window {
    api: FakeApi,
    account: Arc<FakeAccount>,
    engine: Arc<Engine<MemoryCredentials>>,
    rt: Runtime<State>,
    /// The helper of an opened file.
    helper: Fake,
    /// The helper of a New document's template.
    template: Fake,
    edits: Edits,
    /// How many of `edits` an [`Window::open_in_the_editor`] already waited for.
    sessions: usize,
    /// Makes a source as the window's route does.
    make: Box<dyn Fn() -> Box<dyn viewers::source::ViewerSource>>,
    /// How often a file was opened in the viewer (no editing session).
    viewed: Arc<AtomicUsize>,
    state_path: PathBuf,
    root: tempfile::TempDir,
    _state: tempfile::TempDir,
    /// Held for the window's life, dropped last: see [`ONE_AT_A_TIME`].
    _one_at_a_time: std::sync::MutexGuard<'static, ()>,
}

/// The windows share the process's state folder, and so one session-end hook slot and one screen-lock grant (keyed by
/// the folder): a lock in one ends another's grant and a second hook replaces the first. One window at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

const ALL: OfficeShips = OfficeShips {
    docx: true,
    doc: true,
    xlsx: true,
    xls: true,
    pptx: true,
};

/// The engine's own source with the editor on for this source and the helper a fake.
fn source_of(
    engine: &Arc<Engine<MemoryCredentials>>,
    helper: &Fake,
    template: &Fake,
    edits: &Edits,
    viewed: &Arc<AtomicUsize>,
) -> Box<dyn viewers::source::ViewerSource> {
    let mut source = EngineViewerSource::new(engine.clone());
    source.office.on = true;
    source.office.ships = ALL;
    source.office.edit_on = true;
    source.office.edits = EditShips {
        docx: true,
        xlsx: true,
        pptx: true,
    };
    source.office.helper = || Some(PathBuf::from("beebeeb-office-render"));
    let (fake, seen) = (helper.clone(), viewed.clone());
    source.office.opener = Box::new(move |_, kind, _, _| {
        seen.fetch_add(1, Ordering::SeqCst);
        fake.view(kind)
    });
    let (fake, log) = (helper.clone(), edits.clone());
    source.office.edit_opener = Box::new(move |_, kind, bytes, _, _| {
        log.lock().unwrap().push((kind, bytes.len()));
        fake.editor(kind)
    });
    let fake = template.clone();
    source.office.new_opener = Box::new(move |_, kind, _, _, _| fake.editor(kind));
    Box::new(source)
}

fn window(tag: &str) -> Window {
    let one_at_a_time = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let (account, _) = FakeAccount::register(&format!("office-window-{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    account.add_file_with_content(DOC, None, DOC_NAME, &first_version());
    let api = start_fake(account.clone());

    let (channel, messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("office-window-slot-{tag}"),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(
        Engine::open(engine_identity(), config, channel, MemoryCredentials::default()).expect("the engine opens"),
    );
    engine.restore();

    // The account's local state, as its sync left it: the document, which the account may change, at version 1.
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state.db");
    let db = StateDb::open(&state_path).unwrap();
    let entry = FileEntry {
        file_id: DOC.to_string(),
        path: format!("/{DOC_NAME}"),
        status: FileStatus::CloudOnly,
        size_bytes: first_version().len() as i64,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
        contract.permission_bits |= PERMISSION_READ | PERMISSION_WRITE;
    })
    .unwrap();
    let mut contract = db.get_file_contract_state(DOC).unwrap().unwrap();
    contract.current_version = 1;
    db.set_file_contract_state(&contract).unwrap();
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(state_path.clone());
    let root = tempfile::tempdir().unwrap();
    engine.use_sync_root_for_test(root.path().to_path_buf());

    let opened = Opened {
        engine: engine.clone(),
        identity: engine_identity(),
        gate: engine.gate(),
    };
    let live = Arc::new(LiveEngine::new(&opened, messages));
    let platform: Arc<FakePlatform> = Arc::new(FakePlatform::default());
    let shell = Shell::new(
        Arc::new(HostActions::new(Arc::new(NoStart { live }), platform.clone())),
        platform,
        Wiring::default(),
    );
    let mut s = state::init_on_engine(shell, engine.clone());

    // The window's source is the engine's own, with the editor on for this source and the helper a fake.
    let (helper, template, edits) = (Fake::default(), Fake::default(), Edits::default());
    let viewed = Arc::new(AtomicUsize::new(0));
    let makers;
    {
        let (engine, helper, template, edits, viewed) =
            (engine.clone(), helper.clone(), template.clone(), edits.clone(), viewed.clone());
        let make = move || source_of(&engine, &helper, &template, &edits, &viewed);
        makers = Some(Box::new(make.clone()) as Box<dyn Fn() -> Box<dyn viewers::source::ViewerSource>>);
        viewers::wire(&mut s, Box::new(make));
    }

    // The main window's app as `shell::run` builds it (its poll, its keys, its input method hook), headless.
    let spec = AppSpec::<State> {
        poll: Some(host::poll),
        on_key: Some(host::on_key),
        ..AppSpec::new("office-window", (1280.0, 800.0), state::init, registry::view)
    };
    crate::shell::intents::poll_drains_for_this_test();
    let mut rt = crate::shell::main_app(spec)
        .init(move || s)
        .headless_runtime((1280.0, 800.0), ENV);
    rt.frame(0.0);
    Window {
        api,
        account,
        engine,
        rt,
        helper,
        template,
        edits,
        sessions: 0,
        make: makers.expect("the source's maker"),
        viewed,
        state_path,
        root,
        _state: dir,
        _one_at_a_time: one_at_a_time,
    }
}

impl Window {
    fn sign_in(&mut self) {
        self.engine
            .sign_in_password(&self.account.email, Zeroizing::new(PASSWORD.into()))
            .expect("the fake accepts the password");
        self.engine
            .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.to_string()))
            .expect("the account's phrase unlocks");
        assert_eq!(self.engine.gate(), EngineGate::Ready);
        self.settle();
    }

    fn settle(&mut self) {
        for _ in 0..2 {
            self.rt.frame(0.016);
            self.rt.settle_all();
            self.rt.frame(0.016);
        }
    }

    /// Frames until `done` (a worker's answer arrives between frames).
    #[track_caller]
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Window) -> bool) {
        let began = Instant::now();
        loop {
            self.settle();
            if done(self) {
                return;
            }
            assert!(
                began.elapsed() < Duration::from_secs(20),
                "never: {what}; toast {:?}; the window draws:\n{}",
                self.toast(),
                self.drawn()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn drawn(&self) -> String {
        self.rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .flat_map(|e| [e.text, e.label].into_iter().flatten())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn has(&self, key: &str) -> bool {
        self.rt
            .elements()
            .into_iter()
            .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    #[track_caller]
    fn click(&mut self, key: &str) {
        let rect = self
            .rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
            .unwrap_or_else(|| panic!("no element keyed {key}; the window draws:\n{}", self.drawn()))
            .rect;
        let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
        self.rt.pointer_down(x, y);
        self.rt.pointer_up(x, y);
        self.settle();
    }

    fn ctrl(&mut self, c: char) {
        self.rt.key(KeyEvent {
            key: Key::Command(c),
            mods: Mods {
                cmd: true,
                ctrl: true,
                ..Mods::default()
            },
        });
        self.settle();
    }

    fn toast(&self) -> Option<String> {
        self.rt.state.viewer.ui.as_ref().and_then(|ui| ui.toast.clone())
    }

    /// The editor's state of the open document: (unsaved, saving, restored).
    fn edit(&self) -> Option<(bool, bool, Option<String>)> {
        let edit = self.rt.state.viewer.ui.as_ref()?.office.edit.as_ref()?;
        Some((edit.unsaved(), edit.saving, edit.restored.clone()))
    }

    /// Open the document and wait for its editor: the engine downloads and decrypts it, the editing session opens
    /// on its bytes, the helper says its pages, the shell's poll starts the editor.
    #[track_caller]
    fn open_in_the_editor(&mut self) {
        let before = self.sessions;
        if self.rt.state.viewer.ui.is_none() {
            let mut opened = false;
            self.rt.update(|s, _| opened = viewers::open(s, DOC));
            assert!(opened, "the document is the window's to open");
        }
        self.until("the editing session opens on the document's bytes", |w| {
            assert_eq!(
                w.viewed.load(Ordering::SeqCst),
                0,
                "the file opened in the viewer: the engine does not say the account may change it"
            );
            w.edits.lock().unwrap().len() > before
        });
        self.sessions = self.edits.lock().unwrap().len();
        self.helper.set(pages(2));
        self.until("the editor starts", |w| w.edit().is_some());
        assert!(self.has("edit-chrome"), "the editor's frame is drawn: {}", self.drawn());
    }

    /// The uploader's pass over what is queued (what a running sync does between two looks), against the fake server.
    fn upload(&self) -> usize {
        let db = Arc::new(StateDb::open(&self.state_path).unwrap());
        let client = ApiClient::new(
            self.api.base.clone(),
            self.account.session_token.clone(),
            self.account.master_key,
        );
        let bridge = EngineBridge::new(engine_identity(), db, Arc::new(client));
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(bridge.process_due_operations(self.root.path(), chrono::Utc::now().timestamp()))
            .expect("the pass runs");
        assert!(
            outcome.retried_op_ids.is_empty() && outcome.paused_op_ids.is_empty(),
            "an upload did not go through: {} retried, {} paused",
            outcome.retried_op_ids.len(),
            outcome.paused_op_ids.len()
        );
        outcome.completed_op_ids.len()
    }

    fn local_version(&self) -> i64 {
        let db = StateDb::open(&self.state_path).unwrap();
        db.get_file_contract_state(DOC).unwrap().unwrap().current_version
    }
}

/// The done check of the editor's wiring: an Office file opens in the editor, a composition reaches it, Back asks
/// while it is unsaved, Ctrl+S saves, and the fake server holds a new version.
#[test]
fn an_office_file_opens_in_the_editor_and_a_save_is_a_new_version_on_the_server() {
    let mut w = window("save");
    w.sign_in();
    assert_eq!(w.account.item_state(DOC).map(|state| state.0), Some(1));

    w.open_in_the_editor();
    assert_eq!(
        w.edits.lock().unwrap().as_slice(),
        [(EditKind::Docx, first_version().len())],
        "the session edits the bytes the engine decrypted"
    );
    assert_eq!(w.edit(), Some((false, false, None)), "nothing is unsaved yet");

    // A composition: the pre-edit stays in the window, the commit is one edit to the helper.
    w.rt.ime(Ime::Preedit("ni".into(), Some((0, 2))));
    w.settle();
    let preedit = |w: &Window| {
        let edit = w.rt.state.viewer.ui.as_ref()?.office.edit.as_ref()?;
        edit.preedit.clone()
    };
    assert_eq!(preedit(&w).as_deref(), Some("ni"));
    assert_eq!(w.helper.asked_count("edit"), 0, "a pre-edit costs no round trip");
    w.rt.ime(Ime::Commit("\u{4F60}".into()));
    w.settle();
    assert_eq!(preedit(&w), None);
    assert_eq!(w.helper.asked_count("edit"), 1, "the committed text reached the open editor");

    // The helper's answer: the document is at revision 1, which no version holds.
    w.helper.answer(HostEdit::Edited(Box::new(edited(1, Dirty::All))));
    w.until("the edit is unsaved", |w| w.edit().is_some_and(|edit| edit.0));

    // Back does not leave: the question is up, and Keep editing goes back to the document.
    w.rt.update(|s, _| viewers::close(s));
    w.settle();
    let over_leave = |w: &Window| {
        matches!(
            w.rt.state.viewer.ui.as_ref().map(|ui| &ui.office.over),
            Some(Some(Over::Leave))
        )
    };
    assert!(over_leave(&w), "Back with unsaved changes asks first");
    w.rt.update(|s, _| viewers::ui(s).expect("the document stays").office_close_over());
    w.settle();

    // Ctrl+S: the helper is asked for the document, the engine stores it, the window says so.
    w.ctrl('s');
    w.until("the helper is asked for the document's bytes", |w| {
        w.helper.asked_count("save") == 1
    });
    assert_eq!(w.edit().map(|edit| edit.1), Some(true), "a save is on its way");
    w.helper.saved(&edited_version(), 1);
    w.until("the save answered", |w| w.edit() == Some((false, false, None)));
    let said = w.toast().unwrap_or_default();
    assert!(said.starts_with("Saved"), "the window says it is saved: {said:?}");
    assert!(w.has("edit-chrome"), "the editor stays open on the saved document");

    // The engine queued the new version; the uploader's pass sends it. The server holds version 2 now.
    assert_eq!(w.account.item_state(DOC).map(|state| state.0), Some(1), "not sent yet: sync is off here");
    assert_eq!(w.upload(), 1, "one upload was queued by the save");
    assert_eq!(
        w.account.item_state(DOC).map(|state| state.0),
        Some(2),
        "the fake server holds a new version"
    );
    assert_eq!(w.local_version(), 2, "and this PC's record moved to it");
    assert!(w.account.stored_frames(DOC).is_some_and(|frames| !frames.is_empty()));
}

/// New document from Files' New menu: DW-1558 opens, Create makes the document from the template's bytes and the
/// engine stores it as a new file, which the uploader's pass sends to the fake server.
#[test]
fn new_document_from_the_files_menu_makes_a_new_file_on_the_server() {
    let mut w = window("new");
    w.sign_in();
    assert_eq!(w.account.live_file_ids(), [DOC.to_string()]);

    // Sync is off in this harness, so Files' New takes nothing here (the menu's press and the poll that opens DW-1558
    // are held by `shell::intents_tests`). The dialog is put up as `new_file::begin_document` puts it up, on the
    // window's own source.
    let source = (w.make)();
    w.rt.update(|s, _| {
        let doc = new_document::Doc::new(Some(source));
        s.viewer.new_file = Some(viewers::new_file::NewFile::document(None, viewers::new_file::Kind::Word, doc));
    });
    w.settle();
    let step = w.rt.state.viewer.new_file.as_ref().map(|dialog| dialog.step);
    assert_eq!(step, Some(Step::Document), "DW-1558 is open: {}", w.drawn());
    assert!(w.drawn().contains(new_document::TITLE));

    // Create: the template's session opens, its bytes are asked, the engine makes the file.
    w.rt.update(|s, _| new_document::create(s));
    w.settle();
    let creating = |w: &Window| {
        let dialog = w.rt.state.viewer.new_file.as_ref()?;
        dialog.doc.as_ref().map(|doc| (doc.creating, doc.failed.clone()))
    };
    assert_eq!(creating(&w), Some((true, None)), "Create was taken by the source");
    w.template.set(pages(1));
    w.until("the template's bytes are asked", |w| w.template.asked_count("save") == 1);
    w.template.saved(&first_version(), 0);
    w.until("the creation answered", |w| creating(w).is_none_or(|doc| !doc.0));
    // What the window did with the answer (the new file opened here, or the dialog says why not) is not this
    // test's: the file is made either way.
    eprintln!(
        "after Create: dialog {:?}, open file {:?}, toast {:?}",
        creating(&w),
        w.rt.state.viewer.ui.as_ref().map(|ui| ui.file.name.clone()),
        w.toast()
    );

    assert_eq!(w.upload(), 1, "one upload was queued by Create");
    let files = w.account.live_file_ids();
    assert_eq!(files.len(), 2, "the fake server holds the new document beside the first: {files:?}");
}

/// A lock before any snapshot timer fired: the host's flush seals the typed text through the door the editor registered
/// (here the fake host's hook calls that door, as the real host's worker does), while the session is still in memory;
/// after the unlock the reopened document offers it.
#[test]
fn text_typed_before_the_snapshot_timer_is_offered_after_a_lock_and_an_unlock() {
    use beebeeb_engine::app::office_session_end::SessionEnding;
    let mut w = window("lockflush");
    w.sign_in();
    w.open_in_the_editor();
    // (The window's editor holds nothing unsaved, so the lock closes the document, as in the test below.)
    assert_eq!(viewers::office_over::recovery_pending(&w.rt.state).len(), 0, "no snapshot was ever handed over: nothing kept yet");

    let sink = w.helper.flush_sink().expect("the editor registered the door a lock seals through");
    let last_text = Zeroizing::new(edited_version());
    w.engine.set_office_session_end_hook(Arc::new(move |ending| {
        if ending == SessionEnding::Lock {
            sink(&last_text);
        }
    }));
    w.engine.lock();
    w.settle();
    assert!(w.rt.state.viewer.ui.is_none(), "locked: no open file");
    w.engine
        .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.to_string()))
        .expect("the phrase unlocks");
    w.until("the notice after unlock", |w| w.rt.state.actions.toast == Some(Toast::KeptEdits(1)));
    w.open_in_the_editor();
    assert!(
        w.rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.office.recovery.is_some()),
        "the reopened document offers the text typed before the lock"
    );
}

/// A SCREEN lock before any snapshot timer fired: the lock comes first (the engine's call returns with the screen
/// locked and nothing sealed yet), then the seal lands from a background thread through the grant the lock left, and
/// after the unlock the document offers the last text.
#[test]
fn text_typed_before_the_snapshot_timer_is_offered_after_a_screen_lock_and_an_unlock() {
    use beebeeb_engine::app::office_session_end::SessionEnding;
    let mut w = window("screenlockflush");
    w.sign_in();
    w.open_in_the_editor();
    assert_eq!(viewers::office_over::recovery_pending(&w.rt.state).len(), 0, "nothing kept yet");

    let sink = w.helper.flush_sink().expect("the editor registered the door a lock seals through");
    let last_text = Zeroizing::new(edited_version());
    let sealed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (done, when_called) = (sealed.clone(), Arc::new(std::sync::Mutex::new(None)));
    let called = when_called.clone();
    let engine = w.engine.clone();
    w.engine.set_office_session_end_hook(Arc::new(move |ending| {
        if ending != SessionEnding::ScreenLock {
            return;
        }
        // The lock is in place when the hook is called, and the hook returns at once: the seal comes later.
        *called.lock().unwrap() = Some(engine.gate());
        let (sink, text, done) = (sink.clone(), last_text.clone(), done.clone());
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            sink(&text);
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }));
    w.engine.lock_screen().expect("the screen locks");
    assert_eq!(
        *when_called.lock().unwrap(),
        Some(EngineGate::ScreenLocked),
        "the lock frame is first: the screen was locked when the flush began"
    );
    assert!(!sealed.load(std::sync::atomic::Ordering::SeqCst), "the flush has not ended when the lock call returned");
    w.until("the seal landed in the background", |_| sealed.load(std::sync::atomic::Ordering::SeqCst));
    w.engine
        .unlock_screen_platform_verified(w.engine.session_turn())
        .expect("the platform's yes unlocks the screen");
    w.settle();
    let offered = |w: &Window| {
        w.rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.office.recovery.is_some())
            || !viewers::office_over::recovery_pending(&w.rt.state).is_empty()
    };
    w.until("the last text is offered", |w| offered(w));
}

/// Bug 8 (VM 401 run 7): an edited document open, the screen locked and unlocked; the pane says the file was closed,
/// and Files and the Settings gear must answer at once (the recording saw ~60 s of nothing).
#[test]
fn after_a_screen_lock_with_unsaved_edits_files_and_the_settings_gear_answer_at_once() {
    let mut w = window("lockfreeze");
    w.sign_in();
    w.open_in_the_editor();
    w.helper.answer(HostEdit::Edited(Box::new(edited(1, Dirty::All))));
    w.until("the edit is unsaved", |w| w.edit().is_some_and(|edit| edit.0));
    use beebeeb_engine::app::office_session_end::SessionEnding;
    let sink = w.helper.flush_sink().expect("the editor registered the door a lock seals through");
    let last_text = Zeroizing::new(edited_version());
    let sealed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = sealed.clone();
    w.engine.set_office_session_end_hook(Arc::new(move |ending| {
        if ending != SessionEnding::ScreenLock {
            return;
        }
        let (sink, text, done) = (sink.clone(), last_text.clone(), done.clone());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            sink(&text);
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }));
    w.engine.lock_screen().expect("the screen locks");
    w.settle();
    w.until("the seal landed", |_| sealed.load(std::sync::atomic::Ordering::SeqCst));
    w.engine
        .unlock_screen_platform_verified(w.engine.session_turn())
        .expect("the platform's yes unlocks the screen");
    w.until("the notice after unlock", |w| w.rt.state.actions.toast == Some(Toast::KeptEdits(1)));
    let began = Instant::now();
    w.settle();
    w.click("Files-row");
    {
        let ui = w.rt.state.viewer.ui.as_ref().unwrap();
        eprintln!("PROBE body_stopped={} edit={:?} over={} dialog={}", matches!(ui.file.body, viewers::source::Body::Stopped(_)), ui.office.edit.as_ref().map(|e| (e.stopped, e.unsaved())), ui.office.over.is_some(), ui.dialog.is_some());
    }
    w.until("Files left the closed document", |w| {
        w.rt.state.viewer.ui.is_none() && viewers::route_item(&w.rt.state).is_none()
    });
    assert!(began.elapsed() < Duration::from_secs(5), "Files answered late");
    w.click("settings-btn");
    let screen = w.rt.state.shell.app.screen();
    eprintln!("after the gear: {screen:?} in {:?}", began.elapsed());
    assert!(
        matches!(screen, crate::shell::app::Screen::Settings(_)),
        "the gear did not open Settings: {screen:?}; the window draws:\n{}",
        w.drawn()
    );
}

/// Recovery: a snapshot the helper handed over is kept (sealed by the engine); after a lock and an unlock the
/// window says so, the reopened document offers it, and Restore opens the editor on the kept bytes, unsaved.
#[test]
fn an_editing_this_pc_kept_is_said_after_unlock_and_restored_from_the_offer() {
    let mut w = window("recovery");
    w.sign_in();
    assert_eq!(w.rt.state.actions.toast, None, "nothing kept: nothing said at sign-in");
    w.open_in_the_editor();

    // The helper's snapshot of the document (the host asks for one by itself while there are edits). The window's
    // editor holds nothing unsaved, so a lock closes the document (an unsaved one stays open across it, as a text's).
    w.helper.answer(HostEdit::Saved {
        id: 9_999,
        purpose: Purpose::Snapshot,
        revision: 1,
        bytes: Zeroizing::new(edited_version()),
    });
    w.until("the snapshot is kept", |w| {
        viewers::office_over::recovery_pending(&w.rt.state).len() == 1
    });

    // Locked: the document leaves the window. Unlocked: the notice, in Files' toast and the reopened file's.
    w.engine.lock();
    w.settle();
    assert!(w.rt.state.viewer.ui.is_none(), "locked: no open file");
    w.engine
        .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.to_string()))
        .expect("the phrase unlocks");
    w.until("the notice after unlock", |w| {
        w.rt.state.actions.toast == Some(Toast::KeptEdits(1))
    });

    // The route still names the document: it opens again, in a new editing session, and offers what was kept.
    w.open_in_the_editor();
    let offer = w.rt.state.viewer.ui.as_ref().and_then(|ui| ui.office.recovery.clone());
    let at = offer.expect("the reopened document offers the kept editing").at;
    let words = recovery::offer_words(DOC_NAME, &at);
    assert!(w.drawn().contains(&words), "{words:?} is not drawn:\n{}", w.drawn());

    // Restore: a session on the kept bytes; the editor says so and the document is unsaved.
    let sessions = w.edits.lock().unwrap().len();
    w.click("recovery-restore");
    w.helper.set(pages(2));
    w.until("the kept editing is restored", |w| {
        w.edit().is_some_and(|edit| edit.2.is_some())
    });
    let edits = w.edits.lock().unwrap().clone();
    assert_eq!(edits.len(), sessions + 1, "one new session");
    assert_eq!(
        edits.last().copied(),
        Some((EditKind::Docx, edited_version().len())),
        "opened on the kept bytes, not the file's"
    );
    let (unsaved, _, restored) = w.edit().expect("the editor");
    assert!(unsaved, "no version holds the restored document yet");
    assert!(w.drawn().contains(&recovery::brought_words(&restored.expect("when it is from"))));
    assert!(
        w.rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.office.recovery.is_none()),
        "the offer is answered"
    );

    // And it saves: the restored document becomes the next version.
    w.ctrl('s');
    w.until("the helper is asked for the restored document", |w| {
        w.helper.asked_count("save") >= 1
    });
    w.helper.saved(&edited_version(), 0);
    w.until("the restored document is saved", |w| {
        w.edit().is_some_and(|edit| !edit.0 && !edit.1)
    });
    assert_eq!(w.upload(), 1);
    assert_eq!(w.account.item_state(DOC).map(|state| state.0), Some(2));
}

/// The windows of this file share the process's state folder, so one session-end hook slot and one screen-lock grant
/// (both keyed by that folder): another window's lock ends this one's grant and its hook is replaced. They are one at
/// a time. A second window is not made while the first is alive.
#[test]
fn windows_on_the_shared_state_folder_are_one_at_a_time() {
    let first = window("one-at-a-time-a");
    let (made, made_at) = std::sync::mpsc::channel();
    let second = std::thread::spawn(move || {
        let second = window("one-at-a-time-b");
        made.send(()).unwrap();
        drop(second);
    });
    assert!(
        made_at.recv_timeout(Duration::from_millis(1_500)).is_err(),
        "a second window was made while the first was alive: its lock would end the first's grant"
    );
    drop(first);
    made_at.recv_timeout(Duration::from_secs(60)).expect("the second window is made once the first is gone");
    second.join().unwrap();
}
