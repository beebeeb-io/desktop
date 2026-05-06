#![cfg(any(target_os = "linux", test))]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Stable path <-> inode mapping for the lifetime of a mounted FUSE filesystem.
pub struct InodeMap {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    path_to_ino: HashMap<String, u64>,
    ino_to_path: HashMap<u64, String>,
    next: u64,
}

impl InodeMap {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                path_to_ino: HashMap::new(),
                ino_to_path: HashMap::new(),
                next: 2,
            })),
        }
    }

    pub fn get_or_assign(&self, path: &str) -> u64 {
        let mut inner = self.inner.lock().expect("inode map mutex poisoned");
        if let Some(&ino) = inner.path_to_ino.get(path) {
            return ino;
        }

        let ino = inner.next;
        inner.next += 1;
        inner.path_to_ino.insert(path.to_string(), ino);
        inner.ino_to_path.insert(ino, path.to_string());
        ino
    }

    pub fn path_for_ino(&self, ino: u64) -> Option<String> {
        self.inner
            .lock()
            .expect("inode map mutex poisoned")
            .ino_to_path
            .get(&ino)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::InodeMap;

    #[test]
    fn test_inode_assignment_stable() {
        let m = InodeMap::new();
        let id1 = m.get_or_assign("/Beebeeb/file.txt");
        let id2 = m.get_or_assign("/Beebeeb/file.txt");
        assert_eq!(id1, id2);

        let id3 = m.get_or_assign("/Beebeeb/other.txt");
        assert_ne!(id1, id3);
        assert_eq!(m.path_for_ino(id1).as_deref(), Some("/Beebeeb/file.txt"));
    }
}
