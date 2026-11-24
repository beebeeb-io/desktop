//! Review of #452 (P1-2): an edit kept open across a lock of the screen (Lock now, Windows' lock, the idle lock) is
//! saved after the unlock, from the version it began from; a file that changed meanwhile is the two-versions
//! question; a sign-out takes the edit and its text. On the real engine, behind the window: the engine's own save,
//! stored here (no sync runs in this harness). Every name and word is invented.

use beebeeb_engine::app::lock_api::LockState;
use beebeeb_engine::app::lock_password_api::Away;

use super::*;
use crate::frames::viewers::Dialog;

const TYPED: &str = "An invented line typed before the lock.";

impl Window {
    fn db(&self) -> StateDb {
        StateDb::open(self._state.path().join("state.db")).expect("the local state opens")
    }

    /// The note is the account's to change, at `version` (as another device's save leaves it).
    fn note_at(&self, version: i64) {
        let db = self.db();
        let mut contract = db.get_file_contract_state(NOTE).unwrap().expect("the note's record");
        contract.current_version = version;
        contract.permission_bits |=
            beebeeb_engine::state_db::PERMISSION_READ | beebeeb_engine::state_db::PERMISSION_WRITE;
        db.set_file_contract_state(&contract).unwrap();
    }

    /// The base version of every save of the note the engine stored.
    fn stored_saves(&self) -> Vec<Option<i64>> {
        self.db()
            .list_operations_for_file(NOTE)
            .unwrap()
            .iter()
            .map(|op| op.base_version)
            .collect()
    }

    /// Frames until `done` holds (at most 10 s): the engine's answer comes in a later poll.
    fn frames_until(&mut self, what: &str, done: impl Fn(&Window) -> bool) {
        let began = Instant::now();
        while !done(self) {
            assert!(began.elapsed() < Duration::from_secs(10), "{what}");
            self.settle();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The note open in the editor at version 3, with a line typed and not saved.
    fn editing(&mut self) {
        self.note_at(3);
        self.open_row(NOTE_NAME);
        self.until_drawn(NOTE_WORDS);
        self.rt.update(|s, _| {
            let ui = s.viewer.ui.as_mut().expect("the note is open");
            ui.edit();
            let editor = ui
                .editor
                .as_mut()
                .expect("the editor opens on a note the account may change");
            editor.edit.text.push_str(TYPED);
        });
        self.settle();
        assert!(
            self.rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.dirty()),
            "typed, not saved"
        );
    }

    fn save(&mut self) {
        self.rt
            .update(|s, _| s.viewer.ui.as_mut().expect("the edit is still open").save());
    }

    fn unlock(&mut self) {
        self.engine
            .unlock_screen_platform_verified(self.engine.session_turn())
            .expect("the screen opens");
        self.settle();
        assert!(
            self.rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.dirty()),
            "the same account back: the edit is still there"
        );
    }
}

fn signed_in_editing(tag: &str) -> Window {
    let mut w = window(tag);
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.editing();
    w
}

#[test]
fn an_edit_kept_across_a_screen_lock_is_saved_after_the_unlock_from_its_own_version() {
    let mut w = signed_in_editing("edit-lock");
    w.engine.lock_screen().expect("the screen locks");
    w.settle();
    assert!(!w.drawn().contains(TYPED), "the lock screen draws nothing of the edit");
    w.unlock();
    w.save();
    w.frames_until("the save was not stored", |w| !w.stored_saves().is_empty());
    assert_eq!(
        w.stored_saves(),
        [Some(3)],
        "stored from the version the edit began from"
    );
    w.frames_until("the editor did not close on the answer", |w| {
        w.rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.editor.is_none())
    });
}

#[test]
fn an_edit_kept_across_the_idle_lock_is_saved_after_the_unlock() {
    let mut w = signed_in_editing("edit-idle");
    let locked = w
        .engine
        .away_for(w.engine.session_turn(), Away::Idle(Duration::from_secs(3_600)));
    assert_eq!(locked.state, LockState::ScreenLocked);
    w.settle();
    w.unlock();
    w.save();
    w.frames_until("the save was not stored", |w| !w.stored_saves().is_empty());
    assert_eq!(w.stored_saves(), [Some(3)]);
}

#[test]
fn a_file_changed_during_the_lock_is_the_two_versions_question_never_overwritten() {
    let mut w = signed_in_editing("edit-conflict");
    w.engine.lock_screen().expect("the screen locks");
    w.settle();
    // Another device saved version 4 while the screen was locked.
    w.note_at(4);
    w.unlock();
    w.save();
    w.frames_until("the two-versions question did not come", |w| {
        w.rt.state
            .viewer
            .ui
            .as_ref()
            .is_some_and(|ui| matches!(ui.dialog, Some(Dialog::Conflict(_))))
    });
    assert!(w.stored_saves().is_empty(), "nothing stored over the other version");
}

#[test]
fn a_sign_out_takes_the_edit_and_its_text() {
    let mut w = signed_in_editing("edit-signout");
    w.engine.sign_out_keeping_local_data().expect("the sign-out");
    w.settle();
    assert!(w.rt.state.viewer.ui.is_none(), "no editor after a sign-out");
    assert!(!w.drawn().contains(TYPED));
    // The same account signs in again: the edit does not come back, and nothing was stored.
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.settle();
    assert!(w.rt.state.viewer.ui.as_ref().is_none_or(|ui| ui.editor.is_none()));
    assert!(!w.drawn().contains(TYPED));
    assert!(w.stored_saves().is_empty());
}

/// A lock that closes the vault (the key leaves memory; the master key opens it again), the same account: the
/// edit kept open across it is saved after the unlock too.
#[test]
fn an_edit_kept_across_a_closed_vault_is_saved_after_the_phrase_opens_it() {
    let mut w = signed_in_editing("edit-vault");
    w.engine.lock();
    w.settle();
    assert!(!w.drawn().contains(TYPED));
    w.engine
        .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.to_string()))
        .expect("the phrase unlocks");
    w.settle();
    assert!(
        w.rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.dirty()),
        "the same account back: the edit is still there"
    );
    w.save();
    w.frames_until("the save was not stored", |w| !w.stored_saves().is_empty());
    assert_eq!(w.stored_saves(), [Some(3)]);
}

/// Review of #452 (P2-B): a press that reaches the edit while Beebeeb is locked (the lock landed, the screen not redrawn
/// yet: Ctrl+S in that gap) stores nothing and uses nothing up. After the Hello unlock, Save lands.
#[test]
fn a_press_while_locked_uses_nothing_up_and_save_lands_after_the_unlock() {
    let mut w = signed_in_editing("edit-locked-press");
    w.engine.lock_screen().expect("the screen locks");
    w.settle();
    w.save();
    w.settle();
    assert!(w.stored_saves().is_empty(), "nothing is stored while locked");
    w.unlock();
    w.save();
    w.frames_until("the save was not stored after the unlock", |w| {
        !w.stored_saves().is_empty()
    });
    assert_eq!(w.stored_saves(), [Some(3)], "from the version the edit began from");
}
