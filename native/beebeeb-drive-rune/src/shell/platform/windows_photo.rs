//! Windows' own file picker, for "Choose photo" in the compare view of photo previews (DW-0209, DW-0745).
//!
//! `IFileOpenDialog` for one file of the file system, filtered to the pictures this PC decodes (JPEG, PNG, WebP,
//! GIF, BMP), opening in the sync folder. Windows draws it on the calling thread, which it blocks until the person
//! closed it: a worker's call, never the window's thread. The file is only READ on this PC afterwards (to measure a
//! preview of it); nothing is uploaded for it, nothing is written.
//!
//! Must not: log the file or its folder.

use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_CANCELLED, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FileOpenDialog, IFileOpenDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING, PCWSTR};

use super::PlatformError;

/// The pictures offered: the ones the engine decodes for a preview.
const PATTERN: &str = "*.jpg;*.jpeg;*.png;*.webp;*.gif;*.bmp";

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
    PlatformError::Os(format!("the photo picker: {what} (0x{:08X})", code.0 as u32))
}

/// Open the picker with `title`, in `start` when that folder exists. `Ok(None)`: closed without a file. It has no
/// owner window: the window behind it stays usable while it is open.
pub fn pick_photo(title: &str, start: &Path) -> Result<Option<PathBuf>, PlatformError> {
    let _com = Com::enter()?;
    let name = HSTRING::from("Pictures");
    let spec = HSTRING::from(PATTERN);
    let filters = [COMDLG_FILTERSPEC {
        pszName: PCWSTR(name.as_ptr()),
        pszSpec: PCWSTR(spec.as_ptr()),
    }];
    // SAFETY: COM is set up on this thread for the whole block (`_com`). Every interface pointer is owned by its
    // wrapper and released when it drops, before `_com`. The filter strings outlive the dialog (`name`, `spec`).
    // The one raw allocation (`GetDisplayName`'s string) is copied and then freed with `CoTaskMemFree`.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| os("could not be made", error.code()))?;
        let options = dialog
            .GetOptions()
            .map_err(|error| os("its options could not be read", error.code()))?;
        dialog
            .SetOptions(options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_NOCHANGEDIR)
            .map_err(|error| os("its options could not be set", error.code()))?;
        dialog
            .SetFileTypes(&filters)
            .map_err(|error| os("its filter could not be set", error.code()))?;
        let _ = dialog.SetTitle(&HSTRING::from(title));
        if start.is_dir() {
            let shown: Result<IShellItem, _> = SHCreateItemFromParsingName(&HSTRING::from(start.as_os_str()), None);
            if let Ok(shown) = shown {
                let _ = dialog.SetFolder(&shown);
            }
        }
        match dialog.Show(None) {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
            Err(error) => return Err(os("did not open", error.code())),
        }
        let picked = dialog.GetResult().map_err(|error| os("gave no file", error.code()))?;
        let name = picked
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|error| os("gave no path", error.code()))?;
        let file = PathBuf::from(std::ffi::OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(Some(file))
    }
}
