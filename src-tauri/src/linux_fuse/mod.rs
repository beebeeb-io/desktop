#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use fuser::{Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request};

use crate::engine_bridge::EngineBridge;
use crate::state_db::{FileEntry, FileStatus, StateDb};

use self::inode_map::InodeMap;

pub mod inode_map;

const ROOT_INO: u64 = 1;
const TTL: Duration = Duration::from_secs(1);

pub struct BeebeebFS {
    db: Arc<StateDb>,
    bridge: Arc<EngineBridge>,
    inodes: InodeMap,
    rt: tokio::runtime::Handle,
}

impl BeebeebFS {
    pub fn new(db: Arc<StateDb>, bridge: Arc<EngineBridge>, rt: tokio::runtime::Handle) -> Self {
        Self {
            db,
            bridge,
            inodes: InodeMap::new(),
            rt,
        }
    }

    fn entry_for_ino(&self, ino: u64) -> Option<FileEntry> {
        let path = self.inodes.path_for_ino(ino)?;
        self.db.get_file_by_path(&path).ok().flatten()
    }

    fn entry_for_name(&self, name: &OsStr) -> Option<FileEntry> {
        self.db.list_files().ok()?.into_iter().find(|entry| {
            Path::new(&entry.path)
                .file_name()
                .map(|candidate| candidate == name)
                .unwrap_or(false)
        })
    }
}

impl Filesystem for BeebeebFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent != ROOT_INO {
            reply.error(libc::ENOENT);
            return;
        }

        let Some(entry) = self.entry_for_name(name) else {
            reply.error(libc::ENOENT);
            return;
        };

        let ino = self.inodes.get_or_assign(&entry.path);
        reply.entry(&TTL, &file_attr(ino, &entry), 0);
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        if ino == ROOT_INO {
            reply.attr(&TTL, &dir_attr(ROOT_INO));
            return;
        }

        let Some(entry) = self.entry_for_ino(ino) else {
            reply.error(libc::ENOENT);
            return;
        };

        reply.attr(&TTL, &file_attr(ino, &entry));
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }

        let Some(entry) = self.entry_for_ino(ino) else {
            reply.error(libc::ENOENT);
            return;
        };

        let cache_path = std::env::temp_dir().join(format!("bb_{}", entry.file_id));
        if entry.status == FileStatus::CloudOnly {
            let bridge = self.bridge.clone();
            let file_id = entry.file_id.clone();
            // Clone the path into the hydrate future so `cache_path` stays owned
            // here for the `std::fs::read` below (the future moves what it borrows).
            let hydrate_path = cache_path.clone();
            let result = self
                .rt
                .block_on(async move { bridge.hydrate_file(&file_id, &hydrate_path).await });
            if result.is_err() {
                reply.error(libc::EIO);
                return;
            }
        }

        match std::fs::read(&cache_path) {
            Ok(data) => reply_read_slice(reply, &data, offset as usize, size as usize),
            Err(_) => reply.error(libc::EIO),
        }
    }

    fn readdir(&mut self, _req: &Request<'_>, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        if ino != ROOT_INO {
            reply.error(libc::ENOTDIR);
            return;
        }

        let files = match self.db.list_files() {
            Ok(files) => files,
            Err(_) => {
                reply.error(libc::EIO);
                return;
            }
        };

        let mut entries = vec![
            (ROOT_INO, fuser::FileType::Directory, ".".to_string()),
            (ROOT_INO, fuser::FileType::Directory, "..".to_string()),
        ];

        for entry in files {
            let Some(name) = Path::new(&entry.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            let ino = self.inodes.get_or_assign(&entry.path);
            entries.push((ino, fuser::FileType::RegularFile, name));
        }

        for (i, (ino, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(ino, (i + 1) as i64, kind, name) {
                break;
            }
        }
        reply.ok();
    }
}

fn reply_read_slice(reply: ReplyData, data: &[u8], offset: usize, size: usize) {
    if offset >= data.len() {
        reply.data(&[]);
        return;
    }

    let end = offset.saturating_add(size).min(data.len());
    reply.data(&data[offset..end]);
}

fn dir_attr(ino: u64) -> fuser::FileAttr {
    fuser::FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: fuser::FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

fn file_attr(ino: u64, entry: &FileEntry) -> fuser::FileAttr {
    let size = entry.size_bytes.max(0) as u64;
    let modified_at = entry.modified_at.max(0) as u64;

    fuser::FileAttr {
        ino,
        size,
        blocks: (size + 511) / 512,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH + Duration::from_secs(modified_at),
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: fuser::FileType::RegularFile,
        perm: 0o444,
        nlink: 1,
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

pub fn mount(sync_root: &Path, db: Arc<StateDb>, bridge: Arc<EngineBridge>) {
    let rt = tokio::runtime::Handle::current();
    let fs = BeebeebFS::new(db, bridge, rt);
    let options = vec![
        fuser::MountOption::RO,
        fuser::MountOption::FSName("beebeeb".to_string()),
        fuser::MountOption::AutoUnmount,
    ];
    fuser::mount2(fs, sync_root, &options).expect("FUSE mount failed");
}
