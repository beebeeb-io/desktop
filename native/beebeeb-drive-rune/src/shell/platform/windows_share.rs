//! Windows' Share sheet for one file ("Send a copy").
//!
//! `IDataTransferManagerInterop::ShowShareUIForWindow` over the app's window, with the file as a `StorageFile` in
//! the data package. On the window's own thread: the sheet belongs to the window, and `DataRequested` fires there.
//! The sheet is shown and this returns; the person finishes (or closes) it on their own time.
//!
//! The data package wants an `IIterable<IStorageItem>`, which `windows` 0.58 cannot build from a `Vec`. The one
//! the system hands out is a folder's item query, filtered to the file's own name: a one-item list, no copy.
//!
//! Must not: log the path or the file's name.

use std::cell::RefCell;
use std::path::Path;

use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Search::QueryOptions;
use windows::Storage::StorageFolder;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::IDataTransferManagerInterop;
use windows::core::{HSTRING, factory};

use super::PlatformError;

fn os(what: &str, error: &windows::core::Error) -> PlatformError {
    PlatformError::Os(format!("the share sheet: {what} (0x{:08X})", error.code().0 as u32))
}

thread_local! {
    /// The `DataRequested` registration of the last sheet, taken off before the next one is added: the manager is
    /// one per window, so handlers would pile up.
    static LAST: RefCell<Option<(DataTransferManager, windows::Foundation::EventRegistrationToken)>> = const { RefCell::new(None) };
}

/// Show the Share sheet over `window` for the file at `path`. Call on the window's thread.
pub fn share_file(window: Option<&rune::WindowHandle>, path: &Path) -> Result<(), PlatformError> {
    let hwnd = window
        .and_then(|window| {
            window
                .with_raw_handle(|handle| match handle.as_raw() {
                    raw_window_handle::RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
                    _ => None,
                })
                .ok()
                .flatten()
        })
        .ok_or(PlatformError::Os("the share sheet: the window has no handle yet".into()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(PlatformError::Os("the share sheet: the file has no usable name".into()))?
        .to_owned();
    let folder = path
        .parent()
        .ok_or(PlatformError::Os("the share sheet: the file has no folder".into()))?;
    // Windows file names cannot hold a quote, so the name is safe inside the query's quotes.
    if name.contains('"') {
        return Err(PlatformError::Os("the share sheet: the file's name cannot be shared".into()));
    }
    // SAFETY: plain WinRT/COM calls on the window's thread; every interface is owned by its wrapper.
    unsafe {
        let storage_folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(folder.as_os_str()))
            .and_then(|op| op.get())
            .map_err(|e| os("the folder could not be opened", &e))?;
        let options = QueryOptions::new()
            .map_err(|e| os("the query could not be made", &e))?;
        options
            .SetApplicationSearchFilter(&HSTRING::from(format!("System.FileName:=\"{name}\"")))
            .map_err(|e| os("the query could not be set", &e))?;
        let query = storage_folder
            .CreateItemQueryWithOptions(&options)
            .map_err(|e| os("the query could not start", &e))?;
        let items = query
            .GetItemsAsync(0, 2)
            .and_then(|op| op.get())
            .map_err(|e| os("the file could not be read", &e))?;
        if items.Size().unwrap_or(0) != 1 {
            return Err(PlatformError::Os("the share sheet: the file was not found".into()));
        }
        let title = HSTRING::from(name);
        let interop = factory::<DataTransferManager, IDataTransferManagerInterop>()
            .map_err(|e| os("is not available", &e))?;
        let manager: DataTransferManager = interop
            .GetForWindow(HWND(hwnd as *mut _))
            .map_err(|e| os("could not be made for the window", &e))?;
        LAST.with(|last| {
            if let Some((old, token)) = last.borrow_mut().take() {
                let _ = old.RemoveDataRequested(token);
            }
        });
        let handler = TypedEventHandler::new(move |_: &Option<DataTransferManager>, args: &Option<DataRequestedEventArgs>| {
            if let Some(args) = args {
                let data = args.Request()?.Data()?;
                data.Properties()?.SetTitle(&title)?;
                data.SetStorageItems(&items, true)?;
            }
            Ok(())
        });
        let token = manager
            .DataRequested(&handler)
            .map_err(|e| os("could not listen", &e))?;
        LAST.with(|last| *last.borrow_mut() = Some((manager, token)));
        interop
            .ShowShareUIForWindow(HWND(hwnd as *mut _))
            .map_err(|e| os("did not open", &e))
    }
}
