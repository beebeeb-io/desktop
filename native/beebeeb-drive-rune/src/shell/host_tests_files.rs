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
        name: "the Share dialog",
        put: share_with_a_password,
        held: |s| s.actions.dialog.is_some(),
    },
    Piece {
        name: "the link's password",
        put: share_with_a_password,
        held: |s| !s.actions.share_password.text.is_empty(),
    },
    Piece {
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
        name: "what Try again uploads",
        put: |rt| {
            rt.update(|s, _| s.actions.retry_upload = Some((None, vec!["notes.txt".into()])));
        },
        held: |s| s.actions.retry_upload.is_some(),
    },
    Piece {
        name: "what Undo puts back",
        put: |rt| {
            rt.update(|s, tx| assert!(s.trash_selection(tx)));
            settle(rt);
            assert!(!rt.state.actions.undo.is_empty());
        },
        held: |s| !s.actions.undo.is_empty(),
    },
    Piece {
        name: "the toast",
        put: |rt| {
            rt.update(|s, tx| assert!(s.trash_selection(tx)));
            settle(rt);
            assert!(rt.state.actions.toast.is_some());
        },
        held: |s| s.actions.toast.is_some(),
    },
    Piece {
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
        name: "the Move to dialog",
        put: |rt| {
            rt.update(|s, tx| assert!(s.begin_move(tx)));
            settle(rt);
        },
        held: |s| s.actions.dialog.is_some(),
    },
    Piece {
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
        name: "the New folder dialog and its name",
        put: |rt| {
            rt.update(|s, _| assert!(s.begin_new_folder()));
            settle(rt);
        },
        held: |s| s.actions.dialog.is_some() || !s.actions.name_edit.text.is_empty(),
    },
    Piece {
        name: "the Preview and its page",
        put: |rt| {
            press(rt, Key::Text(" ".into()), Mods::default());
            assert!(matches!(rt.state.actions.dialog, Some(FilesDialog::Preview(_))));
        },
        held: |s| s.actions.dialog.is_some(),
    },
    Piece {
        name: "an open menu",
        put: |rt| {
            press(rt, Key::Function(10), shift());
            assert!(rt.state.actions.menu.is_some());
        },
        held: |s| s.actions.menu.is_some(),
    },
    Piece {
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
    for ending in [Ending::Lock, Ending::SignOut, Ending::OtherAccount] {
        for piece in PIECES {
            let (engine, _platform, mut rt) = fixture();
            (piece.put)(&mut rt);
            assert!((piece.held)(&rt.state), "{}: not set up", piece.name);
            end(&mut rt, &engine, ending);
            assert!(!(piece.held)(&rt.state), "{ending:?}: {} survived", piece.name);
            if matches!(ending, Ending::Lock) {
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
    assert_eq!(cases, 3 * 15, "every ending × every piece ran");
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
