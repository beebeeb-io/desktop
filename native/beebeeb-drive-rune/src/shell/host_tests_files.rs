//! The shell's tests of what the Files window forgets (`frames::files::reset`): a session that
//! ends (lock, sign-out, another account) takes the whole window state with it, and a route that
//! leaves the Files window closes its dialogs through the wiping path.

use super::*;
use crate::frames::files::dialogs::Dialog as FilesDialog;
use crate::frames::files::share::{PASSWORD_INPUT, Share};

fn click(rt: &mut Runtime<State>, key: &str) {
    let e = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"));
    let (x, y) = (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        press(rt, Key::Text(c.to_string()), Mods::default());
    }
}

/// The Share dialog of the selected file with a password typed in it.
fn share_with_a_password(rt: &mut Runtime<State>) {
    click(rt, "share-box");
    click(rt, "item-0-2-switch");
    type_text(rt, "s3cret-words");
    assert_eq!(rt.state.actions.share_password.text, "s3cret-words");
}

/// The three ways a session ends under the window.
#[derive(Clone, Copy, Debug)]
enum Ending {
    Lock,
    SignOut,
    OtherAccount,
}

fn end(rt: &mut Runtime<State>, engine: &FixtureEngine, ending: Ending) {
    match ending {
        Ending::Lock => engine.set_gate(GateState::Locked),
        Ending::SignOut => engine.set_gate(GateState::SignedOut),
        // Another account under the same open gate: the engine switched without a lock between.
        Ending::OtherAccount => engine.set_account("user-b"),
    }
    settle(rt);
    settle(rt);
}

/// One piece of the Files window's state: how it gets there, and whether it is still there.
struct Piece {
    name: &'static str,
    /// The shell's own dialog: its route machine keeps it across a lock, by its rule.
    shell_keeps_it_across_a_lock: bool,
    put: fn(&mut Runtime<State>),
    held: fn(&State) -> bool,
}

/// A folder of the folder shown that has a file in it.
fn first_folder(s: &State) -> Option<String> {
    s.listing
        .items
        .iter()
        .filter(|i| i.is_folder && !i.is_protected)
        .find(|i| {
            s.source
                .list(Some(&i.id))
                .is_ok_and(|inside| inside.items.iter().any(|f| !f.is_folder))
        })
        .map(|i| i.id.clone())
}

const PIECES: &[Piece] = &[
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the Share dialog",
        put: share_with_a_password,
        held: |s| s.actions.dialog.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the link's password",
        put: share_with_a_password,
        held: |s| !s.actions.share_password.text.is_empty(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "what Try again shares",
        put: |rt| {
            rt.update(|s, _| {
                let items = vec![s.listing.items[6].clone()];
                s.actions.retry_share = Some(Share {
                    items,
                    access: Default::default(),
                    expires: Default::default(),
                    password_on: true,
                    password_missing: false,
                    menu: None,
                });
            });
        },
        held: |s| s.actions.retry_share.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "what Try again uploads",
        put: |rt| {
            rt.update(|s, _| s.actions.retry_upload = Some((None, vec!["notes.txt".into()])));
        },
        held: |s| s.actions.retry_upload.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "what Undo puts back",
        put: |rt| {
            rt.update(|s, tx| assert!(s.trash_selection(tx)));
            settle(rt);
            assert!(!rt.state.actions.undo.is_empty());
        },
        held: |s| !s.actions.undo.is_empty(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the toast",
        put: |rt| {
            rt.update(|s, tx| assert!(s.trash_selection(tx)));
            settle(rt);
            assert!(rt.state.actions.toast.is_some());
        },
        held: |s| s.actions.toast.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the selected rows",
        put: |rt| {
            rt.update(|s, _| {
                s.actions.selection.extend([2, 3]);
                s.actions.anchor = Some(2);
            });
        },
        held: |s| !s.actions.selection.is_empty() || s.actions.anchor.is_some() || s.selected.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "where Back goes",
        put: |rt| {
            rt.update(|s, _| {
                let folder = first_folder(s).expect("a folder in the sample");
                s.open_folder(Some(folder));
            });
            settle(rt);
            assert!(!rt.state.files.back.is_empty());
        },
        held: |s| !s.files.back.is_empty(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "where Forward goes",
        put: |rt| {
            rt.update(|s, _| {
                let folder = first_folder(s).expect("a folder in the sample");
                s.open_folder(Some(folder));
            });
            settle(rt);
            rt.update(|s, _| assert!(s.back()));
            settle(rt);
            assert!(!rt.state.actions.forward.is_empty());
        },
        held: |s| !s.actions.forward.is_empty(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the Move to dialog",
        put: |rt| {
            rt.update(|s, tx| assert!(s.begin_move(tx)));
            settle(rt);
        },
        held: |s| s.actions.dialog.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the name being typed in a row",
        put: |rt| {
            rt.update(|s, tx| assert!(s.begin_rename(tx)));
            settle(rt);
            type_text(rt, "Mine");
            assert!(rt.state.actions.name_edit.text.contains("Mine"));
        },
        held: |s| s.actions.rename.is_some() || !s.actions.name_edit.text.is_empty(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the New folder dialog and its name",
        put: |rt| {
            rt.update(|s, _| assert!(s.begin_new_folder()));
            settle(rt);
        },
        held: |s| s.actions.dialog.is_some() || !s.actions.name_edit.text.is_empty(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the Preview and its page",
        put: |rt| {
            press(rt, Key::Text(" ".into()), Mods::default());
            assert!(matches!(rt.state.actions.dialog, Some(FilesDialog::Preview(_))));
        },
        held: |s| s.actions.dialog.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "an open menu",
        put: |rt| {
            press(rt, Key::Function(10), shift());
            assert!(rt.state.actions.menu.is_some());
        },
        held: |s| s.actions.menu.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the folders made and the files answered",
        put: |rt| {
            rt.update(|s, _| {
                let made = s.listing.items[0].clone();
                s.actions.made.push(made);
                s.actions.conflicts_done = 1;
            });
        },
        held: |s| !s.actions.made.is_empty() || s.actions.conflicts_done != 0,
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "when the toast goes",
        put: |rt| {
            rt.update(|s, _| {
                s.actions.toast_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
            });
        },
        held: |s| s.actions.toast_until.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "a refused name",
        put: |rt| {
            rt.update(|s, _| {
                s.actions.name_error = Some(("Lisbon".into(), crate::frames::files::names::NameError::Empty));
            });
        },
        held: |s| s.actions.name_error.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the message of a protected folder",
        put: |rt| {
            rt.update(|s, _| {
                s.actions.protected.alert = Some(crate::frames::files::protected::Words {
                    title: "Backups can't be renamed".into(),
                    body: "It holds the backups of Anna's iPhone.".into(),
                });
            });
        },
        held: |s| s.actions.protected.alert.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "Search: the words typed, the narrowing, the selected result",
        put: |rt| {
            rt.update(|s, tx| {
                crate::frames::search::open(s, tx);
                s.search = rune::TextEdit::new("contract");
                s.frames.find.selected = Some("contract".into());
            });
            settle(rt);
            assert!(rt.state.frames.find.open, "Search is open");
            assert_eq!(rt.state.search.text, "contract");
        },
        held: |s| s.frames.find.open || !s.search.text.is_empty() || s.frames.find.selected.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the session-ended rows and refusal count",
        put: |rt| {
            rt.update(|s, _| {
                // The other answer: the rows are held out of the listing while the session ended.
                s.actions.session.keep_the_list = Some(false);
                s.actions.session.asked = 3;
                // Take the listing into session::Ui::held as poll() would.
                crate::frames::files::session::poll(s);
            });
            settle(rt);
            assert!(
                rt.state.actions.session.asked > 0 || rt.state.listing.items.is_empty(),
                "session-ended state not set up"
            );
        },
        // What the window keeps of that state (`session::Ui`). An empty list no longer says
        // so: a gate that closed reads no folder, so the list is empty until it opens again.
        held: |s| s.actions.session.asked > 0 || s.actions.session.keep_the_list.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: true,
        name: "the shell's Two versions dialog",
        put: |rt| {
            rt.update(|s, _| {
                dispatch(s, Event::Request(Intent::ReviewConflict { file: None }));
            });
            settle(rt);
            assert!(rt.state.shell.app.dialog().is_some());
        },
        held: |s| s.shell.app.dialog().is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "a picker that was asked for",
        put: |rt| {
            rt.update(|s, _| s.ask_picker(crate::frames::files::uploads::PickerKind::Files));
            assert!(rt.state.actions.picker.is_some());
        },
        held: |s| s.actions.picker.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "a drag over the list",
        put: |rt| {
            rt.update(|s, _| {
                s.actions.drop_hover = Some(crate::frames::files::uploads::DropHover {
                    row: None,
                    count: Some(3),
                });
            });
        },
        held: |s| s.actions.drop_hover.is_some(),
    },
    Piece {
        shell_keeps_it_across_a_lock: false,
        name: "the open Uploads panel and its row menu",
        put: |rt| {
            rt.update(|s, _| {
                s.actions.uploads_open = true;
                s.actions.upload_menu = Some("u-1".into());
            });
        },
        held: |s| s.actions.uploads_open || s.actions.upload_menu.is_some(),
    },
];

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

/// Lock, sign-out and another account × every piece: it was there, and it is gone. After a lock
/// it is still gone when the app is unlocked again.
#[test]
fn every_ending_of_a_session_takes_every_piece_of_the_files_window_with_it() {
    let mut cases = 0;
    let mut kept_by_the_shell = 0;
    for ending in [Ending::Lock, Ending::SignOut, Ending::OtherAccount] {
        for piece in PIECES {
            let (engine, _platform, mut rt) = fixture();
            (piece.put)(&mut rt);
            assert!((piece.held)(&rt.state), "{}: not set up", piece.name);
            end(&mut rt, &engine, ending);
            let lock = matches!(ending, Ending::Lock);
            if lock && piece.shell_keeps_it_across_a_lock {
                cases += 1;
                kept_by_the_shell += 1;
                continue;
            }
            assert!(!(piece.held)(&rt.state), "{ending:?}: {} survived", piece.name);
            if lock {
                engine.set_gate(GateState::Ready);
                settle(&mut rt);
                settle(&mut rt);
                assert!(!(piece.held)(&rt.state), "unlock: {} came back", piece.name);
                assert!(
                    rt.elements().iter().all(|e| e.role != "dialog"),
                    "unlock: a dialog is drawn again ({})",
                    piece.name
                );
            }
            cases += 1;
        }
    }
    assert_eq!(cases, 3 * 24, "every ending × every piece ran");
    assert_eq!(
        kept_by_the_shell, 1,
        "one case is the shell's rule, not this window's: Two versions across a lock"
    );
}

/// A lock with the link's password on screen: the dialog and the password are gone, and the
/// window is capturable again once the app is unlocked (the lock screen itself is excluded).
#[test]
fn a_lock_over_the_share_dialog_wipes_it_and_unlock_leaves_the_window_capturable() {
    let (engine, platform, mut rt) = fixture();
    let last = |platform: &FakePlatform| platform.capture_exclusions.lock().unwrap().last().copied();
    share_with_a_password(&mut rt);
    assert_eq!(last(&platform), Some(true), "the password field excludes the window");
    end(&mut rt, &engine, Ending::Lock);
    assert_eq!(last(&platform), Some(true), "the lock screen is excluded");
    assert!(rt.state.actions.dialog.is_none());
    assert_eq!(rt.state.actions.share_password.text, "");
    engine.set_gate(GateState::Ready);
    settle(&mut rt);
    settle(&mut rt);
    assert!(rt.state.actions.dialog.is_none(), "the Share dialog is not back");
    assert!(rt.elements().iter().all(|e| e.key.as_deref() != Some(PASSWORD_INPUT)));
    assert_eq!(last(&platform), Some(false), "capturable again after the unlock");
}

/// Sign-out, then another account signs in: no file name of the earlier session is anywhere in
/// the element tree, and Ctrl+Z puts nothing of it back.
#[test]
fn after_a_sign_out_the_next_account_sees_no_file_name_of_the_earlier_session() {
    let (engine, _platform, mut rt) = fixture();
    // Into a folder, and a file of it in three places: the Share dialog (with a password), what
    // Try again shares, and what Undo puts back.
    rt.update(|s, _| {
        let folder = first_folder(s).expect("a folder in the sample");
        s.open_folder(Some(folder));
    });
    settle(&mut rt);
    let inside: Vec<String> = rt.state.listing.items.iter().map(|i| i.name.clone()).collect();
    let file = rt
        .state
        .listing
        .items
        .iter()
        .position(|i| !i.is_folder)
        .expect("a file in the folder");
    let name = inside[file].clone();
    rt.update(move |s, tx| {
        s.select(file);
        assert!(s.trash_selection(tx));
    });
    settle(&mut rt);
    let rows_after_trash = rt.state.listing.items.len();
    rt.update(|s, _| {
        s.select(0);
        let items = s.listing.items.first().cloned().into_iter().collect();
        s.actions.retry_share = Some(Share {
            items,
            access: Default::default(),
            expires: Default::default(),
            password_on: false,
            password_missing: false,
            menu: None,
        });
        assert!(s.begin_share());
    });
    settle(&mut rt);
    assert!(
        texts(&rt).iter().any(|t| inside.contains(t)),
        "the folder's names are drawn"
    );

    engine.set_gate(GateState::SignedOut);
    settle(&mut rt);
    settle(&mut rt);
    engine.set_account("user-b");
    engine.set_gate(GateState::Ready);
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.account(), Some("user-b"));
    // The window is at the root again; nothing of the folder is drawn or named.
    let root: Vec<String> = rt.state.listing.items.iter().map(|i| i.name.clone()).collect();
    let earlier: Vec<&String> = inside.iter().filter(|n| !root.contains(n)).collect();
    assert!(earlier.contains(&&name), "the trashed file is no root row");
    let leaked: Vec<String> = rt
        .elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .filter(|words| earlier.iter().any(|n| words.contains(n.as_str())))
        .collect();
    assert!(leaked.is_empty(), "the earlier session's names are drawn: {leaked:?}");
    assert!(
        rt.elements().iter().all(|e| e.role != "dialog"),
        "no dialog of the earlier session"
    );
    // Ctrl+Z: nothing is put back.
    let rows = rt.state.listing.items.len();
    press(&mut rt, Key::Text("z".into()), ctrl());
    assert_eq!(
        rt.state.listing.items.len(),
        rows,
        "Undo put a row of the earlier session back"
    );
    assert!(rt.state.listing.items.iter().all(|i| i.name != name));
    let _ = rows_after_trash;
}

/// Leaving the Files route (Settings opens over an open Share dialog): the dialog closes through
/// the wiping path, the window is capturable again, and coming back shows no dialog.
#[test]
fn leaving_the_files_route_closes_its_dialogs_and_wipes_the_password() {
    let (_engine, platform, mut rt) = fixture();
    let last = |platform: &FakePlatform| platform.capture_exclusions.lock().unwrap().last().copied();
    share_with_a_password(&mut rt);
    rt.update(|s, _| {
        dispatch(s, Event::OpenSettings(crate::shell::app::SettingsPane::General));
    });
    settle(&mut rt);
    settle(&mut rt);
    assert!(matches!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Settings(_)
    ));
    assert!(
        rt.state.actions.dialog.is_none(),
        "the Share dialog closed with the route"
    );
    assert_eq!(rt.state.actions.share_password.text, "", "its password is wiped");
    assert_eq!(last(&platform), Some(false), "capturable again");
    go(&mut rt, Section::Files);
    assert!(rt.state.actions.dialog.is_none());
    assert!(rt.elements().iter().all(|e| e.role != "dialog"));
    // Another section does the same for a dialog without a secret (Move to).
    rt.update(|s, tx| {
        s.select(6);
        assert!(s.begin_move(tx));
    });
    go(&mut rt, Section::Shared);
    assert!(rt.state.actions.dialog.is_none(), "Move to closed with the route");
}

/// A source that lists PER ACCOUNT: the first account has the board's files, any other one file
/// of its own. The test names the account beside the engine.
#[derive(Clone)]
struct PerAccount {
    who: Arc<std::sync::Mutex<String>>,
}

const FIRST_ACCOUNT: &str = "fixture-account";
const ONLY_THE_SECOND_HAS: &str = "Second account only.txt";

impl crate::source::DriveSource for PerAccount {
    fn list(&self, folder_id: Option<&str>) -> Result<crate::source::Listing, crate::source::SourceError> {
        let mut listing = crate::source::DriveSource::list(&crate::source::BoardFixtureSource::new(), folder_id)?;
        if *self.who.lock().unwrap() != FIRST_ACCOUNT {
            listing.items.retain(|i| !i.is_folder);
            listing.items.truncate(1);
            listing.items[0].name = ONLY_THE_SECOND_HAS.to_string();
        }
        Ok(listing)
    }
    fn storage(&self) -> Option<crate::source::StorageSummary> {
        None
    }
}

fn per_account() -> (Arc<FixtureEngine>, PerAccount, Runtime<State>) {
    let engine = Arc::new(FixtureEngine::ready());
    let source = PerAccount {
        who: Arc::new(std::sync::Mutex::new(FIRST_ACCOUNT.to_string())),
    };
    let shell = Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default()));
    let mut rt = runtime(state::init_from(shell, Box::new(source.clone()), None));
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.account(), Some(FIRST_ACCOUNT));
    (engine, source, rt)
}

fn names(rt: &Runtime<State>) -> Vec<String> {
    rt.state.listing.items.iter().map(|i| i.name.clone()).collect()
}

/// Another account while the gate stays Ready: the rows are the session's too. The earlier
/// account's are gone from the state and from the element tree, and the new account's are read.
#[test]
fn another_account_under_an_open_gate_sees_its_own_rows_and_none_of_the_earlier_ones() {
    let (engine, source, mut rt) = per_account();
    let earlier = names(&rt);
    assert!(earlier.len() > 3 && !earlier.contains(&ONLY_THE_SECOND_HAS.to_string()));
    // Inside a folder, so the path and the Back stack are the earlier account's as well.
    rt.update(|s, _| {
        let folder = first_folder(s).expect("a folder in the sample");
        s.open_folder(Some(folder));
    });
    settle(&mut rt);
    let inside = names(&rt);

    *source.who.lock().unwrap() = "user-b".into();
    engine.set_account("user-b");
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.account(), Some("user-b"));
    assert_eq!(rt.state.shell.app.gate(), GateState::Ready, "the gate never moved");
    assert_eq!(
        names(&rt),
        [ONLY_THE_SECOND_HAS],
        "the root of the account that is signed in now"
    );
    assert!(rt.state.listing.folder.id.is_none(), "at the root");
    let drawn: Vec<String> = rt
        .elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .collect();
    let leaked: Vec<&String> = drawn
        .iter()
        // "Backups" is also the sidebar's own word: it is no file name of an account.
        .filter(|words| {
            earlier
                .iter()
                .chain(&inside)
                .filter(|n| n.as_str() != "Backups")
                .any(|n| words.contains(n.as_str()))
        })
        .collect();
    assert!(leaked.is_empty(), "names of the earlier account are drawn: {leaked:?}");
    assert!(drawn.iter().any(|w| w.contains(ONLY_THE_SECOND_HAS)));
}

/// The same when the poll never sees the change happen: a sign-out and the next sign-in are both
/// first seen by a refresh between polls (the engine's status events refresh the session too).
#[test]
fn a_change_of_account_seen_between_polls_still_reads_the_rows_again() {
    let (engine, source, mut rt) = per_account();
    let earlier = names(&rt);
    engine.set_gate(GateState::SignedOut);
    rt.update(|s, _| {
        s.shell.refresh();
    });
    *source.who.lock().unwrap() = "user-b".into();
    engine.set_account("user-b");
    engine.set_gate(GateState::Ready);
    rt.update(|s, _| {
        s.shell.refresh();
    });
    assert_eq!(names(&rt), earlier, "nothing read yet: no poll ran");
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.account(), Some("user-b"));
    assert_eq!(names(&rt), [ONLY_THE_SECOND_HAS]);
}

/// Share… on a protected item (a device's backup) is refused before its dialog opens: it says
/// so with NO Try again, and nothing makes a plain link afterwards, whatever is selected by then.
#[test]
fn share_refused_on_a_protected_item_offers_no_try_again_and_never_makes_a_plain_link() {
    use crate::frames::files::source::{Action, ActionError, FilesFixture};
    let (_engine, _platform, mut rt) = fixture();
    let fixture = FilesFixture::default();
    let calls = fixture.calls.clone();
    rt.update(move |s, _| {
        s.source = Box::new(crate::frames::files::source::BoardDrive::sharing(&fixture.edits));
        s.actions.source = Box::new(fixture);
    });
    settle(&mut rt);
    click(&mut rt, "Backups-row");
    settle(&mut rt);
    click(&mut rt, "row-0-date");
    assert!(rt.state.protects_selection(), "a device's backup is selected");
    // By the state's own entry (the file menu's and the details panel's Share… both call it).
    rt.update(|s, tx| s.ask_share(tx));
    settle(&mut rt);
    // And the dialog itself refuses a protected item, whoever asks.
    rt.update(|s, _| assert!(!s.begin_share()));
    assert!(
        rt.state.actions.dialog.is_none(),
        "no Share dialog for a protected item"
    );
    assert_eq!(
        rt.state.live_toast(),
        Some(crate::frames::files::Toast::Failed(
            Action::Share,
            ActionError::Protected
        ))
    );
    assert!(
        rt.elements().iter().all(|e| e.key.as_deref() != Some("toast-action")),
        "the refusal offers no Try again"
    );
    // Even asked for directly, Try again repeats nothing.
    rt.update(|s, tx| s.retry(tx));
    settle(&mut rt);
    assert!(
        calls.lock().unwrap().is_empty(),
        "0 calls of the source: {:?}",
        calls.lock().unwrap()
    );
}

/// The sidebar's navigation does not go through `dispatch`: the Files window still closes what
/// was open in it. A name being typed in a row does not come along to another section.
#[test]
fn a_sidebar_click_ends_a_name_being_typed_in_a_row() {
    let (_engine, _platform, mut rt) = fixture();
    rt.update(|s, tx| assert!(s.begin_rename(tx)));
    settle(&mut rt);
    type_text(&mut rt, "Mine");
    assert!(rt.state.actions.rename.is_some());
    rt.update(|s, _| s.sidebar_section = crate::ui::sidebar::SidebarSection::Shared);
    settle(&mut rt);
    assert_eq!(
        registry::group_for(&rt.state.shell.app),
        FrameGroup::Section(Section::Shared)
    );
    assert!(rt.state.actions.rename.is_none(), "the rename ended with the route");
    assert_eq!(rt.state.actions.name_edit.text, "", "and what was typed is gone");
}

/// A message that went takes what its Try again held with it.
#[test]
fn what_try_again_holds_goes_when_its_message_goes() {
    use crate::frames::files::source::{Action, ActionError};
    let (_engine, _platform, mut rt) = fixture();
    rt.update(|s, _| {
        let items = vec![s.listing.items[6].clone()];
        s.actions.retry_share = Some(Share {
            items,
            access: Default::default(),
            expires: Default::default(),
            password_on: true,
            password_missing: false,
            menu: None,
        });
        s.actions.retry_upload = Some((None, vec!["notes.txt".into()]));
        s.actions.toast = Some(crate::frames::files::Toast::Failed(
            Action::ShareLink,
            ActionError::Offline,
        ));
        // Still showing: nothing goes yet.
        s.actions.toast_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
    });
    settle(&mut rt);
    assert!(rt.state.actions.retry_share.is_some() && rt.state.actions.toast.is_some());
    rt.update(|s, _| s.actions.toast_until = Some(std::time::Instant::now()));
    settle(&mut rt);
    assert!(rt.state.actions.toast.is_none(), "the message is gone from the state");
    assert!(
        rt.state.actions.retry_share.is_none(),
        "and the share it would have reopened"
    );
    assert!(
        rt.state.actions.retry_upload.is_none(),
        "and the upload it would have repeated"
    );
}

// ---- Uploading across a session's end (review of the uploads PR) ----

use crate::frames::files::source::{Call, FilesFixture};
use crate::frames::files::uploads::PickerKind;

type Calls = Arc<std::sync::Mutex<Vec<Call>>>;

/// The shell's window on the board's sample, with a Files source that records what it is asked.
fn recording() -> (Arc<FixtureEngine>, Runtime<State>, Calls) {
    let (engine, _platform, mut rt) = fixture();
    let fixture = FilesFixture::default();
    let calls = fixture.calls.clone();
    rt.update(move |s, _| {
        s.source = Box::new(crate::frames::files::source::BoardDrive::sharing(&fixture.edits));
        s.actions.source = Box::new(fixture);
    });
    settle(&mut rt);
    (engine, rt, calls)
}

fn two_files() -> Vec<std::path::PathBuf> {
    vec!["a.heic".into(), "b.heic".into()]
}

fn status_text(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("status-text") && !e.exiting)
        .and_then(|e| e.text)
        .unwrap_or_default()
}

/// The uploads live in the SOURCE. When the account leaves (a sign-out, another account) the
/// source is asked, once, to cancel and forget them: the next account sees no status line, no
/// panel and no upload of the earlier one. A lock is the same account: its uploads go on.
#[test]
fn the_account_that_leaves_takes_its_uploads_with_it_and_a_lock_does_not() {
    for ending in [Ending::Lock, Ending::SignOut, Ending::OtherAccount] {
        let (engine, mut rt, calls) = recording();
        rt.update(|s, tx| assert!(s.upload(None, &two_files(), tx)));
        settle(&mut rt);
        assert!(
            status_text(&rt).starts_with("Uploading"),
            "{ending:?}: something uploads"
        );
        rt.update(|s, _| s.status_pressed());
        settle(&mut rt);
        assert!(rt.state.actions.uploads_open);
        calls.lock().unwrap().clear();

        end(&mut rt, &engine, ending);
        assert!(!rt.state.actions.uploads_open, "{ending:?}: the panel closed");
        let forgotten = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == Call::ForgetUploads)
            .count();
        if matches!(ending, Ending::Lock) {
            assert_eq!(forgotten, 0, "a lock cancels nothing");
            assert!(
                rt.state.actions.source.uploads().is_some(),
                "the uploads go on under a lock"
            );
            continue;
        }
        assert_eq!(forgotten, 1, "{ending:?}: the source is asked once to forget");
        assert!(
            rt.state.actions.source.uploads().is_none(),
            "{ending:?}: nothing uploads"
        );
        // The next account's window.
        engine.set_account("user-b");
        engine.set_gate(GateState::Ready);
        settle(&mut rt);
        settle(&mut rt);
        assert_eq!(rt.state.shell.app.account(), Some("user-b"));
        assert!(
            !status_text(&rt).contains("Uploading"),
            "{ending:?}: no earlier status line"
        );
        assert!(
            rt.elements()
                .iter()
                .all(|e| e.key.as_deref().is_none_or(|k| !k.starts_with("upload"))),
            "{ending:?}: no element of the earlier account's uploads"
        );
    }
}

/// A picker that was asked for under one session and answers under the next: the answer is
/// dropped. Its folder is the earlier account's.
#[test]
fn a_picker_answer_that_comes_back_under_another_session_uploads_nothing() {
    // The control: the same answer under the same session uploads.
    let (_engine, mut rt, calls) = recording();
    rt.update(|s, _| s.ask_picker(PickerKind::Files));
    let ask = rt.state.take_picker().expect("the picker was asked for");
    let same = ask.clone();
    rt.update(move |s, tx| assert!(s.picked(&same, Some(two_files()), tx)));
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "one upload under the session it was asked in"
    );

    for ending in [Ending::Lock, Ending::SignOut, Ending::OtherAccount] {
        let (engine, mut rt, calls) = recording();
        rt.update(|s, _| s.ask_picker(PickerKind::Files));
        let ask = rt.state.take_picker().expect("the picker was asked for");
        end(&mut rt, &engine, ending);
        engine.set_account("user-b");
        engine.set_gate(GateState::Ready);
        settle(&mut rt);
        settle(&mut rt);
        calls.lock().unwrap().clear();
        rt.update(move |s, tx| assert!(!s.picked(&ask, Some(two_files()), tx)));
        settle(&mut rt);
        let made: Vec<Call> = calls.lock().unwrap().clone();
        assert!(
            made.iter().all(|c| !matches!(c, Call::Upload(..))),
            "{ending:?}: a late answer uploaded: {made:?}"
        );
        assert!(rt.state.actions.source.uploads().is_none());
    }
}

/// The session ended (the engine says so; the window keeps its list): a drop, a picked file and
/// the panel's Try again, Pause all and Cancel upload are refused before the source is asked.
#[test]
fn on_a_session_ended_window_no_upload_path_reaches_the_source() {
    use crate::frames::global::source::{GlobalFixture, board};
    let (_engine, mut rt, calls) = recording();
    // Something uploads and a picker is out while the session is still there.
    rt.update(|s, tx| assert!(s.upload(None, &two_files(), tx)));
    rt.update(|s, _| s.ask_picker(PickerKind::Files));
    let ask = rt.state.take_picker().expect("the picker was asked for");
    settle(&mut rt);
    calls.lock().unwrap().clear();
    rt.update(|s, _| {
        s.global = crate::frames::global::Ui::new(Box::new(GlobalFixture::of(board::signed_out())));
    });
    settle(&mut rt);
    assert!(rt.state.session_ended());
    let refused_before = rt.state.actions.session.asked;

    let folder = first_folder(&rt.state);
    rt.update(move |s, tx| {
        // A drop on the list and on a folder row.
        assert!(!s.upload(None, &two_files(), tx));
        assert!(!s.upload(folder.clone(), &two_files(), tx));
        // The picker's answer.
        assert!(!s.picked(&ask, Some(two_files()), tx));
        // The panel.
        s.retry_one_upload("up-scan", tx);
        s.pause_uploads(tx);
        s.cancel_one_upload("up-ponte", tx);
    });
    settle(&mut rt);
    let made: Vec<Call> = calls.lock().unwrap().clone();
    assert!(made.is_empty(), "the source was asked: {made:?}");
    assert_eq!(
        rt.state.actions.session.asked - refused_before,
        6,
        "each of the six was refused by the session guard"
    );
    assert!(rt.state.actions.toast.is_none(), "a refusal is no failure message");
}
