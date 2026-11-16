//! The app's root state: one value the whole main window reads.
//!
//! `ui/**` components take `&State` and `Cx<State>`, so every frame group draws from this
//! struct. A frame lane that needs state of its own adds ONE field here (and its default in
//! [`init`]); the shell's own state lives in [`State::shell`].

use rune::TextEdit;

use super::host::Shell;
use super::identity::AppIdentity;
use crate::source::{self, BoardFixtureSource, DriveSource, Listing};
use crate::ui::sidebar::SidebarSection;

pub struct State {
    /// Which build this window names itself as: the live app reads `AppIdentity::CURRENT` (so
    /// "Beebeeb Test" in a test build); a headless golden export reads `AppIdentity::GOLDEN`
    /// (always "Beebeeb"), so the rendered text matches the design board.
    pub(crate) identity: AppIdentity,
    // --- Files (DW-0401): read by `ui/**` ---
    pub(crate) source: Box<dyn DriveSource>,
    pub(crate) listing: Listing,
    /// Why `listing` could not be read, when it could not: the window shows this instead of rows.
    pub(crate) list_error: Option<source::SourceError>,
    pub(crate) storage: source::StorageSummary,
    /// `false`: the source cannot know the account's storage (a `--db` source never asks the server).
    pub(crate) storage_known: bool,
    /// The storage split by category: `Some((files_bytes, photos_bytes))` from the board fixture
    /// or the engine; `None` when the split is not known (a live window before the engine wires it).
    pub(crate) storage_split: Option<(i64, i64)>,
    /// How the live window reads the signed-in account's storage again (`shell::storage_view`); `None` in a scene.
    pub(crate) storage_reader: Option<super::storage_view::StorageReader>,
    /// The read in flight, stamped with the account that asked.
    pub(crate) storage_pending: Option<super::storage_view::Pending>,
    pub(crate) selected: Option<usize>,
    pub(crate) search: TextEdit,
    pub(crate) sidebar_section: SidebarSection,
    pub(crate) show_detail_pane: bool,
    /// What the Files window keeps beside the listing (`ui::FilesUi`).
    pub(crate) files: crate::ui::FilesUi,
    /// The shell: the route machine, the gate, the b, the actions (`shell/host.rs`).
    pub shell: Shell,
    // --- one line per frame group below ---
    /// What the Files frames keep: selection, menus, toasts (`frames::files`).
    pub(crate) actions: crate::frames::files::Actions,
    /// The global elements: the banner, the loading strip (`frames::global`).
    pub(crate) global: crate::frames::global::Ui,
    /// The live feedback mode (test builds only).
    #[cfg(feature = "feedback")]
    pub(crate) feedback: crate::feedback::Feedback,
    pub(crate) photos: crate::frames::photos::Ui,
    pub frames: crate::frames::Ui,
    pub settings: crate::frames::settings::SettingsUi,
    pub account: crate::frames::account::AccountUi,
    pub(crate) signin: Option<crate::frames::signin::Ui>,
    pub(crate) viewer: crate::frames::viewers::Slot,
    /// The b's flyout as a scene of this window (the frame gate); `None` in the running app.
    pub(crate) tray: Option<crate::frames::tray::Ui>,
    pub(crate) shortcuts: crate::frames::tray::shortcuts::Shortcuts,
    pub(crate) setup: Option<crate::frames::signin::setup::Ui>,
    /// The running app: makes the first run's source for the session that is signed in now (`engine_host::setup`).
    /// `None`: no first run is ever started from the gate (scenes set `setup` themselves).
    pub(crate) setup_source: Option<SetupMaker>,
    /// The first-time guide over the main window (DW-0380, DW-0301 to DW-0305).
    pub(crate) guide: crate::frames::signin::setup::guide::Guide,
    /// Where the guide's "done" is kept, once per account (`guide_marks`).
    pub(crate) guide_marks: Box<dyn crate::frames::signin::setup::guide_marks::GuideMarks>,
    /// Sync history and Available offline (`frames::this_pc`).
    pub(crate) this_pc: crate::frames::this_pc::Ui,
    /// Backups, a section of settings (`frames::backups`).
    pub(crate) backups: crate::frames::backups::BackupsUi,
}

/// The state headless renders and tests start from: the fixture engine, nothing wired.
pub fn init() -> State {
    init_with(Shell::fixture())
}

pub fn init_with(shell: Shell) -> State {
    let board = init_from(shell, Box::new(BoardFixtureSource::new()), Some(6));
    crate::frames::viewers::board_fixture(crate::frames::files::board_fixture(board))
}

/// The folder as the window holds it: the listing, or an empty one and the reason it could not be read.
pub fn load(source: &dyn DriveSource, folder_id: Option<&str>) -> (Listing, Option<source::SourceError>) {
    match source.list(folder_id) {
        Ok(listing) => (listing, None),
        Err(error) => {
            let root = source::Crumb {
                id: None,
                name: "Files".into(),
            };
            let unread = Listing {
                folder: root.clone(),
                path: vec![root],
                items: Vec::new(),
            };
            (unread, Some(error))
        }
    }
}

/// `selected`: the board fixture has Contract draft.pdf (index 6) selected per the board; a real
/// database starts with nothing selected.
pub fn init_from(shell: Shell, source: Box<dyn DriveSource>, selected: Option<usize>) -> State {
    let (listing, list_error) = load(source.as_ref(), None);
    let storage = source.storage();
    // The board's storage split: Files 24 GB, Photos the rest. Only when storage is known
    // (the board fixture path); a live window overrides this to `None` in `init_on_engine`.
    const BOARD_FILES_BYTES: i64 = 24_000_000_000;
    let storage_split = storage.map(|s| (BOARD_FILES_BYTES, (s.used_bytes - BOARD_FILES_BYTES).max(0)));
    State {
        identity: AppIdentity::GOLDEN,
        source,
        listing,
        list_error,
        storage_known: storage.is_some(),
        storage: storage.unwrap_or(source::StorageSummary {
            used_bytes: 0,
            quota_bytes: 0,
        }),
        storage_split,
        storage_reader: None,
        storage_pending: None,
        selected,
        search: TextEdit::new(""),
        sidebar_section: SidebarSection::Files,
        show_detail_pane: true,
        files: crate::ui::FilesUi::default(),
        shell,
        actions: crate::frames::files::Actions::default(),
        global: crate::frames::global::Ui::default(),
        #[cfg(feature = "feedback")]
        feedback: crate::feedback::Feedback::new(),
        photos: crate::frames::photos::start(),
        frames: crate::frames::Ui::default(),
        settings: Default::default(),
        account: Default::default(),
        signin: crate::frames::signin::start(),
        viewer: Default::default(),
        tray: None,
        shortcuts: Default::default(),
        setup: None,
        setup_source: None,
        guide: Default::default(),
        guide_marks: crate::frames::signin::setup::guide_marks::first(crate::shell::instance::DEV_BUILD),
        this_pc: crate::frames::this_pc::Ui::fixture(),
        backups: crate::frames::backups::BackupsUi::first(crate::shell::instance::DEV_BUILD),
    }
}

/// Search on the engine (`engine_host::search`) in the running app. Task 2488's code is in (the owner's go,
/// 2026-11-15): the index and the recent searches are read only through the engine's account-bound handle, so a local
/// state a sign-out kept for the previous account answers nothing to another one. ON since plan40 checkpoint 2
/// (lead ruling L214): the search gates (`b2-p2-search-gates`' three tests) are green in its full run at `9aa4ecb9`,
/// and each of their four mutants (M1, M2a, M2b, M3) turned them red. False hands Search the `Unwired` source and
/// no part of the adapter runs (no index, no worker).
pub(crate) const SEARCH_ON_ENGINE: bool = true;

/// Makes a first-run source for the session that is signed in when it is called.
pub type SetupMaker = Box<dyn Fn() -> Box<dyn crate::frames::signin::setup::source::SetupSource>>;

/// The running app's first run, on the real engine: give the window the way to make its source. Only the app's
/// start calls this (after it read whether a first run is due, `LiveEngine::wire_first_run`): a window that was
/// not wired never shows a first run (the gate stays as the engine says).
pub fn wire_first_run<C: beebeeb_engine::app::CredentialStores + 'static>(
    s: &mut State,
    engine: std::sync::Arc<beebeeb_engine::app::Engine<C>>,
    live: &std::sync::Arc<crate::engine_host::LiveEngine<C>>,
) {
    let hooks = live.setup_hooks(crate::shell::platform::current());
    // Step 2's "off" asks the main window's Windows Hello prompt first.
    let window = s.shell.main_window_slot();
    s.setup_source = Some(Box::new(move || {
        Box::new(
            crate::engine_host::setup::EngineSetupSource::new(engine.clone(), hooks.clone()).with_hello(Box::new(
                crate::engine_host::hello::WindowHello::for_reason(
                    window.clone(),
                    crate::engine_host::hello::TURN_APP_LOCK_OFF,
                ),
            )),
        )
    }));
}

/// The window on the real engine: Files and sign-in ask the handle. Every area not yet
/// backed by the engine starts on its `Unwired` state (the shell's "not built yet" frame),
/// never the board's sample data presented as the person's own.
pub fn init_on_engine<C: beebeeb_engine::app::CredentialStores>(
    shell: Shell,
    engine: std::sync::Arc<beebeeb_engine::app::Engine<C>>,
) -> State {
    let drive: Box<dyn DriveSource> = Box::new(crate::engine_host::EngineDriveSource::new(engine.clone()));
    let mut s = init_from(shell, drive, None);
    s.identity = AppIdentity::CURRENT;
    s.actions =
        crate::frames::files::Actions::new(Box::new(crate::engine_host::EngineFilesSource::new(engine.clone())));
    // The banner's two fields S1 feeds from the engine's view (the session ended, offline).
    s.global = crate::frames::global::Ui::new(Box::new(crate::engine_host::EngineGlobalSource::new(engine.clone())));
    s.settings.source = Box::new(crate::engine_host::settings::EngineSettingsSource::new(engine.clone()));
    // Drive's panes: Storage insights answers; every other pane is the shell's frame (`engine_host::drive_settings`).
    s.settings.drive = Box::new(
        crate::engine_host::drive_settings::EngineDriveSettings::new(engine.clone(), crate::shell::platform::current())
            .with_previews(crate::engine_host::drive_previews::PreviewsHost::new(
                engine.clone(),
                crate::engine_host::drive_previews::shared_run(),
            )),
    );
    // The lock screen's Windows Hello prompt is owned by the main window.
    let hello = crate::engine_host::hello::WindowHello::new(s.shell.main_window_slot());
    s.signin = Some(crate::frames::signin::Ui::new(Box::new(
        crate::engine_host::EngineSignInSource::with_hello(engine.clone(), Box::new(hello)),
    )));
    crate::engine_host::media::install_shell_player(engine.clone());
    // An open file: read from the engine, decrypted in memory (`engine_host::viewer`); one source per opened file.
    let viewer_engine = engine.clone();
    crate::frames::viewers::wire(
        &mut s,
        Box::new(move || {
            Box::new(crate::engine_host::viewer::EngineViewerSource::new(
                viewer_engine.clone(),
            ))
        }),
    );
    // Backups: no source while its constant is off (nothing of the adapter runs then; `engine_host::backups`).
    s.backups = crate::frames::backups::BackupsUi::new(crate::engine_host::backups::source_for(engine.clone()));
    // Every area not yet backed by the engine: the shell's "not built yet" frame, never sample
    // data beside a real account. This depends ONLY on "is this the live engine path", never on
    // the identity or a dev flag.
    // Account and Devices and sessions read the engine; every other account pane is the shell's frame
    // (`engine_host::account`).
    // Turning app lock off asks the main window's Windows Hello prompt first; so does approving another device's
    // sign-in, with its own line ("Approve a sign-in on another device").
    s.account.source = Some(Box::new(
        crate::engine_host::account::EngineAccountSource::new(engine.clone())
            .with_hello(Box::new(crate::engine_host::hello::WindowHello::for_reason(
                s.shell.main_window_slot(),
                crate::engine_host::hello::TURN_APP_LOCK_OFF,
            )))
            .with_approval_window(s.shell.main_window_slot()),
    ));
    s.storage_split = None;
    // The figure follows the account: cleared at a sign-out or another account, read again for the one that signs in.
    let storage_engine = engine.clone();
    s.storage_reader = Some(std::sync::Arc::new(move || {
        storage_engine
            .storage_info()
            .ok()
            .map(|(used, quota)| source::StorageSummary {
                used_bytes: used,
                quota_bytes: quota,
            })
    }));
    // The first-time guide's "done", once per account: the engine keeps it in the account's own local state.
    s.guide_marks = Box::new(crate::engine_host::guide::EngineGuideMarks::new(engine.clone()));
    // Shared reads the engine (`engine_host::shared`): from Beebeeb only, for the session in memory.
    s.frames.shared.source = Box::new(crate::engine_host::shared::EngineSharedSource::new(engine.clone()));
    // Trash reads the engine (`engine_host::trash`); Search only when `SEARCH_ON_ENGINE` (task 2488).
    s.frames.trash.source = Box::new(crate::engine_host::trash::EngineTrashSource::new(engine.clone()));
    // An item's read-only Preview over Trash (Beebeeb only) and Shared (`engine_host::item_preview`).
    s.frames.item_preview = Box::new(crate::engine_host::item_preview::EngineItemPreview::new(engine.clone()));
    s.frames.find = crate::frames::search::SearchUi::default();
    if SEARCH_ON_ENGINE {
        s.frames.find.source = Box::new(crate::engine_host::search::EngineSearchSource::new(engine.clone()));
    }
    s.frames.pc = Box::new(crate::frames::UnwiredPc);
    // Photos and every grid of thumbnails read the engine (`engine_host::photos`, `engine_host::thumbs`): one store
    // of thumbnails for both, in memory only, the session's alone.
    let thumbs = crate::engine_host::thumbs::EngineThumbStore::new(engine.clone());
    s.global.grid.thumbs = Box::new(crate::engine_host::thumbs::EngineThumbs::new(thumbs.clone()));
    s.photos = crate::frames::photos::Ui::new(Box::new(crate::engine_host::photos::EnginePhotos::new(
        engine.clone(),
        thumbs,
    )));
    // This PC: the `Unwired` source while its constant is off (nothing of the adapter runs then).
    s.this_pc = crate::frames::this_pc::Ui::new(crate::engine_host::this_pc::source_for(engine));
    s
}
