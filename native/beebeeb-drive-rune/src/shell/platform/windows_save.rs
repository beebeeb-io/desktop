//! Windows' own save panel, for "Save to PC" (Photos) and "Save a copy…" / "Download to…" (the viewer).
//!
//! `IFileSaveDialog`: the panel Windows itself draws, on the calling thread, which it blocks until the person
//! closed it. A worker's call, never the window's thread. It has no owner window (the window behind it stays
//! usable, like the folder picker) and asks by itself before replacing a file (`FOS_OVERWRITEPROMPT`). Nothing is
//! written here: the caller saves to the path that comes back.
//!
//! Must not: log the name or the path.

use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_CANCELLED, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileSaveDialog, IFileSaveDialog,
    IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING};

use super::SaveAnswer;

/// COM on this thread for as long as the panel lives: taken back only when this call was the one that set it up.
struct Com(bool);

impl Com {
    fn enter() -> Option<Com> {
        // SAFETY: a plain COM set-up call for the calling thread; paired with `CoUninitialize` in `Drop` exactly
        // when it succeeded (S_OK or S_FALSE).
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

fn failed(what: &str) -> SaveAnswer {
    // Fixed words only: no name, no path.
    eprintln!("the save panel: {what}");
    SaveAnswer::Failed
}

/// Open the panel with `suggested_name` filled in, in `start` when that folder exists, else in its parent.
pub fn pick_save_path(suggested_name: &str, start: &Path) -> SaveAnswer {
    let Some(_com) = Com::enter() else {
        return failed("COM did not start");
    };
    // SAFETY: COM is set up on this thread for the whole block (`_com`). Every interface pointer is owned by its
    // wrapper and released when it drops, before `_com`. The one raw allocation (`GetDisplayName`'s string) is
    // copied and then freed with `CoTaskMemFree`, as its contract says.
    unsafe {
        let dialog: IFileSaveDialog = match CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) {
            Ok(dialog) => dialog,
            Err(_) => return failed("could not be made"),
        };
        let Ok(options) = dialog.GetOptions() else {
            return failed("its options could not be read");
        };
        if dialog
            .SetOptions(options | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR | FOS_OVERWRITEPROMPT)
            .is_err()
        {
            return failed("its options could not be set");
        }
        // The name and the folder it opens in are a help, not a condition.
        let _ = dialog.SetFileName(&HSTRING::from(suggested_name));
        let opens_in = if start.is_dir() {
            Some(start)
        } else {
            start.parent().filter(|parent| parent.is_dir())
        };
        if let Some(opens_in) = opens_in {
            let shown: Result<IShellItem, _> = SHCreateItemFromParsingName(&HSTRING::from(opens_in.as_os_str()), None);
            if let Ok(shown) = shown {
                let _ = dialog.SetFolder(&shown);
            }
        }
        match dialog.Show(None) {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return SaveAnswer::Cancelled,
            Err(_) => return failed("did not open"),
        }
        let Ok(picked) = dialog.GetResult() else {
            return failed("gave no file");
        };
        let Ok(name) = picked.GetDisplayName(SIGDN_FILESYSPATH) else {
            return failed("gave no path");
        };
        let path = PathBuf::from(std::ffi::OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0 as *const _));
        SaveAnswer::Chosen(path)
    }
}
