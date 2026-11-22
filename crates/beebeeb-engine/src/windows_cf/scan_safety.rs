//! Backlog 1658: Windows Search and a Defender scan pass over a cloud-only file without hydrating it.
//!
//! Both honour the attributes Cloud Files puts on a dehydrated placeholder: the indexer
//! (SearchProtocolHost) indexes a file with `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` by its properties only,
//! and a Defender scheduled or full scan skips its content (it scans the bytes when they land, through
//! real-time protection). Both also open with `FILE_FLAG_OPEN_NO_RECALL`, which never asks the provider for
//! data. What would make them hydrate is a placeholder WITHOUT the recall bit, or one the user pinned
//! ("Always keep on this device"), which the platform hydrates by itself. So a cloud-only placeholder is
//! minted with no pin of its own (`placeholder_create.rs`: it inherits its folder's) and the platform marks
//! it recall-on-data-access; [`scanners_skip_contents`] is the rule, and the native test below checks a
//! minted placeholder against it and that a scanner-style crawl asks for no bytes.

#![cfg(target_os = "windows")]

/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`: reading the data stream asks the provider for the bytes.
pub const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
/// `FILE_ATTRIBUTE_PINNED`: the platform keeps (and so fetches) the file's bytes.
pub const PINNED: u32 = 0x0008_0000;
/// `FILE_ATTRIBUTE_OFFLINE`: recorded by the native test only; neither scanner needs it.
pub const OFFLINE: u32 = 0x0000_1000;

/// Whether Windows Search and Defender pass over a file with `attributes` without its bytes: it is marked
/// recall-on-data-access (the bytes are in the cloud) and not pinned (nothing hydrates it in the background).
pub fn scanners_skip_contents(attributes: u32) -> bool {
    attributes & RECALL_ON_DATA_ACCESS != 0 && attributes & PINNED == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_cf::callbacks::fail_transfer;
    use crate::windows_cf::placeholders;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use windows::Win32::Storage::CloudFilters::*;
    use windows::core::PCWSTR;

    #[test]
    fn the_rule_wants_recall_and_no_pin() {
        assert!(scanners_skip_contents(RECALL_ON_DATA_ACCESS | 0x80));
        assert!(scanners_skip_contents(RECALL_ON_DATA_ACCESS | OFFLINE));
        assert!(!scanners_skip_contents(0x80), "a file with its bytes is scanned, and needs no fetch");
        assert!(!scanners_skip_contents(RECALL_ON_DATA_ACCESS | PINNED), "a pinned file is hydrated anyway");
    }

    static FETCHES: AtomicUsize = AtomicUsize::new(0);

    /// Counts every FETCH_DATA and fails it, so a test reads how often anything asked for bytes.
    unsafe extern "system" fn counting_fetch(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
        if info.is_null() || params.is_null() {
            return;
        }
        FETCHES.fetch_add(1, Ordering::SeqCst);
        // SAFETY: both are valid for the duration of the callback (the Cloud Files contract).
        let info = unsafe { &*info };
        let fetch = unsafe { (*params).Anonymous.FetchData };
        // SAFETY: the keys of this live request.
        unsafe {
            fail_transfer(
                info.ConnectionKey,
                info.TransferKey,
                info.RequestKey,
                fetch.RequiredFileOffset,
                fetch.RequiredLength,
            )
        };
    }

    struct Registered(std::path::PathBuf);
    impl Drop for Registered {
        fn drop(&mut self) {
            let _ = crate::windows_cf::unregister_any_sync_root(&self.0);
        }
    }
    struct Connected(CF_CONNECTION_KEY);
    impl Drop for Connected {
        fn drop(&mut self) {
            // SAFETY: the key of the connection made below, disconnected once.
            let _ = unsafe { CfDisconnectSyncRoot(self.0) };
        }
    }

    /// Every file under `dir`, as a crawler walks it.
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), out);
            } else {
                out.push(entry.path());
            }
        }
    }

    /// Backlog 1658 on a scratch sync root connected with a counting provider: minted cloud-only
    /// placeholders carry the bits the indexer and Defender honour, and a crawl that reads them the way
    /// those scanners do (attributes, properties, an `OPEN_NO_RECALL` read) asks for 0 bytes. The control,
    /// a plain read, does ask: it shows the counter sees a hydrating read.
    #[test]
    fn task_1658_a_scanner_crawl_hydrates_no_cloud_only_file() {
        const FILE_FLAG_OPEN_NO_RECALL: u32 = 0x0010_0000;
        const FILES: usize = 40;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        crate::windows_cf::register_test_sync_root(&root).unwrap();
        let _registered = Registered(root.clone());
        let table = [
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_FETCH_DATA,
                Callback: Some(counting_fetch),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NONE,
                Callback: None,
            },
        ];
        let root_wide: Vec<u16> = root.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: the path and the table outlive the call; the callback is a static function.
        let key = unsafe { CfConnectSyncRoot(PCWSTR(root_wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_NONE) }
            .expect("connect the scratch root");
        let _connected = Connected(key);

        let sub = root.join("docs");
        assert!(placeholders::create_placeholder(&root, "dir-docs", "docs", 0, 1_700_000_000, true).unwrap());
        for i in 0..FILES {
            let parent = if i % 2 == 0 { &root } else { &sub };
            let name = format!("file-{i}.txt");
            assert!(placeholders::create_placeholder(parent, &format!("id-{i}"), &name, 64 * 1024, 1_700_000_000, false).unwrap());
        }

        let mut files = Vec::new();
        walk(&root, &mut files);
        assert_eq!(files.len(), FILES, "the crawl sees every placeholder");
        let mut offline = 0;
        for path in &files {
            let attributes = placeholders::read_os_placeholder_state(path).unwrap();
            assert!(
                scanners_skip_contents(attributes),
                "a minted cloud-only placeholder is recall-on-data-access and not pinned: {attributes:#x}"
            );
            offline += usize::from(attributes & OFFLINE != 0);
            // Properties, as the indexer reads them.
            let meta = std::fs::metadata(path).unwrap();
            assert_eq!(meta.len(), 64 * 1024, "the size is the placeholder's, without its bytes");
            // A scanner's read: OPEN_NO_RECALL never asks the provider for data.
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_OPEN_NO_RECALL)
                .open(path)
            {
                let mut buf = [0u8; 4096];
                let _ = std::io::Read::read(&mut file, &mut buf);
            }
        }
        println!("task_1658 files={} offline={offline}", files.len());
        assert_eq!(FETCHES.load(Ordering::SeqCst), 0, "a scanner-style crawl asked for bytes");
        for path in &files {
            let attributes = placeholders::read_os_placeholder_state(path).unwrap();
            assert!(scanners_skip_contents(attributes), "still cloud-only after the crawl: {attributes:#x}");
        }

        // The control: a plain read asks the provider (which refuses), so the counter sees hydration.
        let control = std::fs::File::open(&files[0]).and_then(|mut f| std::io::Read::read(&mut f, &mut [0u8; 1]));
        assert!(control.is_err(), "the refusing provider fails the plain read: {control:?}");
        assert!(FETCHES.load(Ordering::SeqCst) >= 1, "a plain read asks for bytes");
    }
}
