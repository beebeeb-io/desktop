//! A toast (`.mk-toast > .kit-toast.glass`): a short message over the bottom of the content
//! area, 42 pt above its lower edge, centred. With an action (`Undo`) it is a button too.
//!
//! Keys: `toast`, `toast-icon`, `toast-text`, and with an action `toast-action`, `toast-action-text`.

use crate::State;
use crate::theme::tokens::Theme;
use rune::{Color, El, Icon, LiveRegion, Shadow, TextStyle, Tx, el, row};

use crate::theme::icons;

use super::kit;

/// How far a toast sits above the lower edge of the content area.
pub const INSET: f32 = 42.0;
/// `.mk-toast .kit-toast`
pub const TEXT: TextStyle = TextStyle::new(12.5, 550.0).lh(17.0);
/// `.mk-toast .kit-toast b`
pub const ACTION: TextStyle = TextStyle::new(12.5, 600.0).lh(17.0);

/// What the glyph says (`.tone-neutral`, `.tone-green`, `.tone-red`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Done,
    /// Something failed (DW-0109).
    Failed,
}

/// The glyph that leads a toast: none, a check (done), a lock (kept private) or a warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lead {
    None,
    Check,
    Lock,
    Warn,
}

/// A toast's button: the words drawn on it ("Undo Ctrl+Z"), the name assistive technology reads
/// ("Undo"), its shortcut if it has one ("Ctrl+Z"), and what it does.
#[derive(Clone, Copy)]
pub struct ToastAction {
    pub text: &'static str,
    pub name: &'static str,
    pub shortcut: Option<&'static str>,
    pub run: fn(&mut State, &mut Tx),
}

/// The layer a toast sits in: as wide as the content area, 34 pt tall, `bottom` points above
/// the area's lower edge. Put it last in a column that is the content area, so it overlays.
pub fn layer(toast: El) -> El {
    row()
        .key("toast-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .pb(INSET)
        .justify_center()
        .items_end()
        .child(toast)
}

/// The one toast: `words`, an optional lead glyph and an optional action element (a button the
/// caller builds). A polite live region, so assistive technology reads it when there is a pause.
/// Keys: `<key>`, `<key>-icon` (with a lead), `<key>-text`.
pub fn toast_with(key: &str, words: &str, lead: Lead, action: Option<El>, t: &Theme) -> El {
    let lead = match lead {
        Lead::None => None,
        Lead::Check => Some((&icons::CHECK_CIRCLE, t.green)),
        Lead::Lock => Some((&icons::LOCK, t.text_secondary)),
        Lead::Warn => Some((&icons::ALERT, t.red)),
    };
    build(key, words, lead, "status", action, t)
}

/// The toast. A failure is read at once (`alert`); the others when there is a pause (`status`).
pub fn toast(ic: &'static Icon, tone: Tone, message: &str, action: Option<ToastAction>, t: &Theme) -> El {
    toast_keyed("toast", ic, tone, message, action, t)
}

/// [`toast`] under keys of its own (`<key>`, `<key>-icon`, `<key>-text`, `<key>-action`, `<key>-action-text`),
/// for a window that can show two at once.
pub fn toast_keyed(
    key: &str,
    ic: &'static Icon,
    tone: Tone,
    message: &str,
    action: Option<ToastAction>,
    t: &Theme,
) -> El {
    let color = match tone {
        Tone::Neutral => t.text_secondary,
        Tone::Done => t.green,
        Tone::Failed => t.red,
    };
    let role = if tone == Tone::Failed { "alert" } else { "status" };
    let button = action.map(|action| {
        let (label, run) = (action.text, action.run);
        let mut button = el().key(format!("{key}-action")).role("button").label(action.name);
        if let Some(keys) = action.shortcut {
            button = button.shortcut_hint(keys);
        }
        button
            .h(24.0)
            .px(8.0)
            .ml(3.25 - 10.0)
            .radius(12.0)
            .fill(t.selection)
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .center()
            .on_press_tx(move |s: &mut State, tx: &mut Tx| run(s, tx))
            .child(kit::label(format!("{key}-action-text"), label, ACTION, t.text))
    });
    build(key, message, Some((ic, color)), role, button, t)
}

fn build(
    key: &str,
    message: &str,
    lead: Option<(&'static Icon, Color)>,
    role: &'static str,
    action: Option<El>,
    t: &Theme,
) -> El {
    // `.glass`: the board's paper at 72 % (68 % in dark, where its paper is the theme's paper-2) over a blur.
    let glass = if t.dark {
        t.paper2.alpha(0.68)
    } else {
        t.paper.alpha(0.72)
    };
    let mut toast = row()
        .key(key.to_string())
        .role(role)
        .label(message)
        .live_region(LiveRegion::Polite)
        .h(34.0)
        .pl(14.0)
        .pr(8.0)
        .gap(10.0)
        .center_y()
        .radius(17.0)
        .backdrop_blur(22.0)
        .fill(glass)
        .border(kit::HAIR, t.separator)
        .border_layout()
        .box_shadow(kit::drop(6.0, 22.0, Color::BLACK.alpha(0.10)))
        .box_shadow(Shadow {
            x: 0.0,
            y: kit::HAIR,
            blur: 0.0,
            spread: 0.0,
            color: t.highlight,
            inset: true,
        });
    if let Some((ic, color)) = lead {
        toast = toast.child(kit::glyph(format!("{key}-icon"), ic, 15.0, color));
    }
    toast = toast.child(kit::label(format!("{key}-text"), message, TEXT, t.text));
    if let Some(action) = action {
        toast = toast.child(action);
    }
    toast
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::theme;
    use rune::{Cx, Env, Runtime};

    struct Page(Lead, bool);

    fn view(p: &Page, _: &mut Cx<Page>) -> El {
        let t = theme(false);
        let action = p.1.then(|| el().key("undo").role("button").label("Undo"));
        layer(toast_with("toast", "Moved to Trash", p.0, action, &t))
    }

    fn render(p: Page) -> Runtime<Page> {
        let env = Env { system_dark: false, scale: 2.0 };
        let mut rt = Runtime::new(p, view, (400.0, 200.0), env);
        rt.frame(0.0);
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn has(rt: &Runtime<Page>, key: &str) -> bool {
        rt.elements().into_iter().any(|e| e.key.as_deref() == Some(key))
    }

    #[test]
    fn keys_lead_icon_and_action() {
        let rt = render(Page(Lead::None, false));
        assert!(has(&rt, "toast") && has(&rt, "toast-text"));
        assert!(!has(&rt, "toast-icon") && !has(&rt, "undo"));
        for lead in [Lead::Check, Lead::Lock, Lead::Warn] {
            let rt = render(Page(lead, true));
            assert!(has(&rt, "toast-icon") && has(&rt, "undo"), "{lead:?}");
        }
    }

    #[test]
    fn is_a_polite_live_region() {
        let rt = render(Page(Lead::Check, false));
        let json = rt.accessibility_json().to_string().to_lowercase();
        assert!(json.contains("moved to trash") && json.contains("polite"), "{json}");
    }

    #[test]
    fn sits_inset_above_the_lower_edge() {
        let rt = render(Page(Lead::Check, false));
        let r = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("toast")).unwrap().rect;
        assert!((200.0 - (r[1] + r[3]) - INSET).abs() < 1.0, "{r:?}");
    }
}
