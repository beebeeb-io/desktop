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
    /// The first-time guide over the main window (DW-0380, DW-0301 to DW-0305).
    pub(crate) guide: crate::frames::signin::setup::guide::Guide,
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
        guide: Default::default(),
        this_pc: crate::frames::this_pc::Ui::fixture(),
        backups: crate::frames::backups::BackupsUi::first(crate::shell::instance::DEV_BUILD),
    }
}

/// Search on the engine (`engine_host::search`) in the running app. OFF until task 2488: the search index is built
/// from this computer's local state, which can still belong to the previous account (a sign-out keeps it, and no
/// read checks the data's owner against the session), so Search could answer with another account's names. While
/// this is false the shell hands Search the `Unwired` source and no part of the adapter runs (no index, no worker).
pub(crate) const SEARCH_ON_ENGINE: bool = false;

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
    s.settings.drive = Box::new(crate::engine_host::drive_settings::EngineDriveSettings::new(
        engine.clone(),
        crate::shell::platform::current(),
    ));
    s.signin = Some(crate::frames::signin::Ui::new(Box::new(
        crate::engine_host::EngineSignInSource::new(engine.clone()),
    )));
    // Backups: no source while its constant is off (nothing of the adapter runs then; `engine_host::backups`).
    s.backups = crate::frames::backups::BackupsUi::new(crate::engine_host::backups::source_for(engine.clone()));
    // Every area not yet backed by the engine: the shell's "not built yet" frame, never sample
    // data beside a real account. This depends ONLY on "is this the live engine path", never on
    // the identity or a dev flag.
    s.account.source = None;
    s.storage_split = None;
    s.frames.shared.source = Box::new(crate::frames::shared::source::Unwired);
    // Trash reads the engine (`engine_host::trash`); Search only when `SEARCH_ON_ENGINE` (task 2488).
    s.frames.trash.source = Box::new(crate::engine_host::trash::EngineTrashSource::new(engine.clone()));
    s.frames.find = crate::frames::search::SearchUi::default();
    if SEARCH_ON_ENGINE {
        s.frames.find.source = Box::new(crate::engine_host::search::EngineSearchSource::new(engine.clone()));
    }
    s.frames.pc = Box::new(crate::frames::UnwiredPc);
    s.photos = crate::frames::photos::Ui::new(Box::new(crate::frames::photos::source::Unwired));
    // This PC: the `Unwired` source while its constant is off (nothing of the adapter runs then).
    s.this_pc = crate::frames::this_pc::Ui::new(crate::engine_host::this_pc::source_for(engine));
    s
}
