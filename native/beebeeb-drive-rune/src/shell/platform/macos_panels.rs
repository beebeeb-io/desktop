//! macOS pickers that work under the App Sandbox: the native NSOpenPanel / NSSavePanel (the "user selected
//! read-write" entitlement covers what the person picks) and a security-scoped bookmark kept for a remembered folder,
//! so the choice still opens after a relaunch. No new crate: the Objective-C runtime is called directly.
//!
//! The panels run on the main thread (`dispatch_sync_f` onto the main queue when the caller is a worker). Paths are
//! the person's own: shown to them, never logged. The bookmark file lives in the app's Application Support, which
//! under the sandbox is inside the container (HOME is the container).

use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use super::SaveAnswer;

// ---- bookmark store: pure, tested -------------------------------------------------------------------------------

/// One line per remembered folder: `<key> <hex of the bookmark data>`. Keys hold no spaces.
pub(super) fn encode_bookmarks(entries: &BTreeMap<String, Vec<u8>>) -> String {
    let mut out = String::new();
    for (key, data) in entries {
        out.push_str(key);
        out.push(' ');
        for b in data {
            out.push_str(&format!("{b:02x}"));
        }
        out.push('\n');
    }
    out
}

pub(super) fn decode_bookmarks(text: &str) -> BTreeMap<String, Vec<u8>> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let Some((key, hex)) = line.split_once(' ') else {
            continue;
        };
        if hex.len() % 2 != 0 || !hex.is_ascii() {
            continue;
        }
        let bytes: Option<Vec<u8>> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok())
            .collect();
        if let Some(bytes) = bytes {
            map.insert(key.to_string(), bytes);
        }
    }
    map
}

fn store_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/beebeeb-drive/bookmarks.txt"))
}

fn remember_at(file: &Path, key: &str, data: Vec<u8>) -> std::io::Result<()> {
    let mut map = std::fs::read_to_string(file)
        .map(|t| decode_bookmarks(&t))
        .unwrap_or_default();
    map.insert(key.to_string(), data);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, encode_bookmarks(&map))
}

/// Bookmark key of one picked backup folder: its own path, hex-encoded (keys hold no spaces), so a second
/// folder never overwrites the first.
pub(super) fn folder_key(path: &Path) -> String {
    let mut key = String::from("folder:");
    for b in path.as_os_str().as_encoded_bytes() {
        key.push_str(&format!("{b:02x}"));
    }
    key
}

pub(super) const FOLDER_PREFIX: &str = "folder:";

/// Removes `key` from the store.
fn forget_at(file: &Path, key: &str) -> std::io::Result<()> {
    let mut map = std::fs::read_to_string(file)
        .map(|t| decode_bookmarks(&t))
        .unwrap_or_default();
    if map.remove(key).is_some() {
        std::fs::write(file, encode_bookmarks(&map))?;
    }
    Ok(())
}

/// Resolves every stored `folder:` bookmark with `resolve` (which opens the scope). A bookmark that no longer
/// resolves to a folder is dropped from the store; its key is returned, so the caller can log why. Never touches
/// the folder's contents or anything remote.
pub(super) fn resolve_all_at(
    file: &Path,
    mut resolve: impl FnMut(&[u8]) -> Option<PathBuf>,
) -> (Vec<PathBuf>, Vec<String>) {
    let map = std::fs::read_to_string(file)
        .map(|t| decode_bookmarks(&t))
        .unwrap_or_default();
    let (mut ok, mut stale) = (Vec::new(), Vec::new());
    for (key, data) in map.iter().filter(|(k, _)| k.starts_with(FOLDER_PREFIX)) {
        match resolve(data).filter(|p| p.is_dir()) {
            Some(path) => ok.push(path),
            None => stale.push(key.clone()),
        }
    }
    for key in &stale {
        let _ = forget_at(file, key);
    }
    (ok, stale)
}

fn recall_at(file: &Path, key: &str) -> Option<Vec<u8>> {
    decode_bookmarks(&std::fs::read_to_string(file).ok()?).remove(key)
}

// ---- Objective-C runtime ----------------------------------------------------------------------------------------

type Id = *mut c_void;
type Sel = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
}
#[link(name = "AppKit", kind = "framework")]
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_sync_f(queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn pthread_main_np() -> c_int;
}

fn sel(name: &str) -> Sel {
    let c = CString::new(name).expect("selector");
    unsafe { sel_registerName(c.as_ptr()) }
}
fn class(name: &str) -> Id {
    let c = CString::new(name).expect("class");
    unsafe { objc_getClass(c.as_ptr()) }
}

macro_rules! send {
    ($ret:ty; $obj:expr, $name:expr $(, $arg:expr => $t:ty)*) => {{
        let f: unsafe extern "C" fn(Id, Sel $(, $t)*) -> $ret =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { f($obj, sel($name) $(, $arg)*) }
    }};
}

fn nsstring(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).expect("nul removed");
    let p = c.as_ptr();
    send!(Id; class("NSString"), "stringWithUTF8String:", p => *const c_char)
}

fn file_url(path: &Path) -> Id {
    let s = nsstring(&path.to_string_lossy());
    send!(Id; class("NSURL"), "fileURLWithPath:", s => Id)
}

fn url_path(url: Id) -> Option<PathBuf> {
    if url.is_null() {
        return None;
    }
    let ns = send!(Id; url, "path");
    if ns.is_null() {
        return None;
    }
    let c = send!(*const c_char; ns, "UTF8String");
    (!c.is_null()).then(|| PathBuf::from(unsafe { CStr::from_ptr(c) }.to_string_lossy().into_owned()))
}

/// Runs `f` on the main thread and returns its result.
fn on_main<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    if unsafe { pthread_main_np() } == 1 {
        return f();
    }
    extern "C" fn run(ctx: *mut c_void) {
        let job = unsafe { &mut *(ctx as *mut Option<Box<dyn FnOnce() + '_>>) };
        if let Some(f) = job.take() {
            f();
        }
    }
    let mut out: Option<T> = None;
    {
        let slot = &mut out;
        let mut job: Option<Box<dyn FnOnce() + '_>> = Some(Box::new(move || *slot = Some(f())));
        // dispatch_sync_f returns only after `run` ended, so the borrows above outlive the call.
        unsafe { dispatch_sync_f(&raw const _dispatch_main_q, &mut job as *mut _ as *mut c_void, run) };
    }
    out.expect("main queue ran the job")
}

const MODAL_OK: c_long = 1;
const BOOKMARK_WITH_SECURITY_SCOPE: c_ulong = 1 << 11;
const RESOLVE_WITH_SECURITY_SCOPE: c_ulong = 1 << 10;

fn set_start(panel: Id, start: &Path) {
    if start.is_dir() {
        let url = file_url(start);
        send!((); panel, "setDirectoryURL:", url => Id);
    }
}

/// Security-scoped bookmark data for a URL the person picked; `None` if the OS refuses.
fn bookmark_of(url: Id) -> Option<Vec<u8>> {
    let nil: Id = std::ptr::null_mut();
    let data = send!(Id; url, "bookmarkDataWithOptions:includingResourceValuesForKeys:relativeToURL:error:",
        BOOKMARK_WITH_SECURITY_SCOPE => c_ulong, nil => Id, nil => Id, nil => Id);
    if data.is_null() {
        return None;
    }
    let len = send!(c_ulong; data, "length") as usize;
    let ptr = send!(*const u8; data, "bytes");
    (!ptr.is_null()).then(|| unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

/// Resolves bookmark data to a path and starts its security scope (held until the process ends).
fn resolve_bookmark(data: &[u8]) -> Option<PathBuf> {
    let nil: Id = std::ptr::null_mut();
    let ptr = data.as_ptr();
    let nsdata =
        send!(Id; class("NSData"), "dataWithBytes:length:", ptr => *const u8, data.len() as c_ulong => c_ulong);
    if nsdata.is_null() {
        return None;
    }
    let mut stale: u8 = 0;
    let stale_ptr = &mut stale as *mut u8;
    let url = send!(Id; class("NSURL"), "URLByResolvingBookmarkData:options:relativeToURL:bookmarkDataIsStale:error:",
        nsdata => Id, RESOLVE_WITH_SECURITY_SCOPE => c_ulong, nil => Id, stale_ptr => *mut u8, nil => Id);
    if url.is_null() {
        return None;
    }
    let _ok = send!(u8; url, "startAccessingSecurityScopedResource");
    url_path(url)
}

/// The folder remembered under `key` by an earlier choice, with its security scope open; `None` if none or stale.
pub fn remembered_folder(key: &str) -> Option<PathBuf> {
    let data = recall_at(&store_file()?, key)?;
    resolve_bookmark(&data).filter(|p| p.is_dir())
}

fn remember(key: &str, url: Id) {
    if let (Some(file), Some(data)) = (store_file(), bookmark_of(url)) {
        let _ = remember_at(&file, key, data);
    }
}

/// At start, before the engine reads any backup folder: reopens the security scope of every folder picked for
/// Backups. A stale one is dropped and logged (no path), so the folder shows as unreadable rather than silently
/// missing.
pub fn resolve_backup_folders() {
    let Some(file) = store_file() else { return };
    let (ok, stale) = resolve_all_at(&file, resolve_bookmark);
    tracing::info!(
        "backup folder bookmarks: {} reopened, {} stale and dropped",
        ok.len(),
        stale.len()
    );
}

/// The folder picker: NSOpenPanel, folders only; the choice is remembered as a security-scoped bookmark under its
/// own per-folder key (`key` is unused here: one key per path).
/// BLOCKS until closed. `Ok(None)`: closed without a folder.
pub fn pick_folder(title: &str, start: &Path, key: &str) -> Result<Option<PathBuf>, String> {
    let (title, start, key) = (title.to_string(), start.to_path_buf(), key.to_string());
    on_main(move || {
        let panel = send!(Id; class("NSOpenPanel"), "openPanel");
        if panel.is_null() {
            return Err("the folder panel could not open".to_string());
        }
        send!((); panel, "setCanChooseDirectories:", 1u8 => u8);
        send!((); panel, "setCanChooseFiles:", 0u8 => u8);
        send!((); panel, "setAllowsMultipleSelection:", 0u8 => u8);
        send!((); panel, "setCanCreateDirectories:", 1u8 => u8);
        let msg = nsstring(&title);
        send!((); panel, "setMessage:", msg => Id);
        set_start(panel, &start);
        if send!(c_long; panel, "runModal") != MODAL_OK {
            return Ok(None);
        }
        let url = send!(Id; panel, "URL");
        let path = url_path(url);
        if let Some(p) = &path {
            remember(&folder_key(p), url);
        }
        let _ = &key;
        Ok(path)
    })
}

/// The app chooser: NSOpenPanel on `/Applications`, application bundles only (dm-63, "Open with another app").
/// BLOCKS until closed. `None`: cancelled.
pub fn pick_app() -> Option<PathBuf> {
    on_main(move || {
        let panel = send!(Id; class("NSOpenPanel"), "openPanel");
        if panel.is_null() {
            return None;
        }
        send!((); panel, "setCanChooseDirectories:", 0u8 => u8);
        send!((); panel, "setCanChooseFiles:", 1u8 => u8);
        send!((); panel, "setAllowsMultipleSelection:", 0u8 => u8);
        let msg = nsstring("Open with");
        send!((); panel, "setMessage:", msg => Id);
        // Apps only: the panel's own filter by type name.
        let ext = nsstring("app");
        let array = send!(Id; class("NSArray"), "arrayWithObject:", ext => Id);
        send!((); panel, "setAllowedFileTypes:", array => Id);
        set_start(panel, Path::new("/Applications"));
        if send!(c_long; panel, "runModal") != MODAL_OK {
            return None;
        }
        url_path(send!(Id; panel, "URL")).filter(|p| p.extension().is_some_and(|e| e == "app"))
    })
}

/// The save panel: NSSavePanel with `name` filled in; the folder is remembered under `key`. BLOCKS until closed.
pub fn pick_save_path(name: &str, start: PathBuf, key: &str) -> SaveAnswer {
    let (name, key) = (name.to_string(), key.to_string());
    on_main(move || {
        let panel = send!(Id; class("NSSavePanel"), "savePanel");
        if panel.is_null() {
            return SaveAnswer::Failed;
        }
        let ns = nsstring(&name);
        send!((); panel, "setNameFieldStringValue:", ns => Id);
        send!((); panel, "setCanCreateDirectories:", 1u8 => u8);
        // A remembered folder (scope reopened) beats the suggestion.
        let start = remembered_folder(&key).unwrap_or(start);
        set_start(panel, &start);
        if send!(c_long; panel, "runModal") != MODAL_OK {
            return SaveAnswer::Cancelled;
        }
        let url = send!(Id; panel, "URL");
        match url_path(url) {
            Some(path) => {
                if let Some(dir) = path.parent() {
                    let dir_url = file_url(dir);
                    remember(&key, dir_url);
                }
                SaveAnswer::Chosen(path)
            }
            None => SaveAnswer::Failed,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_data_round_trips_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub/bookmarks.txt");
        let a: Vec<u8> = (0..=255).collect();
        remember_at(&file, "save_folder", a.clone()).unwrap();
        remember_at(&file, "other", vec![0, 1, 2]).unwrap();
        assert_eq!(recall_at(&file, "save_folder"), Some(a));
        assert_eq!(recall_at(&file, "other"), Some(vec![0, 1, 2]));
        assert_eq!(recall_at(&file, "nothing"), None);
        assert_eq!(decode_bookmarks("bad line\nk zz\nk 0\n"), BTreeMap::new());
    }

    #[test]
    fn two_folders_keep_two_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("b.txt");
        let (a, b) = (Path::new("/Users/x/Photos"), Path::new("/Users/x/Docs"));
        assert_ne!(folder_key(a), folder_key(b));
        remember_at(&file, &folder_key(a), vec![1]).unwrap();
        remember_at(&file, &folder_key(b), vec![2]).unwrap();
        assert_eq!(recall_at(&file, &folder_key(a)), Some(vec![1]));
        assert_eq!(recall_at(&file, &folder_key(b)), Some(vec![2]));
    }

    #[test]
    fn resolve_at_start_opens_each_and_drops_stale() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("b.txt");
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let (k1, k2, k3) = (
            folder_key(&real),
            folder_key(Path::new("/gone")),
            folder_key(Path::new("/gone2")),
        );
        remember_at(&file, &k1, vec![1]).unwrap();
        remember_at(&file, &k2, vec![2]).unwrap();
        remember_at(&file, &k3, vec![3]).unwrap();
        remember_at(&file, "save_folder", vec![9]).unwrap();
        let mut calls = Vec::new();
        let (ok, stale) = resolve_all_at(&file, |d| {
            calls.push(d.to_vec());
            (d == [1]).then(|| real.clone())
        });
        assert_eq!(
            calls.len(),
            3,
            "each folder bookmark is resolved, the save folder is not"
        );
        assert_eq!(ok, vec![real]);
        assert_eq!(stale.len(), 2);
        assert_eq!(recall_at(&file, &k2), None);
        assert_eq!(recall_at(&file, &k1), Some(vec![1]));
        assert_eq!(recall_at(&file, "save_folder"), Some(vec![9]));
    }
}
