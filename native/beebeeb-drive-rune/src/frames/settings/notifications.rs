//! Notifications (DW-0750): which alerts Beebeeb sends on this PC; and notifications turned
//! off for Beebeeb in Windows (DW-0750b): said at the top, the choices kept and dimmed.

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::settings_shell as shell;
use rune::El;

use super::source::ReadState;
use super::{Intent, SettingsUi};

/// What is drawn of the account could not be confirmed with Beebeeb (a change or a mark failed, and so did the
/// read after it): said in the pane, with "Try again". Also Recent notifications' words.
pub const UNCONFIRMED: (&str, &str) = ("Couldn't check with Beebeeb.", "What's shown here may be out of date.");
/// The account's switches could not be read: the rows say so, and this offers the way to ask again.
pub const NOT_READ: (&str, &str) = (
    "Couldn't load your account's alerts.",
    "Nothing was changed. The switches on this PC still work.",
);
/// The index of the "Try again" button (`btn-1`; `btn-0` is "Open Windows Settings").
const AGAIN: usize = 1;

/// "Try again" for what the account's notifications say: asks Beebeeb again.
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
    // Below the last group, so no switch moves under the pointer when this appears: what the rows above could
    // not say (failed), or what they show that could not be confirmed, each with the way to ask again.
    let said = if not_read {
        Some((Tone::Red, NOT_READ))
    } else if ui.source.notifications_unconfirmed() {
        Some((Tone::Warn, UNCONFIRMED))
    } else {
        None
    };
    if let Some((tone, (strong, words))) = said {
        children.push(notices::note_with(
            0,
            tone,
            &ic::WARN_CIRCLE,
            strong,
            words,
            Some(try_again(AGAIN, t)),
            t,
        ));
    }
    (shell::pane(children, t), None)
}
