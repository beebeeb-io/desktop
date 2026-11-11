//! Test doubles: a signing key made inside each test (none is committed), a scripted release server, a clock, a
//! store and an installer launch that only records.

use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::machine::{
    Apply, Clock, Config, EngineStopped, Event, Http, Persisted, Response, State, Store, Unreachable, Updater,
    VerifiedInstaller,
};
use crate::manifest::Channel;
use crate::policy::{KeyScope, NATIVE_FLOOR, Offer, Trust, TrustedKey};
use crate::signature::PublicKey;
use crate::version::Version;

pub const HOST: &str = "releases.beebeeb.io";
pub const BASE: &str = "https://releases.beebeeb.io";
pub const TARGET: &str = "windows-x86_64-nsis";

/// A minisign key pair generated for one test, signing in the form Tauri's signer writes.
pub struct TestKey(minisign::KeyPair);

impl TestKey {
    pub fn generate() -> Self {
        Self(minisign::KeyPair::generate_unencrypted_keypair().expect("a test key pair"))
    }

    /// The same test key for a server thread (`net_tests`).
    #[cfg(feature = "net")]
    pub fn clone_for_thread(&self) -> Self {
        Self(self.0.clone())
    }

    pub fn public(&self) -> PublicKey {
        let text = self.0.pk.to_box().expect("public key box").to_string();
        PublicKey::from_tauri_base64(&BASE64.encode(text)).expect("the test public key decodes")
    }

    /// A `.sig` file's content: base64 of the signature text, trusted comment `timestamp:<t><TAB>file:<file>`.
    pub fn sign(&self, bytes: &[u8], timestamp: u64, file: &str) -> String {
        let trusted = format!("timestamp:{timestamp}\tfile:{file}");
        let signature = minisign::sign(
            Some(&self.0.pk),
            &self.0.sk,
            Cursor::new(bytes),
            Some(&trusted),
            Some("signature from tauri secret key"),
        )
        .expect("signing with a test key");
        BASE64.encode(signature.to_string())
    }
}

pub fn trust(key: &TestKey) -> Trust {
    Trust {
        // For every version, so that the floor and the scope are each tested by their own rule.
        keys: vec![TrustedKey {
            key: key.public(),
            valid_for: KeyScope::from(Version::new(0, 0, 0)),
        }],
        product: "desktop".to_string(),
        allowed_hosts: vec![HOST.to_string()],
        targets: vec![TARGET.to_string(), "windows-x86_64".to_string()],
        floor: NATIVE_FLOOR,
        // The earliest build time there is; an honest test manifest is signed well after it.
        not_before: crate::policy::V2_KEY_CREATED,
    }
}

/// The clock of every test updater; an honest test manifest is signed 1000 seconds before it.
pub const NOW: u64 = 1_800_001_000;

pub fn version(text: &str) -> Version {
    Version::parse(text).expect("a plain semver in a test")
}

pub fn artifact_name(version: &str) -> String {
    format!("Beebeeb_{version}_x64-setup.exe")
}

pub fn artifact_url(version: &str) -> String {
    format!("{BASE}/desktop/artifacts/{version}/{}", artifact_name(version))
}

pub fn manifest_url(channel: Channel) -> String {
    format!("{BASE}/desktop/v2/{}", channel.manifest_file())
}

/// Installer-like bytes: not constant, so a shifted or repeated range is a different file.
pub fn artifact_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(31) ^ (i >> 8)) as u8).collect()
}

/// One published release, honest by default. A test changes one field to break one rule.
pub struct Publish {
    pub channel: Channel,
    pub version: String,
    pub artifact: Vec<u8>,
    /// The signed time of the manifest's signature.
    pub manifest_time: u64,
    pub url: String,
    pub size: Option<u64>,
    /// `None`: the honest signature of `artifact` under its own name.
    pub artifact_signature: Option<String>,
    pub product: Option<&'static str>,
    pub claimed_channel: Option<&'static str>,
    /// The file name the manifest's signature signs. `None`: the channel's manifest file.
    pub signed_manifest_name: Option<&'static str>,
}

impl Publish {
    pub fn new(channel: Channel, version: &str, artifact: Vec<u8>) -> Self {
        Self {
            channel,
            version: version.to_string(),
            size: Some(artifact.len() as u64),
            artifact,
            manifest_time: 1_800_000_000,
            url: artifact_url(version),
            artifact_signature: None,
            product: Some("desktop"),
            claimed_channel: Some(channel.as_str()),
            signed_manifest_name: None,
        }
    }

    /// The manifest's bytes and its `.sig` content.
    pub fn manifest(&self, key: &TestKey) -> (Vec<u8>, String) {
        let signature = self
            .artifact_signature
            .clone()
            .unwrap_or_else(|| key.sign(&self.artifact, self.manifest_time - 60, &artifact_name(&self.version)));
        let mut platform = serde_json::json!({ "url": self.url, "signature": signature });
        if let Some(size) = self.size {
            platform["size"] = size.into();
        }
        let mut manifest = serde_json::json!({
            "version": self.version, "notes": "What changed.", "pub_date": "2026-11-20T10:00:00Z",
            "platforms": { TARGET: platform },
        });
        if let Some(product) = self.product {
            manifest["product"] = product.into();
        }
        if let Some(channel) = self.claimed_channel {
            manifest["channel"] = channel.into();
        }
        let bytes = serde_json::to_vec_pretty(&manifest).expect("manifest json");
        let name = self.signed_manifest_name.unwrap_or(self.channel.manifest_file());
        let detached = key.sign(&bytes, self.manifest_time, name);
        (bytes, detached)
    }

    /// Puts the manifest, its signature and the artefact on the fake server.
    pub fn serve(&self, http: &mut FakeHttp, key: &TestKey) {
        let (manifest, signature) = self.manifest(key);
        let url = manifest_url(self.channel);
        http.files.insert(format!("{url}.sig"), signature.into_bytes());
        http.files.insert(url, manifest);
        http.files.insert(self.url.clone(), self.artifact.clone());
    }
}

/// How the fake server answers the next request for one address, instead of serving the file honestly.
pub enum Serve {
    Unreachable,
    Status(u16),
    /// Sends this many bytes of the requested range, then the connection breaks.
    DropAfter(usize),
    /// Sends this many bytes of the requested range, then ends as if that were all.
    EndAfter(usize),
    /// Answers 200 with the whole file whatever range was asked.
    IgnoreRange,
    /// Honest bytes, but states this as the file's total length.
    ClaimTotal(u64),
    /// A 206 that starts at this offset whatever was asked.
    RangeFrom(u64),
}

#[derive(Default)]
pub struct FakeHttp {
    pub files: BTreeMap<String, Vec<u8>>,
    pub plan: BTreeMap<String, VecDeque<Serve>>,
    /// Every request: the address and the offset asked for.
    pub log: Vec<(String, u64)>,
}

impl FakeHttp {
    pub fn next(&mut self, url: &str, serve: Serve) {
        self.plan.entry(url.to_string()).or_default().push_back(serve);
    }

    pub fn requests_for(&self, url: &str) -> Vec<u64> {
        self.log
            .iter()
            .filter(|(logged, _)| logged == url)
            .map(|(_, from)| *from)
            .collect()
    }
}

struct Body {
    data: Cursor<Vec<u8>>,
    then_break: bool,
}

impl Read for Body {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.data.read(buffer)? {
            0 if self.then_break && !buffer.is_empty() => Err(std::io::ErrorKind::ConnectionReset.into()),
            read => Ok(read),
        }
    }
}

fn response(status: u16, offset: u64, total: Option<u64>, data: &[u8], then_break: bool) -> Response {
    Response {
        status,
        offset,
        total,
        body: Box::new(Body {
            data: Cursor::new(data.to_vec()),
            then_break,
        }),
    }
}

impl Http for FakeHttp {
    fn get(&mut self, url: &str, from: u64) -> Result<Response, Unreachable> {
        self.log.push((url.to_string(), from));
        let planned = self.plan.get_mut(url).and_then(VecDeque::pop_front);
        let Some(file) = self.files.get(url) else {
            return Ok(response(404, 0, None, b"", false));
        };
        let total = file.len() as u64;
        if from > total {
            return Ok(response(416, 0, None, b"", false));
        }
        let rest = &file[from as usize..];
        let status = if from > 0 { 206 } else { 200 };
        Ok(match planned {
            None => response(status, from, Some(total), rest, false),
            Some(Serve::Unreachable) => return Err(Unreachable),
            Some(Serve::Status(code)) => response(code, 0, None, b"", false),
            Some(Serve::DropAfter(sent)) => response(status, from, Some(total), &rest[..sent.min(rest.len())], true),
            Some(Serve::EndAfter(sent)) => response(status, from, Some(total), &rest[..sent.min(rest.len())], false),
            Some(Serve::IgnoreRange) => response(200, 0, Some(total), file, false),
            Some(Serve::ClaimTotal(claimed)) => response(status, from, Some(claimed), rest, false),
            Some(Serve::RangeFrom(offset)) => response(206, offset, Some(total), rest, false),
        })
    }
}

pub struct FakeClock(pub Cell<u64>);

impl Clock for FakeClock {
    fn now(&self) -> u64 {
        self.0.get()
    }
}

#[derive(Default)]
pub struct MemStore {
    pub persisted: Persisted,
    pub refuse_writes: bool,
}

impl Store for MemStore {
    fn load(&self) -> Persisted {
        self.persisted.clone()
    }

    fn save(&mut self, persisted: &Persisted) -> bool {
        if !self.refuse_writes {
            self.persisted = persisted.clone();
        }
        !self.refuse_writes
    }
}

/// What the shell did at "Restart to update", in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Did {
    StoppedEngine,
    EngineRefused,
    Launched(PathBuf, Version),
    LaunchFailed,
    ResumedEngine,
}

/// Runs inside `launch`, with the staged installer's path.
pub type DuringLaunch = Box<dyn FnMut(&Path)>;

#[derive(Default)]
pub struct FakeApply {
    pub did: Vec<Did>,
    pub refuse: bool,
    pub engine_will_not_stop: bool,
    /// Runs inside `launch`, while the updater still holds the staged file.
    pub during_launch: Option<DuringLaunch>,
}

impl FakeApply {
    pub fn launched(&self) -> Vec<(PathBuf, Version)> {
        self.did
            .iter()
            .filter_map(|did| match did {
                Did::Launched(path, version) => Some((path.clone(), *version)),
                _ => None,
            })
            .collect()
    }
}

impl Apply for FakeApply {
    fn stop_engine(&mut self) -> Option<EngineStopped> {
        if self.engine_will_not_stop {
            self.did.push(Did::EngineRefused);
            return None;
        }
        self.did.push(Did::StoppedEngine);
        Some(EngineStopped::engine_stopped_and_uploads_parked())
    }

    fn launch(&mut self, installer: &VerifiedInstaller, offer: &Offer, _stopped: EngineStopped) -> bool {
        let installer = installer.path();
        if let Some(hook) = self.during_launch.as_mut() {
            hook(installer);
        }
        self.did.push(if self.refuse {
            Did::LaunchFailed
        } else {
            Did::Launched(installer.to_path_buf(), offer.version)
        });
        !self.refuse
    }

    fn resume_engine(&mut self) {
        self.did.push(Did::ResumedEngine);
    }
}

pub type TestUpdater = Updater<FakeHttp, FakeClock, MemStore, FakeApply>;

pub fn updater(key: &TestKey, installed: &str, staging: &Path, http: FakeHttp, store: MemStore) -> TestUpdater {
    let config = Config {
        trust: trust(key),
        base_url: BASE.to_string(),
        installed: version(installed),
        staging_dir: staging.to_path_buf(),
    };
    Updater::new(config, http, FakeClock(Cell::new(NOW)), store, FakeApply::default())
}

/// Sends `event`, then `Tick`s until the machine has no work left. Returns every state on the way, the last one
/// being where it settled.
pub fn drive(updater: &mut TestUpdater, event: Event) -> Vec<State> {
    let mut states = vec![updater.step(event).clone()];
    for _ in 0..100_000 {
        if !matches!(
            states.last(),
            Some(
                State::Checking { .. } | State::Rechecking { .. } | State::Downloading { .. } | State::Verifying { .. }
            )
        ) {
            return states;
        }
        states.push(updater.step(Event::Tick).clone());
    }
    panic!("the updater did not settle");
}

pub fn settle(updater: &mut TestUpdater, event: Event) -> State {
    drive(updater, event).pop().expect("at least one state")
}
