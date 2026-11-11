//! The updater as one state and one `step`: check, download (resumable), verify, stage, hand to the installer.
//!
//! The shell owns the loop. It sends an [`Event`], reads the [`State`] back and draws it; a step that has work left
//! (`Checking`, `Downloading`, `Verifying`) is continued with [`Event::Tick`]. Every `Tick` does one bounded piece of
//! blocking work (one request, or one chunk), so the shell runs the loop on a worker thread and can show progress.
//! The network, the clock, the persisted marks and the installer launch are traits, so every rule is tested without
//! any of them. Nothing is installed without [`Event::Apply`], which only a person's click sends.

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

/// Starts the verified installer. On success the shell stops the engine's last work and exits the process; the
/// installer replaces the files and starts the new version.
pub trait Apply {
    fn launch(&mut self, installer: &Path, offer: &Offer) -> bool;
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
    InstallerDidNotStart,
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

    /// Applies one event. An event that means nothing in the current state changes nothing.
    pub fn step(&mut self, event: Event) -> &State {
        let state = std::mem::replace(&mut self.state, State::Idle);
        self.state = match (state, event) {
            // A check never interrupts work in progress.
            (
                busy @ (State::Checking { .. }
                | State::Downloading { .. }
                | State::Verifying { .. }
                | State::Restarting { .. }),
                Event::Check { .. },
            ) => busy,
            // A background check never takes away what the person is looking at: an offer, a download that can be
            // resumed, or a staged update waiting for "Restart to update". A manual check is the person's own.
            (
                kept @ (State::Available { .. } | State::ReadyToRestart { .. } | State::Failed { resume: Some(_), .. }),
                Event::Check {
                    intent: Intent::Background,
                    ..
                },
            ) => kept,
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
            &manifest,
            &signature,
        ) {
            Ok(checked) => checked,
            Err(refusal) => return refused(refusal),
        };

        // Recorded on disk first: a check that could not be recorded did not happen, in memory either.
        let mut recorded = self.persisted.clone();
        recorded.seen.insert(
            channel,
            HighWater {
                manifest_time: checked.manifest_time,
            },
        );
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
    fn staged_file_is_signed(&self, path: &Path, offer: &Offer) -> Result<(), Failure> {
        let file = fs::File::open(path).map_err(|_| Failure::Disk)?;
        if file.metadata().map_err(|_| Failure::Disk)?.len() != offer.size {
            return Err(Failure::WrongSize);
        }
        let signature = Signature::from_base64(&offer.signature).map_err(|_| Failure::BadSignature)?;
        if signature.file != offer.file_name {
            return Err(Failure::BadSignature);
        }
        signature
            .verify_reader(&self.config.trust.key, file)
            .map_err(|_| Failure::BadSignature)
    }

    fn verify(&mut self, offer: Offer) -> State {
        let (part, staged) = (self.part_path(&offer), self.final_path(&offer));
        match self.staged_file_is_signed(&part, &offer) {
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
        // Checked again at the moment of use: the file sat in a folder the person's other programs can write.
        match self.staged_file_is_signed(&installer, &offer) {
            Ok(()) => {}
            Err(Failure::Disk) => {
                return State::Failed {
                    failure: Failure::Disk,
                    resume: Some(offer),
                };
            }
            Err(failure) => {
                let _ = fs::remove_file(&installer);
                return State::Failed { failure, resume: None };
            }
        }
        if self.apply.launch(&installer, &offer) {
            State::Restarting { offer }
        } else {
            State::Failed {
                failure: Failure::InstallerDidNotStart,
                resume: Some(offer),
            }
        }
    }
}
