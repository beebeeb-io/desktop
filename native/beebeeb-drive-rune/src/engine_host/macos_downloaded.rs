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
    /// Whether the system has a list to give: Beebeeb is in Finder (Finder setup is ready). While it is not, nothing
    /// it answers is a list (the bridge answers "no items" for a domain that is not there yet: RC13 showed every
    /// file as online only from such an answer). Default: yes.
    fn ready(&self) -> bool {
        true
    }
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
        // Never a made-up 0: for a domain that is not there the bridge answers "no items".
        if !self.ready() {
            return Err("Beebeeb is not in Finder yet.".into());
        }
        Ok(parse(&bridge::list()?))
    }
    fn evict(&self, id: &str) -> Result<(), String> {
        bridge::evict(id)?;
        finder_state::forget(id);
        Ok(())
    }
    fn ready(&self) -> bool {
        crate::engine_host::finder_setup::is_ready()
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
/// Dm-78: the mark comes from Finder's state alone, and it is read again when Finder setup is ready. RC13, on a
/// slow Finder start, was wrong both ways: during setup the engine's word stood (two files it held a copy of from an
/// earlier build showed "on this Mac" while Finder's copies were dataless), and after setup every row showed the
/// cloud ("0 on this Mac · 16 online only") while six files were not dataless. What was wrong:
/// - a list asked for while Beebeeb was not in Finder yet was taken as a list (the bridge answers "no items" for a
///   domain that is not there), so every file read as online only;
/// - nothing read the list again once setup was ready, and nothing told the window to read its rows again when
///   what is held changed (nobody called [`finder_state::take_changed`]).
///
/// So now:
/// - while Finder setup is not ready no list is asked for, and one held from before is let go
///   ([`Materialisation::ready`], [`finder_state::Held::take_read`]);
/// - once a row asked, the reader keeps looking until setup is ready, reads the list then, and reads it a few times
///   more ([`finder_state::SETTLE_READS`]: the system may still be filling it in);
/// - while no list was read a file gets NO mark: not the engine's own record, and not "online only" either
///   ([`finder_state::mark`]). A file a materialise saw land moments ago is the one exception: it is here;
/// - the window's poll takes [`finder_state::take_changed`] and reads its lists again, as after a finished check
///   (`shell::host`).
///
/// Only where there is no system to ask (not a Mac, a test) does the engine's word stand.
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
    /// How many more times the list is read after the first read that follows Finder setup turning ready, one per
    /// [`FRESH_FOR`]: the system may still be filling the list in for a domain it just added.
    pub(crate) const SETTLE_READS: u8 = 3;
    /// How many reads in a row may fail, while no list was read yet, before the reader waits for the next ask.
    pub(crate) const UNREAD_TRIES: u8 = 5;

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
        /// Finder setup was ready at the last read.
        was_ready: bool,
        /// The reads still to follow the one after setup turned ready ([`SETTLE_READS`]).
        settle: u8,
        /// The reads that failed in a row.
        failed: u8,
    }

    static HELD: Mutex<Held> = Mutex::new(Held {
        here: None,
        read_at: None,
        reading: false,
        wake: None,
        landed: Vec::new(),
        asked: None,
        changed: false,
        was_ready: false,
        settle: 0,
        failed: 0,
    });

    impl Held {
        /// Whether Finder holds the bytes of `id`. `None`: the system has not said.
        pub(crate) fn here(&self, id: &str) -> Option<bool> {
            self.here.as_ref().map(|here| here.contains(id))
        }

        /// What a file's row is told on a Mac (dm-78): whether the system's list names `id`. While no list was read
        /// (`None`: not known, so no mark at all), a file a materialise saw land within [`LANDED_FOR`] is here.
        pub(crate) fn answer(&self, id: &str) -> Option<bool> {
            self.here(id).or_else(|| {
                self.landed
                    .iter()
                    .any(|(held, at)| held == id && at.elapsed() < LANDED_FOR)
                    .then_some(true)
            })
        }

        /// One look at the system, as the reader makes it: its list when Finder setup is ready, nothing when it is
        /// not. `true`: what is held changed.
        #[cfg_attr(not(test), allow(dead_code))]
        pub(in crate::engine_host) fn take(&mut self, source: &dyn Materialisation) -> bool {
            self.take_read(source.ready().then(|| source.items()))
        }

        /// One look of the reader. `None`: Finder setup is not ready, so there is no list: one held from before is
        /// let go (it is of a domain that may be gone), and the files have no mark until setup is ready. `Some`: the
        /// system's answer ([`Self::apply`]); the first one after setup turned ready is followed by
        /// [`SETTLE_READS`] more. `true`: what is held changed.
        pub(in crate::engine_host) fn take_read(&mut self, listed: Option<Result<Vec<Item>, String>>) -> bool {
            let Some(listed) = listed else {
                self.read_at = Some(Instant::now());
                (self.was_ready, self.settle, self.failed) = (false, 0, 0);
                let held = self.here.take().is_some();
                self.changed |= held;
                return held;
            };
            if !self.was_ready {
                tracing::info!("finder state: Finder is set up; the downloaded files are listed");
                (self.was_ready, self.settle) = (true, SETTLE_READS);
            } else {
                self.settle = self.settle.saturating_sub(1);
            }
            self.failed = if listed.is_ok() {
                0
            } else {
                self.failed.saturating_add(1)
            };
            self.apply(listed)
        }

        /// A read that began at `began` is over: does another follow? Yes while Finder setup is not ready (the
        /// reader is the one that sees it turn ready), while the reads after ready are owed, after an ask that came
        /// since it began, and for a few tries while no list could be read at all.
        pub(in crate::engine_host) fn reads_on(&self, began: Instant) -> bool {
            !self.was_ready
                || self.settle > 0
                || self.asked_since(began)
                || (self.here.is_none() && self.failed < UNREAD_TRIES)
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

    /// The reader: one look at the system per [`FRESH_FOR`], for as long as another is due ([`Held::reads_on`]: an
    /// ask came after the last one began, Finder setup is not ready yet, or the reads after ready are owed). The
    /// system is asked outside the lock (it can take seconds); a row meanwhile gets the last list.
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
            let listed = source.ready().then(|| source.items());
            let (wake, again) = {
                let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
                let changed = held.take_read(listed);
                let again = held.reads_on(began);
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

    /// Whether Finder holds the bytes of the file `id`, by the last list read ([`Held::answer`]); a read that
    /// begins after this ask follows. `None`: not known: there is no system to ask (not a Mac, a test), or no list
    /// was read yet (Finder setup is not ready, the system has not answered). Never waits.
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

    /// Dm-77: `true` once after Finder's state changed (a file landed, the list read differs, Finder setup turned
    /// ready or left): the window's poll (`shell::host::poll_screen`) reads the lists it shows again, as after a
    /// finished check, so the marks and the counts are Finder's state now.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // a Mac's window polls it
    pub(crate) fn take_changed() -> bool {
        HELD.lock().unwrap_or_else(PoisonError::into_inner).take_changed()
    }

    /// This app made the file `id` online only: it is not held as landed any longer.
    #[allow(dead_code)] // the Mac's own evict calls it; a test and another system have none
    pub(super) fn forget(id: &str) {
        HELD.lock().unwrap_or_else(PoisonError::into_inner).forget(id);
    }

    /// The mark a file's row gets (`pc_status`), from the engine's `mark` and Finder's state ([`here`]): see
    /// [`mark_on`]. Where there is a system to ask (a Mac), a file it has not said anything of yet has no mark.
    pub(crate) fn mark(mark: Option<String>, here: Option<bool>) -> Option<String> {
        mark_on(mark, here, super::system().is_some())
    }

    /// [`mark`], `finder` saying whether there is a system to ask. A file Finder holds no bytes of is online only
    /// whatever the engine holds, and one Finder holds is on this Mac. While Finder has not said (`here` is `None`
    /// on a Mac: setup is not ready, no list was read yet) the file has NO mark: not the engine's own record of a
    /// copy, and not "online only" for everything either. Without a system to ask, the engine's mark stands. A kept
    /// file keeps its own mark, and so does a row the engine marks another way (a shared place, nothing yet).
    pub(in crate::engine_host) fn mark_on(mark: Option<String>, here: Option<bool>, finder: bool) -> Option<String> {
        match (mark.as_deref(), here) {
            (Some("on_this_pc"), Some(false)) => Some("online_only".to_string()),
            (Some("online_only"), Some(true)) => Some("on_this_pc".to_string()),
            (Some("on_this_pc" | "online_only"), None) if finder => None,
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

    /// What Finder says of a Mac whose setup is under way or ready, and of the files it holds.
    struct Mac {
        ready: std::sync::atomic::AtomicBool,
        list: std::sync::Mutex<Result<Vec<Item>, String>>,
        asked: std::sync::atomic::AtomicUsize,
    }

    impl Mac {
        fn setting_up() -> Mac {
            Mac {
                ready: false.into(),
                // What the bridge answers for a domain that is not there yet: no items.
                list: std::sync::Mutex::new(Ok(Vec::new())),
                asked: 0.into(),
            }
        }
        fn holds(&self, ids: &[&str]) {
            let items = ids.iter().map(|id| Item {
                id: id.to_string(),
                bytes: 1,
            });
            *self.list.lock().unwrap() = Ok(items.collect());
        }
        fn set_ready(&self, ready: bool) {
            self.ready.store(ready, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl Materialisation for Mac {
        fn items(&self) -> Result<Vec<Item>, String> {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.list.lock().unwrap().clone()
        }
        fn evict(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
        fn ready(&self) -> bool {
            self.ready.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    /// The Files root of RC13: what the engine records of each file, and whether Finder's copy has its bytes
    /// (`ls -lO`). "zip" and "md" are copies the engine holds from an earlier build, dataless in Finder; "mov",
    /// "eps", "docx" and "txt" are in Finder, and the engine calls most of them online only.
    const ROOT: [(&str, &str, bool); 6] = [
        ("zip", "on_this_pc", false),
        ("md", "on_this_pc", false),
        ("mov", "online_only", true),
        ("eps", "online_only", true),
        ("docx", "on_this_pc", true),
        ("txt", "online_only", true),
    ];

    /// The marks the rows of [`ROOT`] get on a Mac from what is held now.
    fn marks(held: &finder_state::Held) -> Vec<Option<String>> {
        ROOT.iter()
            .map(|(id, engine, _)| finder_state::mark_on(Some(engine.to_string()), held.answer(id), true))
            .collect()
    }

    /// Dm-78, RC13, both ways. During a slow Finder setup two files the engine held a copy of showed "on this Mac"
    /// while `ls -lO` said `dataless`; after setup was ready every row showed the cloud ("0 on this Mac · 16 online
    /// only") while six files were not dataless. The list is read when setup is ready, the window is told to read
    /// its rows again, and until then a file has no mark at all.
    #[test]
    fn the_marks_are_finders_state_once_setup_is_ready_and_no_mark_at_all_before_it() {
        use super::finder_state::{Held, SETTLE_READS};
        use std::sync::atomic::Ordering::SeqCst;
        let on = |m: &str| Some(m.to_string());
        let truth: Vec<Option<String>> = ROOT
            .iter()
            .map(|(_, _, here)| on(if *here { "on_this_pc" } else { "online_only" }))
            .collect();
        let mac = Mac::setting_up();
        mac.holds(&[]);
        let mut held = Held::default();
        let began = std::time::Instant::now();

        // Setup is adding: nothing is asked of the system (its "no items" is not a list), and no row has a mark:
        // not the engine's ticks for "zip" and "md", and not the cloud for everything either.
        for _ in 0..3 {
            assert!(!held.take(&mac), "nothing changed, the window is not told");
            assert!(held.reads_on(began), "the reader keeps looking until setup is ready");
        }
        assert_eq!(
            mac.asked.load(SeqCst),
            0,
            "no list is asked of a Mac that is not in Finder yet"
        );
        assert_eq!(held.answer("zip"), None);
        assert_eq!(marks(&held), vec![None; ROOT.len()]);
        assert!(!held.take_changed());

        // Setup is ready, and Finder holds four of the files. The reader's next look reads the list, the window is
        // told to read its rows again, and every mark is what `ls -lO` says, both ways.
        mac.holds(&["mov", "eps", "docx", "txt"]);
        mac.set_ready(true);
        assert!(held.take(&mac), "the first list after ready is a change");
        assert!(held.take_changed(), "the window reads its lists again");
        assert_eq!(marks(&held), truth);
        assert_eq!(mac.asked.load(SeqCst), 1);

        // The list is read a few times more after ready, with nobody asking: the system may still be filling it in.
        mac.holds(&["mov", "eps"]);
        let began = std::time::Instant::now();
        assert!(held.reads_on(began));
        assert!(held.take(&mac));
        mac.holds(&["mov", "eps", "docx", "txt"]);
        let mut reads = 1;
        while held.reads_on(began) {
            held.take(&mac);
            reads += 1;
            assert!(reads <= SETTLE_READS, "the reads after ready end");
        }
        assert_eq!(reads, SETTLE_READS);
        assert!(held.take_changed());
        assert_eq!(marks(&held), truth);

        // A row asks later (a list refresh): one more read follows, and a file Finder made online only shows it.
        let began = std::time::Instant::now();
        assert!(!held.reads_on(began));
        std::thread::sleep(std::time::Duration::from_millis(2));
        held.ask_now();
        assert!(held.reads_on(began));
        mac.holds(&["mov", "eps", "docx"]);
        assert!(held.take(&mac));
        assert_eq!(marks(&held)[5], on("online_only"));

        // Setup leaves ready (Beebeeb is added to Finder again): the list held is of a domain that may be gone, so
        // it is let go, the rows lose their marks, and the list is read anew once setup is ready.
        mac.set_ready(false);
        assert!(held.take(&mac));
        assert!(held.take_changed());
        assert_eq!(marks(&held), vec![None; ROOT.len()]);
        assert!(!held.take(&mac));
        mac.holds(&["mov", "eps", "docx", "txt"]);
        mac.set_ready(true);
        assert!(held.take(&mac));
        assert!(held.take_changed());
        assert_eq!(marks(&held), truth);
    }

    /// While no list was read, the engine's record makes no mark on a Mac, and a list that cannot be read does not
    /// either; a file a materialise saw land is the one that is known.
    #[test]
    fn an_engine_copy_never_marks_a_file_as_on_this_mac_before_or_without_the_systems_list() {
        use super::finder_state::{Held, UNREAD_TRIES, mark_on};
        let on = |m: &str| Some(m.to_string());
        // What the row of a file gets on a Mac: the engine's mark, and what the row is told of Finder's state.
        let row = |held: &Held, engine: &str, id: &str| mark_on(on(engine), held.answer(id), true);
        let mac = Mac::setting_up();
        mac.set_ready(true);
        *mac.list.lock().unwrap() = Err("the system did not answer".into());

        let mut held = Held::default();
        assert_eq!(row(&held, "on_this_pc", "zip"), None);
        assert_eq!(row(&held, "online_only", "eps"), None, "not online only for everything");
        // The list cannot be read: still no mark, and the reader tries a few times, then waits for the next ask.
        let began = std::time::Instant::now();
        for tries in 1..=UNREAD_TRIES {
            assert!(!held.take(&mac));
            assert_eq!(held.reads_on(began), tries < UNREAD_TRIES);
        }
        assert!(!held.reads_on(began));
        assert_eq!(row(&held, "on_this_pc", "zip"), None);
        // A kept file's own mark, a shared place's and no mark are the engine's, as ever.
        assert_eq!(row(&held, "available_offline", "zip"), on("available_offline"));
        assert_eq!(row(&held, "name_shared", "zip"), on("name_shared"));
        // Where there is no system to ask (not a Mac), the engine's word stands.
        assert_eq!(mark_on(on("on_this_pc"), None, false), on("on_this_pc"));
        assert_eq!(mark_on(on("online_only"), None, false), on("online_only"));

        // A materialise saw "mov" land meanwhile, by its own attributes: that is Finder's state, so it is marked,
        // and the window is told. Nothing else is.
        assert!(held.land("mov"));
        assert!(held.take_changed());
        assert_eq!(row(&held, "online_only", "mov"), on("on_this_pc"));
        assert_eq!(row(&held, "on_this_pc", "zip"), None);

        // The system answers: its list is the mark, for the engine's copy too.
        mac.holds(&["mov", "docx"]);
        assert!(held.take(&mac));
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
        assert_eq!((held.answer("mov"), held.answer("a")), (Some(true), None));
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
