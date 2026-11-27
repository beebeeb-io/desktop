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

/// Dm-75: what a row's "on this Mac" mark, the details' "Downloaded" line and Available offline say on a Mac come
/// from Finder's real state: the item is in the system's list of materialised items ([`Materialisation::items`]) or
/// its file in Finder is dataless. The engine's own record is not asked: the system makes a file online only again
/// without telling the engine, and the engine may hold a copy Finder does not.
///
/// The list is read on a thread of its own, never by the caller: an answer is the last list read, and asking starts
/// a new read when that one is older than [`finder_state::FRESH_FOR`]. Until the system has answered once nothing is
/// known ([`finder_state::here`] is `None`) and the engine's word stands.
pub(crate) mod finder_state {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use super::Materialisation;

    /// How old the list may be before it is read again.
    pub(crate) const FRESH_FOR: Duration = Duration::from_secs(3);

    /// The system's list at one read.
    #[derive(Default)]
    pub(crate) struct Held {
        here: Option<Arc<HashSet<String>>>,
        read_at: Option<Instant>,
        reading: bool,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
    }

    static HELD: Mutex<Held> = Mutex::new(Held {
        here: None,
        read_at: None,
        reading: false,
        wake: None,
    });

    impl Held {
        /// Whether Finder holds the bytes of `id`. `None`: the system has not said.
        pub(crate) fn here(&self, id: &str) -> Option<bool> {
            self.here.as_ref().map(|here| here.contains(id))
        }

        /// Takes one read of the system's list. `true`: what is held changed. A read that failed keeps the last
        /// list: a file is never called online only because the system could not say.
        pub(in crate::engine_host) fn take(&mut self, source: &dyn Materialisation) -> bool {
            self.read_at = Some(Instant::now());
            let Ok(items) = source.items() else {
                tracing::warn!("finder state: the downloaded files could not be listed");
                return false;
            };
            let now: HashSet<String> = items.into_iter().map(|item| item.id).collect();
            if self.here.as_deref() == Some(&now) {
                return false;
            }
            self.here = Some(Arc::new(now));
            true
        }
    }

    /// The window is woken when the list changed, so a row drawn from the last list is drawn again.
    pub(crate) fn set_wake(wake: impl Fn() + Send + Sync + 'static) {
        HELD.lock().unwrap_or_else(PoisonError::into_inner).wake = Some(Arc::new(wake));
    }

    /// Whether Finder holds the bytes of the file `id`, by the last list read; a new read is started when that one
    /// is old. `None`: not a Mac, or the system has not said yet. Never waits.
    pub(crate) fn here(id: &str) -> Option<bool> {
        let source = super::system()?;
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = held.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh && !held.reading {
            held.reading = true;
            let started = std::thread::Builder::new().name("finder-state".into()).spawn(move || {
                // The system is asked outside the lock (it can take seconds); a reader meanwhile gets the last list.
                let mut read = Held::default();
                let listed = read.take(source);
                let wake = {
                    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
                    held.reading = false;
                    held.read_at = read.read_at;
                    let changed = listed && held.here.as_deref() != read.here.as_deref();
                    if changed {
                        held.here = read.here;
                    }
                    changed.then(|| held.wake.clone()).flatten()
                };
                if let Some(wake) = wake {
                    wake();
                }
            });
            if started.is_err() {
                held.reading = false;
            }
        }
        held.here(id)
    }

    /// The mark a file's row gets (`pc_status`), from the engine's `mark` and Finder's state: a file Finder holds
    /// no bytes of is online only whatever the engine holds, and one Finder holds is on this Mac. A kept file keeps
    /// its own mark, and so does a row the engine marks another way (a shared place, nothing yet).
    pub(crate) fn mark(mark: Option<String>, here: Option<bool>) -> Option<String> {
        match (mark.as_deref(), here) {
            (Some("on_this_pc"), Some(false)) => Some("online_only".to_string()),
            (Some("online_only"), Some(true)) => Some("on_this_pc".to_string()),
            _ => mark,
        }
    }
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

    #[test]
    fn a_dataless_item_shows_the_cloud_mark_even_when_the_engine_holds_a_copy() {
        use super::finder_state::{mark, Held};
        // Finder holds "a" and "kept"; "mov" is dataless there, though the engine records a copy of it.
        let mut held = Held::default();
        assert_eq!(held.here("mov"), None, "nothing is known before the system answered");
        assert!(held.take(&Fake::new(&[("a", 10), ("kept", 5)])));
        let on = |m: &str| Some(m.to_string());
        assert_eq!(mark(on("on_this_pc"), held.here("mov")), on("online_only"));
        // And the other way: Finder downloaded a file the engine still calls online only.
        assert_eq!(mark(on("online_only"), held.here("a")), on("on_this_pc"));
        assert_eq!(mark(on("on_this_pc"), held.here("a")), on("on_this_pc"));
        // A kept file's mark, a shared place's and no mark at all are the engine's.
        assert_eq!(mark(on("available_offline"), held.here("mov")), on("available_offline"));
        assert_eq!(mark(on("name_shared"), held.here("mov")), on("name_shared"));
        assert_eq!(mark(None, held.here("mov")), None);
        // Until the system has answered, the engine's word stands.
        assert_eq!(mark(on("on_this_pc"), None), on("on_this_pc"));
    }

    #[test]
    fn a_list_the_system_could_not_give_keeps_the_last_one() {
        use super::finder_state::Held;
        struct Broken;
        impl Materialisation for Broken {
            fn items(&self) -> Result<Vec<Item>, String> {
                Err("no".into())
            }
            fn evict(&self, _id: &str) -> Result<(), String> {
                Ok(())
            }
        }
        let mut held = Held::default();
        assert!(held.take(&Fake::new(&[("a", 10)])));
        assert!(!held.take(&Broken), "nothing changed");
        assert_eq!((held.here("a"), held.here("b")), (Some(true), Some(false)));
        // The same list again is no change (no wake), a file made online only is one.
        assert!(!held.take(&Fake::new(&[("a", 10)])));
        assert!(held.take(&Fake::new(&[])));
        assert_eq!(held.here("a"), Some(false));
    }
}
