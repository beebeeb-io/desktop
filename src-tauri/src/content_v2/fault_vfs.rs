//! Test-only wrapper over the real platform SQLite VFS. xWrite changes the
//! volatile image; only a successful xSync advances that file's persisted image.
//! After an injected I/O error, freeze the images and replay them in a fresh
//! fixture directory. This models loss of unflushed writes, NOT physical power loss.
#![allow(unsafe_op_in_unsafe_fn)]
use super::*;
use rusqlite::ffi as f;
use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
};
#[derive(Default)]
struct Model {
    persisted: BTreeMap<PathBuf, Vec<u8>>,
    events: Vec<&'static str>,
    cut: Option<usize>,
    frozen: bool,
}
impl Model {
    fn event(&mut self, name: &'static str) -> bool {
        if self.frozen {
            return false;
        }
        self.events.push(name);
        if self.cut == Some(self.events.len()) {
            self.frozen = true;
            return false;
        }
        true
    }
}
#[repr(C)]
struct Registration {
    vfs: f::sqlite3_vfs,
    native: *mut f::sqlite3_vfs,
    model: Arc<Mutex<Model>>,
}
pub(super) struct FaultVfs {
    registration: Box<Registration>,
    name: CString,
}
impl FaultVfs {
    pub(super) fn new() -> Result<Self> {
        unsafe {
            let native = f::sqlite3_vfs_find(std::ptr::null());
            ensure!(!native.is_null(), "native VFS");
            let name = CString::new(format!("bb-slice1-{}", hex(&id())))?;
            let mut registration = Box::new(Registration {
                vfs: *native,
                native,
                model: Arc::new(Mutex::new(Model::default())),
            });
            registration.vfs.zName = name.as_ptr();
            registration.vfs.szOsFile = std::mem::size_of::<Shadow>() as c_int;
            registration.vfs.pNext = std::ptr::null_mut();
            registration.vfs.xOpen = Some(open);
            registration.vfs.xDelete = Some(delete);
            ensure!(
                f::sqlite3_vfs_register(&mut registration.vfs, 0) == f::SQLITE_OK,
                "VFS registration"
            );
            Ok(Self { registration, name })
        }
    }
    pub(super) fn name(&self) -> &str {
        self.name.to_str().unwrap()
    }
    pub(super) fn arm(&self, cut: usize) {
        let mut m = self.registration.model.lock().unwrap();
        m.events.clear();
        m.cut = Some(cut);
        m.frozen = false;
    }
    pub(super) fn event_count(&self) -> usize {
        self.registration.model.lock().unwrap().events.len()
    }
    pub(super) fn sync_count(&self) -> usize {
        self.registration
            .model
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|e| **e == "xSync")
            .count()
    }
    /// All connections must already be closed. Restores only this harness's
    /// established DB/WAL names; SHM is rebuilt by SQLite at the fresh destination.
    pub(super) fn reboot(&self, source: &Path, dest: &Path) -> Result<()> {
        fs::create_dir_all(dest)?;
        let m = self.registration.model.lock().unwrap();
        for (path, bytes) in &m.persisted {
            let relative = path.strip_prefix(source)?;
            let target = dest.join(relative);
            fs::create_dir_all(target.parent().context("snapshot parent")?)?;
            let mut file = fs::File::create(target)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Ok(())
    }
}
impl Drop for FaultVfs {
    fn drop(&mut self) {
        unsafe {
            f::sqlite3_vfs_unregister(&mut self.registration.vfs);
        }
    }
}
#[repr(C)]
struct Shadow {
    base: f::sqlite3_file,
    data: *mut FileData,
}
struct FileData {
    real: *mut f::sqlite3_file,
    layout: Layout,
    model: Arc<Mutex<Model>>,
    path: Option<PathBuf>,
}
unsafe fn data<'a>(file: *mut f::sqlite3_file) -> &'a mut FileData {
    &mut *(*(file as *mut Shadow)).data
}
unsafe fn methods(file: *mut f::sqlite3_file) -> &'static f::sqlite3_io_methods {
    &*(*data(file).real).pMethods
}
unsafe extern "C" fn open(
    vfs: *mut f::sqlite3_vfs,
    name: *const c_char,
    file: *mut f::sqlite3_file,
    flags: c_int,
    out: *mut c_int,
) -> c_int {
    let registration = &*(vfs as *mut Registration);
    let layout = Layout::from_size_align((*registration.native).szOsFile as usize, 16).unwrap();
    let real = alloc_zeroed(layout).cast::<f::sqlite3_file>();
    if real.is_null() {
        return f::SQLITE_NOMEM;
    }
    let rc = ((*registration.native).xOpen.unwrap())(registration.native, name, real, flags, out);
    if rc != f::SQLITE_OK {
        dealloc(real.cast(), layout);
        return rc;
    }
    let path = if name.is_null() {
        None
    } else {
        Some(PathBuf::from(CStr::from_ptr(name).to_string_lossy().as_ref()))
    };
    let state = Box::new(FileData {
        real,
        layout,
        model: registration.model.clone(),
        path,
    });
    let shadow = &mut *(file as *mut Shadow);
    shadow.data = Box::into_raw(state);
    shadow.base.pMethods = &IO;
    f::SQLITE_OK
}
unsafe extern "C" fn delete(vfs: *mut f::sqlite3_vfs, name: *const c_char, sync_dir: c_int) -> c_int {
    let registration = &*(vfs as *mut Registration);
    if !registration.model.lock().unwrap().event("xDelete") {
        return f::SQLITE_IOERR_DELETE;
    }
    let rc = ((*registration.native).xDelete.unwrap())(registration.native, name, sync_dir);
    if rc == f::SQLITE_OK && !name.is_null() {
        let path = PathBuf::from(CStr::from_ptr(name).to_string_lossy().as_ref());
        registration.model.lock().unwrap().persisted.remove(&path);
    }
    rc
}
unsafe extern "C" fn close(file: *mut f::sqlite3_file) -> c_int {
    let data = Box::from_raw((*(file as *mut Shadow)).data);
    let rc = ((*(*data.real).pMethods).xClose.unwrap())(data.real);
    dealloc(data.real.cast(), data.layout);
    (*(file as *mut Shadow)).base.pMethods = std::ptr::null();
    rc
}
unsafe extern "C" fn read(file: *mut f::sqlite3_file, out: *mut c_void, n: c_int, offset: i64) -> c_int {
    (methods(file).xRead.unwrap())(data(file).real, out, n, offset)
}
unsafe extern "C" fn write(file: *mut f::sqlite3_file, input: *const c_void, n: c_int, offset: i64) -> c_int {
    if !data(file).model.lock().unwrap().event("xWrite") {
        return f::SQLITE_IOERR_WRITE;
    }
    (methods(file).xWrite.unwrap())(data(file).real, input, n, offset)
}
unsafe extern "C" fn truncate(file: *mut f::sqlite3_file, size: i64) -> c_int {
    if !data(file).model.lock().unwrap().event("xTruncate") {
        return f::SQLITE_IOERR_TRUNCATE;
    }
    (methods(file).xTruncate.unwrap())(data(file).real, size)
}
unsafe extern "C" fn sync(file: *mut f::sqlite3_file, flags: c_int) -> c_int {
    if !data(file).model.lock().unwrap().event("xSync") {
        return f::SQLITE_IOERR_FSYNC;
    }
    let rc = (methods(file).xSync.unwrap())(data(file).real, flags);
    if rc != f::SQLITE_OK {
        return rc;
    }
    let state = data(file);
    if let Some(path) = &state.path {
        let mut size = 0;
        let rc = ((*(*state.real).pMethods).xFileSize.unwrap())(state.real, &mut size);
        if rc != 0 {
            return rc;
        }
        // Fault fixtures are small. Reject rather than allocating file-sized images
        // for the independent multi-GiB bounded-buffer workload.
        if !(0..=8 * 1024 * 1024).contains(&size) {
            return f::SQLITE_IOERR_FSYNC;
        }
        let mut bytes = vec![0; size as usize];
        if size > 0 {
            let rc =
                ((*(*state.real).pMethods).xRead.unwrap())(state.real, bytes.as_mut_ptr().cast(), size as c_int, 0);
            if rc != 0 {
                return rc;
            }
        }
        state.model.lock().unwrap().persisted.insert(path.clone(), bytes);
    }
    f::SQLITE_OK
}
unsafe extern "C" fn size(file: *mut f::sqlite3_file, out: *mut i64) -> c_int {
    (methods(file).xFileSize.unwrap())(data(file).real, out)
}
unsafe extern "C" fn lock(file: *mut f::sqlite3_file, n: c_int) -> c_int {
    (methods(file).xLock.unwrap())(data(file).real, n)
}
unsafe extern "C" fn unlock(file: *mut f::sqlite3_file, n: c_int) -> c_int {
    (methods(file).xUnlock.unwrap())(data(file).real, n)
}
unsafe extern "C" fn reserved(file: *mut f::sqlite3_file, out: *mut c_int) -> c_int {
    (methods(file).xCheckReservedLock.unwrap())(data(file).real, out)
}
unsafe extern "C" fn control(file: *mut f::sqlite3_file, op: c_int, arg: *mut c_void) -> c_int {
    (methods(file).xFileControl.unwrap())(data(file).real, op, arg)
}
unsafe extern "C" fn sector(file: *mut f::sqlite3_file) -> c_int {
    (methods(file).xSectorSize.unwrap())(data(file).real)
}
unsafe extern "C" fn device(file: *mut f::sqlite3_file) -> c_int {
    (methods(file).xDeviceCharacteristics.unwrap())(data(file).real)
}
unsafe extern "C" fn shm_map(
    file: *mut f::sqlite3_file,
    page: c_int,
    size: c_int,
    extend: c_int,
    out: *mut *mut c_void,
) -> c_int {
    (methods(file).xShmMap.unwrap())(data(file).real, page, size, extend, out)
}
unsafe extern "C" fn shm_lock(file: *mut f::sqlite3_file, offset: c_int, n: c_int, flags: c_int) -> c_int {
    (methods(file).xShmLock.unwrap())(data(file).real, offset, n, flags)
}
unsafe extern "C" fn shm_barrier(file: *mut f::sqlite3_file) {
    (methods(file).xShmBarrier.unwrap())(data(file).real)
}
unsafe extern "C" fn shm_unmap(file: *mut f::sqlite3_file, delete: c_int) -> c_int {
    (methods(file).xShmUnmap.unwrap())(data(file).real, delete)
}
// Disable mmap so every modeled database read/write traverses the wrapped VFS.
unsafe extern "C" fn fetch(_file: *mut f::sqlite3_file, _offset: i64, _size: c_int, out: *mut *mut c_void) -> c_int {
    *out = std::ptr::null_mut();
    f::SQLITE_OK
}
unsafe extern "C" fn unfetch(_file: *mut f::sqlite3_file, _offset: i64, _ptr: *mut c_void) -> c_int {
    f::SQLITE_OK
}
static IO: f::sqlite3_io_methods = f::sqlite3_io_methods {
    iVersion: 3,
    xClose: Some(close),
    xRead: Some(read),
    xWrite: Some(write),
    xTruncate: Some(truncate),
    xSync: Some(sync),
    xFileSize: Some(size),
    xLock: Some(lock),
    xUnlock: Some(unlock),
    xCheckReservedLock: Some(reserved),
    xFileControl: Some(control),
    xSectorSize: Some(sector),
    xDeviceCharacteristics: Some(device),
    xShmMap: Some(shm_map),
    xShmLock: Some(shm_lock),
    xShmBarrier: Some(shm_barrier),
    xShmUnmap: Some(shm_unmap),
    xFetch: Some(fetch),
    xUnfetch: Some(unfetch),
};
