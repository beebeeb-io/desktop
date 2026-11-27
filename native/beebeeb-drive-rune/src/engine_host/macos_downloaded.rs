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
    use std::ffi::{CString, c_char};

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
        bridge::evict(id)?;
        finder_state::forget(id);
        Ok(())
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
/// The list is read on a thread of its own, never by the caller: an answer is the last list read.
///
/// Dm-78: the mark comes from Finder's state alone. Until the system has answered once, the engine's word stood
/// (RC13: Finder setup was slow to start, the list could not be read, and two files the engine held a copy of from
/// an earlier build showed "on this Mac" while Finder's copies were dataless). Now a file is on this Mac only when
/// the system's list names it, or a materialise saw it land moments ago ([`finder_state::Held::answer`]): before
/// the first list, and while the list cannot be read at all, no engine copy makes a file "on this Mac". Only where
/// there is no system to ask (not a Mac, a test) does the engine's word stand ([`finder_state::here`] is `None`).
///
/// Dm-77: when the list is read again. A row's mark is taken when its list is read, so a mark is only as new as the
/// list was then (RC12: a video landed and its row kept the cloud mark, "0 on this Mac"). So:
/// - every ask is followed by a read that began after it (at most one read per [`finder_state::FRESH_FOR`]): what a
///   row was told is checked against Finder's state once more;
/// - a materialise that saw a file land says so at once ([`finder_state::landed`]): the file is held as here from
///   that moment, while the system's list may not name it yet ([`finder_state::LANDED_FOR`]);
/// - the File Provider's own signal of a change asks for a read ([`finder_state::refresh`]);
/// - whenever what is held changed, the window is woken and [`finder_state::take_changed`] answers `true` once: the
///   window's poll reads its lists again then, as after a finished check, and the marks and the counts are new.
pub(crate) mod finder_state {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use super::{Item, Materialisation};

    /// How old the list may be before it is read again.
    pub(crate) const FRESH_FOR: Duration = Duration::from_secs(3);
    /// How long a file a materialise saw land is held as here while the system's list does not name it yet.
    pub(crate) const LANDED_FOR: Duration = Duration::from_secs(30);

    /// The system's list at one read, and what was seen since.
    #[derive(Default)]
    pub(crate) struct Held {
        here: Option<Arc<HashSet<String>>>,
        read_at: Option<Instant>,
        reading: bool,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
        /// The files a materialise saw land (by their own attributes), and when: here whatever the list says, until
        /// the list names them or [`LANDED_FOR`] is over.
        landed: Vec<(String, Instant)>,
        /// When a row (or a signal) last asked.
        asked: Option<Instant>,
        /// What is held changed since the window last took it.
        changed: bool,
    }

    static HELD: Mutex<Held> = Mutex::new(Held {
        here: None,
        read_at: None,
        reading: false,
        wake: None,
        landed: Vec::new(),
        asked: None,
        changed: false,
    });

    impl Held {
        /// Whether Finder holds the bytes of `id`. `None`: the system has not said.
        pub(crate) fn here(&self, id: &str) -> Option<bool> {
            self.here.as_ref().map(|here| here.contains(id))
        }

        /// What a file's row is told on a Mac (dm-78): Finder holds the bytes of `id` only when the system's list
        /// names it, or, while no list names it yet, a materialise saw it land within [`LANDED_FOR`]. Never
        /// "not known": a file the system has not said is here is not marked as here, whatever the engine holds.
        pub(crate) fn answer(&self, id: &str) -> Option<bool> {
            Some(match self.here(id) {
                Some(here) => here,
                None => self
                    .landed
                    .iter()
                    .any(|(held, at)| held == id && at.elapsed() < LANDED_FOR),
            })
        }

        /// Takes one read of the system's list. `true`: what is held changed.
        #[cfg_attr(not(test), allow(dead_code))]
        pub(in crate::engine_host) fn take(&mut self, source: &dyn Materialisation) -> bool {
            self.apply(source.items())
        }

        /// One answer of the system's list. `true`: what is held changed. A read that failed keeps the last list: a
        /// file is never called online only because the system could not say. A file a materialise saw land stays
        /// here while the list does not name it yet.
        pub(in crate::engine_host) fn apply(&mut self, listed: Result<Vec<Item>, String>) -> bool {
            self.read_at = Some(Instant::now());
            let Ok(items) = listed else {
                tracing::warn!("finder state: the downloaded files could not be listed");
                return false;
            };
            let mut now: HashSet<String> = items.into_iter().map(|item| item.id).collect();
            self.landed
                .retain(|(id, at)| !now.contains(id) && at.elapsed() < LANDED_FOR);
            now.extend(self.landed.iter().map(|(id, _)| id.clone()));
            if self.here.as_deref() == Some(&now) {
                return false;
            }
            self.here = Some(Arc::new(now));
            self.changed = true;
            true
        }

        /// A materialise saw the file `id` on this Mac, by its own attributes. `true`: what is held changed. Before
        /// the system has answered once no list is made up from one file (the first list read is given it), but the
        /// file itself is here from now ([`Self::answer`]).
        pub(in crate::engine_host) fn land(&mut self, id: &str) -> bool {
            let was = self.answer(id);
            self.forget(id);
            self.landed.push((id.to_string(), Instant::now()));
            if let Some(here) = &mut self.here
                && !here.contains(id)
            {
                Arc::make_mut(here).insert(id.to_string());
            }
            let changed = was != self.answer(id);
            self.changed |= changed;
            changed
        }

        /// The file `id` is no longer held as landed (this app made it online only, or it lands anew).
        pub(in crate::engine_host) fn forget(&mut self, id: &str) {
            self.landed.retain(|(held, _)| held != id);
        }

        /// `true` once after what is held changed: the window reads its lists again.
        pub(in crate::engine_host) fn take_changed(&mut self) -> bool {
            std::mem::take(&mut self.changed)
        }

        /// Notes an ask, now.
        pub(in crate::engine_host) fn ask_now(&mut self) {
            self.asked = Some(Instant::now());
        }

        /// A read that began at `began` is over: was there an ask after it began? What that ask was told may be
        /// older than Finder's state, so one more read follows.
        pub(in crate::engine_host) fn asked_since(&self, began: Instant) -> bool {
            self.asked.is_some_and(|asked| asked > began)
        }
    }

    /// The window is woken when the list changed, so a row drawn from the last list is drawn again.
    pub(crate) fn set_wake(wake: impl Fn() + Send + Sync + 'static) {
        HELD.lock().unwrap_or_else(PoisonError::into_inner).wake = Some(Arc::new(wake));
    }

    /// Notes the ask and makes sure a read follows it: the reader is started unless one is at work (that one reads
    /// once more when it sees the ask). Called with the lock held; never waits.
    fn ask(held: &mut Held, source: &'static dyn Materialisation) {
        held.ask_now();
        if held.reading {
            return;
        }
        held.reading = true;
        let started = std::thread::Builder::new()
            .name("finder-state".into())
            .spawn(move || read_until_asked_no_more(source));
        if started.is_err() {
            held.reading = false;
        }
    }

    /// The reader: one read of the system's list per [`FRESH_FOR`], for as long as an ask came after the last read
    /// began. The system is asked outside the lock (it can take seconds); a row meanwhile gets the last list.
    fn read_until_asked_no_more(source: &'static dyn Materialisation) {
        loop {
            let fresh_for = {
                let held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
                held.read_at
                    .map_or(Duration::ZERO, |at| FRESH_FOR.saturating_sub(at.elapsed()))
            };
            if !fresh_for.is_zero() {
                std::thread::sleep(fresh_for);
            }
            let began = Instant::now();
            let listed = source.items();
            let (wake, again) = {
                let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
                let changed = held.apply(listed);
                let again = held.asked_since(began);
                held.reading = again;
                (changed.then(|| held.wake.clone()).flatten(), again)
            };
            if let Some(wake) = wake {
                wake();
            }
            if !again {
                return;
            }
        }
    }

    /// Whether Finder holds the bytes of the file `id`, by the last list read ([`Held::answer`]: `false` while the
    /// system has not said it is here); a read that begins after this ask follows. `None`: there is no system to
    /// ask (not a Mac, a test), and only then does the engine's word stand. Never waits.
    pub(crate) fn here(id: &str) -> Option<bool> {
        let source = super::system()?;
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        ask(&mut held, source);
        held.answer(id)
    }

    /// Dm-77: a materialise saw the file `id` land on this Mac (`macos_fp::materialise`). It is held as here at
    /// once, the window is woken, and the system's list is read again. Never waits.
    #[cfg_attr(any(test, not(target_os = "macos")), allow(dead_code))] // the Mac's own finder calls it
    pub(crate) fn landed(id: &str) {
        let Some(source) = super::system() else {
            return;
        };
        let wake = {
            let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
            let changed = held.land(id);
            ask(&mut held, source);
            changed.then(|| held.wake.clone()).flatten()
        };
        if let Some(wake) = wake {
            wake();
        }
    }

    /// Dm-77: the File Provider signalled a change (Finder downloaded a file, or made one online only): the system's
    /// list is read again, and the window is woken when it differs. Never waits.
    #[allow(dead_code)] // for the File Provider's signal (see the module's doc); nothing in this file's reach sends one
    pub(crate) fn refresh() {
        let Some(source) = super::system() else {
            return;
        };
        ask(&mut HELD.lock().unwrap_or_else(PoisonError::into_inner), source);
    }

    /// Dm-77: `true` once after Finder's state changed (a file landed, or the list read differs): the window's poll
    /// reads the lists it shows again, as after a finished check, so the marks and the counts are Finder's state now.
    #[allow(dead_code)] // the window's poll calls it (see the module's doc)
    pub(crate) fn take_changed() -> bool {
        HELD.lock().unwrap_or_else(PoisonError::into_inner).take_changed()
    }

    /// This app made the file `id` online only: it is not held as landed any longer.
    #[allow(dead_code)] // the Mac's own evict calls it; a test and another system have none
    pub(super) fn forget(id: &str) {
        HELD.lock().unwrap_or_else(PoisonError::into_inner).forget(id);
    }

    /// The mark a file's row gets (`pc_status`), from the engine's `mark` and Finder's state ([`here`]): a file
    /// Finder holds no bytes of is online only whatever the engine holds, and one Finder holds is on this Mac. A
    /// kept file keeps its own mark, and so does a row the engine marks another way (a shared place, nothing yet).
    /// `here` is `None` only where there is no system to ask: the engine's mark stands there.
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
        use super::finder_state::{Held, mark};
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
        // Where there is no system to ask (not a Mac), the engine's word stands.
        assert_eq!(mark(on("on_this_pc"), None), on("on_this_pc"));
    }

    /// Dm-78, RC13: Finder setup was slow to start and the system's list could not be read. Two files the engine
    /// held a copy of from an earlier build showed the on-this-Mac mark while `ls -lO` said `dataless`.
    #[test]
    fn an_engine_copy_never_marks_a_file_as_on_this_mac_before_or_without_the_systems_list() {
        use super::finder_state::{Held, mark};
        struct Broken;
        impl Materialisation for Broken {
            fn items(&self) -> Result<Vec<Item>, String> {
                Err("the domain is not there yet".into())
            }
            fn evict(&self, _id: &str) -> Result<(), String> {
                Ok(())
            }
        }
        let on = |m: &str| Some(m.to_string());
        // What the row of a file gets: the engine's mark, and what a Mac's row is told of Finder's state.
        let row = |held: &Held, engine: &str, id: &str| mark(on(engine), held.answer(id));

        // The system has not answered yet: the engine holds a copy of "zip", Finder's copy is dataless.
        let mut held = Held::default();
        assert_eq!(held.answer("zip"), Some(false), "never \"not known\" on a Mac");
        assert_eq!(row(&held, "on_this_pc", "zip"), on("online_only"));
        assert_eq!(row(&held, "online_only", "eps"), on("online_only"));
        // The list cannot be read (setup is still adding): the engine's copy still makes no mark.
        assert!(!held.take(&Broken));
        assert!(!held.take(&Broken));
        assert_eq!(row(&held, "on_this_pc", "zip"), on("online_only"));
        // A kept file's own mark is not this one.
        assert_eq!(row(&held, "available_offline", "zip"), on("available_offline"));

        // A materialise saw "mov" land meanwhile, by its own attributes: that is Finder's state, so it is marked,
        // and the window is told. Nothing else is.
        assert!(held.land("mov"));
        assert!(held.take_changed());
        assert_eq!(row(&held, "online_only", "mov"), on("on_this_pc"));
        assert_eq!(row(&held, "on_this_pc", "zip"), on("online_only"));

        // The system answers: its list is the mark, for the engine's copy too.
        assert!(held.take(&Fake::new(&[("mov", 384), ("docx", 12)])));
        assert_eq!(row(&held, "on_this_pc", "zip"), on("online_only"));
        assert_eq!(row(&held, "online_only", "docx"), on("on_this_pc"));
        assert_eq!(row(&held, "online_only", "mov"), on("on_this_pc"));
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

    /// Dm-77, RC12: a video landed in Finder and its row kept the cloud mark and "0 on this Mac" until a relaunch.
    #[test]
    fn a_completed_materialise_flips_the_mark_at_once_and_tells_the_window_to_read_its_lists_again() {
        use super::finder_state::{Held, mark};
        let on = |m: &str| Some(m.to_string());
        let mut held = Held::default();
        assert!(held.take(&Fake::new(&[("a", 10)])));
        assert!(held.take_changed(), "the first list is a change");
        assert!(!held.take_changed(), "told once");
        assert_eq!(mark(on("online_only"), held.here("mov")), on("online_only"));

        // The materialise saw it land: the badge and every count that asks `here` say on this Mac from now.
        assert!(held.land("mov"));
        assert_eq!(held.here("mov"), Some(true));
        assert_eq!(mark(on("online_only"), held.here("mov")), on("on_this_pc"));
        assert!(held.take_changed(), "the window reads its lists again");
        // Landing again changes nothing, and wakes nothing.
        assert!(!held.land("mov"));
        assert!(!held.take_changed());

        // The system's list does not name it yet: it stays here, and nothing flips back.
        assert!(!held.take(&Fake::new(&[("a", 10)])));
        assert_eq!(held.here("mov"), Some(true));
        // The list names it: the list is the truth from here on.
        assert!(!held.take(&Fake::new(&[("a", 10), ("mov", 384)])));
        assert_eq!(held.here("mov"), Some(true));
        // Finder made it online only again later: the mark follows the list, and the window is told.
        assert!(held.take(&Fake::new(&[("a", 10)])));
        assert_eq!(held.here("mov"), Some(false));
        assert!(held.take_changed());
    }

    #[test]
    fn a_file_that_landed_before_the_system_answered_is_in_the_first_list_and_one_this_app_evicted_is_not_held() {
        use super::finder_state::Held;
        let mut held = Held::default();
        // No list is known yet, and one file is not a list; the file itself is here from now (dm-78).
        assert!(held.land("mov"));
        assert_eq!((held.here("mov"), held.here("a")), (None, None));
        assert_eq!((held.answer("mov"), held.answer("a")), (Some(true), Some(false)));
        assert!(held.take(&Fake::new(&[("a", 10)])));
        assert_eq!((held.here("mov"), held.here("a")), (Some(true), Some(true)));

        // Clear made it online only right after: it is not held as landed over the list.
        held.forget("mov");
        assert!(held.take(&Fake::new(&[("a", 10)])));
        assert_eq!(held.here("mov"), Some(false));
    }

    #[test]
    fn a_file_provider_change_is_read_again_and_an_ask_during_a_read_is_followed_by_one_more() {
        use super::finder_state::Held;
        use std::time::{Duration, Instant};
        let mut held = Held::default();
        assert!(held.take(&Fake::new(&[])));
        assert!(held.take_changed());
        // Finder itself downloaded "doc" (the File Provider's change): the read that follows differs, and the
        // window is told to read its lists again.
        assert!(held.take(&Fake::new(&[("doc", 7)])));
        assert_eq!(held.here("doc"), Some(true));
        assert!(held.take_changed());
        // The same list again: nothing to tell.
        assert!(!held.take(&Fake::new(&[("doc", 7)])));
        assert!(!held.take_changed());

        // A read began; nobody asked since: the reader ends.
        let began = Instant::now();
        assert!(!held.asked_since(began));
        // A row asked while it ran (what it was told is the list from before): one more read follows.
        std::thread::sleep(Duration::from_millis(2));
        held.ask_now();
        assert!(held.asked_since(began));
        assert!(!held.asked_since(Instant::now() + Duration::from_secs(1)));
    }
}
