//! "Use your password" for app lock (no board frame: D-2, `FRAMES.md`). The board's rule is "The Windows app's
//! password works when Windows Hello doesn't" (DW-0716's Rules); no frame draws the step. It is drawn from the
//! account kit as the password confirm of DW-A78 is, with the lock screen's sentences for each answer
//! (`frames::signin::lock`, `FRAMES.md` "Screen lock: password").
//!
//! - The link shows on the Lock Beebeeb row only while the source offers it: Windows Hello did not confirm turning
//!   app lock off or a longer "Locks again after" (cancelled, failed, or not set up on this PC).
//! - The step: "Confirm it's you", the change it is for, the password field, Continue and Cancel. While the password
//!   is checked Continue says "Checking…" and nothing closes the step: the check cannot be called back, so it closes
//!   with its answer. A wrong password keeps it open with the field empty and says so; any other answer closes it (the
//!   toast says the rest).
//! - The typed text leaves the field at the press, moved (never copied) into a wiping buffer that goes to the source
//!   ([`LockPassword`]); the field is a fresh secure one from then on. Escape and Cancel wipe it.
//!
//! Must not: log, print, format or keep the password; show the step while nothing offers it.

use std::fmt;

use rune::El;
use zeroize::Zeroizing;

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_form::{self as fm, Field};
use crate::ui::account_set::{self as set, Look};

use super::form::{self, Form, Problem};
use super::forms::button;
use super::source::{AccountSource, BoardAccount, SecurityFacts};
use super::{AccountUi, Dialog};

/// The row's link ("Use your password"), as `fm::link` keys it (tests find it by this).
#[cfg(test)]
pub const LINK_KEY: &str = "link-9-box";
/// The step's title (the account pages' own words for a password asked to confirm a change).
pub const TITLE: &str = "Confirm it's you";
const FOR_OFF: &str = "Turning off app lock needs your account password.";
const FOR_LONGER: &str = "A longer lock delay needs your account password.";
const CONTINUE: &str = "Continue";
const CHECKING: &str = "Checking…";
/// Continue with the field empty: nothing is sent.
const EMPTY: &str = "Enter your password to confirm it's you.";

/// The account password typed into the step, on its way to the engine for one change. It owns the text in a wiping
/// buffer; its `Debug` and `Display` print a fixed placeholder, never the text; it has no `Clone`, `PartialEq` or
/// `AsRef<str>`, so a pane cannot compare, copy or log it.
pub struct LockPassword(Zeroizing<String>);

impl LockPassword {
    /// The text moved out of the secure field at the press.
    pub fn new(text: Zeroizing<String>) -> Self {
        LockPassword(text)
    }
    /// The wiping buffer, for the engine's proof and nothing else. Called by an `AccountSource` implementation only.
    pub fn into_proof(self) -> Zeroizing<String> {
        self.0
    }
}

impl fmt::Debug for LockPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LockPassword(hidden)")
    }
}

impl fmt::Display for LockPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(hidden)")
    }
}

/// "Use your password" on the Lock Beebeeb row, while the source offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordOffer {
    /// What it is for: turning app lock off (`true`), or a longer "Locks again after".
    pub turning_off: bool,
    /// The password is being checked: Continue waits, and the step closes only with the answer.
    pub checking: bool,
    /// The sentence under the field after the last answer (a wrong password, a wait). `None`: nothing to say.
    pub said: Option<String>,
}

fn offer(source: Option<&dyn AccountSource>) -> Option<PasswordOffer> {
    source.and_then(|s| s.security_facts().app_lock_password)
}

/// The row's link: opens the step.
pub fn link(t: &Theme) -> El {
    fm::link(9, "Use your password", set::SUB, false, t)
        .mr(12.0)
        .on_press(open)
}

/// The link was pressed: the step opens with an empty field in focus. Nothing opens while nothing offers it.
pub fn open(s: &mut State) {
    if offer(s.account.source.as_deref()).is_none() {
        return;
    }
    s.account.form.current = form::secure();
    super::ask(s, Dialog::LockPassword { sent: false });
}

/// The step over the Security page, when it is open and offered.
pub fn over(source: &dyn AccountSource, f: &Form, dialog: Option<&Dialog>, t: &Theme) -> Option<El> {
    if !matches!(dialog, Some(Dialog::LockPassword { .. })) {
        return None;
    }
    Some(step(&source.security_facts().app_lock_password?, f, t))
}

fn step(offer: &PasswordOffer, f: &Form, t: &Theme) -> El {
    let checking = offer.checking;
    let message = if checking {
        None
    } else if f.problem == Some(Problem::WrongPassword) {
        Some(EMPTY)
    } else {
        offer.said.as_deref()
    };
    let mut field = fm::field(
        0,
        &Field {
            label: "Account password",
            icon: None,
            message,
            error: message.is_some(),
            input_key: form::CURRENT_KEY,
        },
        &f.current,
        // While the password is checked nothing can be typed: keys go to a field nothing reads.
        move |s: &mut State| {
            if checking {
                &mut s.account.form.sink
            } else {
                &mut s.account.form.current
            }
        },
        t,
    );
    if checking {
        field = field.disabled(true).focusable(false);
    }
    let (mut go_on, go_b) = button(
        0,
        if checking { CHECKING } else { CONTINUE },
        Look::Primary,
        checking,
        t,
    );
    let (mut cancel, cancel_b) = button(1, "Cancel", Look::Plain, checking, t);
    if !checking {
        go_on = go_on.on_press(super::act(submit));
        if !f.current.text.is_empty() {
            go_on = go_on.default_button();
        }
        cancel = cancel.on_press(|s: &mut State| close(&mut s.account));
    }
    let words = if offer.turning_off { FOR_OFF } else { FOR_LONGER };
    set::dialog(
        TITLE,
        440.0,
        TITLE,
        words,
        vec![field],
        vec![(go_on, go_b), (cancel, cancel_b)],
        t,
    )
    .modal(false, |s: &mut State| {
        on_escape(s);
    })
}

/// Continue: an empty field sends nothing and says so; otherwise the text moves out of the field into a wiping buffer
/// and goes to the source, once. A press while one is checked does nothing.
fn submit(ui: &mut AccountUi) -> Option<&'static str> {
    if offer(ui.source.as_deref()).is_none_or(|offer| offer.checking) {
        return None;
    }
    if ui.form.current.text.is_empty() {
        ui.form.problem = Some(Problem::WrongPassword);
        form::focus(&mut ui.form, Some(form::CURRENT_KEY));
        return None;
    }
    let typed = Zeroizing::new(std::mem::take(&mut ui.form.current.text));
    ui.form.current = form::secure();
    ui.form.problem = None;
    // No caret in a field nothing can be typed in while the password is checked (`follow` gives it back).
    form::focus(&mut ui.form, None);
    ui.dialog = Some(Dialog::LockPassword { sent: true });
    ui.source
        .as_deref_mut()?
        .app_lock_with_password(LockPassword::new(typed));
    None
}

/// The step closes; the field is wiped with it (`form::arrive` gives a page with no dialog a fresh secure field).
fn close(ui: &mut AccountUi) {
    ui.dialog = None;
    form::arrive(ui);
}

/// The window's poll: the step closes once the source no longer offers it (the change was made, an answer ended it,
/// the session moved on); when an answer keeps it open (a wrong password, a wait), the field takes the keys again,
/// once. `true`: what the step shows changed.
pub fn follow(ui: &mut AccountUi) -> bool {
    let Some(Dialog::LockPassword { sent }) = ui.dialog else {
        return false;
    };
    match offer(ui.source.as_deref()) {
        None => close(ui),
        Some(offer) if sent && !offer.checking => {
            ui.dialog = Some(Dialog::LockPassword { sent: false });
            form::focus(&mut ui.form, Some(form::CURRENT_KEY));
        }
        Some(_) => return false,
    }
    true
}

/// Escape, or a press outside the step: it closes with nothing changed, except while the password is checked (the
/// check cannot be called back). `true`: the step was open and took the key.
pub fn on_escape(s: &mut State) -> bool {
    if !matches!(s.account.dialog, Some(Dialog::LockPassword { .. })) {
        return false;
    }
    if !offer(s.account.source.as_deref()).is_some_and(|offer| offer.checking) {
        close(&mut s.account);
    }
    true
}

/// The step's states for a render to look at (`--scene APPLOCK-...`): no board frame (D-2), so no golden and no gate.
/// Ids outside the board's pattern on purpose. Fixture data only.
pub const SCENES: &[(&str, super::SceneFn)] = &[
    ("APPLOCK-password-offered", |s| {
        s.account.source = Some(Box::new(BoardAccount::new().knowing(asked)));
        super::go(s, "DW-A52");
    }),
    ("APPLOCK-password", |s| scene(s, asked, true)),
    ("APPLOCK-password-wrong", |s| scene(s, wrong, false)),
    ("APPLOCK-password-checking", |s| {
        scene(s, checking, false);
        s.account.dialog = Some(Dialog::LockPassword { sent: true });
        form::focus(&mut s.account.form, None);
    }),
];

fn asked(all: SecurityFacts) -> SecurityFacts {
    with_offer(all, false, None)
}

fn wrong(all: SecurityFacts) -> SecurityFacts {
    with_offer(all, false, Some("That password isn't right."))
}

fn checking(all: SecurityFacts) -> SecurityFacts {
    with_offer(all, true, None)
}

fn with_offer(all: SecurityFacts, checking: bool, said: Option<&str>) -> SecurityFacts {
    SecurityFacts {
        app_lock_password: Some(PasswordOffer {
            turning_off: true,
            checking,
            said: said.map(str::to_string),
        }),
        ..all
    }
}

/// `typed`: the field holds a password (bullets); after a wrong one, and while one is checked, it is empty.
fn scene(s: &mut State, facts: fn(SecurityFacts) -> SecurityFacts, typed: bool) {
    s.account.source = Some(Box::new(BoardAccount::new().knowing(facts)));
    super::go(s, "DW-A52");
    open(s);
    if typed {
        form::type_in(&mut s.account.form.current, "............");
    }
}
