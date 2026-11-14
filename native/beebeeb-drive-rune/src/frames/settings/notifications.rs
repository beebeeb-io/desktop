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

/// What is said over Notifications and Recent notifications, where the toast sits (so it is in view however far
/// the pane is scrolled, and no row moves when it comes or goes): `standing`, what the pane could not read or
/// confirm, for as long as that holds, with "Try again"; and above it, or in its place when nothing stands, the
/// toast of the last thing that was done. Keys: `notice-line` (in it `toast`, `toast-action`), `toast-0`.
pub fn said_over(ui: &SettingsUi, standing: Option<&'static str>, t: &Theme) -> Option<El> {
    let again = crate::ui::toast::ToastAction {
        text: "Try again",
        name: "Try again",
        shortcut: None,
        run: |s: &mut State, _: &mut rune::Tx| s.settings.source.reload_notifications(),
    };
    let notice = standing.map(|words| notices::standing("notice-line", words, Some(again), 0, t));
    let toast = ui
        .toast
        .map(|words| notices::toast_on(0, words, usize::from(notice.is_some()), t));
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
    controls::button(index, "Try again", None, Emphasis::Plain, t)
        .on_press(|s: &mut State| s.settings.source.reload_notifications())
}

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let n = ui.source.notifications();
    // An account row whose switch was not read, and no answer on its way.
    let mut not_read = false;
    let mut children = vec![shell::heading("Notifications", None, t)];
    let mut list = 0;

    if n.allowed_in_windows {
        let on = n.allow;
        let switch = lists::switch("item-0-0", "Allow notifications", on, t)
            .on_press(move |s: &mut State| s.settings.source.set_allow_notifications(!on));
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
                    switch.on_press(move |s: &mut State| s.settings.source.set_alert(g, i, !on))
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
        not_read |= !loading && alerts.iter().any(|alert| alert.on.is_none());
    }
    // Over the pane, where the toast sits: what the rows above could not say (failed), or what they show that
    // could not be confirmed, each with the way to ask again. In view at any window size, and no switch moves
    // under the pointer when it comes or goes; the room under the last group lets it be scrolled clear.
    let standing = if not_read {
        Some(NOT_READ)
    } else if ui.source.notifications_unconfirmed() {
        Some(UNCONFIRMED)
    } else {
        None
    };
    if standing.is_some() {
        children.push(notices::toast_room(1));
    }
    (shell::pane(children, t), said_over(ui, standing, t))
}
