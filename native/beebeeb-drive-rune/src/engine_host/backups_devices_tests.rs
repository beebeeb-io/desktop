//! Backups' other devices and "Delete this backup" on the real engine (`DeviceCalls`): the list from the engine's
//! answer with its honest rows, a late answer after a session change dropped, a wrong password and a refused delete
//! that keep the row, a done delete that reads the list again (nothing removed before the answer), one delete at a
//! time and only for a listed device, and the `impl DevicesEngine for Engine` glue on the engine's own handle. Every
//! name is invented.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::backup_devices_api::{BackupDevice, BackupDevices, DeviceClass, DeviceRead, DeviceSummary};
use beebeeb_engine::app::settings_api::AccountCallError;

use super::*;

const A: &str = "invented-a@beebeeb.io";
const B: &str = "invented-b@beebeeb.io";
const PASSWORD: &str = "an invented password";

fn read_device(id: &str, name: &str, class: DeviceClass, model: Option<&str>, folders: u32) -> BackupDevice {
    BackupDevice {
        folder_id: id.into(),
        folder_name: Some(format!("{name} folder")),
        read: DeviceRead::Read(DeviceSummary {
            name: Some(name.into()),
            class,
            model: model.map(str::to_string),
            folders,
            last_backup_ms: None,
        }),
        may_be_this_pc: false,
    }
}

fn account_a_devices() -> Vec<BackupDevice> {
    vec![
        read_device("mac", "Invented Studio", DeviceClass::Computer, None, 2),
        read_device("phone", "Invented phone", DeviceClass::Phone, Some("iPhone 17"), 1),
        BackupDevice {
            folder_id: "old".into(),
            folder_name: Some("Invented old laptop".into()),
            read: DeviceRead::Unreadable,
            may_be_this_pc: false,
        },
        BackupDevice {
            folder_id: "nameless".into(),
            folder_name: None,
            read: DeviceRead::NotReached,
            may_be_this_pc: false,
        },
    ]
}

/// The engine as the device calls see it.
struct Fake {
    session: AtomicU64,
    account: Mutex<Option<String>>,
    unlocked: AtomicBool,
    /// Each account's devices; a done delete removes the row here (Beebeeb's side).
    devices: Mutex<Vec<(String, Vec<BackupDevice>)>>,
    /// What the next reads answer instead of the list.
    read_error: Mutex<Option<DevicesRefused>>,
    delete_answer: Mutex<Result<(), DeleteBackupError>>,
    /// Each delete asked: the session, the id, and whether the password was the typed one.
    deletes: Mutex<Vec<(u64, String, bool)>>,
    reads: AtomicUsize,
    wakes: AtomicUsize,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake {
            session: AtomicU64::new(1),
            account: Mutex::new(Some(A.into())),
            unlocked: AtomicBool::new(true),
            devices: Mutex::new(vec![
                (A.into(), account_a_devices()),
                (
                    B.into(),
                    vec![read_device("b-pc", "Invented B PC", DeviceClass::Computer, None, 1)],
                ),
            ]),
            read_error: Mutex::new(None),
            delete_answer: Mutex::new(Ok(())),
            deletes: Mutex::new(Vec::new()),
            reads: AtomicUsize::new(0),
            wakes: AtomicUsize::new(0),
        })
    }

    /// Another session (and account) is here now.
    fn switch_to(&self, session: u64, account: Option<&str>) {
        self.session.store(session, Ordering::SeqCst);
        *self.account.lock().unwrap() = account.map(str::to_string);
    }
}

impl DevicesEngine for Fake {
    fn stamp(&self) -> Stamp {
        (
            SessionGeneration::for_test(self.session.load(Ordering::SeqCst)),
            self.account.lock().unwrap().clone(),
        )
    }
    fn has_session(&self) -> bool {
        self.unlocked.load(Ordering::SeqCst)
    }
    fn devices_for(&self, session: SessionGeneration) -> Result<BackupDevices, DevicesRefused> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = *self.read_error.lock().unwrap() {
            return Err(error);
        }
        let who = self.account.lock().unwrap().clone();
        let devices = self
            .devices
            .lock()
            .unwrap()
            .iter()
            .find(|(account, _)| Some(account) == who.as_ref())
            .map(|(_, devices)| devices.clone())
            .unwrap_or_default();
        Ok(BackupDevices {
            session,
            devices,
            complete: true,
        })
    }
    fn delete_for(
        &self,
        session: SessionGeneration,
        folder_id: &str,
        password: &[u8],
    ) -> Result<(), DeleteBackupError> {
        let turn = (1..=9)
            .find(|n| SessionGeneration::for_test(*n) == session)
            .unwrap_or(0);
        self.deletes
            .lock()
            .unwrap()
            .push((turn, folder_id.into(), password == PASSWORD.as_bytes()));
        let answer = self.delete_answer.lock().unwrap().clone();
        if answer.is_ok() {
            for (_, devices) in self.devices.lock().unwrap().iter_mut() {
                devices.retain(|device| device.folder_id != folder_id);
            }
        }
        answer
    }
    fn wake(&self) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}

/// Pump until no worker runs; what the pumps said, folded.
fn settle(calls: &mut DeviceCalls<Fake>) -> DevicesPumped {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = DevicesPumped::default();
    loop {
        let pumped = calls.pump();
        seen.changed |= pumped.changed;
        seen.deleted = seen.deleted.or(pumped.deleted);
        if calls.idle() {
            let pumped = calls.pump();
            seen.changed |= pumped.changed;
            seen.deleted = seen.deleted.or(pumped.deleted);
            seen.busy = pumped.busy;
            return seen;
        }
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The next worker waits until the returned sender sends (or is dropped).
fn hold(calls: &mut DeviceCalls<Fake>) -> mpsc::Sender<()> {
    let (release, held) = mpsc::channel();
    calls.hold_next = Some(held);
    release
}

fn ids(calls: &DeviceCalls<Fake>) -> Vec<String> {
    calls.devices().into_iter().map(|device| device.id).collect()
}

fn opened(fake: &Arc<Fake>) -> DeviceCalls<Fake> {
    let mut calls = DeviceCalls::new(fake.clone());
    calls.opened();
    settle(&mut calls);
    calls
}

#[test]
fn the_list_comes_from_the_engine_and_a_device_it_cannot_read_says_so() {
    let fake = Fake::new();
    let mut calls = DeviceCalls::new(fake.clone());
    assert!(calls.devices().is_empty(), "nothing before the section opens");
    let release = hold(&mut calls);
    calls.opened();
    calls.pump();
    assert!(
        calls.devices().is_empty(),
        "nothing is drawn as read before the engine answered"
    );
    assert_eq!(calls.note(), Some(READING));
    release.send(()).unwrap();
    settle(&mut calls);
    assert_eq!(fake.reads.load(Ordering::SeqCst), 1);
    assert!(
        fake.wakes.load(Ordering::SeqCst) >= 1,
        "the window is woken when the answer is in"
    );
    assert_eq!(calls.note(), None);
    let rows = calls.devices();
    assert_eq!(ids(&calls), ["mac", "phone", "old", "nameless"]);
    let mac = &rows[0];
    assert_eq!((mac.name.as_str(), mac.kind), ("Invented Studio", DeviceKind::Computer));
    assert_eq!(
        mac.backed,
        Backed::Folders(2),
        "the folders listed, never a photo count"
    );
    assert_eq!(
        (mac.last_backup.as_deref(), mac.signed_in, mac.size.as_deref()),
        (None, None, None),
        "no time, no signed-in state, no size: the manifest does not say them"
    );
    assert_eq!((rows[1].kind, rows[1].model.as_str()), (DeviceKind::Phone, "iPhone 17"));
    assert_eq!(
        (rows[2].name.as_str(), rows[2].backed, rows[2].kind),
        ("Invented old laptop", Backed::Unreadable, DeviceKind::Unknown)
    );
    assert_eq!((rows[3].name.as_str(), rows[3].backed), (UNNAMED, Backed::Unreadable));
    // Opening again within FRESH_FOR reads nothing more.
    calls.opened();
    settle(&mut calls);
    assert_eq!(fake.reads.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failed_read_keeps_the_older_list_and_says_so_until_the_next_answer() {
    let fake = Fake::new();
    let mut calls = opened(&fake);
    // A delete that may have reached Beebeeb reads the list again; that read fails.
    *fake.read_error.lock().unwrap() = Some(DevicesRefused::Call(AccountCallError::Offline));
    *fake.delete_answer.lock().unwrap() = Err(DeleteBackupError::Offline);
    assert!(calls.delete("mac", PASSWORD));
    let pumped = settle(&mut calls);
    assert_eq!(pumped.deleted, Some(Err(UNCONFIRMED)));
    assert_eq!(fake.reads.load(Ordering::SeqCst), 2);
    assert_eq!(calls.note(), Some(NOT_READ));
    assert_eq!(ids(&calls).len(), 4, "the older list stays");
    // The next delete is done and its read answers: the note goes, the list is Beebeeb's.
    *fake.read_error.lock().unwrap() = None;
    *fake.delete_answer.lock().unwrap() = Ok(());
    assert!(calls.delete("mac", PASSWORD));
    assert_eq!(settle(&mut calls).deleted, Some(Ok(())));
    assert_eq!(calls.note(), None, "an answer clears the note");
    assert_eq!(ids(&calls), ["phone", "old", "nameless"]);
}

#[test]
fn nothing_synced_on_this_pc_and_another_accounts_state_show_no_rows() {
    let fake = Fake::new();
    *fake.read_error.lock().unwrap() = Some(DevicesRefused::Local(LocalStateRefused::NothingSynced));
    let calls = opened(&fake);
    assert!(calls.devices().is_empty());
    assert_eq!(calls.note(), Some(NOT_SYNCED));
    *fake.read_error.lock().unwrap() = Some(DevicesRefused::Local(LocalStateRefused::OtherAccount));
    let calls = opened(&fake);
    assert!(calls.devices().is_empty());
    assert_eq!(calls.note(), None, "the section itself says it is another account's");
    *fake.read_error.lock().unwrap() = Some(DevicesRefused::Local(LocalStateRefused::Unreadable));
    let calls = opened(&fake);
    assert!(calls.devices().is_empty());
    assert_eq!(
        calls.note(),
        Some(NOT_READ),
        "an unreadable local state is said, never an empty list"
    );
}

#[test]
fn a_late_read_after_a_session_change_is_dropped_and_the_new_session_is_read() {
    let fake = Fake::new();
    let mut calls = DeviceCalls::new(fake.clone());
    let release = hold(&mut calls);
    calls.opened();
    // Account B signs in while A's read is out.
    fake.switch_to(2, Some(B));
    calls.pump();
    release.send(()).unwrap();
    // Nothing of A, ever, once B is here.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !calls.idle() {
        calls.pump();
        assert!(
            !ids(&calls).iter().any(|id| id == "mac"),
            "A's devices are never drawn under B"
        );
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(&mut calls);
    assert_eq!(ids(&calls), ["b-pc"], "B's own list is read after the change");
    // Signed out: nothing.
    fake.unlocked.store(false, Ordering::SeqCst);
    assert!(calls.devices().is_empty() && calls.note().is_none());
}

#[test]
fn a_wrong_password_deletes_nothing_keeps_the_row_and_says_why() {
    let fake = Fake::new();
    let mut calls = opened(&fake);
    *fake.delete_answer.lock().unwrap() = Err(DeleteBackupError::WrongPassword);
    let release = hold(&mut calls);
    assert!(calls.delete("mac", PASSWORD), "sent");
    assert!(calls.deleting());
    release.send(()).unwrap();
    let pumped = settle(&mut calls);
    assert_eq!(pumped.deleted, Some(Err(WRONG_PASSWORD)));
    assert!(!calls.deleting());
    assert!(ids(&calls).contains(&"mac".to_string()), "the row stays");
    assert_eq!(
        fake.reads.load(Ordering::SeqCst),
        1,
        "nothing reached Beebeeb: no read again"
    );
    assert_eq!(*fake.deletes.lock().unwrap(), [(1, "mac".to_string(), true)]);
}

#[test]
fn a_refused_delete_keeps_the_row() {
    for (answer, words) in [
        (DeleteBackupError::Refused("the rule's words".into()), NOT_ALLOWED),
        (DeleteBackupError::NotABackup, NOT_ALLOWED),
        (DeleteBackupError::RateLimited(30), TOO_MANY),
        (DeleteBackupError::SignedOut, SIGNED_OUT),
        (DeleteBackupError::Failed("state"), NOT_CHECKED),
    ] {
        let fake = Fake::new();
        let mut calls = opened(&fake);
        *fake.delete_answer.lock().unwrap() = Err(answer);
        assert!(calls.delete("phone", PASSWORD));
        assert_eq!(settle(&mut calls).deleted, Some(Err(words)));
        assert!(ids(&calls).contains(&"phone".to_string()), "{words}");
    }
}

#[test]
fn not_found_is_never_drawn_as_done_and_reads_the_list_again() {
    let fake = Fake::new();
    let mut calls = opened(&fake);
    *fake.delete_answer.lock().unwrap() = Err(DeleteBackupError::NotFound);
    assert!(calls.delete("phone", PASSWORD));
    assert_eq!(settle(&mut calls).deleted, Some(Err(NOT_FOUND)));
    assert_eq!(
        fake.reads.load(Ordering::SeqCst),
        2,
        "read again: Beebeeb's list decides"
    );
}

#[test]
fn a_done_delete_removes_nothing_before_the_answer_and_reads_the_list_again() {
    let fake = Fake::new();
    let mut calls = opened(&fake);
    let release = hold(&mut calls);
    assert!(calls.delete("mac", PASSWORD));
    calls.pump();
    assert!(
        ids(&calls).contains(&"mac".to_string()),
        "nothing is removed before Beebeeb answered"
    );
    release.send(()).unwrap();
    let pumped = settle(&mut calls);
    assert_eq!(pumped.deleted, Some(Ok(())));
    assert_eq!(fake.reads.load(Ordering::SeqCst), 2, "the list is read again");
    assert_eq!(
        ids(&calls),
        ["phone", "old", "nameless"],
        "the row went with Beebeeb's answer"
    );
}

/// Review of desktop#425: a folder the engine says may be this PC's own is drawn as it reads, never deletable, and a
/// delete of it is not sent.
#[test]
fn a_device_that_may_be_this_pcs_is_never_deleted() {
    let fake = Fake::new();
    fake.devices
        .lock()
        .unwrap()
        .iter_mut()
        .find(|(account, _)| account == A)
        .expect("account A's devices")
        .1
        .push(BackupDevice {
            folder_id: "maybe-me".into(),
            folder_name: Some("Invented PC".into()),
            read: DeviceRead::Unreadable,
            may_be_this_pc: true,
        });
    let mut calls = opened(&fake);
    let rows = calls.devices();
    let row = rows.iter().find(|row| row.id == "maybe-me").expect("listed");
    assert_eq!((row.backed, row.deletable), (Backed::Unreadable, false));
    assert!(rows.iter().filter(|row| row.id != "maybe-me").all(|row| row.deletable));
    assert!(!calls.delete("maybe-me", PASSWORD), "not sent");
    settle(&mut calls);
    assert!(fake.deletes.lock().unwrap().is_empty());
    assert!(calls.delete("mac", PASSWORD), "another device's still is");
}

#[test]
fn one_delete_at_a_time_and_only_for_a_listed_device_of_this_session() {
    let fake = Fake::new();
    let mut calls = opened(&fake);
    let release = hold(&mut calls);
    assert!(calls.delete("mac", PASSWORD));
    assert!(
        !calls.delete("phone", PASSWORD),
        "a second press while one is out sends nothing"
    );
    release.send(()).unwrap();
    settle(&mut calls);
    assert!(!calls.delete("not-listed", PASSWORD), "an id the list does not hold");
    assert!(!calls.delete("phone", ""), "an empty password");
    settle(&mut calls);
    assert_eq!(fake.deletes.lock().unwrap().len(), 1);
    // Another account: A's ids are not this session's list.
    fake.switch_to(2, Some(B));
    assert!(!calls.delete("phone", PASSWORD));
    settle(&mut calls);
    assert_eq!(fake.deletes.lock().unwrap().len(), 1);
}

#[test]
fn a_delete_answer_after_a_session_change_is_dropped() {
    let fake = Fake::new();
    let mut calls = opened(&fake);
    let release = hold(&mut calls);
    assert!(calls.delete("mac", PASSWORD));
    fake.switch_to(2, Some(A));
    calls.pump();
    assert!(!calls.deleting(), "the earlier session's delete is not this one's");
    release.send(()).unwrap();
    let pumped = settle(&mut calls);
    assert_eq!(
        pumped.deleted, None,
        "its answer is dropped: no message under another session"
    );
    // It was sent for the session of the press, never the later one.
    assert_eq!(fake.deletes.lock().unwrap()[0].0, 1);
}

/// The glue on the engine's own handle: no session in memory, so the read and the delete are refused before anything
/// is sent.
#[test]
fn the_engine_glue_sends_nothing_without_a_session() {
    let engine = super::super::tests::started().opened.engine.clone();
    assert!(!DevicesEngine::has_session(&*engine));
    let (session, _) = DevicesEngine::stamp(&*engine);
    assert_eq!(
        DevicesEngine::devices_for(&*engine, session).unwrap_err(),
        DevicesRefused::NotSignedIn
    );
    assert_eq!(
        DevicesEngine::delete_for(&*engine, session, "00000000-0000-4000-8000-0000000000c2", b"x").unwrap_err(),
        DeleteBackupError::SignedOut
    );
}

/// The section on [`DeviceCalls`]: nothing else of this PC; Try again reads the devices again (`opened`).
struct DevicesOnly(DeviceCalls<Fake>);

impl crate::frames::backups::source::BackupsSource for DevicesOnly {
    fn pump(&mut self) -> crate::frames::backups::source::Pumped {
        let pumped = self.0.pump();
        crate::frames::backups::source::Pumped {
            changed: pumped.changed,
            busy: pumped.busy,
            ..Default::default()
        }
    }
    fn opened(&mut self) {
        self.0.opened();
    }
    fn devices_note(&self) -> Option<&'static str> {
        self.0.note()
    }
    fn root(&self) -> crate::frames::backups::source::BackupsRoot {
        crate::frames::backups::source::BackupsRoot {
            id: String::new(),
            is_protected: true,
            device_folder: String::new(),
        }
    }
    fn status(&self) -> crate::frames::backups::source::Status {
        crate::frames::backups::source::Status::Plain {
            title: String::new(),
            sub: String::new(),
            can_run: false,
        }
    }
    fn folders(&self) -> Vec<crate::frames::backups::source::Folder> {
        Vec::new()
    }
    fn devices(&self) -> Vec<Device> {
        self.0.devices()
    }
    fn not_backed_up(&self) -> Vec<crate::frames::backups::source::NotBackedUp> {
        Vec::new()
    }
    fn mail_backup_on(&self) -> bool {
        false
    }
    fn back_up_now(&mut self) {}
    fn set_folder(&mut self, _id: &str, _on: bool) {}
    fn add_folder(&mut self, _path: &str) -> bool {
        false
    }
    fn remove_folder(&mut self, _id: &str, _move_copy_to_trash: bool) {}
    fn retry(&mut self, _id: &str) {}
    fn retry_all(&mut self) {}
    fn delete_device_backup(&mut self, _id: &str, _code: &str) -> bool {
        false
    }
}

fn find_key(rt: &rune::Runtime<crate::State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn has_text(rt: &rune::Runtime<crate::State>, words: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| !e.exiting && e.text.as_deref().is_some_and(|t| t.contains(words)))
}

/// Poll and draw until no device worker runs and nothing more came in (a frame polls the section too, and may start a
/// read of its own).
fn settle_section(rt: &mut rune::Runtime<crate::State>, pending: &Arc<AtomicUsize>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut tx = rune::Tx::default();
    loop {
        let changed = crate::frames::backups::poll(&mut rt.state, &mut tx);
        rt.invalidate();
        rt.frame(0.016);
        rt.settle_all();
        rt.frame(0.016);
        if !changed && pending.load(Ordering::SeqCst) == 0 {
            let changed = crate::frames::backups::poll(&mut rt.state, &mut tx);
            if !changed && pending.load(Ordering::SeqCst) == 0 {
                rt.invalidate();
                rt.frame(0.016);
                return;
            }
        }
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

const RETRY: &str = "backups-devices-failed-retry";

#[test]
fn backups_devices_try_again() {
    let fake = Fake::new();
    *fake.read_error.lock().unwrap() = Some(DevicesRefused::Call(AccountCallError::Offline));
    let mut calls = DeviceCalls::new(fake.clone());
    calls.opened();
    settle(&mut calls);
    assert_eq!(calls.note(), Some(NOT_READ));
    let pending = calls.pending.clone();

    let mut state = crate::shell::state::init();
    crate::frames::settings::go(&mut state, "DW-0737");
    state.backups = crate::frames::backups::BackupsUi::new(Some(Box::new(DevicesOnly(calls))));
    let env = rune::Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = rune::Runtime::new(state, crate::shell::registry::view, (958.0, 958.0), env);
    rt.frame(0.0);
    settle_section(&mut rt, &pending);
    assert!(
        find_key(&rt, RETRY).is_some(),
        "a failed devices read draws Try again, as the other views"
    );
    let reads = fake.reads.load(Ordering::SeqCst);

    // The API is back: pressing Try again asks again and the list shows.
    *fake.read_error.lock().unwrap() = None;
    let r = find_key(&rt, RETRY).unwrap().rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle_section(&mut rt, &pending);
    assert!(
        fake.reads.load(Ordering::SeqCst) > reads,
        "Try again asks the engine again"
    );
    let source = rt.state.backups.source().unwrap();
    assert_eq!(
        (source.devices_note(), source.devices().len(), fake.reads.load(Ordering::SeqCst) - reads),
        (None, 4, 1),
        "the read after Try again answered"
    );
    assert!(find_key(&rt, RETRY).is_none());
    assert!(has_text(&rt, "Invented Studio"), "the devices show after Try again");

    // A read that fails while a list is held keeps it, Try again under it; a failed read is never fresh.
    *fake.read_error.lock().unwrap() = Some(DevicesRefused::Call(AccountCallError::Offline));
    rt.state.backups.read_again();
    settle_section(&mut rt, &pending);
    assert!(has_text(&rt, "Invented Studio"), "the older list stays");
    assert!(find_key(&rt, RETRY).is_some());
    let reads = fake.reads.load(Ordering::SeqCst);
    rt.state.backups.read_again();
    settle_section(&mut rt, &pending);
    assert!(
        fake.reads.load(Ordering::SeqCst) > reads,
        "Try again reads again at once"
    );
}
