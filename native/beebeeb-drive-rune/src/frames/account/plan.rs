//! The plan, read only (DW-0708): a dialog over Account, opened from the plan card Drive adds
//! to the shared Account page (DW-A51). Everything about paying happens on the website; there
//! are no purchases in the app, so the dialog shows and links, and changes nothing.

use crate::State;
use crate::frames::settings::Intent;
use crate::theme::tokens::Theme;
use crate::ui::account_set::{self as set, Look, Size};
use crate::ui::settings_dialog::{self as dialogs, DIALOG_TITLE};
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::{dialog, kit, settings_meter as meters};
use rune::{El, el, row};

use super::source::PlanDetails;
use super::{AccountUi, Dialog};

/// The dialog's width (`.wn-dlg`, 400 pt).
const WIDTH: f32 = 400.0;

/// The dialog was just asked for (`go`). When nothing can say what the plan is (no source, or
/// one that does not answer yet), the plan page on the website is the place: `true`, and no
/// empty dialog opens.
pub fn arrive(ui: &mut AccountUi) -> bool {
    if ui.dialog != Some(Dialog::Plan) {
        return false;
    }
    let known = ui.source.as_deref().is_some_and(|s| s.plan_details().is_some());
    if !known {
        ui.dialog = None;
    }
    !known
}

/// The name the shell holds the plan page's address under (`Intent::Web`).
pub const PLAN_PAGE: &str = "plan";

/// The dialog. Its list and buttons are numbered after the Account pane's own (three lists,
/// no button), as the board numbers what a frame draws.
pub fn overlay(plan: &PlanDetails, t: &Theme) -> El {
    let figures = row()
        .key("plan-figures")
        .w_full()
        .items_start()
        .justify_between()
        .child(kit::label("plan-size", plan.size.clone(), DIALOG_TITLE, t.text))
        .child(el().key("plan-used-box").pt(5.5).child(meters::value_then(
            "plan-used",
            &plan.used,
            " used",
            set::SUB,
            t.text_tertiary,
        )));
    let rows = vec![
        lists::item(
            3,
            0,
            &Item::new(None, "Renews").trailing(Trailing::Mono(plan.renews.clone())),
            t,
        ),
        lists::item(
            3,
            1,
            &Item::new(None, "Billing")
                .trailing(Trailing::Value(plan.billing.clone()))
                .ruled(),
            t,
        ),
    ];
    let body = vec![
        figures,
        meters::meter(0, WIDTH - 48.0, 6.0, &[(plan.share, t.text)], t)
            .label(format!("{} of {} used", plan.used, plan.size)),
        lists::list(3, None, rows, None, t).w_full(),
        dialogs::caption(
            0,
            "Plans are changed on beebeeb.io. There are no purchases in the app.",
            t,
        ),
    ];
    let foot = row()
        .w_full()
        .center_y()
        .child(
            set::button(
                "btn-0",
                "Manage on beebeeb.io",
                None,
                Look::Plain,
                Size::Medium,
                false,
                t,
            )
            .label("Manage on beebeeb.io, opens in the browser")
            .on_press(|s: &mut State| s.settings.ask(Intent::Web(PLAN_PAGE))),
        )
        .child(el().key("plan-foot-gap").grow(1.0))
        .child(
            set::button("btn-1", "Done", None, Look::Plain, Size::Medium, false, t)
                .default_button()
                .on_press(|s: &mut State| s.account.dialog = None),
        );
    dialog::scrim(0.0, dialog::card("Plan", WIDTH, "Plan", None, body, foot, t), t)
        .modal(false, |s: &mut State| s.account.dialog = None)
}
