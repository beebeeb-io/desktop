//! Notifications (DW-0750): which alerts Beebeeb sends on this PC; and notifications turned
//! off for Beebeeb in Windows (DW-0750b): said at the top, the choices kept and dimmed.

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::settings_notice as notices;
use crate::ui::settings_shell as shell;
use rune::{El, el};

use super::source::ReadState;
use super::{Intent, SettingsUi};

/// What is drawn of the account could not be confirmed with Beebeeb (a change or a mark failed, and so did the
/// read after it): said over the pane, with "Try again". Also Recent notifications' words.
pub const UNCONFIRMED: &str = "Couldn't check with Beebeeb. What's shown here may be out of date.";
/// The account's switches could not be read: the rows say so, and this offers the way to ask again. Nothing
/// about the other switches: whether they can be pressed is theirs to show.
pub const NOT_READ: &str = "Couldn't load your account's alerts.";
/// What the source's toasts about the account's notifications say. They are these panes' own: drawn only over
/// Notifications and Recent notifications (`super::poll`), each as what it is ([`toast_worked`]).
pub const SAVED: &str = "Notification setting updated";
/// Beebeeb refused, or the check after an unknown result shows the switch as it was.
pub const NOT_SAVED: &str = "Couldn't change the notification setting. Nothing changed.";
/// A timeout, a transport error mid-flight, or an unreadable 2xx: the change may have landed. Over when the
/// check is: its answer is said in its place ([`SAVED`], [`NOT_SAVED`]), or the pane says it could not check.
pub const UNKNOWN: &str = "Couldn't confirm the change. Checking what the server has.";
pub const PC_NOT_SAVED: &str = "Couldn't save the notification setting on this PC.";
/// A read failed. The panes say that themselves (the rows, Recent's card, [`NOT_READ`] over the pane), so this
/// is drawn only where nothing else changes: "Try again" pressed under [`UNCONFIRMED`], and failing again.
pub const NOT_LOADED: &str = "Couldn't load your notifications. Try again in a moment.";
pub const NOT_MARKED: &str = "Couldn't mark your notifications as seen.";
pub const STILL_SAVING: &str = "Still saving the last change. Try again in a moment.";

/// Whether a toast of these panes says something worked. `None`: not one of theirs (another pane's sentence
/// that landed late): drawn as that pane draws it.
pub fn toast_worked(words: &str) -> Option<bool> {
    match words {
        SAVED => Some(true),
        NOT_SAVED | UNKNOWN | PC_NOT_SAVED | NOT_LOADED | NOT_MARKED | STILL_SAVING => Some(false),
        _ => None,
    }
}

/// What stands over the Notifications pane: an account row whose switch was not read and no answer on its way
/// ([`NOT_READ`]), or switches that are drawn but could not be confirmed ([`UNCONFIRMED`]).
pub fn standing(source: &dyn super::source::SettingsSource) -> Option<&'static str> {
    let not_read = source
        .notifications()
        .groups
        .iter()
        .enumerate()
        .any(|(g, (_, alerts))| {
            source.alerts_state(g) != ReadState::Loading && alerts.iter().any(|alert| alert.on.is_none())
        });
    if not_read {
        Some(NOT_READ)
    } else if source.notifications_unconfirmed() {
        Some(UNCONFIRMED)
    } else {
        None
    }
}

/// Ask Beebeeb again. What the last toast said is over: the answer says what is so now.
fn ask_again(s: &mut State) {
    s.settings.toast = None;
    s.settings.source.reload_notifications();
}

/// What is said over Notifications and Recent notifications, where the toast sits (so it is in view however far
/// the pane is scrolled, and no row moves when it comes or goes): `standing`, what the pane could not read or
/// confirm, for as long as that holds, with "Try again"; and above it, or in its place when nothing stands, the
/// toast of the last thing that was done: the settings toast when it worked, the failure toast when it did not.
/// Keys: `notice` (`notice-line`, `notice-text`, `notice-action`), `toast-0` (`toast-0-line`, `toast-0-text`).
pub fn said_over(ui: &SettingsUi, standing: Option<&'static str>, t: &Theme) -> Option<El> {
    let again = crate::ui::toast::ToastAction {
        text: "Try again",
        name: "Try again",
        shortcut: None,
        run: |s: &mut State, _: &mut rune::Tx| ask_again(s),
    };
    let notice = standing.map(|words| notices::standing("notice", words, Some(again), 0, t));
    let line = usize::from(notice.is_some());
    // Something that was not done is never drawn with the mark of a thing that worked.
    let toast = ui
        .toast
        .filter(|words| *words != NOT_LOADED || standing == Some(UNCONFIRMED))
        .map(|words| match toast_worked(words) {
            Some(false) => notices::standing("toast-0", words, None, line, t),
            _ => notices::toast_on(0, words, line, t),
        });
    match (notice, toast) {
        (None, None) => None,
        (Some(one), None) | (None, Some(one)) => Some(one),
        // Both: a layer as large as the body, so each keeps its place over the pane.
        (Some(notice), Some(toast)) => Some(
            el().key("said-over")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .child(notice)
                .child(toast),
        ),
    }
}

/// "Try again" in a pane's own card (Recent notifications that could not be read): asks Beebeeb again.
pub fn try_again(index: usize, t: &Theme) -> El {
    controls::button(index, "Try again", None, Emphasis::Plain, t).on_press(ask_again)
}

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let n = ui.source.notifications();
    let mut children = vec![shell::heading("Notifications", None, t)];
    let mut list = 0;

    if n.allowed_in_windows {
        let on = n.allow;
        let switch = lists::switch("item-0-0", "Allow notifications", on, t).on_press(move |s: &mut State| {
            s.settings.toast = None;
            s.settings.source.set_allow_notifications(!on)
        });
        children.push(lists::list(
            list,
            None,
            vec![lists::item_with(
                list,
                0,
                &Item::new(Some(&ic::BELL), "Allow notifications").sub("Also controlled in Windows Settings"),
                vec![switch],
                t,
            )],
            Some("No marketing, ever."),
            t,
        ));
        list += 1;
    } else {
        // Beebeeb cannot turn them back on itself: the one action opens Windows Settings.
        let open = controls::button(0, "Open Windows Settings", None, Emphasis::Plain, t)
            .on_press(|s: &mut State| s.settings.ask(Intent::OpenWindowsNotificationSettings));
        children.push(controls::banner_toned(
            0,
            true,
            &ic::BELL,
            "Notifications are off",
            "Turned off for Beebeeb in Windows Settings. Nothing below arrives until they're on.",
            Some(open),
            t,
        ));
    }

    let mut recent = Item::new(Some(&ic::INBOX), "Recent notifications")
        .sub("Sign-ins, shares and storage warnings from your account");
    if n.new_recent > 0 {
        recent = recent.trailing(Trailing::Value(format!("{} new", n.new_recent)));
    }
    children.push(lists::list(
        list,
        None,
        vec![lists::item(list, 0, &recent.opens(), t).on_press(|s: &mut State| super::go(s, "DW-0791"))],
        None,
        t,
    ));
    list += 1;

    let security = if n.allowed_in_windows {
        "Always on"
    } else {
        "Off in Windows"
    };
    children.push(lists::list(
        list,
        Some("Security"),
        vec![lists::item(
            list,
            0,
            &Item::new(Some(&ic::SHIELD), "New sign-ins and security changes")
                .trailing(Trailing::Value(security.into())),
            t,
        )],
        Some("These can't be turned off here, so you always hear if someone else gets in."),
        t,
    ));
    list += 1;

    for (g, (title, alerts)) in n.groups.iter().enumerate() {
        // A row whose switch is not read (`on` is `None`: on its way, or the read failed) holds its place, so
        // nothing below it moves when the answer lands, but no switch is drawn: there is no value to draw, and
        // nothing to press. The source only chooses the words; a source that says nothing reads as failed.
        let loading = ui.source.alerts_state(g) == ReadState::Loading;
        let rows = alerts
            .iter()
            .enumerate()
            .map(|(i, alert)| {
                let mut it = Item::new(None, alert.title.clone()).sub(alert.says.clone());
                if i > 0 {
                    it = it.ruled();
                }
                let Some(on) = alert.on else {
                    let words = if loading { "Loading\u{2026}" } else { "Couldn't load" };
                    return lists::item(list, i, &it.trailing(Trailing::Value(words.into())), t).disabled(true);
                };
                let key = format!("item-{list}-{i}");
                let mut switch = lists::switch(&key, &alert.title, on, t);
                switch = if n.allowed_in_windows {
                    // What the last toast said is over with this press: its own answer is said next.
                    switch.on_press(move |s: &mut State| {
                        s.settings.toast = None;
                        s.settings.source.set_alert(g, i, !on)
                    })
                } else {
                    switch.disabled(true).focusable(false)
                };
                lists::item_with(list, i, &it, vec![switch], t)
            })
            .collect();
        let group = lists::list(list, Some(title), rows, None, t);
        // Off in Windows: the choices stay, dimmed, so nothing is lost when they come back.
        children.push(if n.allowed_in_windows {
            group
        } else {
            group.opacity(0.45)
        });
        list += 1;
    }
    // Over the pane, where the toast sits: what the rows above could not say (failed), or what they show that
    // could not be confirmed, each with the way to ask again. In view at any window size, and no switch moves
    // under the pointer when it comes or goes; the room under the last group lets it be scrolled clear.
    let standing = standing(&*ui.source);
    if standing.is_some() {
        children.push(notices::toast_room(1));
    }
    (shell::pane(children, t), said_over(ui, standing, t))
}
