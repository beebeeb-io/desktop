//! Drive's own settings panes on the real engine: [`EngineDriveSettings`], the `DriveSettings` of the running app.
//!
//! Wired (draws what it read): Storage insights (DW-0783, b, c, d): the account's use from Beebeeb
//! ([`Engine::storage_info`]), the breakdown by type and the largest files worked out on this PC from the file list
//! in the local state database (decrypted names, in memory only).
//!
//! Bound but NOT wired (each pane keeps the shell's "not built yet" frame; the PR body names what each one lacks):
//! - Sync and File Explorer: the sync state from the handle's view, Pause and Resume ([`Engine::pause`] /
//!   [`Engine::resume`]), Open at login through the shell's own autostart call (`Platform::set_autostart`, no
//!   registry write of its own), the downloaded and kept sizes, Free up space
//!   ([`Engine::free_up_space`], in the signed-in account's local state through the account-bound handle).
//!   Missing: a repair and a turn-off of File Explorer in the engine crate, a limit that can say "no limit" (the
//!   engine's default), an engine that honours "Show sync status".
//! - Storage: `storage()` holds only what the engine has (use, downloaded, kept); the per-kind use, the Trash size,
//!   the previews' size and limit have no engine call.
//! - Keep on this PC (a dialog of Sync and Storage): the folder tree with sizes and the kept set from the local
//!   state, and Apply through the pin change ([`Engine::set_kept`]), refused while sync does not run.
//! - Bandwidth, Thumbnails and previews, Diagnostics: the trait's defaults (no engine module).
//!
//! Thread rule: nothing here waits on the window's thread but the handle's `view()` (never waits). One worker per
//! source does every read and every change in the order asked and sends each answer down one channel, which
//! [`DriveSettings::pump`] drains every frame.
//!
//! Whose answer (review of desktop#377, P2-5; the model is `engine_host/this_pc.rs`): every job carries the account
//! it was asked for and a generation, and every answer the generation and the account it is for (none when the
//! account was not the same after the work as before it). `pump` compares the gate and the account FIRST: a change
//! drops what the window holds and starts a new generation, and only then are the worker's answers taken in, each
//! only when its generation is the current one and its account the one the window sees. Every reader hands out
//! nothing unless the vault is open and the signed-in account is the one the answer was read for. A change (Pause,
//! Open at login, Free up space, Keep) runs only while the account that asked is still signed in with the vault
//! open; a change that did not happen is said in the window's toast.
//!
//! Whose state (task 2488): the local state database can still belong to an EARLIER account after a sign-out that
//! keeps local data. Nothing is read from it unless its recorded owner is the signed-in account
//! ([`DriveEngine::owns`]); otherwise the local sizes and the folder tree answer empty.
//!
//! When it reads (P2-3): when the gate or the account changes, and when Insights opens and what is held is older than
//! [`INSIGHTS_FRESH_FOR`]. No periodic timer.
//!
//! Must not: log a name, a path or an id; keep a name in memory while the vault is locked or no one is signed in;
//! draw a value it has not read.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::backup_protect::BackupsTree;
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::drive_v2::{dto::DriveItem, model, status};
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::state_db::{FileEntry, PinState, StateDb};
use beebeeb_engine::sync_view::{Explorer, SyncView};

use super::log;
use crate::frames::settings::Pane;
use crate::frames::settings::drive::{
    DriveSettings, Insights, InsightsData, Kept, KeptOnPc, KindShare, LargeFile, Limit, Repair, Storage, SyncPane,
    SyncStatus,
};
use crate::frames::settings::keep::{Folder, Keep};
use crate::shell::platform::Platform;

#[cfg(test)]
#[path = "drive_settings_tests.rs"]
mod tests;

/// How recently insights must have been read for `opened` not to ask again.
pub const INSIGHTS_FRESH_FOR: Duration = Duration::from_secs(30);
/// The largest files the page lists by name; the rest of the top 50 is "And N more, down to …".
const LARGEST_SHOWN: usize = 7;
const LARGEST_TOP: usize = 50;
/// How long a path may be before the page cuts it in the middle (the file name always stays whole).
const PATH_SHOWN: usize = 49;

/// What Storage insights says when it cannot load. Fixed sentences: never a server's words or a path.
pub const INSIGHTS_OFFLINE: &str = "Couldn't reach Beebeeb. Check your connection and try again.";
pub const INSIGHTS_FAILED: &str = "Couldn't work out where your storage goes. Try again.";
pub const INSIGHTS_LOCKED: &str = "Unlock Beebeeb to see where your storage goes.";

/// What the window says when a change the person asked for did not happen. Fixed sentences.
pub const KEEP_NOT_SYNCING: &str = "Sync isn't running, so the folders were not changed.";
pub const KEEP_FAILED: &str = "Couldn't change what is kept on this PC. Try again.";
pub const FREE_UP_FAILED: &str = "Couldn't free up space on this PC. Try again.";
pub const PAUSE_FAILED: &str = "Couldn't pause or resume sync. Try again.";
pub const OPEN_AT_LOGIN_FAILED: &str = "Couldn't change Open at login. Try again.";
/// A change asked while no one is signed in with the vault open, or by an account that has left since.
pub const CHANGE_LOCKED: &str = "Unlock Beebeeb to change this.";

/// What the adapter asks of the engine handle. The app's is `Engine`; a test's is a state database in a folder of
/// its own.
pub trait DriveEngine: Send + Sync + 'static {
    /// The handle's view. Never waits.
    fn view(&self) -> SyncView;
    /// Whose account is signed in (the stored email, else the user id the server named). Never waits.
    fn account(&self) -> Option<String>;
    /// Read the signed-in account's local state, through the engine's account-bound handle (task 2488). `Err`: it
    /// is not handed out, and why (nothing synced yet, another account's, the session changed, unreadable).
    fn local_state<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused>;
    /// Used and quota bytes from Beebeeb. A network call: a worker's.
    fn storage_info(&self) -> Result<(i64, i64), String>;
    fn set_paused(&self, paused: bool) -> Result<(), String>;
    /// Sync runs for this handle. May wait: a worker's.
    fn syncing(&self) -> bool;
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String>;
    /// Make every downloaded copy that is not kept online only again: the bytes freed.
    fn free_up_space(&self) -> Result<u64, String>;
    /// The settings file's limit on downloaded copies (0: no limit) and its "Show sync status on files".
    fn local_settings(&self) -> Result<(i64, bool), String>;
}

impl<C: CredentialStores> DriveEngine for Engine<C> {
    fn view(&self) -> SyncView {
        Engine::view(self)
    }
    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }
    fn local_state<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        Engine::local_state(self).and_then(|state| state.with(read))
    }
    fn storage_info(&self) -> Result<(i64, i64), String> {
        Engine::storage_info(self)
    }
    fn set_paused(&self, paused: bool) -> Result<(), String> {
        if paused { self.pause() } else { self.resume() }
    }
    fn syncing(&self) -> bool {
        self.is_syncing()
    }
    fn set_kept(&self, file_id: &str, kept: bool) -> Result<(), String> {
        Engine::set_kept(self, file_id, kept)
    }
    fn free_up_space(&self) -> Result<u64, String> {
        // The engine's own, in the signed-in account's local state (the account-bound handle, task 2488).
        Engine::free_up_space(self)
    }
    fn local_settings(&self) -> Result<(i64, bool), String> {
        let config = DesktopConfig::load_for(crate::shell::identity::engine_identity())?;
        Ok((config.local_cache_limit_bytes, config.sync_overlays.unwrap_or(true)))
    }
}

/// What this PC holds, as last read.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct OnThisPc {
    downloaded: i64,
    kept_items: usize,
    kept_bytes: i64,
    limit_bytes: i64,
    status_on_files: bool,
}

enum Ask {
    /// The local sizes, the settings, Open at login and the folder tree.
    Read,
    Insights,
    Pause(bool),
    OpenAtLogin(bool),
    FreeUp,
    Keep(Vec<String>, Vec<String>),
}

/// One job: what is asked, for whose account, in which generation.
struct Job {
    ask: Ask,
    account: String,
    generation: u64,
}

enum Answer {
    OnThisPc(Option<OnThisPc>),
    OpenAtLogin(Option<bool>),
    Keep(Keep),
    Insights(Insights, Option<(i64, i64)>),
    /// A change that did not happen, in words the window says (P2-6).
    Refused(&'static str),
}

/// A worker's answer: the generation it was asked in and the account it is for (`None`: the account that asked was
/// no longer signed in with the vault open when the work ended).
struct Tagged {
    generation: u64,
    whose: Option<String>,
    answer: Answer,
}

/// Drive's settings panes on the real engine. See the module's doc.
pub struct EngineDriveSettings<E: DriveEngine> {
    engine: Arc<E>,
    platform: Arc<dyn Platform>,
    jobs: Option<mpsc::Sender<Job>>,
    pending: Arc<AtomicUsize>,
    answers: mpsc::Sender<Tagged>,
    inbox: mpsc::Receiver<Tagged>,
    on_pc: Option<OnThisPc>,
    open_at_login: Option<bool>,
    usage: Option<(i64, i64)>,
    insights: Insights,
    keep: Keep,
    /// When insights were last answered: `opened(Insights)` re-asks when this is older than [`INSIGHTS_FRESH_FOR`].
    insights_at: Option<Instant>,
    /// Whether an insights read is out: no second one is asked.
    insights_reading: bool,
    /// The (gate, account) the last pump saw: a change drops everything held and starts a new generation.
    seen: (GateState, Option<String>),
    generation: u64,
    /// What `insights()` lends while what is held is not the signed-in account's.
    locked: Insights,
    loading: Insights,
    /// A refused change: the window's toast says it once.
    toast: Option<&'static str>,
}

impl<E: DriveEngine> EngineDriveSettings<E> {
    /// The source of the running app. The first reads are asked now.
    pub fn new(engine: Arc<E>, platform: Arc<dyn Platform>) -> Self {
        let (answers, inbox) = mpsc::channel();
        let gate = engine.view().gate;
        let account = engine.account();
        let mut source = Self {
            engine,
            platform,
            jobs: None,
            pending: Arc::default(),
            answers,
            inbox,
            on_pc: None,
            open_at_login: None,
            usage: None,
            insights: Insights::Loading,
            keep: Keep::Failed,
            insights_at: None,
            insights_reading: false,
            seen: (gate, account),
            generation: 0,
            locked: Insights::Failed(INSIGHTS_LOCKED.into()),
            loading: Insights::Loading,
            toast: None,
        };
        source.send(Ask::Read);
        source.ask_insights();
        source
    }

    /// Send `ask` for the account the window sees now. Nothing is sent while no one is signed in with the vault open.
    fn send(&mut self, ask: Ask) -> bool {
        let (GateState::Ready, Some(account)) = self.seen.clone() else {
            return false;
        };
        if self.jobs.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let (engine, platform, answers, pending) = (
                self.engine.clone(),
                self.platform.clone(),
                self.answers.clone(),
                self.pending.clone(),
            );
            let started = std::thread::Builder::new()
                .name("drive-settings".into())
                .spawn(move || {
                    // Ends when the source is dropped (its sender with it).
                    for job in rx {
                        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run(&*engine, &*platform, &answers, job)
                        }));
                        if done.is_err() {
                            log("drive settings: a read or a change ended early");
                        }
                        pending.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            match started {
                Ok(_) => self.jobs = Some(tx),
                Err(_) => {
                    log("drive settings: could not start its worker");
                    return false;
                }
            }
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        let job = Job {
            ask,
            account,
            generation: self.generation,
        };
        let sent = self.jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok());
        if !sent {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        sent
    }

    /// Send a change the person asked for; one that cannot be asked (no one signed in with the vault open) is said.
    fn change(&mut self, ask: Ask) {
        if !self.mine() || !self.send(ask) {
            self.toast = Some(CHANGE_LOCKED);
        }
    }

    fn unlocked(&self) -> bool {
        self.engine.view().gate == GateState::Ready
    }

    /// Whether what the window holds may be handed out: the vault is open, and the signed-in account is the one the
    /// last `pump` saw (what is held was read for that account, or is nothing). Asked on every reader, so nothing of
    /// an account that left is drawn even before the next `pump` drops it.
    fn mine(&self) -> bool {
        if self.seen.0 != GateState::Ready || !self.unlocked() {
            return false;
        }
        let account = self.engine.account();
        account.is_some() && account == self.seen.1
    }

    fn ask_insights(&mut self) {
        if !self.insights_reading {
            self.insights_reading = self.send(Ask::Insights);
            if !self.insights_reading {
                self.insights = Insights::Failed(INSIGHTS_LOCKED.into());
            }
        }
    }
}

/// The account `job` was asked for is still the signed-in one, with the vault open.
fn still_for<E: DriveEngine>(engine: &E, account: &str) -> bool {
    engine.view().gate == GateState::Ready && engine.account().as_deref() == Some(account)
}

/// One job, on the worker. A change runs only while the account that asked is still signed in with the vault open,
/// and is followed by the read it changes. Every answer says the generation it was asked in and, checked after the
/// work, whose it is.
fn run<E: DriveEngine>(engine: &E, platform: &dyn Platform, answers: &mpsc::Sender<Tagged>, job: Job) {
    let Job {
        ask,
        account,
        generation,
    } = job;
    let send = |answer| {
        let whose = still_for(engine, &account).then(|| account.clone());
        let _ = answers.send(Tagged {
            generation,
            whose,
            answer,
        });
    };
    let refuse = |words| send(Answer::Refused(words));
    if !matches!(ask, Ask::Read | Ask::Insights) && !still_for(engine, &account) {
        // The account that asked has left, or the vault is locked: nothing is changed for whoever is here now.
        log("drive settings: a change was not made (the account that asked is not signed in)");
        return refuse(CHANGE_LOCKED);
    }
    match ask {
        Ask::Read => read_local(engine, platform, &account, &send),
        Ask::Insights => {
            let (insights, usage) = insights(engine, &account);
            send(Answer::Insights(insights, usage));
        }
        Ask::Pause(paused) => {
            if engine.set_paused(paused).is_err() {
                log("drive settings: sync was not paused or resumed (the settings file)");
                refuse(PAUSE_FAILED);
            }
        }
        Ask::OpenAtLogin(on) => {
            if platform.set_autostart(on).is_err() {
                log("drive settings: open at login was not changed");
                refuse(OPEN_AT_LOGIN_FAILED);
            }
            send(Answer::OpenAtLogin(platform.autostart_enabled().ok()));
        }
        Ask::FreeUp => {
            if engine.free_up_space().is_err() {
                log("drive settings: free up space did not finish");
                refuse(FREE_UP_FAILED);
            }
            read_local(engine, platform, &account, &send);
        }
        Ask::Keep(keep, release) => {
            if !engine.syncing() {
                log("drive settings: keep on this PC was not changed: sync does not run");
                refuse(KEEP_NOT_SYNCING);
            } else {
                let changes = keep
                    .iter()
                    .map(|id| (id, true))
                    .chain(release.iter().map(|id| (id, false)));
                // Each folder only while the account that asked is still the one signed in.
                let failed = changes
                    .filter(|(id, on)| !still_for(engine, &account) || engine.set_kept(id, *on).is_err())
                    .count();
                if failed > 0 {
                    log(format!("drive settings: {failed} folders were not changed"));
                    refuse(KEEP_FAILED);
                }
            }
            read_local(engine, platform, &account, &send);
        }
    }
}

fn read_local<E: DriveEngine>(engine: &E, platform: &dyn Platform, account: &str, send: &dyn Fn(Answer)) {
    send(Answer::OpenAtLogin(platform.autostart_enabled().ok()));
    if !still_for(engine, account) {
        // Locked, signed out or another account: no folder name stays in memory.
        send(Answer::OnThisPc(None));
        send(Answer::Keep(Keep::Failed));
        return;
    }
    let read = engine.local_state(|db| -> Result<_, String> {
        let rows = rows(db)?;
        let (limit_bytes, status_on_files) = engine.local_settings()?;
        let downloaded = db.cache_bytes_by_effective_pin(false).map_err(|_| "cache")?;
        let summary = db.kept_summary().map_err(|_| "kept")?;
        let on_pc = OnThisPc {
            downloaded,
            kept_items: summary.items as usize,
            kept_bytes: summary.bytes,
            limit_bytes,
            status_on_files,
        };
        Ok(Some((on_pc, keep_of(&rows))))
    });
    // Task 2488: a local state that is not this account's reads as nothing; so does one read across a lock or a
    // sign-out. Anything else that is not handed out could not be read.
    let read = match read {
        Ok(read) => read,
        Err(refused) if refused.not_this_accounts() || refused == LocalStateRefused::SessionChanged => Ok(None),
        Err(_) => Err("not handed out".to_string()),
    };
    match read {
        Ok(Some((on_pc, keep))) => {
            send(Answer::OnThisPc(Some(on_pc)));
            send(Answer::Keep(keep));
        }
        Ok(None) => {
            // Not this account's state (task 2488): nothing.
            send(Answer::OnThisPc(None));
            send(Answer::Keep(Keep::Failed));
        }
        Err(_) => {
            log("drive settings: the local state could not be read");
            send(Answer::Keep(Keep::Failed));
        }
    }
}

/// The live rows a page may list (never a device's manifest in Backups), each with whether it is kept.
fn rows(db: &StateDb) -> Result<Vec<(FileEntry, bool, DriveItem)>, String> {
    let backups = BackupsTree::read(db);
    let mut rows = Vec::new();
    for entry in model::live_rows(db).map_err(|_| "rows")? {
        if backups.is_hidden(&entry.path) {
            continue;
        }
        let contract = db.get_file_contract_state(&entry.file_id).map_err(|_| "contract")?;
        let kept = contract
            .as_ref()
            .is_some_and(|c| c.effective_pin_state() == PinState::Pinned);
        let item = model::item_from(&entry, contract.as_ref(), None);
        rows.push((entry, kept, item));
    }
    Ok(rows)
}

/// The files among `rows` (no folders).
fn files_of(rows: &[(FileEntry, bool, DriveItem)]) -> Vec<&FileEntry> {
    rows.iter().map(|(e, _, _)| e).filter(|e| !e.is_dir()).collect()
}

/// Storage insights: the account's use from Beebeeb, the rest from this PC's file list.
fn insights<E: DriveEngine>(engine: &E, account: &str) -> (Insights, Option<(i64, i64)>) {
    if !still_for(engine, account) {
        return (Insights::Failed(INSIGHTS_LOCKED.into()), None);
    }
    let usage = match engine.storage_info() {
        Ok(usage) => usage,
        Err(kind) => {
            let words = if kind == "offline" {
                INSIGHTS_OFFLINE
            } else {
                INSIGHTS_FAILED
            };
            return (Insights::Failed(words.into()), None);
        }
    };
    // Task 2488: another account's local state reads as nothing for insights.
    let read = engine.local_state(|db| rows(db).map(|rows| insights_of(&files_of(&rows), usage)));
    match read {
        Ok(Ok(insights)) => (insights, Some(usage)),
        // Not this account's state: show usage only, no breakdown.
        Err(refused) if refused.not_this_accounts() => (insights_of(&[], usage), Some(usage)),
        Ok(Err(_)) | Err(_) => {
            log("drive settings: the file list could not be read for insights");
            (Insights::Failed(INSIGHTS_FAILED.into()), Some(usage))
        }
    }
}

/// What a file counts as on the page.
fn kind_name(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp" | "raw" | "dng" | "cr2"
        | "cr3" | "nef" | "arw" | "raf" | "orf" | "braw" | "mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm" | "wmv"
        | "mts" | "mp3" | "m4a" | "wav" | "flac" | "aac" | "ogg" | "opus" | "aiff" => "Media",
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "pages" | "xls" | "xlsx" | "xlsm" | "csv" | "ods"
        | "numbers" | "ppt" | "pptx" | "key" | "odp" | "epub" => "Documents",
        _ => "Other",
    }
}

/// The page for these files and this use. No files and nothing used: the empty state.
pub(crate) fn insights_of(files: &[&FileEntry], (used, quota): (i64, i64)) -> Insights {
    if files.is_empty() && used == 0 {
        return Insights::Empty;
    }
    let mut kinds: Vec<(&'static str, i64, usize)> = Vec::new();
    for file in files {
        let name = kind_name(&file.path);
        match kinds.iter_mut().find(|(k, _, _)| *k == name) {
            Some(kind) => {
                kind.1 += file.size_bytes.max(0);
                kind.2 += 1;
            }
            None => kinds.push((name, file.size_bytes.max(0), 1)),
        }
    }
    kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let largest_kind = kinds.first().map_or(0, |k| k.1).max(1) as f32;
    let mut largest: Vec<&FileEntry> = files.to_vec();
    largest.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.path.cmp(&b.path)));
    largest.truncate(LARGEST_TOP);
    let more = (largest.len() > LARGEST_SHOWN).then(|| {
        (
            format!("And {} more, down to ", largest.len() - LARGEST_SHOWN),
            size_words(largest.last().map_or(0, |f| f.size_bytes)),
        )
    });
    let total_bytes: i64 = files.iter().map(|f| f.size_bytes.max(0)).sum();
    let share = if quota > 0 {
        (used as f64 / quota as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    Insights::Ready(Box::new(InsightsData {
        used: size_words(used),
        total: size_words(quota),
        plan: format!("{} plan", size_words(quota)),
        share,
        used_says: format!("{}% used", (share * 100.0).round() as u32),
        free_says: format!("{} free", size_words((quota - used).max(0))),
        totals: format!(
            "{} · {}",
            count_words(files.len(), "file", "files"),
            size_words(total_bytes)
        ),
        kinds: kinds
            .iter()
            .map(|(name, bytes, n)| KindShare {
                name: (*name).to_string(),
                says: format!("{} · {}", size_words(*bytes), count_words(*n, "file", "files")),
                share: *bytes as f32 / largest_kind,
            })
            .collect(),
        largest_says: format!("top {}", largest.len()),
        largest: largest
            .iter()
            .take(LARGEST_SHOWN)
            .map(|f| {
                let path = f.path.trim_matches('/').to_string();
                LargeFile {
                    shown: cut_middle(&path),
                    kind: kind_name(&path).to_string(),
                    size: size_words(f.size_bytes),
                    path,
                }
            })
            .collect(),
        more,
    }))
}

/// A path cut in the middle so the file name stays whole ("Backups/Anna's…ook Pro/Documents/Archive 2024.zip").
pub(crate) fn cut_middle(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= PATH_SHOWN {
        return path.to_string();
    }
    let name = path.rsplit('/').next().unwrap_or(path).chars().count();
    let head = 14.min(PATH_SHOWN.saturating_sub(name + 1));
    let tail = (PATH_SHOWN - head - 1).max(name);
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail.min(chars.len())..].iter().collect();
    format!("{start}\u{2026}{end}")
}

/// "240 GB", "3.1 GB", "940 MB": decimal units, one decimal where it says something.
pub(crate) fn size_words(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    let (value, unit) = [(1e12, "TB"), (1e9, "GB"), (1e6, "MB"), (1e3, "KB")]
        .into_iter()
        .find(|(size, _)| bytes >= *size)
        .map_or((bytes, "bytes"), |(size, unit)| (bytes / size, unit));
    if unit == "bytes" {
        return format!("{} bytes", value as u64);
    }
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 || rounded >= 1000.0 {
        format!("{} {unit}", rounded.round() as u64)
    } else {
        format!("{rounded:.1} {unit}")
    }
}

/// "1 file", "3,140 files".
pub(crate) fn count_words(n: usize, one: &str, many: &str) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped} {}", if n == 1 { one } else { many })
}

/// Keep on this PC's folder tree from the live rows: by path (a live row's place is its path), each folder with
/// what it holds, its size and whether it is kept.
pub(crate) fn keep_of(rows: &[(FileEntry, bool, DriveItem)]) -> Keep {
    let trim = |p: &str| p.trim_matches('/').to_string();
    let parent = |p: &str| p.rsplit_once('/').map(|(head, _)| head.to_string()).unwrap_or_default();
    let mut bytes: HashMap<String, i64> = HashMap::new();
    let mut holds: HashMap<String, usize> = HashMap::new();
    let mut total: i64 = 0;
    for (entry, _, _) in rows {
        let path = trim(&entry.path);
        *holds.entry(parent(&path)).or_default() += 1;
        if entry.is_dir() {
            continue;
        }
        let size = entry.size_bytes.max(0);
        total += size;
        let mut up = parent(&path);
        while !up.is_empty() {
            *bytes.entry(up.clone()).or_default() += size;
            up = parent(&up);
        }
    }
    let folders: Vec<(String, &FileEntry, bool, &DriveItem)> = rows
        .iter()
        .filter(|(e, _, _)| e.is_dir())
        .map(|(e, kept, item)| (trim(&e.path), e, *kept, item))
        .collect();
    fn level(
        at: &str,
        folders: &[(String, &FileEntry, bool, &DriveItem)],
        bytes: &HashMap<String, i64>,
        holds: &HashMap<String, usize>,
        parent: &dyn Fn(&str) -> String,
    ) -> Vec<Folder> {
        let mut out: Vec<Folder> = folders
            .iter()
            .filter(|(path, _, _, _)| parent(path) == at)
            .map(|(path, entry, kept, item)| {
                let children = level(path, folders, bytes, holds, parent);
                Folder {
                    id: entry.file_id.clone(),
                    name: item.name.clone(),
                    count: count_words(holds.get(path).copied().unwrap_or(0), "item", "items"),
                    mb: bytes.get(path).copied().unwrap_or(0) as f32 / 1e6,
                    kept: *kept,
                    expandable: !children.is_empty(),
                    open: false,
                    children,
                }
            })
            .collect();
        out.sort_by_key(|f| f.name.to_lowercase());
        out
    }
    Keep::Ready {
        folders: level("", &folders, &bytes, &holds, &parent),
        total_mb: total as f32 / 1e6,
    }
}

impl<E: DriveEngine> DriveSettings for EngineDriveSettings<E> {
    /// Storage insights only: every other pane keeps the shell's frame (the module's doc says why).
    fn wired(&self, pane: Pane) -> bool {
        matches!(pane, Pane::Insights)
    }

    /// The gate or the account changed: what the window held goes now, a new generation starts, and an open vault
    /// is read again. Only then the worker's answers: one counts when it was asked in this generation and is for
    /// the account the window sees.
    fn pump(&mut self, _seconds: f64) -> (bool, bool) {
        let mut changed = false;
        let now = (self.engine.view().gate, self.engine.account());
        if now != self.seen {
            self.seen = now;
            self.generation += 1;
            self.on_pc = None;
            self.open_at_login = None;
            self.usage = None;
            self.keep = Keep::Failed;
            self.insights_at = None;
            self.insights_reading = false;
            self.insights = if self.seen.0 == GateState::Ready {
                Insights::Loading
            } else {
                Insights::Failed(INSIGHTS_LOCKED.into())
            };
            self.send(Ask::Read);
            self.ask_insights();
            changed = true;
        }
        while let Ok(Tagged {
            generation,
            whose,
            answer,
        }) = self.inbox.try_recv()
        {
            if generation != self.generation {
                // Asked before the gate or the account changed: dropped, never drawn.
                continue;
            }
            changed = true;
            let mine = self.seen.0 == GateState::Ready && whose.is_some() && whose == self.seen.1;
            match answer {
                Answer::Refused(words) => self.toast = Some(words),
                Answer::Insights(insights, usage) => {
                    self.insights_reading = false;
                    if mine {
                        self.insights_at = Some(Instant::now());
                        self.insights = insights;
                        self.usage = usage;
                    } else {
                        // The account left while it was worked out: nothing of it is kept; Try again asks anew.
                        self.insights = Insights::Failed(INSIGHTS_FAILED.into());
                    }
                }
                _ if !mine => {}
                Answer::OnThisPc(on_pc) => self.on_pc = on_pc,
                Answer::OpenAtLogin(on) => self.open_at_login = on,
                Answer::Keep(keep) => self.keep = keep,
            }
        }
        (changed, !self.idle())
    }

    /// A pane of Drive's settings opened: insights reads again when what it holds is old (P2-3).
    fn opened(&mut self, pane: Pane) {
        if pane == Pane::Insights {
            let fresh = self.insights_at.is_some_and(|at| at.elapsed() < INSIGHTS_FRESH_FOR) && self.mine();
            if !fresh {
                self.ask_insights();
            }
        }
    }

    /// A refused change: the window's toast says it once (P2-6).
    fn take_toast(&mut self) -> Option<&'static str> {
        self.toast.take()
    }

    fn storage(&self) -> Storage {
        let mine = self.mine();
        let on_pc = self.on_pc.filter(|_| mine);
        let usage = self.usage.filter(|_| mine);
        let limit_gb = on_pc.map_or(0, |p| {
            u32::try_from(p.limit_bytes.max(0) / 1_000_000_000).unwrap_or(u32::MAX)
        });
        Storage {
            used: usage.map(|(used, _)| size_words(used)).unwrap_or_default(),
            total: usage.map(|(_, quota)| size_words(quota)).unwrap_or_default(),
            shares: [
                usage
                    .filter(|(_, quota)| *quota > 0)
                    .map_or(0.0, |(used, quota)| (used as f64 / quota as f64).clamp(0.0, 1.0) as f32),
                0.0,
            ],
            kinds: Vec::new(),
            trash: String::new(),
            downloaded: KeptOnPc {
                used: on_pc.map(|p| size_words(p.downloaded)).unwrap_or_default(),
                limit: Limit {
                    gb: limit_gb,
                    min_gb: 1,
                    max_gb: limit_gb.max(1),
                    at: 1.0,
                    says: String::new(),
                },
            },
            previews: KeptOnPc {
                used: String::new(),
                limit: Limit {
                    gb: 0,
                    min_gb: 0,
                    max_gb: 0,
                    at: 0.0,
                    says: String::new(),
                },
            },
            offline: on_pc.map(|p| size_words(p.kept_bytes)).unwrap_or_default(),
        }
    }

    /// Not bound: the limit cannot say "no limit", the engine's default (PR body).
    fn set_limit(&mut self, _which: Kept, _gb: u32) {}

    /// Downloaded copies: the engine's Free up space (never a kept file, never an unsent change). Previews: no engine
    /// call.
    fn clear(&mut self, which: Kept) {
        if which == Kept::Downloaded {
            self.change(Ask::FreeUp);
        }
    }

    fn sync(&self) -> SyncPane {
        let view = self.engine.view();
        let mine = self.mine();
        let on_pc = self.on_pc.filter(|_| mine);
        let says = super::sync::sync_word(&view).unwrap_or("").to_string();
        SyncPane {
            in_explorer: view.explorer == Explorer::Added,
            status_on_files: on_pc.is_some_and(|p| p.status_on_files),
            repair: Repair::Idle,
            status: if view.paused {
                SyncStatus::Paused
            } else {
                SyncStatus::Running(says)
            },
            open_at_login: self.open_at_login.filter(|_| mine).unwrap_or(false),
            downloaded: on_pc.map(|p| size_words(p.downloaded)).unwrap_or_default(),
            limit_gb: on_pc.map_or(0, |p| {
                u32::try_from(p.limit_bytes.max(0) / 1_000_000_000).unwrap_or(u32::MAX)
            }),
            offline: on_pc
                .map(|p| {
                    format!(
                        "{} · {}",
                        count_words(p.kept_items, "item", "items"),
                        size_words(p.kept_bytes)
                    )
                })
                .unwrap_or_default(),
            not_uploaded: None,
        }
    }

    /// Not bound: the engine stores the setting and nothing honours it yet (PR body).
    fn set_status_on_files(&mut self, _on: bool) {}

    fn set_open_at_login(&mut self, on: bool) {
        self.change(Ask::OpenAtLogin(on));
    }

    /// The handle's view says paused at once when the settings file took it.
    fn set_paused(&mut self, paused: bool) {
        self.change(Ask::Pause(paused));
    }

    /// Not bound: no metered state is read here, so `sync()` never says sync waits on one.
    fn sync_anyway(&mut self) {}

    /// Not bound: the repair lives in the Tauri app only (PR body).
    fn repair(&mut self) {}

    /// Not bound: the turn-off lives in the Tauri app only (PR body).
    fn turn_off_explorer(&mut self) {}

    /// What is held, only while it is the signed-in account's (P2-5): a locked vault lends the locked sentence, an
    /// account the last `pump` has not seen yet lends "loading", whatever is still stored.
    fn insights(&self) -> &Insights {
        if self.mine() {
            &self.insights
        } else if self.unlocked() {
            &self.loading
        } else {
            &self.locked
        }
    }

    fn reload_insights(&mut self) {
        self.insights = Insights::Loading;
        self.ask_insights();
    }

    fn keep(&self) -> Keep {
        if self.mine() { self.keep.clone() } else { Keep::Failed }
    }

    fn reload_keep(&mut self) {
        self.send(Ask::Read);
    }

    /// Only the folders named: each newly kept or newly released folder alone, through the pin change. Refused
    /// while sync does not run (the tree then still shows the old choice).
    fn apply_keep(&mut self, keep: &[String], release: &[String]) {
        if keep.is_empty() && release.is_empty() {
            return;
        }
        self.change(Ask::Keep(keep.to_vec(), release.to_vec()));
    }
}

impl<E: DriveEngine> EngineDriveSettings<E> {
    /// No job is waiting or running.
    pub(crate) fn idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }
}
