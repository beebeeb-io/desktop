//! The steps after the password and beside it: the two-factor code (DW-A26 to DW-A28), the
//! twelve words on a new device (DW-A23 to DW-A25), a passkey (DW-A29, DW-A30) and the honest
//! exits of "Forgot password?" (DW-A45, DW-A46). State and rules only; `view_steps.rs` and
//! `view_recovery.rs` draw them.
//!
//! Every answer of the engine is applied only by the wait it answers ([`Ui::apply_step`]: each
//! arm has a guard), as in `mod.rs`.

use rune::TextEdit;

use super::source::{
    CodeKind, NextAfterSignIn, PasskeyOutcome, PhraseProblem, SignInEvent, SignInProblem, TwoFactorProblem,
};
use super::{Intent, PASSWORD_KEY, Screen, TITLE_KEY, Ui};

/// The authenticator code's one input.
pub const CODE_KEY: &str = "signin-code";
/// The backup code's field.
pub const BACKUP_KEY: &str = "signin-backup-code";
/// The twelve word boxes' inputs.
pub const WORD_KEYS: [&str; 12] = [
    "signin-word-1",
    "signin-word-2",
    "signin-word-3",
    "signin-word-4",
    "signin-word-5",
    "signin-word-6",
    "signin-word-7",
    "signin-word-8",
    "signin-word-9",
    "signin-word-10",
    "signin-word-11",
    "signin-word-12",
];

/// "Confirm password" (DW-A49).
pub const CONFIRM_KEY: &str = "signin-confirm-password";

pub const CODE_DIGITS: usize = 6;
pub const BACKUP_DIGITS: usize = 8;

/// The field a board frame draws holding the keyboard. A headless render has no keyboard
/// focus, so a scene names the field here and the view draws the board's focused look as
/// shapes (`ui::au_entry`). Never set by the product; any press or typed character clears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pictured {
    Word(usize),
    BackupCode,
    VaultPassword,
    NewPassword,
}

/// Keep only the characters `keep` allows, at most `max` of them, without the text passing
/// through a heap copy that outlives this call: the kept bytes go through a stack buffer that
/// is overwritten before it is left. ASCII only (digits, the word list's letters).
fn keep_only(edit: &mut TextEdit, max: usize, keep: fn(u8) -> bool) {
    let clean = edit.text.len() <= max && edit.text.bytes().all(keep);
    if clean {
        return;
    }
    let mut kept = [0u8; 16];
    let mut n = 0;
    for b in edit.text.bytes().filter(|b| keep(*b)).take(max.min(kept.len())) {
        kept[n] = b;
        n += 1;
    }
    // `keep` passes ASCII only, so the buffer is valid UTF-8.
    edit.replace_text(std::str::from_utf8(&kept[..n]).unwrap_or_default());
    kept.fill(0);
    std::hint::black_box(&kept);
}

impl Ui {
    /// What follows a step that succeeded: the next screen of this group, or the frame outside
    /// it that `gate.rs` names.
    pub(super) fn proceed(&mut self, next: NextAfterSignIn) {
        match next {
            NextAfterSignIn::TwoFactor => {
                self.code.clear();
                self.backup_code.clear();
                self.code_problem = None;
                self.screen = Screen::TwoFactor { backup: false };
                self.focus = Some(CODE_KEY);
            }
            NextAfterSignIn::RecoveryPhrase => {
                self.clear_words();
                self.phrase_problem = None;
                self.screen = Screen::Phrase;
                self.focus = Some(WORD_KEYS[0]);
            }
            other => self.go(Intent::Next(other)),
        }
    }

    /// The typed secrets of these steps are forgotten, and their waits are over.
    pub(super) fn drop_steps(&mut self) {
        let partial = matches!(
            self.screen,
            Screen::TwoFactor { .. } | Screen::Phrase | Screen::NoPhrase | Screen::UnlockVault
        );
        if partial {
            // "Back to sign-in; the partial session is discarded" (DW-A26).
            self.source.discard_partial_sign_in();
        }
        if self.passkey_waiting {
            self.source.cancel_passkey();
            self.passkey_waiting = false;
        }
        // A new password that was being chosen is not the sign-in form's.
        if matches!(self.screen, Screen::NewPassword) {
            self.password.clear();
        }
        self.confirm.clear();
        self.recovering = false;
        self.setting = false;
        self.set_problem = None;
        self.code.clear();
        self.backup_code.clear();
        self.clear_words();
        self.verifying = false;
        self.restoring = false;
        self.unlocking = false;
        self.code_problem = None;
        self.phrase_problem = None;
        self.unlock_problem = None;
        self.pictured = None;
    }

    fn clear_words(&mut self) {
        for word in &mut self.words {
            word.clear();
        }
    }

    // ---- Two-factor (DW-A26, DW-A27, DW-A28) ----

    pub fn verifying(&self) -> bool {
        self.verifying
    }

    pub fn code_problem(&self) -> Option<TwoFactorProblem> {
        self.code_problem
    }

    /// The Verify button works once every digit is there and no check runs.
    pub fn can_verify(&self) -> bool {
        let (edit, digits) = match self.screen {
            Screen::TwoFactor { backup: false } => (&self.code, CODE_DIGITS),
            Screen::TwoFactor { backup: true } => (&self.backup_code, BACKUP_DIGITS),
            _ => return false,
        };
        edit.text.len() == digits && !self.verifying
    }

    /// A key is about to reach the code's input: after a wrong code "the boxes clear on the
    /// next keystroke" (DW-A27).
    pub fn before_code_key(&mut self, types: bool) {
        self.pictured = None;
        if types && self.code_problem.is_some() && !self.verifying {
            self.code.clear();
            self.code_problem = None;
        }
    }

    /// The authenticator code changed: digits only, six at most, and "the code is checked as
    /// soon as you type the sixth digit".
    pub fn code_changed(&mut self) {
        keep_only(&mut self.code, CODE_DIGITS, |b| b.is_ascii_digit());
        if self.code.text.len() == CODE_DIGITS {
            self.verify_code();
        }
    }

    /// The backup code changed: digits only, eight at most. It is checked by its button.
    pub fn backup_changed(&mut self) {
        self.pictured = None;
        keep_only(&mut self.backup_code, BACKUP_DIGITS, |b| b.is_ascii_digit());
        if self.code_problem.is_some() && !self.verifying {
            self.code_problem = None;
        }
    }

    /// Verify, Verify backup code, Enter, and the sixth digit.
    pub fn verify_code(&mut self) {
        if !self.can_verify() {
            return;
        }
        self.verifying = true;
        self.code_problem = None;
        match self.screen {
            Screen::TwoFactor { backup: true } => {
                self.source.begin_two_factor(&self.backup_code.text, CodeKind::Backup);
            }
            _ => self.source.begin_two_factor(&self.code.text, CodeKind::Authenticator),
        }
        self.pump();
    }

    /// "Use a backup code" and "Use the authenticator code".
    pub fn switch_code(&mut self, backup: bool) {
        if !matches!(self.screen, Screen::TwoFactor { .. }) || self.verifying {
            return;
        }
        self.code.clear();
        self.backup_code.clear();
        self.code_problem = None;
        self.screen = Screen::TwoFactor { backup };
        self.focus = Some(if backup { BACKUP_KEY } else { CODE_KEY });
    }

    // ---- The twelve words (DW-A23, DW-A24, DW-A25) ----

    pub fn restoring(&self) -> bool {
        self.restoring
    }

    pub fn phrase_problem(&self) -> Option<&PhraseProblem> {
        self.phrase_problem.as_ref()
    }

    /// Restore keys is dimmed until all twelve boxes hold a word (DW-A23).
    pub fn can_restore(&self) -> bool {
        self.screen == Screen::Phrase && !self.restoring && self.words.iter().all(|w| !w.text.is_empty())
    }

    /// A word box changed: "words are lowercase letters only; the field lowercases and trims".
    pub fn word_changed(&mut self, i: usize) {
        self.pictured = None;
        let Some(word) = self.words.get_mut(i) else { return };
        word.text.make_ascii_lowercase();
        keep_only(word, 12, |b| b.is_ascii_lowercase());
        // The mark was about the phrase that was tried, not about this one.
        if !self.restoring {
            self.phrase_problem = None;
        }
    }

    /// A paste into a word box: several words fill the boxes from the first one ("pasting 12
    /// words into any box fills them all"). `false`: one word, which the box pastes itself.
    pub fn paste_words(&mut self, text: &str) -> bool {
        if text.split_whitespace().nth(1).is_none() {
            return false;
        }
        self.pictured = None;
        self.clear_words();
        let mut last = 0;
        for (i, word) in text.split_whitespace().take(self.words.len()).enumerate() {
            self.words[i].replace_text(word);
            self.word_changed(i);
            last = i;
        }
        self.focus = Some(WORD_KEYS[(last + 1).min(WORD_KEYS.len() - 1)]);
        true
    }

    /// Restore keys, and Enter in a box.
    pub fn restore(&mut self) {
        if !self.can_restore() {
            return;
        }
        self.restoring = true;
        self.phrase_problem = None;
        let words: Vec<&str> = self.words.iter().map(|w| w.text.as_str()).collect();
        self.source.begin_restore(&words);
        self.pump();
    }

    /// "I've lost my recovery phrase" (DW-A25) and "I found my phrase".
    pub fn lost_phrase(&mut self, lost: bool) {
        if self.restoring {
            return;
        }
        match (&self.screen, lost) {
            (Screen::Phrase, true) => {
                self.screen = Screen::NoPhrase;
                self.focus = Some(TITLE_KEY);
            }
            (Screen::NoPhrase, false) => {
                self.screen = Screen::Phrase;
                self.focus = Some(WORD_KEYS[0]);
            }
            _ => {}
        }
    }

    /// A row of DW-A25: Drive's own dialog for it is the shell's to open.
    pub fn exit(&mut self, intent: Intent) {
        if matches!(self.screen, Screen::NoPhrase | Screen::CannotRecover) {
            self.go(intent);
        }
    }

    // ---- A passkey (DW-A29, DW-A30) ----

    pub fn passkey_waiting(&self) -> bool {
        self.passkey_waiting
    }

    pub fn unlocking(&self) -> bool {
        self.unlocking
    }

    pub fn unlock_problem(&self) -> Option<SignInProblem> {
        self.unlock_problem
    }

    /// "Use a passkey": Windows asks; the app never sees the passkey.
    pub fn use_passkey(&mut self) {
        if self.busy || self.passkey_waiting || self.screen != Screen::SignIn {
            return;
        }
        self.passkey_waiting = true;
        self.problem = None;
        self.deleted = false;
        self.source.begin_passkey_for(self.email.text.trim());
        self.pump();
    }

    pub fn can_unlock(&self) -> bool {
        self.screen == Screen::UnlockVault && !self.unlocking && !self.password.text.is_empty()
    }

    /// Unlock, and Enter in the password field (DW-A30).
    pub fn unlock(&mut self) {
        if !self.can_unlock() {
            return;
        }
        self.unlocking = true;
        self.unlock_problem = None;
        self.source.begin_unlock(&self.password.text);
        self.pump();
    }

    // ---- Forgot password? (DW-A45, DW-A46) ----

    /// "Forgot password?": the three honest options.
    pub fn forgot(&mut self) {
        if self.step_running() || self.passkey_waiting {
            return;
        }
        self.drop_waits();
        self.screen = Screen::Forgot;
        self.focus = Some(TITLE_KEY);
    }

    /// "Recover with phrase" (DW-A45), and Back from the new password: recovery's first step,
    /// the email and the twelve words (DW-A47).
    pub fn recover_with_phrase(&mut self) {
        if !matches!(self.screen, Screen::Forgot | Screen::NewPassword) || self.setting {
            return;
        }
        self.drop_steps();
        self.screen = Screen::Recover;
        self.focus = Some(if self.email.text.trim().is_empty() {
            super::EMAIL_KEY
        } else {
            WORD_KEYS[0]
        });
    }

    /// Back from recovery's first step: the three options, and the words forgotten.
    pub fn leave_recovery(&mut self) {
        if self.screen != Screen::Recover {
            return;
        }
        self.drop_steps();
        self.screen = Screen::Forgot;
        self.focus = Some(TITLE_KEY);
    }

    pub fn recovering(&self) -> bool {
        self.recovering
    }

    /// Verify phrase is dimmed until the email and all twelve words are in (DW-A47).
    pub fn can_verify_phrase(&self) -> bool {
        self.screen == Screen::Recover
            && !self.recovering
            && self.email.text.contains('@')
            && self.words.iter().all(|w| !w.text.is_empty())
    }

    /// Verify phrase: the key is derived on this device; the phrase never leaves it.
    pub fn verify_phrase(&mut self) {
        if !self.can_verify_phrase() {
            return;
        }
        self.recovering = true;
        self.phrase_problem = None;
        let words: Vec<&str> = self.words.iter().map(|w| w.text.as_str()).collect();
        self.source.begin_recovery_verify(self.email.text.trim(), &words);
        self.pump();
    }

    pub fn setting(&self) -> bool {
        self.setting
    }

    pub fn set_problem(&self) -> Option<SignInProblem> {
        self.set_problem
    }

    /// What the meter under the new password shows: the engine's verdict, by sign-up's rules.
    pub fn strength(&self) -> Option<super::source::PasswordStrength> {
        if self.password.text.is_empty() {
            return None;
        }
        self.source.password_strength(&self.password.text)
    }

    /// Set new password works once both fields hold the same password. Whether it is strong
    /// enough is the engine's answer (`PasswordSet`), by the same rules as sign-up.
    pub fn can_set_password(&self) -> bool {
        self.screen == Screen::NewPassword
            && !self.setting
            && !self.password.text.is_empty()
            && self.password.text == self.confirm.text
    }

    /// Set new password (DW-A49): the vault key is wrapped under it; other sessions are signed out.
    pub fn set_password(&mut self) {
        if !self.can_set_password() {
            return;
        }
        self.setting = true;
        self.set_problem = None;
        self.source.begin_set_password(&self.password.text);
        self.pump();
    }

    /// "Continue to Drive" (DW-A50).
    pub fn continue_to_drive(&mut self) {
        if self.screen == Screen::PasswordUpdated {
            self.go(Intent::Next(NextAfterSignIn::Drive));
        }
    }

    /// "Show my options" and "Back to recovery options"; "I don't have it" (DW-A48).
    pub fn cannot_recover(&mut self, open: bool) {
        match (&self.screen, open) {
            (Screen::Recover, true) if !self.recovering => {
                self.drop_steps();
                self.screen = Screen::CannotRecover;
            }
            (Screen::Forgot, true) => self.screen = Screen::CannotRecover,
            (Screen::CannotRecover, false) => self.screen = Screen::Forgot,
            _ => return,
        }
        self.focus = Some(TITLE_KEY);
    }

    // ---- The engine's answers ----

    pub(super) fn apply_step(&mut self, event: SignInEvent) {
        match event {
            SignInEvent::TwoFactor(result) if self.verifying && matches!(self.screen, Screen::TwoFactor { .. }) => {
                self.verifying = false;
                let backup = self.screen == Screen::TwoFactor { backup: true };
                match result {
                    Ok(next) => {
                        // The code did its work.
                        self.code.clear();
                        self.backup_code.clear();
                        self.proceed(next);
                    }
                    // The partial sign-in ran out: back to the password.
                    Err(TwoFactorProblem::Expired) => {
                        self.drop_steps();
                        self.screen = Screen::SignIn;
                        self.problem = Some(SignInProblem::Other);
                        self.focus = Some(PASSWORD_KEY);
                    }
                    Err(problem) => {
                        self.code_problem = Some(problem);
                        self.focus = Some(if backup { BACKUP_KEY } else { CODE_KEY });
                    }
                }
            }
            SignInEvent::PhraseRestored(result) if self.restoring && self.screen == Screen::Phrase => {
                self.restoring = false;
                match result {
                    Ok(next) => {
                        self.clear_words();
                        self.proceed(next);
                    }
                    Err(problem) => {
                        // Focus goes to the word that is not in the list, if one is marked.
                        let at = match &problem {
                            PhraseProblem::WrongPhrase { bad_word: Some(i) } if *i < WORD_KEYS.len() => *i,
                            _ => 0,
                        };
                        self.phrase_problem = Some(problem);
                        self.focus = Some(WORD_KEYS[at]);
                    }
                }
            }
            SignInEvent::Passkey(outcome) if self.passkey_waiting && self.screen == Screen::SignIn => {
                self.passkey_waiting = false;
                match outcome {
                    PasskeyOutcome::Succeeded(next) => self.proceed(next),
                    PasskeyOutcome::NeedsPassword => {
                        // The password that was typed for another way in is not this step's.
                        self.password.clear();
                        self.unlock_problem = None;
                        self.screen = Screen::UnlockVault;
                        // "Focus lands on the password field" (DW-A30).
                        self.focus = Some(PASSWORD_KEY);
                    }
                    // "Cancel: back to sign-in" (DW-A29): the form is there, unchanged.
                    PasskeyOutcome::Cancelled => {}
                    PasskeyOutcome::Unavailable => self.problem = Some(SignInProblem::Other),
                }
            }
            SignInEvent::VaultUnlocked(result) if self.unlocking && self.screen == Screen::UnlockVault => {
                self.unlocking = false;
                match result {
                    Ok(next) => {
                        self.password.clear();
                        self.proceed(next);
                    }
                    Err(problem) => {
                        self.unlock_problem = Some(problem);
                        self.focus = Some(PASSWORD_KEY);
                    }
                }
            }
            SignInEvent::RecoveryPhraseVerified(result) if self.recovering && self.screen == Screen::Recover => {
                self.recovering = false;
                match result {
                    Ok(()) => {
                        // The words did their work. The new password is typed into the one
                        // password field, which starts empty.
                        self.clear_words();
                        self.password.clear();
                        self.confirm.clear();
                        self.set_problem = None;
                        self.screen = Screen::NewPassword;
                        self.focus = Some(PASSWORD_KEY);
                    }
                    Err(problem) => {
                        let at = match &problem {
                            PhraseProblem::WrongPhrase { bad_word: Some(i) } if *i < WORD_KEYS.len() => *i,
                            _ => 0,
                        };
                        self.phrase_problem = Some(problem);
                        self.focus = Some(WORD_KEYS[at]);
                    }
                }
            }
            SignInEvent::PasswordSet(result) if self.setting && self.screen == Screen::NewPassword => {
                self.setting = false;
                match result {
                    Ok(()) => {
                        self.password.clear();
                        self.confirm.clear();
                        self.screen = Screen::PasswordUpdated;
                        // "Success is announced": focus moves to the heading.
                        self.focus = Some(TITLE_KEY);
                    }
                    Err(problem) => {
                        self.set_problem = Some(problem);
                        self.focus = Some(PASSWORD_KEY);
                    }
                }
            }
            // An answer nobody is waiting for changes nothing.
            _ => {}
        }
    }
}
