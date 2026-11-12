//! Create an account (DW-A31 to DW-A44): email, verify, terms, password, the recovery phrase
//! shown and confirmed, this device. State and rules only; `view_create.rs` and
//! `view_create_phrase.rs` draw them.
//!
//! Secrets. The password is typed into the group's ONE password field and "Confirm password"
//! (`Ui::password`, `Ui::confirm`), handed to the source as a borrowed `&str` for the breach
//! check, and cleared here the moment the check passes: the engine keeps it from then on. The
//! recovery phrase is never in this state: the view borrows it for one draw
//! (`SignInSource::with_sign_up_phrase`). The email code and the three words typed back are
//! secure fields made ONCE in [`Create::new`], shown as typed, cleared when their step is over.
//!
//! Every answer of the engine is applied only by the wait it answers ([`Ui::apply_sign_up`]:
//! each arm has a guard), as in `mod.rs` and `steps.rs`.

use std::time::Duration;

use rune::{SecretCopy, TextEdit, Tx};

use super::create_source::{
    CONFIRM_WORDS, CodeProblem, CodeSent, CreateProblem, PasswordProblem, PasswordVerdict, SendProblem, SignUpEvent,
};
use super::source::NextAfterSignIn;
use super::{EMAIL_KEY, FIRST_BUTTON_KEY, Intent, PASSWORD_KEY, Screen, TITLE_KEY, Ui};

/// The email code's field.
pub const CODE_KEY: &str = "signup-code";
/// The three fields of the words typed back.
pub const WORD_KEYS: [&str; CONFIRM_WORDS] = ["signup-word-a", "signup-word-b", "signup-word-c"];
pub const DEVICE_KEY: &str = "signup-device-name";

/// "Up to 40 characters" (DW-A42).
pub const DEVICE_NAME_MAX: usize = 40;
/// "The clipboard is cleared after 60 seconds" (DW-A39).
pub const PHRASE_COPY_SECONDS: u64 = 60;

/// The six steps (the progress line counts them), and the two screens after the last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// DW-A31.
    Email,
    /// DW-A32, DW-A33, DW-A34.
    Verify,
    /// DW-A35.
    Terms,
    /// DW-A36, DW-A37, DW-A38.
    Password,
    /// DW-A39. The one screen that shows the recovery phrase.
    Phrase,
    /// DW-A40, DW-A41.
    Confirm,
    /// DW-A42.
    Device,
    /// DW-A43.
    Creating,
    /// DW-A44.
    Ready,
}

impl Step {
    /// The password is typed, or the phrase is shown or typed back, on this step.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn shows_secret(self) -> bool {
        matches!(self, Step::Password | Step::Phrase | Step::Confirm)
    }
}

/// The field a board frame draws holding the keyboard (scenes only; see `steps::Pictured`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pictured {
    Email,
    Code,
    Password,
    Word(usize),
    Device,
}

/// What stands under the email field when no code went out (DW-A31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailProblem {
    /// "Validation on submit".
    NotAnAddress,
    Send(SendProblem),
}

pub struct Create {
    /// The digits from the email: a secure field, shown as typed.
    pub code: TextEdit,
    /// The three words typed back: secure fields, shown as typed.
    pub words: [TextEdit; CONFIRM_WORDS],
    pub device: TextEdit,
    /// Sign-up was opened from the sign-in screen, not from the first screen: Back returns there.
    from_sign_in: bool,
    sending: bool,
    email_problem: Option<EmailProblem>,
    sent: Option<CodeSent>,
    resend_left: u32,
    /// A new code is being asked for (DW-A34 once it answers).
    resending: bool,
    resent: bool,
    resend_problem: Option<SendProblem>,
    verifying: bool,
    code_problem: Option<CodeProblem>,
    /// The code ran out while the account was being created (DW-A43's rule).
    code_expired: bool,
    accepted: bool,
    understood: bool,
    checking: bool,
    password_problem: Option<PasswordProblem>,
    saved: bool,
    /// The phrase on the clipboard, while this window put it there.
    phrase_copy: Option<SecretCopy>,
    positions: [usize; CONFIRM_WORDS],
    /// The word field that was typed in last: the suggestions are for it.
    active_word: usize,
    confirming: bool,
    mismatch: bool,
    creating: bool,
    create_problem: Option<CreateProblem>,
    pub(super) pictured: Option<Pictured>,
}

impl Create {
    /// The one place sign-up's secret fields are made. Scenes and tests fill THESE.
    pub fn new() -> Self {
        Create {
            code: TextEdit::new("").secure().revealed(true),
            words: std::array::from_fn(|_| TextEdit::new("").secure().revealed(true)),
            device: TextEdit::new(""),
            from_sign_in: false,
            sending: false,
            email_problem: None,
            sent: None,
            resend_left: 0,
            resending: false,
            resent: false,
            resend_problem: None,
            verifying: false,
            code_problem: None,
            code_expired: false,
            accepted: false,
            understood: false,
            checking: false,
            password_problem: None,
            saved: false,
            phrase_copy: None,
            positions: [0, 1, 2],
            active_word: 0,
            confirming: false,
            mismatch: false,
            creating: false,
            create_problem: None,
            pictured: None,
        }
    }

    /// Everything typed and every wait of sign-up is forgotten. The fields stay the ones made
    /// in [`Create::new`]: they are emptied (and so wiped), never replaced.
    fn reset(&mut self) {
        self.code.clear();
        for word in &mut self.words {
            word.clear();
        }
        self.device.clear();
        if let Some(copy) = self.phrase_copy.take() {
            copy.clear_now();
        }
        let (code, words, device) = (
            std::mem::replace(&mut self.code, TextEdit::new("")),
            std::mem::replace(&mut self.words, std::array::from_fn(|_| TextEdit::new(""))),
            std::mem::replace(&mut self.device, TextEdit::new("")),
        );
        *self = Create {
            code,
            words,
            device,
            ..Create::new()
        };
    }
}

/// `steps::keep_only`, for this file's fields: keep the characters `keep` allows, at most `max`,
/// without the text passing through a heap copy that outlives the call. ASCII only.
fn keep_only(edit: &mut TextEdit, max: usize, keep: fn(u8) -> bool) {
    if edit.text.len() <= max && edit.text.bytes().all(keep) {
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

fn looks_like_address(email: &str) -> bool {
    email
        .split_once('@')
        .is_some_and(|(name, host)| !name.is_empty() && host.contains('.') && !host.ends_with('.'))
}

impl Ui {
    fn step(&self) -> Option<Step> {
        match self.screen {
            Screen::Create(step) => Some(step),
            _ => None,
        }
    }

    fn at(&self, step: Step) -> bool {
        self.screen == Screen::Create(step)
    }

    fn to(&mut self, step: Step, focus: &'static str) {
        self.screen = Screen::Create(step);
        self.focus = Some(focus);
    }

    /// "Create a Beebeeb account" (DW-A01) and "Create an account" (DW-A02, DW-A03): sign-up
    /// at its email step. `false`: this source has no sign-up, and nothing changed.
    pub fn start_create(&mut self) -> bool {
        if self.busy || self.passkey_waiting || !self.source.sign_up_available() {
            return false;
        }
        if self.step().is_some() {
            return true;
        }
        let from_sign_in = self.screen == Screen::SignIn;
        self.drop_waits();
        self.problem = None;
        self.deleted = false;
        // A password typed into the sign-in form is not the new account's.
        self.password.clear();
        self.confirm.clear();
        self.create.reset();
        self.create.from_sign_in = from_sign_in;
        self.to(Step::Email, EMAIL_KEY);
        true
    }

    /// Sign-up is left: the engine forgets what it held for it, and so does the window.
    fn leave_create(&mut self) {
        self.source.cancel_sign_up();
        self.password.clear();
        self.password.set_revealed(false);
        self.confirm.clear();
        self.create.reset();
    }

    /// The back link, and Escape. DW-A43 has no Back and no Cancel while it runs.
    pub fn create_back(&mut self) {
        let Some(step) = self.step() else { return };
        self.create.pictured = None;
        match step {
            Step::Email => {
                let from_sign_in = self.create.from_sign_in;
                self.leave_create();
                (self.screen, self.focus) = if from_sign_in {
                    (Screen::SignIn, Some(EMAIL_KEY))
                } else {
                    (Screen::Welcome, Some(FIRST_BUTTON_KEY))
                };
            }
            // "Use a different email" and Back are the same way: the code that was sent is not
            // asked for again, and a verified one is spent.
            Step::Verify | Step::Terms => self.different_email(),
            Step::Password => {
                if self.create.checking {
                    return;
                }
                self.password.clear();
                self.password.set_revealed(false);
                self.confirm.clear();
                self.create.password_problem = None;
                self.to(Step::Terms, TITLE_KEY);
            }
            Step::Phrase => {
                self.hide_phrase();
                self.to(Step::Password, PASSWORD_KEY);
            }
            Step::Confirm => self.show_words_again(),
            Step::Device => {
                self.clear_typed_words();
                self.to(Step::Confirm, WORD_KEYS[0]);
            }
            Step::Creating | Step::Ready => {}
        }
    }

    // ---- Step 1: the email (DW-A31) ----

    pub fn create_state(&self) -> &Create {
        &self.create
    }

    pub fn back_goes_to_sign_in(&self) -> bool {
        self.create.from_sign_in
    }

    pub fn sending(&self) -> bool {
        self.create.sending
    }

    pub fn email_problem(&self) -> Option<EmailProblem> {
        self.create.email_problem
    }

    /// The email changed: what stood under it was about the address that was tried.
    pub fn sign_up_email_changed(&mut self) {
        self.create.pictured = None;
        if !self.create.sending {
            self.create.email_problem = None;
        }
    }

    /// Send the code, and Enter. The address is checked when it is sent, not while it is typed.
    pub fn send_code(&mut self) {
        if !self.at(Step::Email) || self.create.sending {
            return;
        }
        if !looks_like_address(self.email.text.trim()) {
            self.create.email_problem = Some(EmailProblem::NotAnAddress);
            self.focus = Some(EMAIL_KEY);
            return;
        }
        self.create.sending = true;
        self.create.email_problem = None;
        self.source.begin_sign_up_code(self.email.text.trim());
        self.pump();
    }

    // ---- Step 2: the code (DW-A32, DW-A33, DW-A34) ----

    pub fn code_digits(&self) -> usize {
        self.create.sent.map_or(0, |sent| usize::from(sent.digits))
    }

    pub fn resend_left(&self) -> u32 {
        self.create.resend_left
    }

    /// DW-A34: a new code was sent, and the line under the field says so.
    pub fn resent(&self) -> bool {
        self.create.resent
    }

    pub fn resend_problem(&self) -> Option<SendProblem> {
        self.create.resend_problem
    }

    pub fn email_verifying(&self) -> bool {
        self.create.verifying
    }

    pub fn email_code_problem(&self) -> Option<CodeProblem> {
        self.create.code_problem
    }

    pub fn code_expired(&self) -> bool {
        self.create.code_expired
    }

    /// The code's field changed: "digits only; pasting strips spaces".
    pub fn sign_up_code_changed(&mut self) {
        self.create.pictured = None;
        let digits = self.code_digits();
        keep_only(&mut self.create.code, digits, |b| b.is_ascii_digit());
        // The message was about the code that was tried. A spent guess budget stays said.
        if !self.create.verifying && self.create.code_problem != Some(CodeProblem::TooManyWrong) {
            self.create.code_problem = None;
        }
    }

    /// Verify is dimmed until every digit is in, and once the guesses are spent.
    pub fn can_verify_email(&self) -> bool {
        self.at(Step::Verify)
            && !self.create.verifying
            && !self.create.resending
            && self.code_digits() > 0
            && self.create.code.text.len() == self.code_digits()
            && self.create.code_problem != Some(CodeProblem::TooManyWrong)
    }

    /// Verify, and Enter.
    pub fn verify_email(&mut self) {
        if !self.can_verify_email() {
            return;
        }
        self.create.verifying = true;
        self.create.code_problem = None;
        self.source.begin_sign_up_verify(&self.create.code.text);
        self.pump();
    }

    /// "Send a new code" stands in for the countdown when it reaches zero, and at once after
    /// too many wrong codes.
    pub fn can_resend(&self) -> bool {
        self.at(Step::Verify)
            && !self.create.resending
            && !self.create.verifying
            && (self.create.resend_left == 0 || self.create.code_problem == Some(CodeProblem::TooManyWrong))
    }

    pub fn resend_code(&mut self) {
        if !self.can_resend() {
            return;
        }
        self.create.resending = true;
        self.create.resend_problem = None;
        self.source.begin_sign_up_code(self.email.text.trim());
        self.pump();
    }

    /// "Use a different email": back to step 1. The code that was typed is forgotten.
    pub fn different_email(&mut self) {
        if !matches!(self.step(), Some(Step::Verify | Step::Terms)) || self.create.verifying {
            return;
        }
        // The engine's wait for this address (its ticks, a late answer) is over.
        self.source.cancel_sign_up();
        let from_sign_in = self.create.from_sign_in;
        self.create.reset();
        self.create.from_sign_in = from_sign_in;
        self.to(Step::Email, EMAIL_KEY);
    }

    // ---- Step 3: the terms (DW-A35) ----

    pub fn terms_accepted(&self) -> (bool, bool) {
        (self.create.accepted, self.create.understood)
    }

    pub fn toggle_accepted(&mut self) {
        if self.at(Step::Terms) {
            self.create.accepted = !self.create.accepted;
        }
    }

    pub fn toggle_understood(&mut self) {
        if self.at(Step::Terms) {
            self.create.understood = !self.create.understood;
        }
    }

    /// The Terms of Service and the Privacy Policy open in the browser, not in the app.
    pub fn open_page(&mut self, page: Intent) {
        if self.at(Step::Terms) && matches!(page, Intent::OpenTerms | Intent::OpenPrivacy) {
            self.go(page);
        }
    }

    pub fn can_accept_terms(&self) -> bool {
        self.at(Step::Terms) && self.create.accepted && self.create.understood
    }

    pub fn accept_terms(&mut self) {
        if !self.can_accept_terms() {
            return;
        }
        self.password.clear();
        self.confirm.clear();
        self.create.password_problem = None;
        self.to(Step::Password, PASSWORD_KEY);
    }

    // ---- Step 4: the password (DW-A36, DW-A37, DW-A38) ----

    pub fn password_checking(&self) -> bool {
        self.create.checking
    }

    pub fn password_problem(&self) -> Option<PasswordProblem> {
        self.create.password_problem
    }

    /// The engine's verdict on what is typed; nothing while the field is empty.
    pub fn password_verdict(&self) -> Option<PasswordVerdict> {
        if self.password.text.is_empty() {
            return None;
        }
        self.source.sign_up_password(&self.password.text)
    }

    /// A password field changed: "clears the error as soon as the text changes" (DW-A38).
    pub fn sign_up_password_changed(&mut self) {
        self.create.pictured = None;
        if !self.create.checking {
            self.create.password_problem = None;
        }
    }

    /// "Does not match yet." stands under the second field once it is as long as the first.
    pub fn passwords_differ(&self) -> bool {
        !self.confirm.text.is_empty()
            && self.confirm.text.len() >= self.password.text.len()
            && self.confirm.text != self.password.text
    }

    /// Continue works when the password is long enough, not known to be breached, and the two
    /// match (DW-A36). The other guidance never blocks.
    pub fn can_set_new_password(&self) -> bool {
        self.at(Step::Password)
            && !self.create.checking
            && self.create.password_problem.is_none()
            && self.password.text == self.confirm.text
            && self.password_verdict().is_some_and(|verdict| verdict.long_enough)
    }

    /// Continue: the breach check. Nobody goes on without its answer.
    pub fn check_password(&mut self) {
        if !self.can_set_new_password() {
            return;
        }
        self.create.checking = true;
        self.source.begin_sign_up_password(&self.password.text);
        self.pump();
    }

    // ---- Step 5: the phrase shown (DW-A39) ----

    pub fn phrase_saved(&self) -> bool {
        self.create.saved
    }

    pub fn toggle_saved(&mut self) {
        if self.at(Step::Phrase) {
            self.create.saved = !self.create.saved;
        }
    }

    /// The words are on screen for one draw at a time: `show` borrows them from the engine.
    pub fn with_phrase(&self, show: &mut dyn FnMut(&[&str])) {
        if self.at(Step::Phrase) {
            self.source.with_sign_up_phrase(show);
        }
    }

    /// Copy: the twelve words go to the clipboard, kept out of clipboard histories, and are
    /// cleared after 60 seconds or when sign-up ends, whichever is first. The one `String`
    /// that holds them is built at its final size and moved to Rune, which wipes it.
    pub fn copy_phrase(&mut self, tx: &mut Tx) {
        if !self.at(Step::Phrase) {
            return;
        }
        let mut joined: Option<String> = None;
        self.source.with_sign_up_phrase(&mut |words: &[&str]| {
            let size = words.iter().map(|w| w.len() + 1).sum::<usize>();
            let mut text = String::with_capacity(size);
            for (i, word) in words.iter().enumerate() {
                if i > 0 {
                    text.push(' ');
                }
                text.push_str(word);
            }
            joined = Some(text);
        });
        let Some(text) = joined else { return };
        if let Some(old) = self.create.phrase_copy.take() {
            old.clear_now();
        }
        self.create.phrase_copy = Some(tx.copy_secret(text, Duration::from_secs(PHRASE_COPY_SECONDS)));
    }

    /// Save as file and Print are the shell's (a save dialog, a print-ready page): it asks the
    /// engine for the words itself. No word travels in the intent.
    pub fn phrase_out(&mut self, how: Intent) {
        if self.at(Step::Phrase) && matches!(how, Intent::SavePhraseAsFile | Intent::PrintPhrase) {
            self.go(how);
        }
    }

    fn hide_phrase(&mut self) {
        self.create.saved = false;
        self.source.sign_up_phrase_left();
    }

    fn clear_typed_words(&mut self) {
        for word in &mut self.create.words {
            word.clear();
        }
        self.create.mismatch = false;
        self.create.confirming = false;
        self.create.active_word = 0;
    }

    /// "I saved it, verify": on once the box is ticked.
    pub fn verify_saved(&mut self) {
        if !self.at(Step::Phrase) || !self.create.saved {
            return;
        }
        let Some(positions) = self.source.sign_up_confirm_positions() else {
            return;
        };
        self.create.positions = positions;
        self.hide_phrase();
        self.clear_typed_words();
        self.to(Step::Confirm, WORD_KEYS[0]);
    }

    // ---- Step 5, second half: three words typed back (DW-A40, DW-A41) ----

    /// Which words are asked for, counted from zero.
    pub fn confirm_positions(&self) -> [usize; CONFIRM_WORDS] {
        self.create.positions
    }

    pub fn confirming(&self) -> bool {
        self.create.confirming
    }

    /// DW-A41. It does not say which word is wrong.
    pub fn words_mismatch(&self) -> bool {
        self.create.mismatch
    }

    /// A word field changed: lowercase letters only, as the phrase's own boxes (DW-A23).
    pub fn typed_word_changed(&mut self, i: usize) {
        self.create.pictured = None;
        let Some(word) = self.create.words.get_mut(i) else {
            return;
        };
        word.text.make_ascii_lowercase();
        keep_only(word, 12, |b| b.is_ascii_lowercase());
        self.create.active_word = i;
        if !self.create.confirming {
            self.create.mismatch = false;
        }
    }

    /// Words of the list that complete the word being typed. They only complete what the
    /// person typed: nothing is offered for an empty field.
    pub fn suggestions(&self) -> Vec<&'static str> {
        if !self.at(Step::Confirm) {
            return Vec::new();
        }
        let typed = &self.create.words[self.create.active_word.min(CONFIRM_WORDS - 1)].text;
        if typed.is_empty() {
            return Vec::new();
        }
        self.source.sign_up_suggestions(typed)
    }

    /// A suggestion was pressed: it fills the field being typed in, and the next one is next.
    pub fn take_suggestion(&mut self, n: usize) {
        let Some(word) = self.suggestions().get(n).copied() else {
            return;
        };
        let at = self.create.active_word.min(CONFIRM_WORDS - 1);
        self.create.words[at].replace_text(word);
        self.create.pictured = None;
        self.create.mismatch = false;
        let next = (at + 1).min(CONFIRM_WORDS - 1);
        self.create.active_word = next;
        self.focus = Some(WORD_KEYS[next]);
    }

    pub fn can_confirm_words(&self) -> bool {
        self.at(Step::Confirm) && !self.create.confirming && self.create.words.iter().all(|w| !w.text.is_empty())
    }

    /// Confirm: the engine compares, so the phrase is not held here to compare with.
    pub fn confirm_words(&mut self) {
        if !self.can_confirm_words() {
            return;
        }
        self.create.confirming = true;
        self.create.mismatch = false;
        let words: [&str; CONFIRM_WORDS] = std::array::from_fn(|i| self.create.words[i].text.as_str());
        self.source.begin_sign_up_confirm(&words);
        self.pump();
    }

    /// "Show the words again": back to the phrase, its box unticked. Always available.
    pub fn show_words_again(&mut self) {
        if !self.at(Step::Confirm) || self.create.confirming {
            return;
        }
        self.clear_typed_words();
        self.create.saved = false;
        self.to(Step::Phrase, TITLE_KEY);
    }

    // ---- Step 6: this device (DW-A42), creating (DW-A43), ready (DW-A44) ----

    /// The name changed: 40 characters at most.
    pub fn device_name_changed(&mut self) {
        self.create.pictured = None;
        if self.create.device.text.chars().count() > DEVICE_NAME_MAX {
            let cut: String = self.create.device.text.chars().take(DEVICE_NAME_MAX).collect();
            self.create.device.replace_text(cut);
        }
    }

    pub fn create_problem(&self) -> Option<CreateProblem> {
        self.create.create_problem
    }

    /// Create my account, Enter, and Try again: registers the account and this device. An
    /// empty name is not allowed: it falls back to the system name.
    pub fn create_account(&mut self) {
        let retry = self.at(Step::Creating) && self.create.create_problem == Some(CreateProblem::NothingStored);
        if !(self.at(Step::Device) || retry) || self.create.creating {
            return;
        }
        if self.create.device.text.trim().is_empty() {
            let system = self.source.sign_up_device_name();
            self.create.device.replace_text(system);
        }
        self.create.creating = true;
        self.create.create_problem = None;
        self.to(Step::Creating, TITLE_KEY);
        self.source.begin_sign_up_create(self.create.device.text.trim());
        self.pump();
    }

    /// The account exists and this device was not finished: "the only way forward is Sign in
    /// (never register again)".
    pub fn sign_in_after_create(&mut self) {
        if !self.at(Step::Creating) || self.create.create_problem != Some(CreateProblem::NotFinishedHere) {
            return;
        }
        self.leave_create();
        self.screen = Screen::SignIn;
        self.focus = Some(PASSWORD_KEY);
    }

    /// Continue on DW-A44: the product's own first step (DW-0201).
    pub fn finish_create(&mut self) {
        if !self.at(Step::Ready) {
            return;
        }
        self.go(Intent::Next(NextAfterSignIn::Drive));
    }

    // ---- The engine's answers ----

    pub(super) fn apply_sign_up(&mut self, event: SignUpEvent) {
        match event {
            SignUpEvent::CodeSent(result) if self.create.sending && self.at(Step::Email) => {
                self.create.sending = false;
                match result {
                    Ok(sent) => {
                        self.create.code.clear();
                        self.create.code_problem = None;
                        self.create.sent = Some(sent);
                        self.create.resend_left = sent.resend_seconds;
                        self.create.resent = false;
                        self.to(Step::Verify, CODE_KEY);
                    }
                    Err(problem) => {
                        self.create.email_problem = Some(EmailProblem::Send(problem));
                        self.focus = Some(EMAIL_KEY);
                    }
                }
            }
            SignUpEvent::CodeSent(result) if self.create.resending && self.at(Step::Verify) => {
                self.create.resending = false;
                match result {
                    Ok(sent) => {
                        // "The field is cleared when a new code is sent" (DW-A34).
                        self.create.code.clear();
                        self.create.code_problem = None;
                        self.create.code_expired = false;
                        self.create.sent = Some(sent);
                        self.create.resend_left = sent.resend_seconds;
                        self.create.resent = true;
                        self.focus = Some(CODE_KEY);
                    }
                    Err(problem) => self.create.resend_problem = Some(problem),
                }
            }
            SignUpEvent::ResendTick { seconds } if self.at(Step::Verify) && !self.create.resending => {
                self.create.resend_left = seconds;
            }
            SignUpEvent::Verified(result) if self.create.verifying && self.at(Step::Verify) => {
                self.create.verifying = false;
                match result {
                    Ok(()) => {
                        // The code did its work.
                        self.create.code.clear();
                        self.create.code_expired = false;
                        self.create.resent = false;
                        // The code ran out while the account was being made: the password and
                        // the phrase are kept, so the way on is the last step again.
                        if self.create.accepted && self.create.understood && !self.create.device.text.is_empty() {
                            self.to(Step::Device, DEVICE_KEY);
                        } else {
                            self.to(Step::Terms, TITLE_KEY);
                        }
                    }
                    Err(problem) => {
                        self.create.code_problem = Some(problem);
                        self.focus = Some(CODE_KEY);
                    }
                }
            }
            SignUpEvent::PasswordChecked(result) if self.create.checking && self.at(Step::Password) => {
                self.create.checking = false;
                match result {
                    Ok(()) => {
                        // The engine holds the password now: it does not stay behind the next screens.
                        self.password.clear();
                        self.password.set_revealed(false);
                        self.confirm.clear();
                        self.create.saved = false;
                        self.to(Step::Phrase, TITLE_KEY);
                    }
                    Err(problem) => {
                        self.create.password_problem = Some(problem);
                        self.focus = Some(PASSWORD_KEY);
                    }
                }
            }
            SignUpEvent::PhraseConfirmed(right) if self.create.confirming && self.at(Step::Confirm) => {
                self.create.confirming = false;
                if right {
                    self.clear_typed_words();
                    if self.create.device.text.is_empty() {
                        let system = self.source.sign_up_device_name();
                        self.create.device.replace_text(system);
                    }
                    self.to(Step::Device, DEVICE_KEY);
                } else {
                    self.create.mismatch = true;
                    self.focus = Some(WORD_KEYS[0]);
                }
            }
            SignUpEvent::Created(result) if self.create.creating && self.at(Step::Creating) => {
                self.create.creating = false;
                match result {
                    Ok(()) => {
                        // Sign-up is over: a phrase still on the clipboard goes now.
                        if let Some(copy) = self.create.phrase_copy.take() {
                            copy.clear_now();
                        }
                        self.to(Step::Ready, TITLE_KEY);
                    }
                    Err(CreateProblem::CodeExpired) => {
                        self.create.code.clear();
                        self.create.code_problem = None;
                        self.create.code_expired = true;
                        self.create.resent = false;
                        self.create.resend_left = 0;
                        self.to(Step::Verify, CODE_KEY);
                    }
                    Err(problem) => {
                        self.create.create_problem = Some(problem);
                        self.focus = Some(FIRST_BUTTON_KEY);
                    }
                }
            }
            // An answer nobody is waiting for changes nothing.
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Scenes: each board frame, reached through the same calls a person's presses make
// ---------------------------------------------------------------------------

use super::create_source::{SignUpScript, sample};
use super::source::{Script, board};
use crate::State;

pub const FRAME_IDS: [&str; 14] = [
    "DW-A31", "DW-A32", "DW-A33", "DW-A34", "DW-A35", "DW-A36", "DW-A37", "DW-A38", "DW-A39", "DW-A40", "DW-A41",
    "DW-A42", "DW-A43", "DW-A44",
];

/// The frames of sign-up, for `frames::scenes`.
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    ("DW-A31", |s| super::set_scene(s, "DW-A31")),
    ("DW-A32", |s| super::set_scene(s, "DW-A32")),
    ("DW-A33", |s| super::set_scene(s, "DW-A33")),
    ("DW-A34", |s| super::set_scene(s, "DW-A34")),
    ("DW-A35", |s| super::set_scene(s, "DW-A35")),
    ("DW-A36", |s| super::set_scene(s, "DW-A36")),
    ("DW-A37", |s| super::set_scene(s, "DW-A37")),
    ("DW-A38", |s| super::set_scene(s, "DW-A38")),
    ("DW-A39", |s| super::set_scene(s, "DW-A39")),
    ("DW-A40", |s| super::set_scene(s, "DW-A40")),
    ("DW-A41", |s| super::set_scene(s, "DW-A41")),
    ("DW-A42", |s| super::set_scene(s, "DW-A42")),
    ("DW-A43", |s| super::set_scene(s, "DW-A43")),
    ("DW-A44", |s| super::set_scene(s, "DW-A44")),
];

/// Sign-up on the first screen of a PC with no Beebeeb session, the email typed (DW-A31).
fn at_email(sign_up: SignUpScript) -> Ui {
    let mut ui = Ui::new(Box::new(super::source::FixtureSignInSource::new(Script {
        shared_session: false,
        sign_up,
        ..Script::default()
    })));
    ui.start_create();
    ui.email.replace_text(board::EMAIL);
    ui
}

fn at_verify() -> Ui {
    let mut ui = at_email(SignUpScript::default());
    ui.send_code();
    ui
}

fn at_password(sign_up: SignUpScript, typed: usize) -> Ui {
    let mut ui = at_email(sign_up);
    ui.send_code();
    ui.create.code.replace_text(sample::CODE);
    ui.verify_email();
    ui.toggle_accepted();
    ui.toggle_understood();
    ui.accept_terms();
    // The fixture's password is x's, never a real one.
    ui.password.replace_text("x".repeat(typed));
    ui.confirm.replace_text("x".repeat(typed));
    ui.create.pictured = Some(Pictured::Password);
    ui
}

fn at_phrase(sign_up: SignUpScript) -> Ui {
    let mut ui = at_password(sign_up, board::PASSWORD_LEN);
    ui.check_password();
    ui.create.pictured = None;
    ui.toggle_saved();
    ui
}

fn at_confirm(words: [&str; CONFIRM_WORDS]) -> Ui {
    let mut ui = at_phrase(SignUpScript::default());
    ui.verify_saved();
    for (i, word) in words.iter().enumerate() {
        ui.create.words[i].replace_text(*word);
    }
    ui.create.active_word = CONFIRM_WORDS - 1;
    ui.create.pictured = Some(Pictured::Word(CONFIRM_WORDS - 1));
    ui
}

fn at_device(sign_up: SignUpScript) -> Ui {
    let mut ui = at_phrase(sign_up);
    ui.verify_saved();
    for (i, at) in sample::POSITIONS.iter().enumerate() {
        ui.create.words[i].replace_text(board::WORDS[*at]);
    }
    ui.confirm_words();
    ui.create.pictured = Some(Pictured::Device);
    ui
}

/// The state of one board frame of sign-up, by its id.
pub fn scene(frame: &str) -> Option<Ui> {
    Some(match frame {
        "DW-A31" => {
            let mut ui = at_email(SignUpScript::default());
            ui.create.pictured = Some(Pictured::Email);
            ui
        }
        // Four digits typed.
        "DW-A32" => {
            let mut ui = at_verify();
            ui.create.code.replace_text(&sample::CODE[..4]);
            ui.create.pictured = Some(Pictured::Code);
            ui
        }
        "DW-A33" => {
            let mut ui = at_verify();
            ui.create.code.replace_text(sample::WRONG_CODE);
            ui.verify_email();
            ui.create.pictured = Some(Pictured::Code);
            ui
        }
        // The countdown ran out, a new code was asked for, and four of its digits are typed.
        "DW-A34" => {
            let mut ui = at_verify();
            ui.apply_sign_up(SignUpEvent::ResendTick { seconds: 0 });
            ui.resend_code();
            ui.create.code.replace_text(&sample::CODE[..4]);
            ui.create.pictured = Some(Pictured::Code);
            ui
        }
        "DW-A35" => {
            let mut ui = at_verify();
            ui.create.code.replace_text(sample::CODE);
            ui.verify_email();
            ui.toggle_accepted();
            ui.toggle_understood();
            ui
        }
        "DW-A36" => at_password(SignUpScript::default(), board::PASSWORD_LEN),
        "DW-A37" => at_password(SignUpScript::default(), sample::SHORT_PASSWORD_LEN),
        "DW-A38" => {
            let mut ui = at_password(
                SignUpScript {
                    password: Some(PasswordProblem::Breached),
                    ..SignUpScript::default()
                },
                board::PASSWORD_LEN,
            );
            ui.check_password();
            ui.create.pictured = Some(Pictured::Password);
            ui
        }
        "DW-A39" => at_phrase(SignUpScript::default()),
        // Two words typed, the third begun.
        "DW-A40" => at_confirm([board::WORDS[2], board::WORDS[6], &board::WORDS[10][..4]]),
        "DW-A41" => {
            let mut ui = at_confirm([board::WORDS[2], sample::WRONG_WORD, board::WORDS[10]]);
            ui.confirm_words();
            ui.create.pictured = Some(Pictured::Word(CONFIRM_WORDS - 1));
            ui
        }
        "DW-A42" => at_device(SignUpScript::default()),
        // The account is being made: no answer yet.
        "DW-A43" => {
            let mut ui = at_device(SignUpScript {
                create_answers: false,
                ..SignUpScript::default()
            });
            ui.create.pictured = None;
            ui.create_account();
            ui
        }
        "DW-A44" => {
            let mut ui = at_device(SignUpScript::default());
            ui.create.pictured = None;
            ui.create_account();
            ui
        }
        _ => return None,
    })
}
