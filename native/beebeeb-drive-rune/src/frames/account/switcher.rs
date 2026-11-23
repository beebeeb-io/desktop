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
//! call runs on a thread of its own, never the window's. Nothing of an account (email, id) is logged.
//!
//! RC3-1 (VM 401, rc3/f3b f4: "Switch to" pressed 1.3 s after the list opened did nothing, with no log line; after a
//! restart the list showed no account). Reading the list and acting on it are apart: a read never disables a row and
//! never drops a press. One action runs at a time (`Menu::acting`); a press while one runs is said in the list and
//! logged, never dropped. The list is read once at [`install`], and the press that opens it waits a moment for its
//! read ([`READ_WAIT`]), so the list is drawn as it is now: nothing draws the window again when a read ends later
//! (with sync idle or paused no other event does).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

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
/// The window's one list.
static MENU: OnceLock<Arc<Menu>> = OnceLock::new();

/// How long the press that opens the list waits for its read, so the list is drawn as it is now and not as it was
/// last read. A credential store slower than this shows the last read; its rows still take a press.
const READ_WAIT: Duration = Duration::from_millis(250);
/// What is said when a press arrives while another request still runs.
const STILL_WORKING: &str = "Beebeeb is still working on the last request. Try again in a moment.";

/// The list's state. One for the window ([`MENU`]); a test makes its own.
#[derive(Default)]
struct Menu {
    open: AtomicBool,
    /// A switch, an add or a sign-out runs: one at a time. Reading the list never sets it.
    acting: AtomicBool,
    /// The last refusal, in a person's words; cleared when the list closes.
    said: Mutex<Option<String>>,
    /// The list as last read, so a frame never waits on the credential store.
    listed: Mutex<Vec<Listed>>,
}

/// Ends an action however its thread ends (a panic too), so the next press is taken.
struct Acting(Arc<Menu>);

impl Drop for Acting {
    fn drop(&mut self) {
        self.0.acting.store(false, Ordering::SeqCst);
    }
}

impl Menu {
    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    fn acting(&self) -> bool {
        self.acting.load(Ordering::SeqCst)
    }

    fn said(&self) -> Option<String> {
        self.said.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn say(&self, words: Option<String>) {
        *self.said.lock().unwrap_or_else(PoisonError::into_inner) = words;
    }

    fn listed(&self) -> Vec<Listed> {
        self.listed.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Read the list again on a thread of its own, waiting at most `wait` for it. Never holds a press back: a read
    /// sets no flag a row looks at.
    fn refresh(self: &Arc<Self>, switcher: Arc<dyn Switcher>, wait: Duration) {
        let (done, read) = std::sync::mpsc::channel();
        let menu = self.clone();
        let started = std::thread::Builder::new()
            .name("account-switcher-list".to_string())
            .spawn(move || {
                let accounts = switcher.accounts();
                *menu.listed.lock().unwrap_or_else(PoisonError::into_inner) = accounts;
                let _ = done.send(());
            });
        if started.is_err() {
            tracing::warn!("account switcher: the list could not be read (no thread)");
        } else if !wait.is_zero() {
            let _ = read.recv_timeout(wait);
        }
    }

    /// The pill: open (reading the list) or close.
    fn toggle(self: &Arc<Self>, switcher: Option<Arc<dyn Switcher>>) {
        let open = !self.is_open();
        self.open.store(open, Ordering::SeqCst);
        self.say(None);
        if open && let Some(switcher) = switcher {
            self.refresh(switcher, READ_WAIT);
        }
    }

    /// One action (a switch, an add, a sign-out) on its own thread; one at a time. A press while one runs is said
    /// in the list and logged, never dropped. A refusal is shown in the list; the list is read again after.
    fn act(
        self: &Arc<Self>,
        name: &'static str,
        switcher: Arc<dyn Switcher>,
        call: impl FnOnce(&dyn Switcher) -> Result<(), String> + Send + 'static,
    ) {
        if self.acting.swap(true, Ordering::SeqCst) {
            tracing::warn!(
                request = name,
                "account switcher: not taken, another request is still running"
            );
            self.say(Some(STILL_WORKING.to_string()));
            self.open.store(true, Ordering::SeqCst);
            return;
        }
        let menu = self.clone();
        let started = std::thread::Builder::new().name(name.to_string()).spawn(move || {
            let acting = Acting(menu);
            let menu = &acting.0;
            tracing::info!(request = name, "account switcher: asked");
            if let Err(words) = call(&*switcher) {
                tracing::warn!(request = name, "account switcher: refused");
                menu.say(Some(words));
                menu.open.store(true, Ordering::SeqCst);
            }
            let accounts = switcher.accounts();
            *menu.listed.lock().unwrap_or_else(PoisonError::into_inner) = accounts;
        });
        if started.is_err() {
            self.acting.store(false, Ordering::SeqCst);
            tracing::warn!(request = name, "account switcher: not started (no thread)");
            self.say(Some("Beebeeb could not do that just now. Try again.".to_string()));
            self.open.store(true, Ordering::SeqCst);
        }
    }

    /// A press on an account: the list closes and the engine is asked to switch.
    fn switch_to(self: &Arc<Self>, switcher: Arc<dyn Switcher>, id: String) {
        self.open.store(false, Ordering::SeqCst);
        self.act("account-switch", switcher, move |switcher| switcher.switch_to(&id));
    }
}

fn menu() -> &'static Arc<Menu> {
    MENU.get_or_init(Arc::default)
}

/// Installed once by the engine host at start; a second call changes nothing.
pub fn install(switcher: Arc<dyn Switcher>) {
    if SWITCHER.set(switcher.clone()).is_ok() {
        // The first open draws what was last read: read once now, so that is the accounts and never an empty list.
        menu().refresh(switcher, Duration::ZERO);
    }
}

fn switcher() -> Option<Arc<dyn Switcher>> {
    SWITCHER.get().cloned()
}

/// Whether the list is open.
pub fn is_open() -> bool {
    menu().is_open()
}

fn said() -> Option<String> {
    menu().said()
}

fn say(words: Option<String>) {
    menu().say(words);
}

fn listed() -> Vec<Listed> {
    menu().listed()
}

/// The pill's press: open (reading the list) or close.
pub fn toggle(_s: &mut State) {
    toggle_open();
}

fn toggle_open() {
    menu().toggle(switcher());
}

fn close() {
    menu().open.store(false, Ordering::SeqCst);
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

fn switch_to(id: String) {
    let Some(switcher) = switcher() else { return };
    menu().switch_to(switcher, id);
}

fn add_another() {
    let Some(switcher) = switcher() else { return };
    close();
    menu().act("account-add", switcher, |switcher| switcher.add_another());
}

fn sign_out(id: String) {
    let Some(switcher) = switcher() else { return };
    menu().act("account-sign-out-other", switcher, move |switcher| {
        switcher.sign_out(&id)
    });
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
    // Only an action in flight holds the rows back: a read of the list never does (RC3-1).
    let busy = menu().acting();
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

    /// A credential store that answers slowly (as Windows' can): `accounts` waits until the test lets it go.
    #[derive(Default)]
    struct SlowStore {
        open: Mutex<bool>,
        opened: std::sync::Condvar,
        switched: Mutex<Vec<String>>,
    }

    impl SlowStore {
        fn let_go(&self) {
            *self.open.lock().unwrap() = true;
            self.opened.notify_all();
        }
    }

    impl Switcher for SlowStore {
        fn accounts(&self) -> Vec<Listed> {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.opened.wait(open).unwrap();
            }
            vec![
                Listed { id: "b".into(), email: "b@example.eu".into(), current: true },
                Listed { id: "a".into(), email: "a@example.eu".into(), current: false },
            ]
        }
        fn switch_to(&self, id: &str) -> Result<(), String> {
            self.switched.lock().unwrap().push(id.to_string());
            Ok(())
        }
        fn add_another(&self) -> Result<(), String> {
            Ok(())
        }
        fn sign_out(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn until(what: impl Fn() -> bool) -> bool {
        let asked = std::time::Instant::now();
        while !what() && asked.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        what()
    }

    /// RC3-1 (c), rc3/f3b: the list was opened and "Switch to ..." pressed 1.3 s later; 142 s on, the old account,
    /// no dialog and no log line. Opening the list reads it off the window's thread, and while that read ran a press
    /// on an account was dropped without a word (red at 63e293320). A press reaches the engine whatever the read does.
    #[test]
    fn rc3_a_press_on_an_account_while_the_list_is_read_reaches_the_engine() {
        let (menu, store) = (Arc::new(Menu::default()), Arc::new(SlowStore::default()));
        menu.toggle(Some(store.clone()));
        assert!(
            menu.is_open() && !menu.acting(),
            "the list is open and its rows take a press while it is read"
        );

        menu.switch_to(store.clone(), "a".to_string());

        assert!(
            until(|| !store.switched.lock().unwrap().is_empty()),
            "the press asked the engine to switch"
        );
        assert_eq!(*store.switched.lock().unwrap(), vec!["a".to_string()]);
        assert!(!menu.is_open(), "and the list closed");

        // The switch has not ended (its read of the list waits on the store): a second press is said, not dropped.
        menu.switch_to(store.clone(), "a".to_string());
        assert_eq!(menu.said().as_deref(), Some(STILL_WORKING));
        assert!(menu.is_open(), "where the person reads it");
        assert_eq!(store.switched.lock().unwrap().len(), 1, "and asked nothing twice");

        store.let_go();
        assert!(until(|| !menu.acting()), "the request ended");
        assert_eq!(menu.listed().len(), 2, "and the list was read again");
    }

    /// rc3/f4: after a restart the first open showed no account at all (the list as last read: never). The press
    /// that opens the list waits a moment for its read, so the accounts are there when the list is first drawn.
    #[test]
    fn rc3_the_list_is_read_by_the_time_it_is_first_drawn() {
        let (menu, store) = (Arc::new(Menu::default()), Arc::new(SlowStore::default()));
        store.let_go();
        menu.toggle(Some(store.clone()));
        assert_eq!(menu.listed().len(), 2, "the press that opened the list read it");
        assert!(
            menu.listed().iter().any(|account| !account.current),
            "the other account is listed"
        );
    }

    #[test]
    fn initials_come_from_the_first_letter_only() {
        assert_eq!(initials("anna@example.eu"), "A");
        assert_eq!(initials("_9b@example.eu"), "9");
        assert_eq!(initials(""), "");
    }
}
