//! Slice B6 on Windows (this module is compiled on Windows only): which paths the launch Explorer started reads, the
//! registry values of the menu, and what the running window does with a verb, in the shell's own runtime (the board's
//! files, the fixture engine, a lookup that stands in for the engine's account-bound local state). The pure rules
//! that hold on every platform are tested in `control.rs` and `instance.rs`.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use rune::{Env, Runtime};

use super::*;
use crate::frames::files::dialogs::Dialog;
use crate::frames::files::source::{BoardDrive, Call, FilesFixture};
use crate::shell::actions::FixtureEngine;
use crate::shell::app::GateState;
use crate::shell::host::Shell;
use crate::shell::instance::{Claim, Plan};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, state};

/// The fixture engine's account (`FixtureEngine::ready`).
const ACCOUNT: &str = "fixture-account";
const ROOT: &str = r"C:\Users\Someone\Beebeeb";

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

/// Lets leaving elements finish leaving (`host_tests::settle`).
fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

/// The shell's window on the board's files, signed in as [`ACCOUNT`], with what the Files source is asked recorded.
fn window() -> (Runtime<State>, Arc<Mutex<Vec<Call>>>, Arc<FixtureEngine>) {
    let engine = Arc::new(FixtureEngine::ready());
    let mut state = state::init_with(Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default())));
    let fixture = FilesFixture::default();
    let calls = fixture.calls.clone();
    state.source = Box::new(BoardDrive::sharing(&fixture.edits));
    state.actions.source = Box::new(fixture);
    let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.account(), Some(ACCOUNT));
    (rt, calls, engine)
}

/// The engine's account-bound local state, as a test holds it: whose session it is, and which items it has where.
struct Items {
    account: &'static str,
    held: &'static [(&'static str, Option<&'static str>)],
    asked: Cell<u32>,
}

impl Items {
    /// The board's "Contract draft.pdf" and "Budget 2026.xlsx", both at the top of Files, for `account`.
    fn of(account: &'static str) -> Items {
        Items {
            account,
            held: &[("contract", None), ("budget", None)],
            asked: Cell::new(0),
        }
    }
}

impl ItemLookup for Items {
    fn account(&self) -> Option<String> {
        Some(self.account.to_string())
    }
    fn folder_of(&self, id: &str) -> Option<Option<String>> {
        self.asked.set(self.asked.get() + 1);
        self.held
            .iter()
            .find(|(held, _)| *held == id)
            .map(|(_, folder)| folder.map(str::to_owned))
    }
}

fn row_id(rt: &Runtime<State>) -> Option<String> {
    let s = &rt.state;
    s.selected
        .and_then(|row| s.listing.items.get(row))
        .map(|item| item.id.clone())
}

fn select(rt: &mut Runtime<State>, id: &str) {
    rt.update(|s, _| {
        let row = row_of(s, id).expect("a row of the board");
        s.select_only(row);
    });
    settle(rt);
}

/// Spec rule: a path outside the sync folder, or one with no item id, forwards nothing and opens nothing. Outside is
/// never even read; the folder itself, a name that only starts like it, another drive, a `..`, another spelling of the
/// folder are outside. Mutant: drop the `inside` check in `item_in_root` → the outside paths come back as an item.
#[test]
fn a_path_outside_the_sync_root_forwards_nothing() {
    let root = Path::new(ROOT);
    let outside = [
        r"C:\Users\Someone\Beebeeb",
        r"C:\Users\Someone\Beebeeb2\a.txt",
        r"C:\Users\Someone\BeebeebOld\a.txt",
        r"C:\Users\Someone\a.txt",
        r"D:\Users\Someone\Beebeeb\a.txt",
        r"C:\Users\Someone\Beebeeb\..\secret.txt",
        r"C:\Users\Someone\Beebeeb\Docs\..\..\secret.txt",
        r"Beebeeb\a.txt",
        r"\Users\Someone\Beebeeb\a.txt",
        r"\\?\C:\Users\Someone\Beebeeb\a.txt",
        r"\\server\share\Users\Someone\Beebeeb\a.txt",
        "",
    ];
    for path in outside {
        let mut read = 0;
        let found = item_in_root(Path::new(path), root, |_| {
            read += 1;
            Some("contract".into())
        });
        assert_eq!(found, None, "{path}");
        assert_eq!(read, 0, "not even read: {path}");
    }
    let inside_paths = [
        r"C:\Users\Someone\Beebeeb\a.txt",
        r"c:\users\someone\beebeeb\Docs\b.pdf",
        r"C:\Users\Someone\BEEBEEB\Lisbon trip\c.heic",
    ];
    for path in inside_paths {
        assert_eq!(
            item_in_root(Path::new(path), root, |_| Some("contract".into())),
            Some("contract".to_string()),
            "{path}"
        );
    }
    // Inside, and what the placeholder holds is nothing, or no item id: nothing either.
    for held in [
        None,
        Some(String::new()),
        Some("a b".to_string()),
        Some(r"C:\x".to_string()),
        Some("x".repeat(65)),
    ] {
        assert_eq!(
            item_in_root(Path::new(inside_paths[0]), root, |_| held.clone()),
            None,
            "{held:?}"
        );
    }
    // The launch with nothing to hand on: nothing is handed, nothing starts, whoever serves the channel.
    let launch = instance::parse_args(
        ["--explorer-verb=share".to_string(), outside[1].to_string()],
        false,
        false,
    );
    let ask = launch.explorer.expect("an explorer launch");
    for claim in [Claim::Running, Claim::First] {
        let plan = instance::explorer_plan(&ask, claim, |path| {
            item_in_root(path, root, |_| Some("contract".into()))
        });
        assert_eq!(plan, Plan::ExitQuietly, "{claim:?}");
    }
}

/// The menu as the registry gets it: the board's four words in its order, keys that sort in that order, each for files
/// under the sync folder, each command this program with its own flag and never a link, and the test build's keys its
/// own. A command line reads back as the verb launch it is.
#[test]
fn the_registration_lists_the_verbs_in_the_board_order() {
    let exe = r"C:\Program Files\Beebeeb\beebeeb-drive.exe";
    let root = Path::new(ROOT);
    let values = menu_entries(AppIdentity::Real, exe, root);
    assert_eq!(values.len(), 16, "four values for each of four verbs");
    let words: Vec<&RegistryValue> = values.iter().filter(|value| value.name == "MUIVerb").collect();
    let labels: Vec<&str> = words.iter().map(|value| value.data.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Share with Beebeeb\u{2026}",
            "Copy Beebeeb link",
            "Show in Beebeeb",
            "Version history\u{2026}"
        ]
    );
    let keys: Vec<&str> = words.iter().map(|value| value.key.as_str()).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        sorted, keys,
        "Explorer lists the keys by name: they sort in the board's order"
    );
    // The keys are named by the engine identity of THIS build (a test build is `BeebeebTest`), never a literal.
    let here = crate::shell::identity::engine_identity();
    let name = here.cloud_files_provider_name();
    assert_eq!(keys[0], format!("Software\\Classes\\*\\shell\\{name}.1.share"));
    for value in &values {
        assert!(
            value.key.starts_with(&format!("Software\\Classes\\*\\shell\\{name}.")),
            "{}",
            value.key
        );
    }
    for (each, key) in ExplorerVerb::ALL.into_iter().zip(&keys) {
        assert_eq!(*key, verb_key(here, each));
    }
    for value in values.iter().filter(|value| value.name == "AppliesTo") {
        assert_eq!(value.data, "System.ItemPathDisplay:~<\"C:\\Users\\Someone\\Beebeeb\"");
    }
    let commands: Vec<&str> = values
        .iter()
        .filter(|value| value.key.ends_with("\\command"))
        .map(|value| value.data.as_str())
        .collect();
    assert_eq!(
        commands,
        [
            "\"C:\\Program Files\\Beebeeb\\beebeeb-drive.exe\" --explorer-verb=share \"%1\"",
            "\"C:\\Program Files\\Beebeeb\\beebeeb-drive.exe\" --explorer-verb=copy-link \"%1\"",
            "\"C:\\Program Files\\Beebeeb\\beebeeb-drive.exe\" --explorer-verb=show \"%1\"",
            "\"C:\\Program Files\\Beebeeb\\beebeeb-drive.exe\" --explorer-verb=versions \"%1\"",
        ]
    );
    assert!(
        values
            .iter()
            .all(|value| !value.data.contains("://") && !value.data.contains(instance::OPEN_LINK_FLAG)),
        "nothing a web page can open"
    );
    // Windows splits the command and puts the file where `%1` is: the launch is that verb on that file.
    for verb in ExplorerVerb::ALL {
        let launch = instance::parse_args(
            [
                format!("{}{}", instance::EXPLORER_VERB_FLAG, verb.word()),
                r"C:\Users\Someone\Beebeeb\a.txt".to_string(),
            ],
            false,
            false,
        );
        let ask = launch.explorer.expect("an explorer launch");
        assert_eq!(ask.verb, Some(verb));
        assert_eq!(ask.path, Path::new(r"C:\Users\Someone\Beebeeb\a.txt"));
    }
    let test = menu_entries(AppIdentity::Test, exe, root);
    assert_eq!(test.len(), 16);
    // The two apps' keys never meet: both names through their own identity.
    for verb in ExplorerVerb::ALL {
        assert_ne!(
            verb_key(AppIdentity::Real.engine(), verb),
            verb_key(AppIdentity::Test.engine(), verb),
            "the test build never writes over the real app's keys"
        );
    }
    assert_eq!(test[0].data, "Share with Beebeeb Test\u{2026}");
}

/// Spec rule: Share and Copy link never act by themselves. Each opens the window's Share dialog on the file, on the
/// board's defaults, and the source is asked nothing: no link is made, nothing is copied, "Link copied" is not said.
/// Mutant: in `open`, Copy link calls `s.copy_link(tx)` (or Share presses `s.share_copy_link(tx)`) → a `CopyLink` or
/// `ShareLink` call is recorded.
#[test]
fn share_opens_the_dialog_and_shares_nothing_by_itself() {
    let mut opened = 0;
    for verb in [ExplorerVerb::Share, ExplorerVerb::CopyLink] {
        let (mut rt, calls, _) = window();
        select(&mut rt, "budget");
        let engine = Items::of(ACCOUNT);
        rt.update(|s, tx| handle_with(s, verb, "contract", Some(&engine), tx));
        settle(&mut rt);
        let s = &rt.state;
        let Some(Dialog::Share(share)) = &s.actions.dialog else {
            panic!("{verb:?}: the Share dialog is open");
        };
        let shared: Vec<&str> = share.items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(shared, ["contract"], "{verb:?}: on the file Explorer named");
        assert!(!share.password_on, "{verb:?}: the board's defaults");
        let asked = calls.lock().unwrap().clone();
        assert!(
            !asked
                .iter()
                .any(|call| matches!(call, Call::CopyLink(..) | Call::ShareLink(..))),
            "{verb:?}: no link was asked for: {asked:?}"
        );
        assert_eq!(s.live_toast(), None, "{verb:?}: nothing said as copied");
        assert_eq!(row_id(&rt), Some("contract".to_string()), "{verb:?}");
        assert_eq!(engine.asked.get(), 1, "{verb:?}");
        opened += 1;
    }
    assert_eq!(opened, 2);
}

/// Show in Beebeeb: Files, the file selected, nothing open. Version history…: DW-0413 on the file. An id the account's
/// state holds that is not in its folder's rows (a row the listing leaves out): the window is left as it was, the
/// selection and the open dialog with it. Mutant: in `open`, drop the early `return` when the folder's listing has no
/// such row → `close_dialogs` and `open_folder` run, the Version history dialog closes and the selection goes.
#[test]
fn show_selects_the_file_and_version_history_opens_its_versions() {
    let (mut rt, calls, _) = window();
    select(&mut rt, "budget");
    let engine = Items::of(ACCOUNT);
    rt.update(|s, tx| handle_with(s, ExplorerVerb::Show, "contract", Some(&engine), tx));
    settle(&mut rt);
    assert_eq!(row_id(&rt), Some("contract".to_string()));
    assert!(rt.state.actions.dialog.is_none(), "Show opens nothing");
    assert!(matches!(rt.state.shell.app.screen(), Screen::Drive(route) if route.section == Section::Files));

    rt.update(|s, tx| handle_with(s, ExplorerVerb::Versions, "budget", Some(&engine), tx));
    settle(&mut rt);
    assert!(
        matches!(&rt.state.actions.dialog, Some(Dialog::Versions(item)) if item.id == "budget"),
        "Version history of the file"
    );
    assert_eq!(row_id(&rt), Some("budget".to_string()));

    // The engine holds it, the folder's rows do not: the window stays as it was.
    let hidden = Items {
        account: ACCOUNT,
        held: &[("not-a-row", None)],
        asked: Cell::new(0),
    };
    let before = row_id(&rt);
    rt.update(|s, tx| handle_with(s, ExplorerVerb::Share, "not-a-row", Some(&hidden), tx));
    settle(&mut rt);
    assert_eq!(row_id(&rt), before);
    assert_eq!(before, Some("budget".to_string()));
    assert!(
        matches!(&rt.state.actions.dialog, Some(Dialog::Versions(item)) if item.id == "budget"),
        "the open dialog stays: no Share dialog for a row that is not there"
    );
    assert_eq!(hidden.asked.get(), 1);
    let asked = calls.lock().unwrap().clone();
    assert!(
        !asked
            .iter()
            .any(|call| matches!(call, Call::CopyLink(..) | Call::ShareLink(..))),
        "{asked:?}"
    );
}

/// Task 050: the running app reads a folder on a worker. A verb for a file in another folder shows the folder, and
/// selects the file's row (and runs the verb) once the read lands, not before and not never.
#[test]
fn a_verb_on_a_slow_folder_read_selects_its_row_once_it_lands() {
    let (mut rt, _, _) = window();
    let (folder, file) = rt
        .state
        .listing
        .items
        .iter()
        .filter(|i| i.is_folder && !i.is_protected)
        .find_map(|f| {
            let inside = rt.state.source.list(Some(&f.id)).ok()?;
            let file = inside.items.iter().find(|i| !i.is_folder)?;
            Some((f.id.clone(), file.id.clone()))
        })
        .expect("a folder with a file in the board");
    let (let_one_read, wait) = std::sync::mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    rt.state.folder_reads.reader = Some(Arc::new(move |folder: Option<&str>| {
        let _ = wait.lock().unwrap().recv();
        state::load(&BoardDrive::sharing(&FilesFixture::default().edits), folder)
    }));
    let (folder_s, file_s): (&'static str, &'static str) = (Box::leak(folder.into()), Box::leak(file.into()));
    let held: &'static [(&'static str, Option<&'static str>)] = Box::leak(vec![(file_s, Some(folder_s))].into_boxed_slice());
    let engine = Items {
        account: ACCOUNT,
        held,
        asked: Cell::new(0),
    };
    // The board starts with Contract draft.pdf selected (`state::init_from`): while the read is out the rows shown
    // stay, and their selection with them.
    let before = row_id(&rt);
    assert_ne!(before.as_deref(), Some(file_s), "the file's row is not the one selected at the start");
    rt.update(|s, tx| handle_with(s, ExplorerVerb::Show, file_s, Some(&engine), tx));
    assert!(rt.state.folder_reads.pending.is_some(), "the read is out");
    assert_ne!(rt.state.listing.folder.id.as_deref(), Some(folder_s), "the folder is not shown before its read lands");
    assert_eq!(row_id(&rt), before, "the file's row is not selected before the read lands: the selection is as it was");
    let_one_read.send(()).unwrap();
    for _ in 0..200 {
        if row_id(&rt).as_deref() == Some(file_s) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        settle(&mut rt);
    }
    assert_eq!(rt.state.listing.folder.id.as_deref(), Some(folder_s), "the file's folder is shown");
    assert_eq!(row_id(&rt), Some(file_s.to_string()), "the row is selected once the read landed");
}

/// The window locked: the verb is dropped, the engine is not asked, nothing opens, and nothing is kept for after the
/// unlock (the route machine keeps requests while the gate is closed; this one never reaches it). An account other
/// than the window's is dropped the same way.
#[test]
fn a_locked_window_only_comes_forward_and_the_verb_is_not_kept() {
    let (mut rt, calls, fixture) = window();
    let engine = Items::of(ACCOUNT);
    fixture.set_gate(GateState::Locked);
    settle(&mut rt);
    for verb in ExplorerVerb::ALL {
        rt.update(|s, tx| handle_with(s, verb, "contract", Some(&engine), tx));
        settle(&mut rt);
    }
    assert_eq!(engine.asked.get(), 0, "the engine is not asked behind a lock");
    assert!(rt.state.actions.dialog.is_none());
    fixture.set_gate(GateState::Ready);
    settle(&mut rt);
    assert!(rt.state.actions.dialog.is_none(), "nothing opens at the unlock");
    let other = Items::of("someone-else");
    rt.update(|s, tx| handle_with(s, ExplorerVerb::Share, "contract", Some(&other), tx));
    settle(&mut rt);
    assert!(
        rt.state.actions.dialog.is_none(),
        "another account's verb opens nothing"
    );
    assert_eq!(other.asked.get(), 0, "and its state is not read");
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| !matches!(call, Call::CopyLink(..) | Call::ShareLink(..)))
    );
}

/// Review of #463, P3-7: a verb that comes while the window is covered (the away lock's wake cover: the engine has not
/// said whether the wake locks) waits. Nothing is looked up, selected or opened behind the cover; once the cover is gone
/// the verb runs, checked again as it came. A cover that became the lock drops it, and nothing opens at the unlock. The
/// window's own poll runs what waits.
/// Mutants: in `handle_with`, delete the `if registry::covered(s) { … return; }` block -> red at "the engine is not asked
/// behind the cover" (Version history opens behind it); in `host::poll_screen`, delete the `after_the_cover` call -> red
/// at "the window's poll took what waited".
#[test]
fn a_verb_under_the_wake_cover_waits_for_its_end() {
    let waits = || WAITING.with_borrow(Option::is_some);
    // Driven by hand: the window's own poll would run what waits with the running instance's lookup (none here).
    let (mut rt, _, _) = window();
    rt.poll = None;
    select(&mut rt, "contract");
    let engine = Items::of(ACCOUNT);
    rt.update(|s, _| s.shell.away.set_cover_for_test(true));
    assert!(registry::covered(&rt.state));
    rt.update(|s, tx| handle_with(s, ExplorerVerb::Versions, "budget", Some(&engine), tx));
    settle(&mut rt);
    assert_eq!(engine.asked.get(), 0, "the engine is not asked behind the cover");
    assert!(rt.state.actions.dialog.is_none(), "nothing opens behind the cover");
    assert_eq!(
        row_id(&rt),
        Some("contract".to_string()),
        "nothing is selected behind it"
    );
    let mut ran = false;
    rt.update(|s, tx| ran = after_the_cover_with(s, Some(&engine), tx));
    assert!(!ran && waits(), "it waits while the window is covered");
    rt.update(|s, tx| {
        s.shell.away.set_cover_for_test(false);
        ran = after_the_cover_with(s, Some(&engine), tx);
    });
    settle(&mut rt);
    assert!(ran && !waits(), "the cover is gone: it ran");
    assert!(
        matches!(&rt.state.actions.dialog, Some(Dialog::Versions(item)) if item.id == "budget"),
        "Version history, once the cover is gone"
    );
    assert_eq!(engine.asked.get(), 1, "checked again as it came");

    // The cover became the lock: the verb goes, and nothing opens at the unlock.
    let (mut rt, calls, fixture) = window();
    rt.poll = None;
    let engine = Items::of(ACCOUNT);
    rt.update(|s, _| s.shell.away.set_cover_for_test(true));
    rt.update(|s, tx| handle_with(s, ExplorerVerb::Share, "contract", Some(&engine), tx));
    assert!(waits());
    fixture.set_gate(GateState::Locked);
    rt.update(|s, tx| {
        s.shell.refresh();
        ran = after_the_cover_with(s, Some(&engine), tx);
    });
    assert_eq!(rt.state.shell.app.gate(), GateState::Locked);
    assert!(!ran && !waits(), "the lock drops it");
    fixture.set_gate(GateState::Ready);
    rt.update(|s, tx| {
        s.shell.refresh();
        s.shell.away.set_cover_for_test(false);
        ran = after_the_cover_with(s, Some(&engine), tx);
    });
    settle(&mut rt);
    assert!(!ran, "nothing kept for after the unlock");
    assert!(rt.state.actions.dialog.is_none(), "nothing opens at the unlock");
    assert_eq!(engine.asked.get(), 0, "the engine was never asked");
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| !matches!(call, Call::CopyLink(..) | Call::ShareLink(..)))
    );

    // The window's own poll runs what waits once the cover is gone.
    let (mut rt, _, _) = window();
    rt.update(|s, _| s.shell.away.set_cover_for_test(true));
    rt.update(|s, tx| handle_with(s, ExplorerVerb::Show, "contract", Some(&engine), tx));
    assert!(waits());
    rt.update(|s, _| s.shell.away.set_cover_for_test(false));
    settle(&mut rt);
    assert!(!waits(), "the window's poll took what waited");
}
