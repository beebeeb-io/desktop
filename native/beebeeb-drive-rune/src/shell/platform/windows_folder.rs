//! Windows' own folder picker (DW-0738: "the file picker: Windows draws this"), for the first run's sync folder.
//!
//! `IFileOpenDialog` with "pick folders": the dialog Windows itself draws, on the calling thread, which it blocks
//! until the person closed it. A worker's call, never the window's thread. Only folders of the file system can be
//! picked (no library, no "This PC"). Nothing is created, and nothing is read but the folder the person picked.
//!
//! Must not: log the folder.

use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_CANCELLED, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
    IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING};

use super::PlatformError;

/// COM on this thread for as long as the picker lives: taken back only when this call was the one that set it up.
struct Com(bool);

impl Com {
    fn enter() -> Result<Com, PlatformError> {
        // SAFETY: a plain COM set-up call for the calling thread; it is paired with `CoUninitialize` in `Drop`
        // exactly when it succeeded (S_OK or S_FALSE).
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if result.is_ok() {
            Ok(Com(true))
        } else if result == RPC_E_CHANGED_MODE {
            // The thread already has COM in another mode: usable, and not this call's to take back.
            Ok(Com(false))
        } else {
            Err(os("COM did not start", result))
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

fn os(what: &str, code: HRESULT) -> PlatformError {
    PlatformError::Os(format!("the folder picker: {what} (0x{:08X})", code.0 as u32))
}

/// Open the picker with `title`, showing `start`'s parent folder with `start`'s name filled in when that parent
/// exists. `Ok(None)`: the person closed it without picking.
pub fn pick_folder(title: &str, start: &Path) -> Result<Option<PathBuf>, PlatformError> {
    let _com = Com::enter()?;
    // SAFETY: COM is set up on this thread for the whole block (`_com`). Every interface pointer is owned by its
    // wrapper and released when it drops, before `_com`. The one raw allocation (`GetDisplayName`'s string) is
    // copied and then freed with `CoTaskMemFree`, as its contract says.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| os("could not be made", error.code()))?;
        let options = dialog
            .GetOptions()
            .map_err(|error| os("its options could not be read", error.code()))?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR)
            .map_err(|error| os("its options could not be set", error.code()))?;
        // The words and the folder it opens in are a help, not a condition: a picker without them still picks.
        let _ = dialog.SetTitle(&HSTRING::from(title));
        if let Some(parent) = start.parent().filter(|parent| parent.is_dir()) {
            let shown: Result<IShellItem, _> = SHCreateItemFromParsingName(&HSTRING::from(parent.as_os_str()), None);
            if let Ok(shown) = shown {
                let _ = dialog.SetFolder(&shown);
            }
        }
        match dialog.Show(None) {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
            Err(error) => return Err(os("did not open", error.code())),
        }
        let picked = dialog.GetResult().map_err(|error| os("gave no folder", error.code()))?;
        let name = picked
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|error| os("gave no path", error.code()))?;
        let folder = PathBuf::from(std::ffi::OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(Some(folder))
    }
}
