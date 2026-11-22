//! DW-1562's Open With: the apps Windows reports for a type, and the start of one of them on the copy of a file kept
//! on this PC (decision D-OV-14: "the handlers Windows reports for the type, no registry writes, no activation by
//! path").
//!
//! - The apps: `SHAssocEnumHandlers(<extension>, ASSOC_FILTER_RECOMMENDED)`, each handler's `GetUIName` as the words
//!   and `GetName` as its id, in Windows' order, at most [`MAX_APPS`], none that is this app's own program. Asked once
//!   per type for the run of the app, on a COM thread of its own (never the window's), and held.
//! - The start: on a COM thread of its own the handlers are listed again, the one whose `GetName` is the id picked is
//!   found (a handler gone since the menu: refused), and `IAssocHandler::Invoke` gets the kept file as a data object
//!   (`SHCreateItemFromParsingName` → `BindToHandler(BHID_DataObject)`), under the launch lock (`launch.rs`), and only
//!   while the session of the press is still the one once the lock is taken (`files_launch.rs`).
//! - Never: a registry write, `MakeDefault` / `SetAsDefaultFor`, a temporary copy, a path made from a name.
//!
//! Must not: log a path or a file's name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::frames::files::source::{ActionError, AppChoice};

/// At most this many apps in the submenu.
#[cfg(test)]
pub(crate) const MAX_APPS: usize = 8;

/// The extension Windows is asked about, as it takes it: ".docx", lower case. `None`: the name has none.
pub(crate) fn extension_of(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| format!(".{}", ext.to_lowercase()))
}

/// Is the handler with this name (`GetName`: a program's path, or its file name) this app's own program? By the
/// program's file name, without case, as Windows compares them.
fn is_own(handler: &str, own_exe: &str) -> bool {
    let file = |path: &str| path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase();
    !own_exe.is_empty() && file(handler) == file(own_exe)
}

/// The submenu's apps from the handlers Windows reported, `(GetName, GetUIName)` in its order: none whose program is
/// `own_exe` (Beebeeb is the submenu's first line already, and is never made a type's app), none without words, at
/// most [`MAX_APPS`].
#[cfg(any(test, windows))]
pub(crate) fn choices(handlers: impl IntoIterator<Item = (String, String)>, own_exe: &str) -> Vec<AppChoice> {
    handlers
        .into_iter()
        .filter(|(name, label)| !is_own(name, own_exe) && !label.trim().is_empty())
        .take(MAX_APPS)
        .map(|(id, label)| AppChoice { id, label })
        .collect()
}

/// The apps held per type for the run of the app (`None`: asked, not answered yet).
#[derive(Clone, Default)]
pub(crate) struct Apps {
    known: Arc<Mutex<HashMap<String, Option<Vec<AppChoice>>>>>,
}

impl Apps {
    /// The apps for a file named `name`: what is held. The first time a type is asked, Windows is asked on a thread of
    /// its own, and `wake` runs once it answered (the menu is drawn again). Never waits.
    pub(crate) fn for_name(&self, name: &str, wake: impl FnOnce() + Send + 'static) -> Vec<AppChoice> {
        let Some(ext) = extension_of(name) else {
            return Vec::new();
        };
        {
            let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
            match known.get(&ext) {
                Some(Some(apps)) => return apps.clone(),
                Some(None) => return Vec::new(),
                None => {
                    known.insert(ext.clone(), None);
                }
            }
        }
        let known = self.known.clone();
        ask_windows(ext.clone(), move |apps| {
            known
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(ext, Some(apps));
            wake();
        });
        Vec::new()
    }
}

/// The friendly name of the app the system opens `ext` (".pptx") with by default, held per type for the run of the
/// app (`None` inside: asked, no name). The first ask goes to the system on a thread of its own and `wake` runs once
/// it answered; until then, and when there is no default (or it is this app), `None`: the caller says "another app".
/// Never waits.
pub(crate) fn default_app_name(ext: &str, wake: impl FnOnce() + Send + 'static) -> Option<String> {
    static KNOWN: std::sync::OnceLock<Mutex<HashMap<String, Option<Option<String>>>>> = std::sync::OnceLock::new();
    let known = KNOWN.get_or_init(Mutex::default);
    let ext = ext.to_lowercase();
    {
        let mut held = known.lock().unwrap_or_else(PoisonError::into_inner);
        match held.get(&ext) {
            Some(Some(name)) => return name.clone(),
            Some(None) => return None,
            None => {
                held.insert(ext.clone(), None);
            }
        }
    }
    let asked = ext.clone();
    let started = std::thread::Builder::new().name("default-app-name".into()).spawn(move || {
        let name = default_name(ask_default(&asked), &own_exe());
        known.lock().unwrap_or_else(PoisonError::into_inner).insert(asked, Some(name));
        wake();
    });
    if started.is_err() {
        known.lock().unwrap_or_else(PoisonError::into_inner).insert(ext, Some(None));
    }
    None
}

/// The system's answer as a name to show: none when empty or when it is this app's own program.
fn default_name(answer: Option<(String, String)>, own_exe: &str) -> Option<String> {
    let (exe, name) = answer?;
    let name = name.trim();
    (!name.is_empty() && !is_own(&exe, own_exe)).then(|| name.to_owned())
}

/// Windows: `AssocQueryStringW` for the default `open` verb of `ext`: `(executable, friendly app name)`.
#[cfg(windows)]
fn ask_default(ext: &str) -> Option<(String, String)> {
    let exe = win::assoc(ext, windows::Win32::UI::Shell::ASSOCSTR_EXECUTABLE).unwrap_or_default();
    let name = win::assoc(ext, windows::Win32::UI::Shell::ASSOCSTR_FRIENDLYAPPNAME)?;
    Some((exe, name))
}

/// Every other system: not asked here (the Office editor, and so its guard, runs on Windows only).
#[cfg(not(windows))]
fn ask_default(_ext: &str) -> Option<(String, String)> {
    None
}

/// The path of this app's own program, as Windows names a handler's.
fn own_exe() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Ask Windows for the apps of `ext` on a COM thread of its own; `answer` gets them. A thread that does not start
/// answers nothing: the submenu keeps Beebeeb and Other….
#[cfg(windows)]
fn ask_windows(ext: String, answer: impl FnOnce(Vec<AppChoice>) + Send + 'static) {
    let own = own_exe();
    let _ = std::thread::Builder::new()
        .name("files-open-with-apps".into())
        .spawn(move || {
            let apps = match win::Com::enter() {
                Some(_com) => choices(win::handlers(&ext).iter().filter_map(win::names), &own),
                None => Vec::new(),
            };
            answer(apps);
        });
}

/// Every other system: Windows' handlers are not there, so there are none.
#[cfg(not(windows))]
fn ask_windows(_ext: String, answer: impl FnOnce(Vec<AppChoice>) + Send + 'static) {
    let _ = own_exe;
    answer(Vec::new());
}

/// What starts the app picked on the kept copy, on the thread [`open_in`] makes: the path, the app's id, and `still`,
/// which it asks inside the launch lock right before the start (`false`: the press's session moved on; nothing
/// starts). The system's own is [`SYSTEM`]; a test hands a stand-in.
pub(crate) type Invoke = fn(&Path, &str, &dyn Fn() -> bool) -> Result<(), ActionError>;

/// Windows' handlers, with COM set up on the thread that starts one.
#[cfg(windows)]
pub(crate) const SYSTEM: Option<Invoke> = Some(invoke_with_com);
/// Every other system has no handlers to start.
#[cfg(not(windows))]
pub(crate) const SYSTEM: Option<Invoke> = None;

#[cfg(windows)]
fn invoke_with_com(path: &Path, app_id: &str, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    match win::Com::enter() {
        Some(_com) => win::invoke(path, app_id, still),
        None => Err(ActionError::Failed),
    }
}

/// Open `path`, the copy kept on this PC, in the handler whose `GetName` is `app_id`, with `invoke` on a thread of its
/// own. `Ok`: the thread started, and `done` gets what happened (a handler gone since the menu, a start that failed,
/// or a session that moved on before the lock was taken: an `Err`). `None` (every other system): not there.
pub(crate) fn open_in(
    invoke: Option<Invoke>,
    path: PathBuf,
    app_id: String,
    still: impl Fn() -> bool + Send + 'static,
    done: impl FnOnce(Result<(), ActionError>) + Send + 'static,
) -> Result<(), ActionError> {
    let Some(invoke) = invoke else {
        return Err(ActionError::NotOnThisSystem);
    };
    std::thread::Builder::new()
        .name("files-open-in-app".into())
        .spawn(move || done(invoke(&path, &app_id, &still)))
        .map(|_| ())
        .map_err(|_| ActionError::Failed)
}

#[cfg(windows)]
mod win {
    use std::path::Path;

    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree, CoUninitialize, IDataObject,
    };
    use windows::Win32::UI::Shell::{
        ASSOC_FILTER_RECOMMENDED, BHID_DataObject, IAssocHandler, IShellItem, SHAssocEnumHandlers,
        SHCreateItemFromParsingName,
    };
    use windows::core::{HSTRING, PWSTR};

    use crate::engine_host::launch;
    use crate::frames::files::source::ActionError;

    /// COM on this thread while it lives: taken back only when this call was the one that set it up.
    pub(super) struct Com(bool);

    impl Com {
        pub(super) fn enter() -> Option<Com> {
            // SAFETY: a plain COM set-up call for the calling thread, paired with `CoUninitialize` in `Drop` exactly
            // when it succeeded.
            let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            if result.is_ok() {
                Some(Com(true))
            } else if result == RPC_E_CHANGED_MODE {
                Some(Com(false))
            } else {
                None
            }
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: pairs the successful `CoInitializeEx` of `enter`, on the same thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// The handlers Windows recommends for `ext` (".docx"), in its order. None when it reports none.
    pub(super) fn handlers(ext: &str) -> Vec<IAssocHandler> {
        // SAFETY: COM is set up on this thread by the caller (`Com`). Each interface is owned by its wrapper and
        // released when it drops. `Next` writes at most one handler into the one slot it is given.
        unsafe {
            let Ok(list) = SHAssocEnumHandlers(&HSTRING::from(ext), ASSOC_FILTER_RECOMMENDED) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            // A bound, in case a list never ends: far more than the submenu shows.
            while out.len() < 64 {
                let mut one: [Option<IAssocHandler>; 1] = [None];
                let mut fetched = 0u32;
                if list.Next(&mut one, Some(&raw mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                if let Some(handler) = one[0].take() {
                    out.push(handler);
                }
            }
            out
        }
    }

    /// One string of the default association of `ext` (its `open` verb), as `AssocQueryStringW` answers it. `None`:
    /// no association, or an empty answer.
    pub(super) fn assoc(ext: &str, what: windows::Win32::UI::Shell::ASSOCSTR) -> Option<String> {
        use windows::Win32::UI::Shell::{ASSOCF_INIT_IGNOREUNKNOWN, ASSOCF_NOTRUNCATE, AssocQueryStringW};
        use windows::core::w;
        let ext = HSTRING::from(ext);
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` is `len` u16s long and outlives the call; the strings are nul-terminated HSTRINGs / literals.
        // The call writes at most `len` u16s and sets `len` to the count written, the nul included.
        let ok = unsafe {
            AssocQueryStringW(
                ASSOCF_INIT_IGNOREUNKNOWN | ASSOCF_NOTRUNCATE,
                what,
                &ext,
                w!("open"),
                windows::core::PWSTR(buf.as_mut_ptr()),
                &raw mut len,
            )
        };
        if ok.is_err() {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let text = String::from_utf16_lossy(&buf[..end]);
        (!text.trim().is_empty()).then_some(text)
    }

    /// A handler's `(GetName, GetUIName)`. `None`: it gives one of them not.
    pub(super) fn names(handler: &IAssocHandler) -> Option<(String, String)> {
        // SAFETY: COM is set up on this thread by the caller. Each string Windows allocates is copied, then freed
        // with `CoTaskMemFree`, as the two calls' contract says.
        unsafe {
            let name = take(handler.GetName().ok()?);
            let label = take(handler.GetUIName().ok()?);
            Some((name, label))
        }
    }

    /// The text of a string Windows allocated, which is freed here.
    unsafe fn take(text: PWSTR) -> String {
        if text.is_null() {
            return String::new();
        }
        // SAFETY: a non-null, NUL-ended string from a COM call that hands its ownership to the caller.
        unsafe {
            let copy = String::from_utf16_lossy(text.as_wide());
            CoTaskMemFree(Some(text.0 as *const _));
            copy
        }
    }

    /// Start the handler named `app_id` on the file at `path` (the copy kept on this PC). The handlers are listed again
    /// here: one gone since the menu was drawn is refused. It starts only while `still()` holds inside the launch lock.
    pub(super) fn invoke(path: &Path, app_id: &str, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = super::extension_of(&name).ok_or(ActionError::Failed)?;
        let handler = handlers(&ext)
            .into_iter()
            .find(|handler| names(handler).is_some_and(|(name, _)| name == app_id))
            .ok_or(ActionError::Failed)?;
        // SAFETY: COM is set up on this thread by the caller. The path is one wide string (never a command line);
        // every interface is owned by its wrapper and released when it drops.
        unsafe {
            let item: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).map_err(|_| ActionError::Failed)?;
            let data: IDataObject = item
                .BindToHandler(None, &BHID_DataObject)
                .map_err(|_| ActionError::Failed)?;
            launch::locked(|| still().then(|| handler.Invoke(&data)))
                .ok_or(ActionError::Unavailable)?
                .map_err(|_| ActionError::Failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(name: &str, label: &str) -> (String, String) {
        (name.to_string(), label.to_string())
    }

    #[test]
    fn apps_for_leaves_out_beebeeb_and_caps_the_list() {
        let own = r"C:\Program Files\Beebeeb\Beebeeb.exe";
        let mut reported: Vec<(String, String)> = (1..=9)
            .map(|n| handler(&format!(r"C:\Apps\app{n}.exe"), &format!("App {n}")))
            .collect();
        // The app's own program, reported third (another path, another case: still ours).
        reported.insert(2, handler(r"c:\program files\beebeeb\BEEBEEB.EXE", "Beebeeb"));
        assert_eq!(reported.len(), 10);
        let apps = choices(reported, own);
        assert_eq!(apps.len(), MAX_APPS);
        assert!(
            apps.iter().all(|app| !app.id.to_lowercase().ends_with("beebeeb.exe")),
            "{apps:?}"
        );
        // Windows' order is kept: app1, app2, then app3 (ours was between them).
        let labels: Vec<&str> = apps.iter().map(|app| app.label.as_str()).collect();
        assert_eq!(
            labels,
            ["App 1", "App 2", "App 3", "App 4", "App 5", "App 6", "App 7", "App 8"]
        );
        assert_eq!(apps[0].id, r"C:\Apps\app1.exe");
        // A handler with no words is not a line of the menu.
        assert_eq!(choices([handler("a.exe", " ")], own), []);
    }

    #[test]
    fn default_app_name_leaves_out_this_app_and_empty_names() {
        let own = r"C:\Program Files\Beebeeb\beebeeb-drive.exe";
        let ppt = r"C:\Program Files\Microsoft Office\root\Office16\POWERPNT.EXE";
        assert_eq!(default_name(Some((ppt.into(), "PowerPoint".into())), own), Some("PowerPoint".into()));
        assert_eq!(default_name(Some((own.into(), "Beebeeb".into())), own), None);
        assert_eq!(default_name(Some((ppt.into(), "  ".into())), own), None);
        assert_eq!(default_name(None, own), None);
    }

    #[test]
    fn open_with_extension_is_windows_own_form() {
        assert_eq!(extension_of("Service agreement.docx").as_deref(), Some(".docx"));
        assert_eq!(extension_of("REPORT.XLSX").as_deref(), Some(".xlsx"));
        assert_eq!(extension_of("notes"), None);
        assert_eq!(extension_of(".hidden"), None);
    }

    #[test]
    fn open_with_apps_are_asked_once_per_type_and_held() {
        let apps = Apps::default();
        let (woke, heard) = std::sync::mpsc::channel();
        let first = apps.for_name("a.docx", move || woke.send(()).unwrap());
        assert_eq!(first, [], "nothing is held before Windows answered");
        heard
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the answer wakes the window");
        let (woke, heard) = std::sync::mpsc::channel::<()>();
        apps.for_name("b.DOCX", move || woke.send(()).unwrap());
        assert!(
            heard.recv_timeout(std::time::Duration::from_millis(200)).is_err(),
            "a type is asked once"
        );
    }

    /// The instrument first, on a type every Windows has: `.txt` has handlers, Notepad among them.
    #[cfg(windows)]
    #[test]
    #[ignore = "O-CP3: the Windows test machine"]
    fn windows_reports_handlers_for_txt() {
        let _com = win::Com::enter().expect("COM");
        let reported: Vec<(String, String)> = win::handlers(".txt").iter().filter_map(win::names).collect();
        assert!(!reported.is_empty(), "Windows reports no handler for .txt");
        assert!(
            reported.iter().any(|(name, _)| name.to_lowercase().contains("notepad")),
            "{reported:?}"
        );
    }

    /// A scratch `.txt` in a temp root opened in Notepad through its handler: a Notepad process appears, and is ended
    /// by its PID. Mutant: invoke with an empty data object → no process.
    #[cfg(windows)]
    #[test]
    #[ignore = "O-CP3: the Windows test machine"]
    fn invoke_opens_the_kept_path() {
        let notepads = || -> Vec<u32> {
            let out = std::process::Command::new("tasklist")
                .args(["/FI", "IMAGENAME eq notepad.exe", "/FO", "CSV", "/NH"])
                .output()
                .expect("tasklist");
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.split("\",\"").nth(1)?.parse().ok())
                .collect()
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("open-with scratch.txt");
        std::fs::write(&path, "open with").unwrap();
        let before = notepads();
        let _com = win::Com::enter().expect("COM");
        let notepad = win::handlers(".txt")
            .iter()
            .filter_map(win::names)
            .find(|(name, _)| name.to_lowercase().contains("notepad"))
            .expect("Notepad is a handler of .txt")
            .0;
        win::invoke(&path, &notepad, &|| true).expect("invoked");
        let started = (0..50)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(100));
                notepads().into_iter().find(|pid| !before.contains(pid))
            })
            .expect("a Notepad process appeared");
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &started.to_string(), "/F"])
            .output();
        assert!(
            win::invoke(&path, "no such handler", &|| true).is_err(),
            "a handler that is not there is refused"
        );
    }
}
