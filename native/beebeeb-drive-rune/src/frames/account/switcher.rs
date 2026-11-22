//! dc-167, the account switcher (Guus 2026-11-21: a switch keeps the key, a sign-out forgets). Pressing the account
//! pill at the foot of the sidebar opens a list, in place above it: every account signed in on this PC (the current
//! one marked), "Add another account", and a Sign out per account that is not the current one (the current one signs
//! out in Settings > Security, as before).
//!
//! Choosing an account asks the engine to switch (`Engine::switch_account`): the current session waits in its own
//! slot, the chosen one comes back with its own key (no twelve words), and sync starts for it, so the previous
//! account's local data is cleared in 014's order and an unsent file of it is offered "Discard and switch" (111) by
//! the sync-off banner. A sign-out of a listed account forgets only that account.
//!
//! The engine is reached through [`Switcher`], which the engine host installs once at start ([`install`]). Every
//! call runs on a thread of its own, never the window's; the engine wakes the window when it is done. Nothing of an
//! account (email, id) is logged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::State;
use crate::theme::tokens::{self, Theme, radius, type_};
use crate::ui::button::{self, ButtonState, Size, Variant};
use crate::ui::kit;
use rune::{El, col, row};

/// One account signed in on this PC, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The engine's handle for the account (its server id). Never shown and never logged.
    pub id: String,
    /// What the list shows: the email, else "Account".
    pub email: String,
    pub current: bool,
}

/// What the switcher asks of the engine. Each call may block: it is run off the window's thread.
pub trait Switcher: Send + Sync {
    /// Every account signed in on this PC, the current one first.
    fn accounts(&self) -> Vec<Listed>;
    /// Make `id` the current account, then start its sync.
    fn switch_to(&self, id: &str) -> Result<(), String>;
    /// Keep the current account signed in and show the sign-in for another one.
    fn add_another(&self) -> Result<(), String>;
    /// Sign out of `id`, which is not the current account: only it is forgotten.
    fn sign_out(&self, id: &str) -> Result<(), String>;
}

static SWITCHER: OnceLock<Arc<dyn Switcher>> = OnceLock::new();
static OPEN: AtomicBool = AtomicBool::new(false);
static BUSY: AtomicBool = AtomicBool::new(false);
/// The last refusal, in a person's words; cleared when the list closes.
static SAID: Mutex<Option<String>> = Mutex::new(None);
/// The list as last read, so a frame never waits on the credential store.
static LISTED: Mutex<Vec<Listed>> = Mutex::new(Vec::new());

/// Installed once by the engine host at start; a second call changes nothing.
pub fn install(switcher: Arc<dyn Switcher>) {
    let _ = SWITCHER.set(switcher);
}

fn switcher() -> Option<Arc<dyn Switcher>> {
    SWITCHER.get().cloned()
}

/// Whether the list is open.
pub fn is_open() -> bool {
    OPEN.load(Ordering::SeqCst)
}

fn said() -> Option<String> {
    SAID.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn say(words: Option<String>) {
    *SAID.lock().unwrap_or_else(PoisonError::into_inner) = words;
}

fn listed() -> Vec<Listed> {
    LISTED.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Read the list again, off the window's thread.
fn refresh() {
    let Some(switcher) = switcher() else { return };
    run_off_thread("account-switcher-list", move || {
        let accounts = switcher.accounts();
        *LISTED.lock().unwrap_or_else(PoisonError::into_inner) = accounts;
        Ok(())
    });
}

/// The pill's press: open (reading the list) or close.
pub fn toggle(_s: &mut State) {
    toggle_open();
}

fn toggle_open() {
    let open = !OPEN.load(Ordering::SeqCst);
    OPEN.store(open, Ordering::SeqCst);
    say(None);
    if open {
        refresh();
    }
}

fn close() {
    OPEN.store(false, Ordering::SeqCst);
}

/// The pill's key and the last item's: `rune` sends Return on a button and the arrows outside a group to the app.
const PILL: &str = "account-pill";
const ADD: &str = "switcher-add";

/// The switcher's keys (RC2-8), heard by the shell before any frame: Enter on the pill opens or closes the list, as
/// Space does; Down on the pill goes to the list's first item, Up to its last ("Add another account", the item next
/// to the pill); Escape closes the list and gives the keyboard back to the pill. Inside the list the arrows are its
/// focus group's.
pub fn key(ev: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    use rune::Key;
    if switcher().is_none() || ev.mods != rune::Mods::default() {
        return false;
    }
    let focused = tx.focused();
    let on_pill = focused == Some(PILL);
    let in_list = focused.is_some_and(|key| key.starts_with("switcher-"));
    match ev.key {
        Key::Enter if on_pill => {
            if !tx.key_repeat() {
                toggle_open();
            }
            true
        }
        Key::Up | Key::Down if on_pill && is_open() => {
            let to = if ev.key == Key::Up {
                ADD.to_string()
            } else {
                first_item()
            };
            tx.focus(to, true);
            true
        }
        Key::Escape if is_open() && (on_pill || in_list) => {
            close();
            say(None);
            tx.return_focus(PILL, true);
            true
        }
        _ => false,
    }
}

/// The first item the keyboard can be on: the first account that is not the current one, else "Add another account".
fn first_item() -> String {
    listed()
        .iter()
        .position(|account| !account.current)
        .map_or_else(|| ADD.to_string(), |i| format!("switcher-account-{i}"))
}

/// One engine call on its own thread; one at a time. A refusal is shown in the list; the list is read again after.
fn run_off_thread(name: &str, call: impl FnOnce() -> Result<(), String> + Send + 'static) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let started = std::thread::Builder::new().name(name.to_string()).spawn(move || {
        let result = call();
        if let Err(words) = result {
            say(Some(words));
        }
        if let Some(switcher) = switcher() {
            let accounts = switcher.accounts();
            *LISTED.lock().unwrap_or_else(PoisonError::into_inner) = accounts;
        }
        BUSY.store(false, Ordering::SeqCst);
    });
    if started.is_err() {
        BUSY.store(false, Ordering::SeqCst);
        say(Some("Beebeeb could not do that just now. Try again.".to_string()));
    }
}

fn switch_to(id: String) {
    let Some(switcher) = switcher() else { return };
    close();
    run_off_thread("account-switch", move || switcher.switch_to(&id));
}

fn add_another() {
    let Some(switcher) = switcher() else { return };
    close();
    run_off_thread("account-add", move || switcher.add_another());
}

fn sign_out(id: String) {
    let Some(switcher) = switcher() else { return };
    run_off_thread("account-sign-out-other", move || switcher.sign_out(&id));
}

fn initials(email: &str) -> String {
    email
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default()
}

/// The list, drawn above the account row while it is open (`None` while closed, or with no switcher installed).
/// Keys: `switcher`, per account `switcher-account-<i>` (`-email`, `-current`, `-sign-out`), `switcher-add`,
/// `switcher-said`.
pub fn panel(t: &Theme) -> Option<El> {
    if !is_open() || switcher().is_none() {
        return None;
    }
    let busy = BUSY.load(Ordering::SeqCst);
    let mut list = col()
        .key("switcher")
        .role("menu")
        .label("Accounts on this PC")
        // One Tab stop whose items the arrows walk, to "Add another account" (RC2-8).
        .focus_group()
        .gap(crate::theme::scale::space::S2)
        .px(crate::theme::scale::space::S4)
        .py(crate::theme::scale::space::S4)
        .radius(radius::CONTROL)
        .fill(t.card)
        .border(kit::HAIR, t.separator)
        .child(
            kit::label("switcher-head", "Accounts on this PC", type_::CAPTION, t.text_tertiary)
                .px(crate::theme::scale::space::S6)
                .py(crate::theme::scale::space::S2),
        );
    for (i, account) in listed().into_iter().enumerate() {
        let key = format!("switcher-account-{i}");
        let mut line = row()
            .key(key.clone())
            .role("menuitem")
            .label(if account.current {
                format!("{}, signed in now", account.email)
            } else {
                format!("Switch to {}", account.email)
            })
            .center_y()
            .gap(crate::theme::scale::space::S8)
            .px(crate::theme::scale::space::S6)
            .py(crate::theme::scale::space::S4)
            .radius(radius::CONTROL)
            .child(kit::avatar(
                format!("{key}-avatar"),
                &initials(&account.email),
                20.0,
                tokens::account_avatar_tint(),
                t.card,
                t,
            ))
            .child(
                kit::label(format!("{key}-email"), &account.email, type_::BODY_SM, t.text)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if account.current {
            line = line.child(kit::label(
                format!("{key}-current"),
                "Current",
                type_::CAPTION,
                t.text_tertiary,
            ));
        } else {
            let id = account.id.clone();
            line = line
                .hover(t.selection)
                .press(t.press)
                .focusable(!busy)
                .disabled(busy)
                .on_press(move |_s: &mut State| switch_to(id.clone()));
            let id = account.id.clone();
            let state = if busy { ButtonState::Busy } else { ButtonState::Enabled };
            line = line.child(
                button::button(
                    button::keys(&format!("{key}-sign-out")),
                    "Sign out",
                    None,
                    Variant::Ghost,
                    Size::Sm,
                    state,
                    t,
                )
                .label(format!("Sign out of {}", account.email))
                .on_press(move |_s: &mut State| sign_out(id.clone())),
            );
        }
        list = list.child(line);
    }
    list = list.child(
        row()
            .key(ADD)
            .role("menuitem")
            .label("Add another account")
            .px(crate::theme::scale::space::S6)
            .py(crate::theme::scale::space::S6)
            .radius(radius::CONTROL)
            .hover(t.selection)
            .press(t.press)
            .focusable(!busy)
            .disabled(busy)
            .on_press(|_s: &mut State| add_another())
            .child(kit::label(
                "switcher-add-label",
                "Add another account",
                type_::BODY_SM,
                t.text,
            )),
    );
    if let Some(words) = said() {
        list = list.child(
            kit::label("switcher-said", words, type_::CAPTION, t.text_secondary)
                .px(crate::theme::scale::space::S6)
                .wrap(),
        );
    }
    Some(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_come_from_the_first_letter_only() {
        assert_eq!(initials("anna@example.eu"), "A");
        assert_eq!(initials("_9b@example.eu"), "9");
        assert_eq!(initials(""), "");
    }
}
