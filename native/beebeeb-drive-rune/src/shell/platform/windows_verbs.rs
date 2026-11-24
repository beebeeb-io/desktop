//! Slice B6 (plan40 batch 1): Beebeeb's four items in File Explorer's classic menu (DW-1183, "Show more options"):
//! Share with Beebeeb…, Copy Beebeeb link, Show in Beebeeb, Version history… (`control::ExplorerVerb`).
//!
//! Three parts, one per process and moment (no URI scheme):
//!
//! - **The menu** ([`register`]): per-user verbs under `HKCU\Software\Classes\*\shell`, one key per verb, for files
//!   whose path starts with the sync folder's (`AppliesTo`). Each runs `"<this program>" --explorer-verb=<verb> "%1"`,
//!   never a `beebeeb://` link, which any web page could open. Written when the engine opens and someone is signed
//!   in (the same values again: idempotent); a watcher takes them away when the account signs out (the sync folder
//!   goes with it) and writes them again for the next sign-in's folder.
//! - **The launch Explorer started** ([`item_of`], asked by `instance::plan`): the path is the file Explorer named.
//!   It counts only below this app's sync folder, in a sync root Windows has registered to THIS build's provider, as a
//!   placeholder with an id, read through `placeholders::open_for_state` (attributes only, so nothing is fetched).
//!   Only the verb and that id go to the running instance (`control::Request::Explorer`); anything else forwards
//!   nothing and opens nothing.
//! - **The running instance** ([`handle`], from `host::on_control`, which already brought the window forward):
//!   checks everything again (`control::explorer_verb`: the window open for its account, the item in that account's
//!   own local state), then Files, the item's folder, the item selected, and the verb's own surface. Share and Copy
//!   link open the window's Share dialog (DW-0407), whose own button is Copy link: nothing is shared or copied until
//!   the person presses there. Version history… opens DW-0413. Show in Beebeeb stops at the selection.
//!   Under the away lock's wake cover (`registry::covered`: the engine has not said whether the wake locks) the verb
//!   waits, and runs once the cover is gone ([`after_the_cover`]; review of #463, P3-7).
//!
//! Must not: log a path, a name or an id; share, copy or change anything from the menu by itself.

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::identity::EngineIdentity;
use beebeeb_engine::signin::gate::GateState as EngineGate;
use rune::Tx;

use super::PlatformError;
use crate::State;
use crate::shell::app::{Intent, Route, Screen, Section};
use crate::shell::autostart::quote_arg_always;
use crate::shell::control::{self, ExplorerVerb, ItemLookup, VerbPlan};
use crate::shell::deeplink::{self, RegistryValue};
use crate::shell::host;
use crate::shell::identity::AppIdentity;
use crate::shell::instance;
use crate::shell::registry;
use crate::shell::session_gate;

#[cfg(test)]
#[path = "windows_verbs_tests.rs"]
mod tests;

/// How often the watcher looks whether the account signed out or another signed in.
const WATCH_EVERY: Duration = Duration::from_secs(5);

/// The running instance's lookup ([`handle`]): the engine's account-bound local state, set by [`register`].
static ENGINE: Mutex<Option<Arc<dyn ItemLookup + Send + Sync>>> = Mutex::new(None);

/// A verb that came while the window was covered ([`handle_with`]), with the account the window showed then.
struct Waiting {
    verb: ExplorerVerb,
    item: String,
    account: Option<String>,
}

thread_local! {
    /// The verb that waits for the cover's end, on the window's thread. One: a later verb replaces it.
    static WAITING: RefCell<Option<Waiting>> = const { RefCell::new(None) };
    /// A verb whose folder is being read on the worker (task 050): its row is selected when the read lands.
    static OPENING: RefCell<Option<Waiting>> = const { RefCell::new(None) };
}

fn unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// The menu
// ---------------------------------------------------------------------------

/// The menu's items for this app's files, written now and kept as the session changes (the module doc). Called once,
/// when the engine is open (`shell::open_engine`). Also hands the running instance its engine lookup ([`handle`]).
/// Nothing here waits on the window: the registry is written on the watcher's own thread.
pub(crate) fn register<C: CredentialStores>(identity: AppIdentity, engine: &Arc<Engine<C>>) {
    *unpoisoned(&ENGINE) = Some(Arc::new(EngineItems(Arc::downgrade(engine))));
    let engine = Arc::downgrade(engine);
    let spawned = std::thread::Builder::new()
        .name("explorer-verbs".into())
        .spawn(move || {
            // What the registry holds as far as this run knows: nothing done yet, then the folder written (or `None`:
            // taken away). A write that failed is tried again on the next look.
            let mut applied: Option<Option<PathBuf>> = None;
            while keep_menu(identity, &engine, &mut applied) {
                std::thread::sleep(WATCH_EVERY);
            }
        });
    if spawned.is_err() {
        host::log("File Explorer's menu items were not written: no thread for them");
    }
}

/// One look at the session: the menu is there for the signed-in account's sync folder, and gone while nobody is
/// signed in (also at a start that is signed out: an earlier run's items go). Writes only on a change. `false`: the
/// engine is gone (the app is ending).
fn keep_menu<C: CredentialStores>(
    identity: AppIdentity,
    engine: &Weak<Engine<C>>,
    applied: &mut Option<Option<PathBuf>>,
) -> bool {
    let Some(engine) = engine.upgrade() else {
        return false;
    };
    let wanted = match engine.view().gate {
        EngineGate::SignedOut => None,
        // A sync folder the settings cannot name: no menu rather than one for a folder that may not be the account's.
        _ => engine.sync_root().ok(),
    };
    drop(engine);
    if applied.as_ref() == Some(&wanted) {
        return true;
    }
    let done = match &wanted {
        Some(root) => write_menu(identity, root),
        None => remove_menu(identity),
    };
    match done {
        Ok(()) => *applied = Some(wanted),
        Err(error) => host::log(format!("File Explorer's menu items: {error}")),
    }
    true
}

/// Writes every value of [`menu_entries`] (the same values again on every start).
fn write_menu(identity: AppIdentity, root: &Path) -> Result<(), PlatformError> {
    let exe = current_exe()?;
    for value in menu_entries(identity, &exe, root) {
        reg_set(&value.key, value.name, &value.data)?;
    }
    Ok(())
}

/// Takes this build's four keys away, with everything under them. A key that is not there is not an error.
fn remove_menu(_identity: AppIdentity) -> Result<(), PlatformError> {
    for verb in ExplorerVerb::ALL {
        reg_delete_tree(&verb_key(crate::shell::identity::engine_identity(), verb))?;
    }
    Ok(())
}

/// The menu's registry values (each key under `HKEY_CURRENT_USER`), verb by verb in the board's order: the words, the
/// files it applies to (those whose path starts with the sync folder's), the icon, and the command. Pure, so the rule
/// is tested without touching the registry. The uninstaller removes the same four keys ([`verb_key`]).
pub(crate) fn menu_entries(identity: AppIdentity, exe: &str, root: &Path) -> Vec<RegistryValue> {
    let icon = format!("{},0", quote_arg_always(exe));
    let applies_to = format!("System.ItemPathDisplay:~<\"{}\"", root.to_string_lossy());
    ExplorerVerb::ALL
        .into_iter()
        .flat_map(|verb| {
            let key = verb_key(crate::shell::identity::engine_identity(), verb);
            [
                RegistryValue {
                    key: key.clone(),
                    name: "MUIVerb",
                    data: verb.label(identity),
                },
                RegistryValue {
                    key: key.clone(),
                    name: "AppliesTo",
                    data: applies_to.clone(),
                },
                RegistryValue {
                    key: key.clone(),
                    name: "Icon",
                    data: icon.clone(),
                },
                RegistryValue {
                    key: format!("{key}\\command"),
                    name: "",
                    data: command_line(exe, verb),
                },
            ]
        })
        .collect()
}

/// The command a verb runs: this program, the verb's own flag, and the file Explorer names, in quotes. Never a link.
pub(crate) fn command_line(exe: &str, verb: ExplorerVerb) -> String {
    format!(
        "{} {}{} \"%1\"",
        quote_arg_always(exe),
        instance::EXPLORER_VERB_FLAG,
        verb.word()
    )
}

/// One key per verb under every file's classic menu, named for this build (the test build never writes over the real
/// app's keys) and numbered in the board's order (Explorer lists a menu's own keys by name).
fn verb_key(engine: &'static EngineIdentity, verb: ExplorerVerb) -> String {
    let place = ExplorerVerb::ALL.iter().position(|each| *each == verb).unwrap_or(0) + 1;
    format!(
        "Software\\Classes\\*\\shell\\{}.{place}.{}",
        engine.cloud_files_provider_name(),
        verb.word()
    )
}

fn current_exe() -> Result<String, PlatformError> {
    let exe = std::env::current_exe().map_err(|_| PlatformError::Os("finding this program failed".into()))?;
    exe.to_str()
        .map(str::to_owned)
        .ok_or_else(|| PlatformError::Os("this program's path is not valid Unicode".into()))
}

/// One string value under `HKEY_CURRENT_USER\<key>`, the key made when it is not there.
fn reg_set(key: &str, name: &str, data: &str) -> Result<(), PlatformError> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    let (key_w, name_w, data_w) = (
        super::windows::wide(key),
        super::windows::wide(name),
        super::windows::wide(data),
    );
    // SAFETY: the three are NUL-terminated and outlive the call; the size is the data's bytes, its terminator included.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key_w.as_ptr(),
            name_w.as_ptr(),
            REG_SZ,
            data_w.as_ptr().cast(),
            (data_w.len() * 2) as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(PlatformError::Os(format!(
            "writing the registry failed: Win32 error {status}"
        )))
    }
}

/// `HKEY_CURRENT_USER\<key>` with everything under it. Not there: nothing to do.
fn reg_delete_tree(key: &str) -> Result<(), PlatformError> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
    let key_w = super::windows::wide(key);
    // SAFETY: the name is NUL-terminated and outlives the call.
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key_w.as_ptr()) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(PlatformError::Os(format!(
            "writing the registry failed: Win32 error {status}"
        )))
    }
}

// ---------------------------------------------------------------------------
// The launch Explorer started
// ---------------------------------------------------------------------------

/// The id of the item at `path`, for the launch Explorer started (`instance::plan`). `None`: no item of this app's
/// there, and nothing is handed on. This process has no engine: the sync folder comes from the settings, the way the
/// engine reads it (`Engine::sync_root`).
pub(crate) fn item_of(engine: &'static EngineIdentity, path: &Path) -> Option<String> {
    let root = beebeeb_engine::config::DesktopConfig::load_for(engine)
        .ok()?
        .sync_root
        .unwrap_or_else(|| beebeeb_engine::config::default_sync_root_suggestion(engine));
    item_in_root(path, &root, |path| placeholder_id(engine, path))
}

/// The rule, apart from Windows: a path below `root` is read by `read`, any other is not read at all, and only an item
/// id (`deeplink::is_item_id`) comes back.
fn item_in_root(path: &Path, root: &Path, read: impl FnOnce(&Path) -> Option<String>) -> Option<String> {
    if !inside(path, root) {
        return None;
    }
    read(path).filter(|id| deeplink::is_item_id(id))
}

/// Is `path` below `root` (never `root` itself)? Both absolute, neither with a `..`, compared part by part, names
/// without case as Windows compares them (`engine_host::backups::may_back_up`'s shape, never a string prefix: a
/// `Beebeeb2` beside `Beebeeb` is outside). Another spelling of the same folder (`\\?\`, a short 8.3 name) reads as
/// outside, which forwards nothing.
fn inside(path: &Path, root: &Path) -> bool {
    let plain = |p: &Path| {
        p.is_absolute()
            && p.components()
                .all(|part| matches!(part, Component::Prefix(_) | Component::RootDir | Component::Normal(_)))
    };
    if !plain(path) || !plain(root) {
        return false;
    }
    let same = |a: &std::ffi::OsStr, b: &std::ffi::OsStr| {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    };
    let (root, path): (Vec<_>, Vec<_>) = (root.components().collect(), path.components().collect());
    !root.is_empty()
        && path.len() > root.len()
        && root.iter().zip(&path).all(|(a, b)| same(a.as_os_str(), b.as_os_str()))
}

/// The id Windows holds for the placeholder at `path`, read as the sync engine reads it: attributes only, the reparse
/// point itself, so a file that is online only is not fetched. Only in a sync root registered to THIS build's
/// provider: a folder linked into the sync folder from another provider's root is not ours.
fn placeholder_id(engine: &'static EngineIdentity, path: &Path) -> Option<String> {
    use beebeeb_engine::windows_cf::{placeholders, registered_provider_name};
    if registered_provider_name(path).ok()? != engine.cloud_files_provider_name() {
        return None;
    }
    let file = placeholders::open_for_state(path).ok()?;
    match placeholders::handle_identity(&file) {
        (true, id) => id,
        (false, _) => None,
    }
}

// ---------------------------------------------------------------------------
// The running instance
// ---------------------------------------------------------------------------

/// The engine as the running instance asks it about a forwarded item. Weak: it never keeps the engine alive.
struct EngineItems<C: CredentialStores>(Weak<Engine<C>>);

impl<C: CredentialStores> ItemLookup for EngineItems<C> {
    /// The words `EngineActions::session` gives the window (`engine_host::LiveEngine::session`).
    fn account(&self) -> Option<String> {
        let engine = self.0.upgrade()?;
        engine.account_email().or_else(|| engine.user_id())
    }

    /// The account-bound handle (task 2488): refused while locked and for a local state that is not the session's;
    /// an answer read across a lock or a sign-out is dropped.
    fn folder_of(&self, id: &str) -> Option<Option<String>> {
        let engine = self.0.upgrade()?;
        let state = engine.local_state().ok()?;
        let entry = state.with(|db| db.get_file(id).ok().flatten()).ok()??;
        Some(entry.parent_id)
    }
}

/// A verb from File Explorer's menu (`control::Request::Explorer`), on the window thread. `host::on_control` has
/// already brought the window forward (a locked one to its lock screen).
pub(crate) fn handle(s: &mut State, verb: ExplorerVerb, item: &str, tx: &mut Tx) {
    let engine = unpoisoned(&ENGINE).clone();
    handle_with(s, verb, item, engine.as_deref().map(|e| e as &dyn ItemLookup), tx);
}

/// [`handle`], with the engine lookup handed in (a test's).
pub(crate) fn handle_with(s: &mut State, verb: ExplorerVerb, item: &str, engine: Option<&dyn ItemLookup>, tx: &mut Tx) {
    // Under the wake cover the engine has not said whether the wake locks: nothing is looked up, navigated or opened
    // behind it. The verb waits for the cover's end (review of #463, P3-7). Also a verb that comes before the main
    // window's first poll after an unannounced wake (`host::under_the_wake_cover`; review of #464, P3-1).
    if host::under_the_wake_cover(s) {
        let account = s.shell.app.account().map(str::to_owned);
        WAITING.set(Some(Waiting {
            verb,
            item: item.to_owned(),
            account,
        }));
        host::log("a File Explorer verb waits for the lock screen");
        return;
    }
    let window_open = session_gate::open(s);
    let account = s.shell.app.account().map(str::to_owned);
    match control::explorer_verb(verb, item, window_open, account.as_deref(), engine) {
        VerbPlan::ShowOnly(why) => host::log(format!("a File Explorer verb was not followed: {why}")),
        VerbPlan::Open { verb, folder, item } => open(s, verb, folder, &item, tx),
    }
}

/// The main window's poll, after the away lock's (`host::poll_screen`): a verb that waited under the cover runs once
/// the cover is gone, checked again as it came ([`handle`]). A window that locked or signed out meanwhile (the cover
/// became the lock), or another account, drops it, as a locked window drops a verb: nothing is kept for after an
/// unlock. `true`: it ran.
pub(crate) fn after_the_cover(s: &mut State, tx: &mut Tx) -> bool {
    // Every poll comes here: nothing waits, nothing to read.
    let landed = finish_open(s, tx);
    if WAITING.with_borrow(Option::is_none) {
        return landed;
    }
    let engine = unpoisoned(&ENGINE).clone();
    after_the_cover_with(s, engine.as_deref().map(|e| e as &dyn ItemLookup), tx) || landed
}

/// [`after_the_cover`], with the engine lookup handed in (a test's).
pub(crate) fn after_the_cover_with(s: &mut State, engine: Option<&dyn ItemLookup>, tx: &mut Tx) -> bool {
    let account = s.shell.app.account();
    let Some(same_account) =
        WAITING.with_borrow(|waiting| waiting.as_ref().map(|waiting| waiting.account.as_deref() == account))
    else {
        return false;
    };
    if !session_gate::open(s) || !same_account {
        WAITING.set(None);
        host::log("a File Explorer verb that waited was not followed: the window locked, or another account");
        return false;
    }
    if registry::covered(s) {
        return false;
    }
    let Some(waiting) = WAITING.take() else {
        return false;
    };
    handle_with(s, waiting.verb, &waiting.item, engine, tx);
    true
}

/// The row of the item `id` in the folder the Files window holds.
fn row_of(s: &State, id: &str) -> Option<usize> {
    s.listing.items.iter().position(|row| row.id == id)
}

/// Files, in the item's folder, with the item selected; then the verb's own surface. The row is looked for first, in
/// the folder the engine names: a row that folder's listing does not hold leaves the window as it was (it only came
/// forward). Otherwise whatever the Files window had open closes, through its wiping path (a Share dialog's password
/// with it), before the folder is shown.
fn open(s: &mut State, verb: ExplorerVerb, folder: Option<String>, item: &str, tx: &mut Tx) {
    let shown = s.listing.folder.id == folder && row_of(s, item).is_some();
    if !shown {
        let (listing, _) = crate::shell::state::load(s.source.as_ref(), folder.as_deref());
        if !listing.items.iter().any(|row| row.id == item) {
            host::log("a File Explorer verb found no row");
            return;
        }
    }
    host::request(s, Intent::Go(Route::section(Section::Files)));
    if !matches!(s.shell.app.screen(), Screen::Drive(route) if route.section == Section::Files) {
        host::log("a File Explorer verb did not reach Files");
        return;
    }
    crate::frames::files::reset::close_dialogs(s);
    if !shown {
        s.open_folder(folder);
        // The running app reads the folder on the worker: the row is selected once the read lands (`finish_open`).
        if s.folder_reads.opening() {
            let account = s.shell.app.account().map(str::to_owned);
            OPENING.set(Some(Waiting {
                verb,
                item: item.to_owned(),
                account,
            }));
            return;
        }
    }
    select_and_run(s, verb, item, tx);
}

/// The poll after a folder opened for a verb was read: the row is selected and the verb's surface follows. A read
/// still out waits; a window that locked, or another account, drops it; a row the read does not hold says so.
/// `true`: the window changed.
fn finish_open(s: &mut State, tx: &mut Tx) -> bool {
    if OPENING.with_borrow(Option::is_none) || s.folder_reads.opening() {
        return false;
    }
    let Some(waiting) = OPENING.take() else {
        return false;
    };
    if !session_gate::open(s) || waiting.account.as_deref() != s.shell.app.account() {
        host::log("a File Explorer verb that waited for its folder was not followed: the window locked, or another account");
        return false;
    }
    select_and_run(s, waiting.verb, &waiting.item, tx);
    true
}

fn select_and_run(s: &mut State, verb: ExplorerVerb, item: &str, tx: &mut Tx) {
    let Some(row) = row_of(s, item) else {
        host::log("a File Explorer verb found no row");
        return;
    };
    s.select_only(row);
    match verb {
        // The dialog only: its own Copy link makes the link, when the person presses it.
        ExplorerVerb::Share | ExplorerVerb::CopyLink => s.ask_share(tx),
        ExplorerVerb::Show => {}
        ExplorerVerb::Versions => {
            if !s.begin_versions() {
                host::log("a File Explorer verb's Version history did not open");
            }
        }
    }
}
