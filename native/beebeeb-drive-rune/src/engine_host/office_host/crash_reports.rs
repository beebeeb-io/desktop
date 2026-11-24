//! Windows Error Reporting keeps no copy of the Office helper's memory.
//!
//! A helper that crashes or runs out of memory could otherwise leave a dump of its memory on disk, and that memory
//! holds the document's plaintext. The helper cannot change this itself (the sandbox gives it no registry), so the app
//! excludes the helper's executable from error reporting for this user before EVERY helper it starts: one value under
//! the current user's key (the lead's answer 4, 2026-11-15 19:03; the installer writes the same). It is written each
//! time, not once per process: a value removed during the session (a cleanup tool, a policy refresh) is back before the
//! next helper runs, and the write is cheap. The name written is the file name that is started.
//! Other operating systems have no such reporter here: nothing to do.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Under the current user's root: the list Windows Error Reporting reads, one DWORD value per executable name.
pub(crate) const KEY: &str = r"Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications";

/// The value's data: 1 = excluded.
pub(crate) const EXCLUDED: u32 = 1;

/// Writes of the value in this process (a test hook: every start writes).
static WRITES: AtomicUsize = AtomicUsize::new(0);

/// Excludes `exe_name` (a file name, no path). Called before every start. `Err` carries the system's code.
pub(crate) fn exclude(exe_name: &str) -> Result<(), u32> {
    write(exe_name)
}

/// How many times the value was written in this process.
pub(crate) fn writes() -> usize {
    WRITES.load(Ordering::SeqCst)
}

#[cfg(windows)]
fn write(exe_name: &str) -> Result<(), u32> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_DWORD, RegSetKeyValueW};

    WRITES.fetch_add(1, Ordering::SeqCst);
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };
    let (key, name) = (wide(KEY), wide(exe_name));
    let data = EXCLUDED.to_le_bytes();
    // SAFETY: both names are NUL-terminated UTF-16 that live through the call; `data` is exactly the 4 bytes given.
    // The key is created when it does not exist yet.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_DWORD,
            data.as_ptr().cast(),
            data.len() as u32,
        )
    };
    if status == ERROR_SUCCESS { Ok(()) } else { Err(status) }
}

#[cfg(not(windows))]
fn write(_exe_name: &str) -> Result<(), u32> {
    WRITES.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
