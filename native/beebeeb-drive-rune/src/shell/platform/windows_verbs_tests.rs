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
    assert_eq!(keys[0], "Software\\Classes\\*\\shell\\Beebeeb.1.share");
    for value in &values {
        assert!(
            value.key.starts_with("Software\\Classes\\*\\shell\\Beebeeb."),
            "{}",
            value.key
        );
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
    assert!(
        test.iter().all(|t| values.iter().all(|real| real.key != t.key)),
        "the test build never writes over the real app's keys"
    );
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
