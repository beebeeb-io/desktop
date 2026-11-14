//! Which folder may be chosen as the sync folder: the rule a first run asks before the first start.
//!
//! [`refusal`] is the whole decision, as a pure function of the folder's path as text and of [`Facts`] (what the
//! OS says, gathered by `Engine::check_sync_folder`). It reads the path as text, by Windows' rules or by Unix's
//! as `Facts::windows` says, so every rule is tested on any OS: `std::path` on Linux does not know a drive
//! letter or a `\\server\share`.
//!
//! Refused, in this order:
//! 1. a path that is not a full one (relative, a device path, one with `.` or `..` in it);
//! 2. a network path: a `\\server\share` by its text, a mapped drive by what the OS says (`Facts::remote_drive`);
//! 3. a drive's root and the folders the system and its programs live in;
//! 4. the home folder itself, anything that holds it, and the folder apps keep their data in;
//! 5. Beebeeb's own data (the local state and the settings), and anything that holds it;
//! 6. something that is not a folder, or that cannot be read;
//! 7. another app's sync folder: one Windows has registered to another sync provider, and the sync folder of
//!    this app's other build (the test build never syncs into the real app's folder, nor the other way around).
//!
//! This is the check BEFORE a start. The start has its own (`Engine::start_sync`: another provider's folder again,
//! under its lock, and whose local data this is), and a folder that passes here can still be refused there.
//!
//! What it does not see (said in the PR): a folder reached through a link or a junction in the middle of its
//! path (only the folder itself is looked at on disk), and, off Windows, a network mount. It is asked on
//! Windows only today (the first run is wired there).
//!
//! Must not: log or return a path. A refusal is a kind, never a sentence with the folder in it.

#[cfg(test)]
#[path = "sync_folder_rule_tests.rs"]
mod tests;

/// Why a folder cannot be the sync folder. Nothing was created, registered or written for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncFolderRefused {
    /// Not a full path to a folder on this computer.
    NotAFullPath,
    /// A folder on the network (a share, or a drive that is one).
    Network,
    /// A drive's root, or a folder of the system or its programs.
    System,
    /// The home folder itself (or a folder that holds it), or the folder apps keep their data in.
    Home,
    /// Beebeeb's own data, or a folder that holds it.
    OwnData,
    /// It exists and is not a folder (a file, a link).
    NotAFolder,
    /// Whether it is a folder, or whose sync folder it is, could not be read. Never read as "free".
    Unreadable,
    /// Another app's sync folder.
    AnotherApps,
}

impl SyncFolderRefused {
    /// The refusal's name, for a log line.
    pub fn kind(self) -> &'static str {
        match self {
            Self::NotAFullPath => "not_a_full_path",
            Self::Network => "network",
            Self::System => "system",
            Self::Home => "home",
            Self::OwnData => "own_data",
            Self::NotAFolder => "not_a_folder",
            Self::Unreadable => "unreadable",
            Self::AnotherApps => "another_apps",
        }
    }
}

/// What is at the path now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDisk {
    /// Nothing: the first start makes the folder.
    Missing,
    Folder,
    /// A file, or a link.
    NotAFolder,
    Unreadable,
}

/// What the OS says about the folder and this computer. Paths are text, as the OS wrote them.
#[derive(Debug, Clone)]
pub struct Facts {
    /// Read paths by Windows' rules (drive letters, both slashes, no case, `\\server\share`).
    pub windows: bool,
    /// The person's home folder.
    pub home: Option<String>,
    /// Folders of the system that this computer names itself (Windows: `%SystemRoot%`, `%ProgramFiles%`…), beside
    /// the ones every computer has.
    pub system: Vec<String>,
    /// Beebeeb's own data on this computer.
    pub own: Vec<String>,
    /// The sync folders of this app's other builds.
    pub other_builds: Vec<String>,
    /// The folder's drive is a network drive (Windows: a mapped drive letter).
    pub remote_drive: bool,
    pub on_disk: OnDisk,
    /// Whose sync folder Windows says it is.
    pub owner: Owner,
}

/// Whose sync folder the OS says a folder is (`sync_root_owner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// Nobody's, or this app's own.
    Free,
    /// Another sync provider's.
    Another,
    /// The OS could not say. Never read as free.
    Unreadable,
}

/// A full path, as the rule compares it: the root (`c:`, or `/`) and the folder names below it. On Windows the
/// names are folded the way Windows compares them (no case, no dots or spaces at a name's end).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Place {
    root: String,
    parts: Vec<String>,
}

impl Place {
    /// `self` is `other` or inside it.
    fn within(&self, other: &Place) -> bool {
        self.root == other.root && self.parts.len() >= other.parts.len() && self.parts.starts_with(&other.parts)
    }

    /// `self` is `other`, inside it, or holds it.
    fn touches(&self, other: &Place) -> bool {
        self.within(other) || other.within(self)
    }

    fn below(&self, names: &[&str]) -> Place {
        let mut parts = self.parts.clone();
        parts.extend(names.iter().map(|name| (*name).to_string()));
        Place {
            root: self.root.clone(),
            parts,
        }
    }
}

fn names(rest: &str, separators: &[char], fold: impl Fn(&str) -> String) -> Result<Vec<String>, SyncFolderRefused> {
    let mut parts = Vec::new();
    for name in rest.split(separators).filter(|name| !name.is_empty()) {
        if name == "." || name == ".." {
            return Err(SyncFolderRefused::NotAFullPath);
        }
        let name = fold(name);
        if name.is_empty() {
            return Err(SyncFolderRefused::NotAFullPath);
        }
        parts.push(name);
    }
    Ok(parts)
}

/// Read `text` as a full path on this kind of OS.
fn place(text: &str, windows: bool) -> Result<Place, SyncFolderRefused> {
    if !windows {
        let rest = text.strip_prefix('/').ok_or(SyncFolderRefused::NotAFullPath)?;
        return Ok(Place {
            root: "/".into(),
            parts: names(rest, &['/'], str::to_string)?,
        });
    }
    let slashes = ['\\', '/'];
    let mut text = text;
    // `\\?\C:\…` is `C:\…`; `\\?\UNC\server\share` is a share; any other `\\?\` or `\\.\` form is a device.
    for verbatim in [r"\\?\", r"\\.\"] {
        if let Some(rest) = text.strip_prefix(verbatim) {
            let upper = rest.to_ascii_uppercase();
            if upper.starts_with(r"UNC\") || upper.starts_with("UNC/") {
                return Err(SyncFolderRefused::Network);
            }
            text = rest;
        }
    }
    let mut chars = text.chars();
    let (first, second) = (chars.next(), chars.next());
    if first.is_some_and(|c| slashes.contains(&c)) && second.is_some_and(|c| slashes.contains(&c)) {
        return Err(SyncFolderRefused::Network);
    }
    let (Some(letter), Some(':')) = (first, second) else {
        return Err(SyncFolderRefused::NotAFullPath);
    };
    let rest = chars.as_str();
    if !letter.is_ascii_alphabetic() || !rest.is_empty() && !rest.starts_with(slashes) {
        // `C:folder` is relative to that drive's current folder.
        return Err(SyncFolderRefused::NotAFullPath);
    }
    Ok(Place {
        root: format!("{}:", letter.to_ascii_lowercase()),
        parts: names(rest, &slashes, |name| name.trim_end_matches(['.', ' ']).to_lowercase())?,
    })
}

/// The folders of the system every computer of this kind has, as their first names below a root.
const WINDOWS_SYSTEM: [&str; 7] = [
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    "recovery",
];
const UNIX_SYSTEM: [&str; 13] = [
    "usr",
    "etc",
    "bin",
    "sbin",
    "lib",
    "lib64",
    "boot",
    "dev",
    "proc",
    "sys",
    "System",
    "Library",
    "Applications",
];
/// Folders that hold every person's home: never the sync folder themselves.
const WINDOWS_HOMES: [&str; 1] = ["users"];
const UNIX_HOMES: [&str; 2] = ["home", "Users"];

/// [`refusal`] for a folder `given` as text and reached as `resolved` (its final path: links, junctions and short
/// names resolved). The text given is read first; then the rule judges the final path; one that could not be
/// resolved is `Unreadable`, never let through.
pub fn refusal_resolved(given: &str, resolved: Option<&str>, facts: &Facts) -> Option<SyncFolderRefused> {
    // STUB: judges `given`.
    let _ = resolved;
    refusal(given, facts)
}

/// Why `folder` cannot be the sync folder, or `None` when it may. See the module's doc for the order.
pub fn refusal(folder: &str, facts: &Facts) -> Option<SyncFolderRefused> {
    let place = match place(folder, facts.windows) {
        Ok(place) => place,
        Err(refused) => return Some(refused),
    };
    if facts.remote_drive {
        return Some(SyncFolderRefused::Network);
    }
    // A path the OS named that does not read as a full one protects nothing; it is not a reason to refuse.
    let known = |text: &String| self::place(text, facts.windows).ok();

    let (system, homes): (&[&str], &[&str]) = if facts.windows {
        (&WINDOWS_SYSTEM, &WINDOWS_HOMES)
    } else {
        (&UNIX_SYSTEM, &UNIX_HOMES)
    };
    let first = place.parts.first().map(String::as_str);
    let is_system = match first {
        None => true,
        Some(first) => system.contains(&first) || (place.parts.len() == 1 && homes.contains(&first)),
    };
    if is_system || facts.system.iter().filter_map(known).any(|named| place.touches(&named)) {
        return Some(SyncFolderRefused::System);
    }

    if let Some(home) = facts.home.as_ref().and_then(known) {
        let apps_data: &[&str] = if facts.windows { &["appdata"] } else { &["Library"] };
        if home.within(&place) || place.within(&home.below(apps_data)) {
            return Some(SyncFolderRefused::Home);
        }
    }

    if facts.own.iter().filter_map(known).any(|own| place.touches(&own)) {
        return Some(SyncFolderRefused::OwnData);
    }

    match facts.on_disk {
        OnDisk::Missing | OnDisk::Folder => {}
        OnDisk::NotAFolder => return Some(SyncFolderRefused::NotAFolder),
        OnDisk::Unreadable => return Some(SyncFolderRefused::Unreadable),
    }

    match facts.owner {
        Owner::Free => {}
        Owner::Another => return Some(SyncFolderRefused::AnotherApps),
        Owner::Unreadable => return Some(SyncFolderRefused::Unreadable),
    }
    if facts
        .other_builds
        .iter()
        .filter_map(known)
        .any(|other| place.touches(&other))
    {
        return Some(SyncFolderRefused::AnotherApps);
    }
    None
}
