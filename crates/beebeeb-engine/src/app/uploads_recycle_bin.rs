//! The move of ONE file to the Recycle Bin for Cancel upload, never a delete for good.
//!
//! The file is the only copy (it was never uploaded), so the shell's own answer decides, per item, at the moment of
//! the delete: the operation is asked to recycle (`FOFX_RECYCLEONDELETE`, `FOF_ALLOWUNDO`) with no UI at all, and a
//! progress sink is told before the delete whether the item WILL be recycled (`TSF_DELETE_RECYCLE_IF_POSSIBLE` in
//! `PreDeleteItem`'s flags). When it will not (a volume with no Recycle Bin, a file over the bin's limit, a bin
//! switched off for that volume), the sink stops the delete and the move is refused. A move counts as done only when
//! the shell also said after the delete that the item is in the bin (`PostDeleteItem` with a new item), the
//! operation reported no failure and no abort, and the file is no longer at its place. Anything else (COM could not
//! start, the item could not be made, any shell error) is a refusal: there is no fallback to another delete.
//!
//! `IFileOperation` needs a single-threaded apartment, so each move runs on a thread of its own that starts and ends
//! COM itself, whatever apartment the caller's thread is in.
//!
//! Must not: log the path or the name.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::E_ABORT;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOFX_EARLYFAILURE,
    FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl,
    IShellItem, SHCreateItemFromParsingName, TSF_DELETE_RECYCLE_IF_POSSIBLE,
};
use windows::core::{HRESULT, PCWSTR, Result as WinResult, implement};

/// What the shell told the sink about the one item.
#[derive(Default)]
struct Told {
    /// Before the delete: the item would NOT have been recycled, and the delete was stopped.
    would_not_recycle: AtomicBool,
    /// After the delete: it worked and the item is in the Recycle Bin.
    in_bin: AtomicBool,
}

/// The progress sink: stops any delete the shell would not recycle. Every other notice is taken as it comes.
#[implement(IFileOperationProgressSink)]
struct BinOnly {
    told: Arc<Told>,
}

impl IFileOperationProgressSink_Impl for BinOnly_Impl {
    fn StartOperations(&self) -> WinResult<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hrresult: HRESULT) -> WinResult<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _dwflags: u32, _psiitem: Option<&IShellItem>, _psznewname: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psznewname: &PCWSTR,
        _hrrename: HRESULT,
        _psinewlycreated: Option<&IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
        _hrmove: HRESULT,
        _psinewlycreated: Option<&IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
        _hrcopy: HRESULT,
        _psinewlycreated: Option<&IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PreDeleteItem(&self, dwflags: u32, _psiitem: Option<&IShellItem>) -> WinResult<()> {
        if dwflags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
            // The shell would delete this item for good: it is stopped before anything happens to it.
            self.told.would_not_recycle.store(true, Ordering::SeqCst);
            return Err(E_ABORT.into());
        }
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _dwflags: u32,
        _psiitem: Option<&IShellItem>,
        hrdelete: HRESULT,
        psinewlycreated: Option<&IShellItem>,
    ) -> WinResult<()> {
        // The new item is the file as it now stands in the Recycle Bin; a delete for good has none.
        if hrdelete.is_ok() && psinewlycreated.is_some() {
            self.told.in_bin.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    fn PreNewItem(
        &self,
        _dwflags: u32,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
    ) -> WinResult<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _dwflags: u32,
        _psidestinationfolder: Option<&IShellItem>,
        _psznewname: &PCWSTR,
        _psztemplatename: &PCWSTR,
        _dwfileattributes: u32,
        _hrnew: HRESULT,
        _psinewitem: Option<&IShellItem>,
    ) -> WinResult<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _iworktotal: u32, _iworksofar: u32) -> WinResult<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> WinResult<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> WinResult<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> WinResult<()> {
        Ok(())
    }
}

fn refused(why: &'static str) -> std::io::Error {
    std::io::Error::other(why)
}

/// See the module's doc. Runs on a thread of its own and waits for it.
pub(super) fn move_one(path: &Path) -> std::io::Result<()> {
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .name("recycle-bin".into())
        .spawn(move || in_its_own_apartment(&path))
        .map_err(|_| refused("the move to the Recycle Bin could not start"))?
        .join()
        .map_err(|_| refused("the move to the Recycle Bin ended early"))?
}

fn in_its_own_apartment(path: &Path) -> std::io::Result<()> {
    // SAFETY: a thread of our own; every `CoInitializeEx` that succeeded is balanced below, after every COM object
    // made inside `move_with_the_shell` was released (they live in its frame).
    let started = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if started.is_err() {
        return Err(refused("COM could not start for the move to the Recycle Bin"));
    }
    let moved = move_with_the_shell(path);
    // SAFETY: balances the successful `CoInitializeEx` above on the same thread.
    unsafe { CoUninitialize() };
    moved
}

fn move_with_the_shell(path: &Path) -> std::io::Result<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let told = Arc::new(Told::default());
    let sink: IFileOperationProgressSink = BinOnly { told: told.clone() }.into();
    // SAFETY: `wide` is NUL-terminated and outlives the call; every interface is used on this thread, in its
    // single-threaded apartment, and released at the end of this frame.
    let performed = unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
            .map_err(|_| refused("the file could not be handed to the shell"))?;
        let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)
            .map_err(|_| refused("the shell's file operation could not be made"))?;
        operation
            .SetOperationFlags(FILEOPERATION_FLAGS(
                FOF_ALLOWUNDO.0
                    | FOFX_RECYCLEONDELETE.0
                    | FOF_NOCONFIRMATION.0
                    | FOF_NOERRORUI.0
                    | FOF_SILENT.0
                    | FOFX_EARLYFAILURE.0,
            ))
            .map_err(|_| refused("the shell refused the move's options"))?;
        let cookie = operation
            .Advise(&sink)
            .map_err(|_| refused("the shell refused the move's watcher"))?;
        let queued = operation.DeleteItem(&item, None);
        let performed = queued.and_then(|()| operation.PerformOperations());
        let aborted = operation.GetAnyOperationsAborted().map_or(true, |any| any.as_bool());
        let _ = operation.Unadvise(cookie);
        performed.is_ok() && !aborted
    };
    if told.would_not_recycle.load(Ordering::SeqCst) {
        return Err(refused("this file can't go to the Recycle Bin"));
    }
    if !performed || !told.in_bin.load(Ordering::SeqCst) {
        return Err(refused("the shell did not confirm the move to the Recycle Bin"));
    }
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused("the file is still at its place")),
    }
}
