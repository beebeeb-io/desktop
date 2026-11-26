//! dm-65: what Beebeeb holds on a Mac, from the File Provider itself. On macOS a downloaded copy is the system's
//! materialised item in the File Provider domain, not a row in the engine's cache, so the cache sum reads 0. The
//! system lists the materialised items ([`Materialisation::items`]) and makes one cloud-only again
//! ([`Materialisation::evict`]); Storage sums the ones not kept, and Clear… evicts each.

use std::collections::HashMap;

/// One item the system holds on disk: the File Provider's identifier (the engine's file id) and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Item {
    pub id: String,
    pub bytes: u64,
}

/// The system's own account of what is downloaded. Blocks: a worker's.
pub(super) trait Materialisation: Send + Sync {
    /// Every materialised file. `Err`: the system could not say (the pane then says nothing, never a made-up 0).
    fn items(&self) -> Result<Vec<Item>, String>;
    /// Make one item cloud-only again.
    fn evict(&self, id: &str) -> Result<(), String>;
}

/// The downloaded copies Clear… may remove: the materialised ones that are not kept (Available offline stays).
/// `kept` is the engine's answer per file id (`StateDb::sizes_and_kept`); an item it does not know counts as a copy.
pub(super) fn removable(items: &[Item], kept: &HashMap<String, (i64, bool)>) -> Vec<Item> {
    items
        .iter()
        .filter(|item| !kept.get(&item.id).is_some_and(|(_, kept)| *kept))
        .cloned()
        .collect()
}

/// The bytes of those copies, for "Downloaded files".
pub(super) fn bytes_of(items: &[Item]) -> i64 {
    items.iter().fold(0_i64, |sum, item| {
        sum.saturating_add(i64::try_from(item.bytes).unwrap_or(i64::MAX))
    })
}

/// The copies Clear… may remove right now, asked of the system and of the engine's kept set. `Err`: the system
/// could not say.
pub(super) fn copies<E: super::drive_settings::DriveEngine>(
    engine: &E,
    source: &dyn Materialisation,
) -> Result<Vec<Item>, String> {
    let items = source.items()?;
    let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
    let kept = engine
        .local_state(|db| db.sizes_and_kept(&ids))
        .map_err(|_| "not handed out".to_string())?
        .map_err(|_| "kept".to_string())?;
    Ok(removable(&items, &kept))
}

/// Evict every item; the number that did not go. Each is tried even after one fails.
pub(super) fn evict_all(source: &dyn Materialisation, items: &[Item]) -> usize {
    items.iter().filter(|item| source.evict(&item.id).is_err()).count()
}

/// The system's source on a Mac; none elsewhere (the engine's cache is the truth there).
pub(super) fn system() -> Option<&'static dyn Materialisation> {
    // Tests never reach the real File Provider: they pass a fake.
    #[cfg(all(target_os = "macos", not(test)))]
    {
        static SYSTEM: System = System;
        Some(&SYSTEM)
    }
    #[cfg(any(test, not(target_os = "macos")))]
    {
        None
    }
}

#[cfg(all(target_os = "macos", not(test)))]
struct System;

#[cfg(all(target_os = "macos", not(test)))]
mod bridge {
    use crate::shell::platform::macos::macos_fp::bridge::BeebeebFpErrorC;
    use std::ffi::{c_char, CString};

    unsafe extern "C" {
        fn beebeeb_fp_materialised(buffer: *mut c_char, buffer_len: usize, out_error: *mut BeebeebFpErrorC) -> i32;
        fn beebeeb_fp_evict_item(identifier: *const c_char, out_error: *mut BeebeebFpErrorC) -> i32;
    }

    /// Room for about 1 800 items of 36-character ids; a longer list is refused rather than cut short.
    const LIST: usize = 1 << 20;

    pub fn list() -> Result<String, String> {
        let mut buffer = vec![0 as c_char; LIST];
        let mut error = BeebeebFpErrorC::zeroed();
        let count = unsafe { beebeeb_fp_materialised(buffer.as_mut_ptr(), buffer.len(), &mut error) };
        if count < 0 {
            return Err("The system could not list the downloaded files.".into());
        }
        let bytes: Vec<u8> = buffer.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        // The bridge bounds its copy: fewer lines than it counted means the buffer was too small.
        if text.lines().count() < count as usize {
            return Err("There are too many downloaded files to list.".into());
        }
        Ok(text)
    }

    pub fn evict(id: &str) -> Result<(), String> {
        let id = CString::new(id).map_err(|_| "bad item id".to_string())?;
        let mut error = BeebeebFpErrorC::zeroed();
        if unsafe { beebeeb_fp_evict_item(id.as_ptr(), &mut error) } < 0 {
            return Err("The system could not make the file online only.".into());
        }
        Ok(())
    }
}

#[cfg(all(target_os = "macos", not(test)))]
impl Materialisation for System {
    fn items(&self) -> Result<Vec<Item>, String> {
        Ok(parse(&bridge::list()?))
    }
    fn evict(&self, id: &str) -> Result<(), String> {
        bridge::evict(id)
    }
}

/// The bridge's list: "<id>\t<bytes>" per line. A line that does not read is skipped, never counted as 0.
#[cfg_attr(not(any(test, all(target_os = "macos", not(test)))), allow(dead_code))]
fn parse(text: &str) -> Vec<Item> {
    text.lines()
        .filter_map(|line| {
            let (id, bytes) = line.rsplit_once('\t')?;
            (!id.is_empty()).then_some(Item {
                id: id.to_string(),
                bytes: bytes.trim().parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
pub(super) mod fake {
    use super::*;
    use std::sync::Mutex;

    /// A source with a fixed list that records every evict.
    pub struct Fake {
        pub list: Vec<Item>,
        pub evicted: Mutex<Vec<String>>,
        pub fail: Vec<String>,
    }

    impl Fake {
        pub fn new(list: &[(&str, u64)]) -> Self {
            Fake {
                list: list
                    .iter()
                    .map(|(id, bytes)| Item {
                        id: id.to_string(),
                        bytes: *bytes,
                    })
                    .collect(),
                evicted: Mutex::new(Vec::new()),
                fail: Vec::new(),
            }
        }
    }

    impl Materialisation for Fake {
        fn items(&self) -> Result<Vec<Item>, String> {
            Ok(self.list.clone())
        }
        fn evict(&self, id: &str) -> Result<(), String> {
            self.evicted.lock().unwrap().push(id.to_string());
            if self.fail.iter().any(|f| f == id) {
                Err("no".into())
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Fake;
    use super::*;

    #[test]
    fn downloaded_is_the_sum_of_the_materialised_copies_that_are_not_kept() {
        let fake = Fake::new(&[("a", 367_000_000), ("b", 1_000), ("kept", 5_000_000)]);
        let kept = HashMap::from([
            ("kept".to_string(), (5_000_000, true)),
            ("a".to_string(), (367_000_000, false)),
        ]);
        let copies = removable(&fake.items().unwrap(), &kept);
        assert_eq!(copies.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(bytes_of(&copies), 367_001_000);
    }

    #[test]
    fn clear_evicts_each_copy_and_counts_the_failures() {
        let mut fake = Fake::new(&[("a", 1), ("b", 2), ("c", 3)]);
        fake.fail = vec!["b".into()];
        let items = fake.items().unwrap();
        assert_eq!(evict_all(&fake, &items), 1);
        assert_eq!(
            *fake.evicted.lock().unwrap(),
            ["a", "b", "c"],
            "one failure stops nothing"
        );
    }

    #[test]
    fn the_bridge_list_reads_and_skips_what_it_cannot() {
        let got = parse("id-1\t367\nid-2\t0\nbroken\tx\n\n\t5\n");
        assert_eq!(
            got,
            vec![
                Item {
                    id: "id-1".into(),
                    bytes: 367
                },
                Item {
                    id: "id-2".into(),
                    bytes: 0
                }
            ]
        );
    }
}
