//! The updater as one state and one `step`: check, download (resumable), verify, stage, hand to the installer.
//!
//! The shell owns the loop. It sends an [`Event`], reads the [`State`] back and draws it; a step that has work left
//! (`Checking`, `Downloading`, `Verifying`) is continued with [`Event::Tick`]. Every `Tick` does one bounded piece of
//! blocking work (one request, or one chunk), so the shell runs the loop on a worker thread and can show progress.
//! The network, the clock, the persisted marks and the installer launch are traits, so every rule is tested without
//! any of them. Nothing is installed without [`Event::Apply`], which only a person's click sends, and the installer
//! is started only with an [`EngineStopped`] in hand: the machine asks [`Apply::stop_engine`] for it first.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::{Channel, MAX_MANIFEST_BYTES};
use crate::policy::{self, HighWater, Offer, Refusal, Trust, Verdict};
use crate::signature::Signature;
use crate::version::Version;

/// Bytes read from the network per `Tick`.
pub const CHUNK_BYTES: usize = 64 * 1024;
/// Attempts in a row that add no byte before a download stops and waits for the person.
pub const MAX_INTERRUPTIONS: u32 = 3;
/// A background check is due this long after the last successful one (0.8.x checks every 4 hours).
pub const CHECK_INTERVAL_SECS: u64 = 4 * 60 * 60;
/// After a check that failed, the next background check waits this long: a release host in trouble is not asked
/// again on every timer tick by every install.
pub const RETRY_AFTER_FAILURE_SECS: u64 = 15 * 60;
/// With no verified check for this long the UI says so ([`Updater::freshness`]): the release server may be out of
/// reach, or something between it and this PC is holding updates back.
pub const UNVERIFIED_WARNING_SECS: u64 = 14 * 24 * 60 * 60;

/// A response to one GET. `offset` is where `body` starts in the whole file (0 for a 200, the range start for a
/// 206) and `total` is the whole file's length when the server states it.
pub struct Response {
    pub status: u16,
    pub offset: u64,
    pub total: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

/// The request never produced a response (no route, refused, timed out, TLS failed). No detail: reqwest's text can
/// hold an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreachable;

/// The network. An implementation speaks https only and never follows a redirect.
pub trait Http {
    /// GETs `url`; `from > 0` asks for the bytes from that offset on (`Range: bytes=<from>-`).
    fn get(&mut self, url: &str, from: u64) -> Result<Response, Unreachable>;
}

/// Unix seconds.
pub trait Clock {
    fn now(&self) -> u64;
}

/// What the updater remembers between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persisted {
    /// Per channel, the newest manifest accepted (replay protection).
    #[serde(default)]
    pub seen: BTreeMap<Channel, HighWater>,
    /// When the last check was answered by a manifest that passed every rule.
    #[serde(default)]
    pub last_check: Option<u64>,
}

/// Where [`Persisted`] lives. `save` is durable before it returns `true`.
pub trait Store {
    fn load(&self) -> Persisted;
    fn save(&mut self, persisted: &Persisted) -> bool;
}

/// Proof that the sync engine is stopped and every upload in progress is parked (resumable after the restart).
/// [`Apply::launch`] cannot be called without one, and the machine gets it only from [`Apply::stop_engine`], so
/// the installer is never started over a running engine.
#[derive(Debug)]
pub struct EngineStopped {
    _only_from_the_engine_stop_path: (),
}

impl EngineStopped {
    /// For the engine-stop path alone: call it after the engine has confirmed that it stopped and parked its
    /// uploads, inside [`Apply::stop_engine`]. Nothing else may mint one.
    pub fn engine_stopped_and_uploads_parked() -> Self {
        Self {
            _only_from_the_engine_stop_path: (),
        }
    }
}

/// The staged installer, verified, and held so that nothing can write, replace, rename or delete it while this
/// value lives. Only the machine makes one, at `Event::Apply`, after the file passed its length and signature
/// through this very handle: there is no way to start an installer the machine has not just verified.
pub struct VerifiedInstaller {
    path: PathBuf,
    _held: fs::File,
}

impl VerifiedInstaller {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Starts the installer the way Tauri's updater does for 0.8.x (`/P` passive: progress only; `/R` start the
    /// app afterwards; `/UPDATE` keep shortcuts and the autostart entry). A per-user install: no elevation. The
    /// installer waits for this process to exit; it never ends it.
    #[cfg(windows)]
    pub fn start_nsis(&self, _stopped: EngineStopped) -> bool {
        std::process::Command::new(&self.path)
            .args(["/P", "/R", "/UPDATE"])
            .spawn()
            .is_ok()
    }
}

/// The two things only the shell can do at "Restart to update", in the order the machine calls them.
pub trait Apply {
    /// Stops the sync engine and parks uploads in progress, then returns the proof. `None` when the engine did not
    /// stop in time: nothing is started, the staged update is kept, and the person can try again.
    fn stop_engine(&mut self) -> Option<EngineStopped>;

    /// Starts the verified installer (Windows: `installer.start_nsis(stopped)`). On `true` the shell exits the
    /// process; the installer waits for that, replaces the files and starts the new version.
    fn launch(&mut self, installer: &VerifiedInstaller, offer: &Offer, stopped: EngineStopped) -> bool;

    /// The installer did not start after the engine was stopped: start the engine again.
    fn resume_engine(&mut self);
}

/// Why a check was asked for: a background check holds an older channel build back, a manual one reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Background,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Check {
        channel: Channel,
        intent: Intent,
    },
    /// Start or resume the download of the offered upgrade.
    Download,
    /// Start the download of an offered older version; the version is the one the person was shown.
    DownloadDowngrade(Version),
    /// Continue the work of `Checking`, `Downloading` or `Verifying`.
    Tick,
    /// "Restart to update": start the staged installer.
    Apply,
    Dismiss,
}

/// What the UI shows. Plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Idle,
    Checking {
        channel: Channel,
        intent: Intent,
    },
    /// A background check while the person has an offer, a resumable download or a staged update in front of them:
    /// the UI keeps drawing `kept`. One `Tick` later it is `kept` again when the channel still offers exactly that,
    /// and otherwise what the channel says now (an offer that was withdrawn or replaced does not stay installable).
    Rechecking {
        channel: Channel,
        kept: Box<State>,
    },
    UpToDate {
        installed: Version,
        channel: Channel,
    },
    Available {
        offer: Offer,
    },
    Downloading {
        offer: Offer,
        received: u64,
        total: u64,
    },
    Verifying {
        offer: Offer,
    },
    ReadyToRestart {
        offer: Offer,
        installer: PathBuf,
    },
    /// The installer was started; the shell exits now.
    Restarting {
        offer: Offer,
    },
    /// `resume` is the offer a [`Event::Download`] continues, when the bytes on disk are worth keeping.
    Failed {
        failure: Failure,
        resume: Option<Offer>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// No answer from the release server.
    Offline,
    /// The release server answered with this status instead of the file.
    ServerStatus(u16),
    /// The manifest broke a rule; nothing was downloaded.
    Refused(Refusal),
    /// The server's stated length, or the staged file's, is not the manifest's.
    WrongSize,
    /// The downloaded file is not the one that was signed. It was deleted.
    BadSignature,
    /// The connection kept dropping without progress. The bytes so far are kept.
    DownloadInterrupted {
        received: u64,
        total: u64,
    },
    /// The older version the person confirmed is no longer the one on offer.
    DowngradeChanged,
    /// The update folder or the updater's own record could not be written.
    Disk,
    /// The engine did not stop, so the installer was not started. The staged update is kept.
    EngineStillRunning,
    InstallerDidNotStart,
}

/// Whether this install has recently heard from the release server in a way it could verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// A check passed every rule within [`UNVERIFIED_WARNING_SECS`] (or the build is that young).
    Verified,
    /// No check has passed every rule for this many whole days, counted from the last one that did, or from when
    /// this build was made if none ever did. The UI says so; "up to date" alone would be a guess.
    Unverified { days: u64 },
}

pub struct Config {
    pub trust: Trust,
    /// `https://releases.beebeeb.io`: the manifests are `<base>/desktop/v2/<channel>.json` and `….json.sig`.
    pub base_url: String,
    pub installed: Version,
    /// A folder only the updater writes (`%LOCALAPPDATA%\io.beebeeb.app\updates`).
    pub staging_dir: PathBuf,
}

pub struct Updater<H, C, S, A> {
    pub(crate) config: Config,
    pub(crate) http: H,
    pub(crate) clock: C,
    pub(crate) store: S,
    pub(crate) apply: A,
    persisted: Persisted,
    state: State,
    /// The open download and the part file it is written to. Not part of `State`: the UI's copy stays plain data.
    body: Option<Box<dyn Read + Send>>,
    part: Option<fs::File>,
    interruptions: u32,
    /// When the last check started, in this run only (a restart may check at once).
    last_attempt: Option<u64>,
}

impl<H: Http, C: Clock, S: Store, A: Apply> Updater<H, C, S, A> {
    pub fn new(config: Config, http: H, clock: C, store: S, apply: A) -> Self {
        let persisted = store.load();
        Self {
            config,
            http,
            clock,
            store,
            apply,
            persisted,
            state: State::Idle,
            body: None,
            part: None,
            interruptions: 0,
            last_attempt: None,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    #[cfg(test)]
    pub(crate) fn set_state_for_test(&mut self, state: State) {
        self.state = state;
    }

    /// True when no check has passed in the last [`CHECK_INTERVAL_SECS`] and none was tried in the last
    /// [`RETRY_AFTER_FAILURE_SECS`]. A clock that went backwards never silences checks.
    pub fn background_check_due(&self) -> bool {
        let now = self.clock.now();
        let at_least = |since: Option<u64>, wait: u64| {
            since.is_none_or(|then| now.checked_sub(then).is_none_or(|elapsed| elapsed >= wait))
        };
        at_least(self.persisted.last_check, CHECK_INTERVAL_SECS)
            && at_least(self.last_attempt, RETRY_AFTER_FAILURE_SECS)
    }

    /// For the UI's "last checked" line and its warning. A clock set back never raises the warning.
    pub fn freshness(&self) -> Freshness {
        let since = self.persisted.last_check.unwrap_or(0).max(self.config.trust.not_before);
        let unverified = self.clock.now().saturating_sub(since);
        if unverified >= UNVERIFIED_WARNING_SECS {
            Freshness::Unverified {
                days: unverified / (24 * 60 * 60),
            }
        } else {
            Freshness::Verified
        }
    }

    /// Applies one event. An event that means nothing in the current state changes nothing.
    pub fn step(&mut self, event: Event) -> &State {
        let state = std::mem::replace(&mut self.state, State::Idle);
        self.state = match (state, event) {
            // A check never interrupts work in progress.
            (
                busy @ (State::Checking { .. }
                | State::Rechecking { .. }
                | State::Downloading { .. }
                | State::Verifying { .. }
                | State::Restarting { .. }),
                Event::Check { .. },
            ) => busy,
            // A background check does not blank what the person is looking at (an offer, a download that can be
            // resumed, a staged update waiting for "Restart to update"), and it does not keep it for ever either:
            // it is checked against the channel again. A manual check is the person's own.
            (
                kept @ (State::Available { .. } | State::ReadyToRestart { .. } | State::Failed { resume: Some(_), .. }),
                Event::Check {
                    channel,
                    intent: Intent::Background,
                },
            ) => State::Rechecking {
                channel,
                kept: Box::new(kept),
            },
            (State::Rechecking { channel, kept }, Event::Tick) => self.recheck(channel, *kept),
            (_, Event::Check { channel, intent }) => State::Checking { channel, intent },
            (State::Checking { channel, intent }, Event::Tick) => self.run_check(channel, intent),
            (State::Available { offer }, Event::Download) if !offer.downgrade => self.begin_download(offer),
            (State::Available { offer }, Event::DownloadDowngrade(version)) if offer.downgrade => {
                if version == offer.version {
                    self.begin_download(offer)
                } else {
                    State::Failed {
                        failure: Failure::DowngradeChanged,
                        resume: None,
                    }
                }
            }
            (
                State::Failed {
                    resume: Some(offer), ..
                },
                Event::Download,
            ) => self.begin_download(offer),
            (State::Downloading { offer, received, .. }, Event::Tick) => self.download_tick(offer, received),
            (State::Verifying { offer }, Event::Tick) => self.verify(offer),
            (State::ReadyToRestart { offer, installer }, Event::Apply) => self.apply(offer, installer),
            (
                State::UpToDate { .. } | State::Available { .. } | State::Failed { .. } | State::ReadyToRestart { .. },
                Event::Dismiss,
            ) => State::Idle,
            (state, _) => state,
        };
        &self.state
    }

    fn manifest_url(&self, channel: Channel) -> String {
        format!(
            "{}/desktop/v2/{}",
            self.config.base_url.trim_end_matches('/'),
            channel.manifest_file()
        )
    }

    /// What a background check makes of an offer that was already on screen. No answer, or a refused one, is not
    /// news about the offer: it stays (and `Apply` verifies the file again in any case).
    fn recheck(&mut self, channel: Channel, kept: State) -> State {
        let on_screen = match &kept {
            State::Available { offer }
            | State::ReadyToRestart { offer, .. }
            | State::Failed {
                resume: Some(offer), ..
            } => offer.clone(),
            _ => return kept,
        };
        // Asked as a manual check would, to see what the channel offers, older builds included.
        match self.run_check(channel, Intent::Manual) {
            State::Failed { .. } => kept,
            State::Available { offer } if offer == on_screen => kept,
            // An older channel build that the person was not already looking at: a background check holds it.
            State::Available { offer } if offer.downgrade => State::UpToDate {
                installed: self.config.installed,
                channel,
            },
            now => now,
        }
    }

    fn run_check(&mut self, channel: Channel, intent: Intent) -> State {
        let refused = |refusal| State::Failed {
            failure: Failure::Refused(refusal),
            resume: None,
        };
        self.last_attempt = Some(self.clock.now());
        let url = self.manifest_url(channel);
        if policy::trusted_file_name(&url, &self.config.trust.allowed_hosts).is_none() {
            return refused(Refusal::UntrustedAddress);
        }
        let manifest = match self.fetch_small(&url) {
            Ok(bytes) => bytes,
            Err(failure) => return State::Failed { failure, resume: None },
        };
        let signature = match self.fetch_small(&format!("{url}.sig")) {
            Ok(bytes) => bytes,
            Err(failure) => return State::Failed { failure, resume: None },
        };
        let Ok(signature) = String::from_utf8(signature) else {
            return refused(Refusal::ManifestSignature);
        };
        let seen = self.persisted.seen.get(&channel).copied();
        let checked = match policy::check(
            &self.config.trust,
            channel,
            self.config.installed,
            seen,
            self.clock.now(),
            &manifest,
            &signature,
        ) {
            Ok(checked) => checked,
            Err(refusal) => return refused(refusal),
        };

        // Recorded on disk first: a check that could not be recorded did not happen, in memory either. The mark
        // only ever moves forward: the larger of this manifest's time and the mark it was checked against (a mark
        // the policy did not believe, beyond the clock tolerance, is the one thing that is replaced).
        let believable = self.clock.now().saturating_add(policy::MAX_CLOCK_SKEW_SECS);
        let kept = seen.map_or(0, |seen| seen.manifest_time);
        let mark = if kept <= believable {
            checked.manifest_time.max(kept)
        } else {
            checked.manifest_time
        };
        let mut recorded = self.persisted.clone();
        recorded.seen.insert(channel, HighWater { manifest_time: mark });
        recorded.last_check = Some(self.clock.now());
        if !self.store.save(&recorded) {
            return State::Failed {
                failure: Failure::Disk,
                resume: None,
            };
        }
        self.persisted = recorded;
        let up_to_date = State::UpToDate {
            installed: self.config.installed,
            channel,
        };
        match checked.verdict {
            Verdict::UpToDate => {
                self.clear_staging(None);
                up_to_date
            }
            Verdict::Upgrade(offer) => State::Available { offer },
            Verdict::Downgrade(offer) if intent == Intent::Manual => State::Available { offer },
            // A background check holds the installed build until the channel catches up (as 0.8.x does).
            Verdict::Downgrade(_) => up_to_date,
        }
    }

    /// A manifest or its signature: a 200 of at most [`MAX_MANIFEST_BYTES`].
    fn fetch_small(&mut self, url: &str) -> Result<Vec<u8>, Failure> {
        let response = self.http.get(url, 0).map_err(|Unreachable| Failure::Offline)?;
        if response.status != 200 {
            return Err(Failure::ServerStatus(response.status));
        }
        let mut bytes = Vec::new();
        let limit = MAX_MANIFEST_BYTES as u64 + 1;
        response
            .body
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(|_| Failure::Offline)?;
        if bytes.len() as u64 == limit {
            return Err(Failure::Refused(Refusal::ManifestUnreadable));
        }
        Ok(bytes)
    }

    fn part_path(&self, offer: &Offer) -> PathBuf {
        self.config.staging_dir.join(format!("{}.part", offer.file_name))
    }

    fn final_path(&self, offer: &Offer) -> PathBuf {
        self.config.staging_dir.join(&offer.file_name)
    }

    /// Removes everything in the staging folder except `keep`'s two files. Best effort: a leftover costs disk only,
    /// and nothing in the folder is trusted before it verifies.
    fn clear_staging(&self, keep: Option<&Offer>) {
        let kept = keep.map(|offer| [self.part_path(offer), self.final_path(offer)]);
        let Ok(entries) = fs::read_dir(&self.config.staging_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !kept.as_ref().is_some_and(|kept| kept.contains(&path)) {
                let _ = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
            }
        }
    }

    fn begin_download(&mut self, offer: Offer) -> State {
        self.close_download();
        self.interruptions = 0;
        if fs::create_dir_all(&self.config.staging_dir).is_err() {
            return State::Failed {
                failure: Failure::Disk,
                resume: Some(offer),
            };
        }
        self.clear_staging(Some(&offer));
        let (part, staged) = (self.part_path(&offer), self.final_path(&offer));
        // A file staged by an earlier run is verified again like any download.
        if staged.exists() && fs::rename(&staged, &part).is_err() {
            return State::Failed {
                failure: Failure::Disk,
                resume: Some(offer),
            };
        }
        let mut received = fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
        if received > offer.size {
            let _ = fs::remove_file(&part);
            received = 0;
        }
        let total = offer.size;
        State::Downloading { offer, received, total }
    }

    fn download_tick(&mut self, offer: Offer, mut received: u64) -> State {
        let total = offer.size;
        if received == total {
            self.close_download();
            return State::Verifying { offer };
        }
        let part = self.part_path(&offer);
        if self.body.is_none() || self.part.is_none() {
            self.close_download();
            let response = match self.http.get(&offer.url, received) {
                Ok(response) => response,
                Err(Unreachable) => return self.interrupted(offer, received),
            };
            match (response.status, response.offset) {
                // The server sent the whole file (it may ignore a range): start over.
                (200, 0) => {
                    if received > 0 && fs::remove_file(&part).is_err() {
                        return State::Failed {
                            failure: Failure::Disk,
                            resume: Some(offer),
                        };
                    }
                    received = 0;
                }
                (206, offset) if offset == received && received > 0 => {}
                (status, _) => {
                    return State::Failed {
                        failure: Failure::ServerStatus(status),
                        resume: Some(offer),
                    };
                }
            }
            if response.total.is_some_and(|stated| stated != total) {
                let _ = fs::remove_file(&part);
                return State::Failed {
                    failure: Failure::WrongSize,
                    resume: None,
                };
            }
            let Ok(file) = fs::OpenOptions::new().create(true).append(true).open(&part) else {
                return State::Failed {
                    failure: Failure::Disk,
                    resume: Some(offer),
                };
            };
            (self.body, self.part) = (Some(response.body), Some(file));
        }

        // Never more than the manifest's length: what a server sends beyond it is not read.
        let want = (total - received).min(CHUNK_BYTES as u64) as usize;
        let mut chunk = vec![0u8; want];
        let read = match self.body.as_mut().expect("the body was opened above").read(&mut chunk) {
            Ok(0) | Err(_) => return self.interrupted(offer, received),
            Ok(read) => read,
        };
        let appended = self
            .part
            .as_mut()
            .expect("the part file was opened above")
            .write_all(&chunk[..read]);
        if appended.is_err() {
            self.close_download();
            return State::Failed {
                failure: Failure::Disk,
                resume: Some(offer),
            };
        }
        received += read as u64;
        self.interruptions = 0;
        if received == total {
            self.close_download();
            return State::Verifying { offer };
        }
        State::Downloading { offer, received, total }
    }

    /// Drops the connection and closes the part file (flushed: `File` writes are unbuffered).
    fn close_download(&mut self) {
        (self.body, self.part) = (None, None);
    }

    /// The connection ended before the file did. The next `Tick` asks for the rest; after [`MAX_INTERRUPTIONS`] in
    /// a row without a new byte the download stops, keeping what it has for a later [`Event::Download`].
    fn interrupted(&mut self, offer: Offer, received: u64) -> State {
        self.close_download();
        self.interruptions += 1;
        let total = offer.size;
        if self.interruptions >= MAX_INTERRUPTIONS {
            return State::Failed {
                failure: Failure::DownloadInterrupted { received, total },
                resume: Some(offer),
            };
        }
        State::Downloading { offer, received, total }
    }

    /// The whole staged file against the offer's length and signature. `Ok` only for exactly the signed bytes.
    fn staged_file_is_signed(&self, file: &fs::File, offer: &Offer) -> Result<(), Failure> {
        if file.metadata().map_err(|_| Failure::Disk)?.len() != offer.size {
            return Err(Failure::WrongSize);
        }
        let signature = Signature::from_base64(&offer.signature).map_err(|_| Failure::BadSignature)?;
        if signature.file != offer.file_name {
            return Err(Failure::BadSignature);
        }
        let key = self
            .config
            .trust
            .signer_for(&signature, offer.version)
            .ok_or(Failure::BadSignature)?;
        signature.verify_reader(key, file).map_err(|_| Failure::BadSignature)
    }

    /// Removes everything in the staging folder but the staged installer, and says whether it is then alone
    /// there. An installer loads from its own folder first: a stranger's file beside it must not be there.
    fn staged_installer_is_alone(&self, installer: &Path) -> bool {
        let Ok(entries) = fs::read_dir(&self.config.staging_dir) else {
            return false;
        };
        let mut alone = true;
        for entry in entries.flatten() {
            let path = entry.path();
            if path != installer {
                let removed = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                alone &= removed.is_ok();
            }
        }
        alone
    }

    fn verify(&mut self, offer: Offer) -> State {
        let (part, staged) = (self.part_path(&offer), self.final_path(&offer));
        let checked = fs::File::open(&part)
            .map_err(|_| Failure::Disk)
            .and_then(|file| self.staged_file_is_signed(&file, &offer));
        match checked {
            Ok(()) => {}
            // The file could not be read just now (a scanner holds it, say): it is kept and checked again later.
            Err(Failure::Disk) => {
                return State::Failed {
                    failure: Failure::Disk,
                    resume: Some(offer),
                };
            }
            Err(failure) => {
                let _ = fs::remove_file(&part);
                return State::Failed { failure, resume: None };
            }
        }
        if fs::rename(&part, &staged).is_err() {
            return State::Failed {
                failure: Failure::Disk,
                resume: Some(offer),
            };
        }
        State::ReadyToRestart {
            offer,
            installer: staged,
        }
    }

    fn apply(&mut self, offer: Offer, installer: PathBuf) -> State {
        let kept = |failure, offer| State::Failed {
            failure,
            resume: Some(offer),
        };
        if installer != self.final_path(&offer) || !self.staged_installer_is_alone(&installer) {
            return kept(Failure::Disk, offer);
        }
        // Checked again at the moment of use: the file sat in a folder the person's other programs can write.
        // From this open until the installer process exists nothing else can write, replace or delete the file
        // (`host::open_denying_writes`), so what is verified here is what runs.
        let Ok(held) = crate::host::open_denying_writes(&installer) else {
            return kept(Failure::Disk, offer);
        };
        match self.staged_file_is_signed(&held, &offer) {
            Ok(()) => {}
            Err(Failure::Disk) => return kept(Failure::Disk, offer),
            Err(failure) => {
                drop(held);
                let _ = fs::remove_file(&installer);
                return State::Failed { failure, resume: None };
            }
        }
        // The engine stops before the installer starts, never after: the installer replaces the running program.
        let Some(stopped) = self.apply.stop_engine() else {
            return kept(Failure::EngineStillRunning, offer);
        };
        let verified = VerifiedInstaller {
            path: installer,
            _held: held,
        };
        let started = self.apply.launch(&verified, &offer, stopped);
        drop(verified);
        if started {
            State::Restarting { offer }
        } else {
            self.apply.resume_engine();
            kept(Failure::InstallerDidNotStart, offer)
        }
    }
}
