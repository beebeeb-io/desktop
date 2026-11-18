//! What Settings (and the sign-in screens) asked the shell for: web pages, the clipboard, Windows Settings.
//! Every variant has an arm; one this build does not do is logged as `NotWired(<variant>)`, never dropped.
//! And what the frames of Trash, Shared and Search asked for (`frames::Intent`): an item's Details and its
//! read-only Preview (`frames::item`); every other intent is taken and logged by its fixed words only.
//! And Backups' (`frames::backups::Intent`): Add a folder… opens Windows' own folder picker (DW-0738) through the
//! generic [`pick_folder_for`], whose answer goes back to the section that asked ([`FolderFor`]); the rest are logged.
//! The picker is modeless (the window stays usable while it is open), so its answer is handed over only for the session
//! of the press: one that changed while it was open (a lock, a sign-out, another account) drops it.
//! And what comes from outside the window under the away lock's wake cover (`host::under_the_wake_cover`: the engine has
//! not said whether the wake locks; review of #464, P3-1): the picker's folder, and an ask to show a place (a
//! `beebeeb://` link, the b's Settings…, the flyout: `host::request`). Each waits, and goes on once the cover is gone,
//! checked again ([`after_the_cover`]); a lock, a sign-out or another account drops it, as File Explorer's verbs do
//! (`platform::windows_verbs`, P3-7). One exception (the lead's ruling of 2026-11-16 05:05): a link only opens a view, so
//! it follows the route machine's rule for a link that comes while locked: kept across a lock of the same account and
//! opened after the unlock, checked again; a sign-out, an ended session or another account drops it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use rune::Tx;

use super::app::{GateState, Intent as Ask};
use super::host::log;
use super::platform::{Platform, PlatformError};
use super::{registry, session_gate, web_pages};
use crate::State;
use crate::frames::settings::Intent;

/// Opens `address` (one of the table's) and says so when it did not open: a log line, and the toast of the
/// pane that asked.
fn open(s: &mut State, address: &str) {
    if crate::engine_host::signin::open_web_page(address).is_err() {
        log("a web page did not open");
        s.settings.toast = Some("Could not open that page");
    }
}

#[cfg(test)]
thread_local! {
    static POLL_DRAINS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests only: let the shell's poll act on Settings' and the frames' queues. By default a test's window leaves
/// both alone, because the frames' own tests read what a press asked for (`settings.intents`, `frames.intents`).
#[cfg(test)]
pub fn poll_drains_for_this_test() {
    POLL_DRAINS.set(true);
}

/// The shell's poll: acts on the queues (in a test, only where the test asked for it).
pub fn drain_in_poll(s: &mut State, tx: &mut Tx) -> bool {
    #[cfg(test)]
    if !POLL_DRAINS.get() {
        return false;
    }
    let settings = drain_settings_intents(s, tx);
    let frames = drain_frame_intents(s, tx);
    let viewer = drain_viewer_intents(s, tx);
    let backups = drain_backups_intents(s);
    let picked = take_picked_folder(s, tx);
    settings | frames | viewer | backups | picked
}

/// Which section asked for Windows' folder picker, and gets the folder: one variant per section. A later section
/// (Photos' folders, say) adds its variant, its title and its arm in [`hand_folder`]; the picker itself is shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderFor {
    /// Backups' Add a folder… (DW-0738): the folder goes to `BackupsUi::folder_picked`.
    Backups,
}

impl FolderFor {
    /// The picker's title, in the section's words.
    fn title(self) -> &'static str {
        match self {
            Self::Backups => "Choose a folder to back up",
        }
    }
}

/// The picker's answer: a folder, none (closed without one), or the platform's refusal.
pub type PickAnswer = Result<Option<PathBuf>, PlatformError>;

/// A picker that is open: who asked, the session of the press, and where its answer will come.
struct Picking {
    asker: FolderFor,
    /// Whose session the press was made under: the account the window showed, read through the session gate when the
    /// picker opened (the poll of the press, before that poll's refresh).
    account: String,
    /// A poll saw the session differ from the press's while the picker was open (the gate closed: a lock, a sign-out, an
    /// ended session; or another account). It stays set: a session that seems to be back (the same account signed in
    /// again) is another session, and the answer is dropped.
    session_changed: bool,
    answer: mpsc::Receiver<PickAnswer>,
}

/// A folder picked while the window was under the wake cover, for the session of the press.
struct HeldFolder {
    asker: FolderFor,
    /// The press's account (`Picking::account`).
    account: String,
    folder: PathBuf,
}

/// An ask to show a place (`host::request`) that came while the window was under the wake cover, with the account the
/// window showed then.
struct HeldAsk {
    ask: Ask,
    account: Option<String>,
    /// A `beebeeb://` link (`host::on_open_url`): kept across a lock of the same account.
    link: bool,
}

thread_local! {
    /// The one picker open from this window's thread (the window's loop runs on one thread).
    static PICKING: RefCell<Option<Picking>> = const { RefCell::new(None) };
    /// The picker's folder that waits for the wake cover's end. One: one picker is open at a time.
    static HELD_FOLDER: RefCell<Option<HeldFolder>> = const { RefCell::new(None) };
    /// The ask to show a place that waits for the wake cover's end. One: a later ask replaces it.
    static HELD_ASK: RefCell<Option<HeldAsk>> = const { RefCell::new(None) };
}

/// How often the poll looks for the picker's answer while it is open.
const PICKER_POLL: Duration = Duration::from_millis(250);

/// A folder picker is open from this window's thread.
pub fn a_folder_picker_is_open() -> bool {
    PICKING.with(|picking| picking.borrow().is_some())
}

/// Opens `platform`'s folder picker for `asker` on a worker of its own (it BLOCKS until the person closes it), showing
/// where `start` is, for the session of the press ([`pick_folder_with`]). Its answer is handed over by
/// [`take_picked_folder`] on the shell's poll.
pub fn pick_folder_for(s: &State, asker: FolderFor, platform: Arc<dyn Platform>, start: PathBuf) -> bool {
    let title = asker.title();
    pick_folder_with(s, asker, move || platform.pick_folder(title, &start))
}

/// Opens a folder picker for `asker`: `pick` runs on a worker of its own and answers when the person closes it. The
/// picker carries the session of the press, read here: no picker opens while the window may not act on the account's
/// files (`session_gate::open`). One at a time: while one is open another ask is not opened. `false`: not opened
/// (logged by fixed words).
pub fn pick_folder_with(s: &State, asker: FolderFor, pick: impl FnOnce() -> PickAnswer + Send + 'static) -> bool {
    let account = match s.shell.app.account() {
        Some(account) if session_gate::open(s) => account.to_owned(),
        _ => {
            log("the folder picker was not opened: no session the window may act on");
            return false;
        }
    };
    PICKING.with(|picking| {
        let mut picking = picking.borrow_mut();
        if picking.is_some() {
            log("a folder picker is open already");
            return false;
        }
        let (answers, answer) = mpsc::channel();
        let started = std::thread::Builder::new().name("folder-picker".into()).spawn(move || {
            let _ = answers.send(pick());
        });
        if started.is_err() {
            log("the folder picker could not start");
            return false;
        }
        *picking = Some(Picking {
            asker,
            account,
            session_changed: false,
            answer,
        });
        true
    })
}

/// The open picker's answer, once it came: the folder goes to the section that asked, for the session of the press;
/// closed without one, nothing changes; a picker that did not open is logged in the platform's own words (no path is
/// in them). While it is still open the poll looks again soon. Every poll first compares the session with the
/// press's: once it differed, the folder is dropped (never handed to the section, never asked of the engine). A folder
/// that comes under the wake cover waits for the cover's end ([`after_the_cover`]). `true`: an answer was taken. The
/// folder itself is never logged.
pub fn take_picked_folder(s: &mut State, tx: &mut Tx) -> bool {
    if !a_folder_picker_is_open() {
        return false;
    }
    // The session as it is NOW (the engine's, through the gate), and whose it is.
    let now = session_gate::open(s)
        .then(|| s.shell.app.account().map(str::to_owned))
        .flatten();
    let answered = PICKING.with(|picking| {
        let mut picking = picking.borrow_mut();
        let open = picking.as_mut()?;
        open.session_changed |= now.as_deref() != Some(open.account.as_str());
        let got = open.answer.try_recv();
        match got {
            Err(mpsc::TryRecvError::Empty) => {
                tx.wake_after(PICKER_POLL);
                None
            }
            got => picking
                .take()
                .map(|open| (open.asker, open.account, open.session_changed, got.ok())),
        }
    });
    let Some((asker, account, session_changed, answer)) = answered else {
        return false;
    };
    match answer {
        Some(Ok(Some(_))) if session_changed => {
            log("a picked folder was dropped: the session changed while the picker was open")
        }
        // Under the wake cover the engine has not said whether the wake locks: nothing is added behind it. The
        // folder waits for the cover's end (review of #464, P3-1).
        Some(Ok(Some(folder))) if super::host::under_the_wake_cover(s) => {
            HELD_FOLDER.set(Some(HeldFolder { asker, account, folder }));
            log("a picked folder waits for the lock screen");
        }
        Some(Ok(Some(folder))) => hand_folder(s, asker, &folder),
        Some(Ok(None)) => {}
        Some(Err(error)) => log(format!("the folder picker did not open ({error})")),
        None => log("the folder picker ended early"),
    }
    true
}

/// Hands a picked folder to the section that asked.
fn hand_folder(s: &mut State, asker: FolderFor, folder: &Path) {
    match asker {
        FolderFor::Backups => match folder.to_str() {
            // `false`: nothing was asked of the engine (signed out, the vault closed): logged by fixed words.
            Some(path) => {
                if !s.backups.folder_picked(path) {
                    log("Backups did not take the picked folder");
                }
            }
            None => log("the picked folder's name is not text this app can pass on"),
        },
    }
}

/// `host::request` under the wake cover: the ask waits for the cover's end ([`after_the_cover`]), with the account the
/// window shows now. `link`: it came as a `beebeeb://` link. One waits: a later ask replaces it.
pub(super) fn hold_under_the_cover(s: &State, ask: Ask, link: bool) {
    let account = s.shell.app.account().map(str::to_owned);
    HELD_ASK.set(Some(HeldAsk { ask, account, link }));
    log("an ask to show a place waits for the lock screen");
}

/// An ask from outside came and was not held (`host::request`): it replaces the one that waits, which would otherwise
/// go on after it (a link kept across a lock, released after the unlock, over a newer ask the route machine kept).
pub(super) fn a_later_ask_came() {
    HELD_ASK.set(None);
}

/// The main window's poll, right after the away lock's (`host::poll_screen`): what waited under the wake cover goes on
/// once the cover is gone, checked again as it came. The session is compared first: a lock (the cover became the
/// lock), a sign-out, an ended session or another account drops it, so nothing is kept for after an unlock, but a link
/// (the module doc). `true`: something went on or was dropped.
pub(crate) fn after_the_cover(s: &mut State) -> bool {
    let folder = release_the_folder(s);
    let ask = release_the_ask(s);
    folder | ask
}

/// The folder that waited goes to the section that asked, for the press's session only.
fn release_the_folder(s: &mut State) -> bool {
    // Every poll comes here: nothing waits, nothing to read.
    let Some(account) = HELD_FOLDER.with_borrow(|held| held.as_ref().map(|held| held.account.clone())) else {
        return false;
    };
    // Before the cover: a lock takes the cover away too (the lock screen is not the cover), and must not hand it over.
    if !(session_gate::open(s) && s.shell.app.account() == Some(account.as_str())) {
        HELD_FOLDER.set(None);
        log("a picked folder that waited was dropped: the window locked, signed out or shows another account");
        return true;
    }
    if registry::covered(s) {
        return false;
    }
    let Some(held) = HELD_FOLDER.take() else {
        return false;
    };
    hand_folder(s, held.asker, &held.folder);
    true
}

/// The ask that waited goes to the route machine, as it came (`host::dispatch`; the window came forward already): once
/// the cover is gone, or for a link kept across a lock, once the same account unlocked.
fn release_the_ask(s: &mut State) -> bool {
    let Some((account, link)) =
        HELD_ASK.with_borrow(|held| held.as_ref().map(|held| (held.account.clone(), held.link)))
    else {
        return false;
    };
    let session = s.shell.actions.session();
    // The window and the engine still show the account the ask came for (a locked session keeps its account).
    let same_account = account.is_some()
        && s.shell.app.account() == account.as_deref()
        && session.account.as_deref() == account.as_deref();
    if !(same_account && under_an_open_gate(s, &session)) {
        // A link waits across a lock of the same account, for the unlock (the module doc). A sign-out names no account.
        if link && same_account && !s.session_ended() {
            return false;
        }
        HELD_ASK.set(None);
        log("an ask to show a place that waited was dropped: the window locked, signed out or shows another account");
        return true;
    }
    if registry::covered(s) {
        return false;
    }
    let Some(held) = HELD_ASK.take() else {
        return false;
    };
    super::host::dispatch(s, super::app::Event::Request(held.ask));
    true
}

/// The window's session, as the engine has it now, is still open under one of the gates the wake cover lies over
/// (`registry::covered`: Ready, the first run, no plan) for the account the window shows: not locked, signed out or
/// ended.
fn under_an_open_gate(s: &State, session: &super::app::Session) -> bool {
    let open = |gate| matches!(gate, GateState::Ready | GateState::FirstRun | GateState::NoPlan);
    open(s.shell.app.gate())
        && open(session.gate)
        && !s.session_ended()
        && session.account.as_deref() == s.shell.app.account()
}

/// Where the picker opens: this PC's user folder (the person's own folders are in it); without one, where Windows
/// opens it.
fn picker_start() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Takes every intent Backups queued: Add a folder… opens the folder picker for Backups; every other intent is logged
/// as `NotWired(<variant>)` (no id in the line). `true`: something was taken.
pub fn drain_backups_intents(s: &mut State) -> bool {
    use crate::frames::backups::Intent as Backups;
    let intents = s.backups.take_intents();
    for intent in &intents {
        match intent {
            // Not opened (no session the window may act on, or one is open already): logged by its own words.
            Backups::PickFolder => {
                pick_folder_for(s, FolderFor::Backups, super::platform::current(), picker_start());
            }
            Backups::OpenInPhotos(_) => log("NotWired(OpenInPhotos)"),
            Backups::OpenInFiles(_) => log("NotWired(OpenInFiles)"),
            Backups::MailBackup => log("NotWired(MailBackup)"),
            Backups::OpenMailFolder => log("NotWired(OpenMailFolder)"),
        }
    }
    !intents.is_empty()
}

/// Takes every intent the frames of Trash, Shared and Search queued. Details (`About("DW-0411", id)`) and Preview
/// (`Do("preview", id)`) open over the section shown when the id is in its current list; every other intent is
/// logged as `NotWired(<its fixed words>)`. No line carries an id, a name or typed words. `true`: something was
/// taken.
pub fn drain_frame_intents(s: &mut State, tx: &mut Tx) -> bool {
    let intents = std::mem::take(&mut s.frames.intents);
    for intent in &intents {
        use crate::frames::item::Origin;
        let taken = match crate::frames::item::origin_shown(s) {
            Some(Origin::Shared) => crate::frames::shared::actions::take(s, tx, intent),
            Some(Origin::Trash) => crate::frames::trash::actions::take(s, tx, intent),
            Some(Origin::Search) => crate::frames::search::actions::take(s, tx, intent),
            None => false,
        } || crate::frames::item::take(s, tx, intent);
        if !taken {
            log(format!("NotWired({})", crate::frames::item::tag(intent)));
        }
    }
    !intents.is_empty()
}

/// Takes every intent the open viewer queued: `viewer_intents::take` acts on it; one nothing takes yet shows
/// "Not built yet: <what>" in the viewer's toast (where it has a name) and is logged by its fixed words. `true`:
/// something was taken.
pub fn drain_viewer_intents(s: &mut State, tx: &mut Tx) -> bool {
    let Some(ui) = crate::frames::viewers::ui(s) else {
        return false;
    };
    let intents = ui.take_intents();
    for intent in &intents {
        if super::viewer_intents::take(s, tx, intent.clone()) {
            continue;
        }
        log("NotWired(viewer intent)");
        if let (Some(what), Some(ui)) = (intent.not_built(), crate::frames::viewers::ui(s)) {
            ui.toast = Some(format!("{}: {what}", crate::frames::viewers::NOT_BUILT));
        }
    }
    !intents.is_empty()
}

/// Takes every intent Settings queued and acts on it. `true`: something was taken.
pub fn drain_settings_intents(s: &mut State, tx: &mut Tx) -> bool {
    let intents = s.settings.take_intents();
    for intent in &intents {
        match intent {
            Intent::Web(name) => match web_pages::url(name) {
                Some(address) => open(s, address),
                None => log(format!("NotWired(web page {name})")),
            },
            Intent::Copy(text) => {
                tx.copy(text.clone());
                s.settings.toast = Some("Copied");
            }
            Intent::OpenWindowsNotificationSettings => open(s, web_pages::WINDOWS_NOTIFICATIONS),
            Intent::Show(_) => log("NotWired(Show)"),
            Intent::Open(target) => super::settings_open::open(s, tx, target),
            // Taken by the settings source's updater (`SettingsSource::check_for_updates`, `restart_to_update`): the
            // shell's part of the hand-off (quit once the installer started, start sync again after a failed start)
            // comes through `frames::settings::poll`.
            Intent::CheckForUpdates | Intent::RestartToUpdate => {}
            // The export already saved the bundle and opened the mail draft (`drive_settings_tools.rs`).
            Intent::SaveBundleAndWriteEmail => {}
            Intent::ShowInFiles(id) => super::settings_show_in_files::show(s, tx, id),
        }
    }
    !intents.is_empty()
}

pub(super) fn open_for_signin(address: &str) {
    if crate::engine_host::signin::open_web_page(address).is_err() {
        log("a web page did not open");
    }
}

/// Tests only: a picked folder waits for the wake cover's end.
#[cfg(test)]
pub(crate) fn a_picked_folder_waits() -> bool {
    HELD_FOLDER.with_borrow(Option::is_some)
}

/// Tests only: an ask to show a place waits for the wake cover's end (or, a link, for the unlock).
#[cfg(test)]
pub(crate) fn an_ask_waits() -> bool {
    HELD_ASK.with_borrow(Option::is_some)
}

#[cfg(test)]
#[path = "intents_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "intents_cover_tests.rs"]
mod cover_tests;
