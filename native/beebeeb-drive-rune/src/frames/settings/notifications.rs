//! Notifications (DW-0750): which alerts Beebeeb sends on this PC; and notifications turned
//! off for Beebeeb in Windows (DW-0750b): said at the top, the choices kept and dimmed.

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::settings_shell as shell;
use rune::El;

use super::{Intent, SettingsUi};

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let n = ui.source.notifications();
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
        let rows = alerts
            .iter()
            .enumerate()
            .map(|(i, alert)| {
                let key = format!("item-{list}-{i}");
                let on = alert.on;
                let mut switch = lists::switch(&key, &alert.title, on, t);
                switch = if n.allowed_in_windows {
                    switch.on_press(move |s: &mut State| s.settings.source.set_alert(g, i, !on))
                } else {
                    switch.disabled(true).focusable(false)
                };
                let mut it = Item::new(None, alert.title.clone()).sub(alert.says.clone());
                if i > 0 {
                    it = it.ruled();
                }
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
    (shell::pane(children, t), None)
}
