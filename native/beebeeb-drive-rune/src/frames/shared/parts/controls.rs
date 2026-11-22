//! Buttons (`.kit-btn`), worded segments (`.kit-seg`) and pills (`.kit-pill`).

use super::style;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use rune::{El, Icon, el, row};

/// A key's words part: lower case letters and digits, one `-` for every run of anything else
/// ("Invite someone…" is `invite-someone`). The one copy the group's controls and dialogs key by.
pub(crate) fn slug(words: &str) -> String {
    let mut out = String::new();
    for c in words.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Paper with a drawn line.
    Outline,
    /// Words only, in red: a destructive action that still asks.
    GhostDanger,
    /// Words only, unavailable for now: drawn at 40 %, not a Tab stop.
    GhostOff,
    /// A dialog's main action (`variant="amber"`): ink on Amber.
    Amber,
    /// Words only, in the second ink (`variant="ghost"`): a dialog's way out.
    Ghost,
    /// White on red (`variant="danger"`): the destructive answer of a confirmation.
    Danger,
    /// `Danger` that cannot be pressed yet (`.opacity-50`), not a Tab stop.
    DangerOff,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    /// `.kit-btn-sm`: 22 pt, in a toolbar or a card.
    Small,
    /// `.kit-btn-md`: 28 pt, an empty state's or a dialog's.
    Medium,
}

/// A small button (`.kit-btn.kit-btn-sm`, 22 pt). Keys: `btn-<words>`, `-icon`, `-label`.
pub fn button(words: &str, kind: ButtonKind, ic: Option<&'static Icon>, t: &Theme) -> El {
    button_keyed(format!("btn-{}", slug(words)), words, kind, ButtonSize::Small, ic, t)
}

/// A dialog's button (`.kit-btn.kit-btn-md`, 28 pt). Keys: `btn-<words>`, `-icon`, `-label`.
pub fn button_md(words: &str, kind: ButtonKind, ic: Option<&'static Icon>, t: &Theme) -> El {
    button_keyed(format!("btn-{}", slug(words)), words, kind, ButtonSize::Medium, ic, t)
}

/// A button under the key `k` (the second button with the same words in a frame is `btn-<words>-2`).
pub fn button_keyed(
    k: String,
    words: &str,
    kind: ButtonKind,
    size: ButtonSize,
    ic: Option<&'static Icon>,
    t: &Theme,
) -> El {
    use crate::ui::button::{ButtonState, Size, Variant, button as ui_button, keys};
    let (variant, state) = match kind {
        ButtonKind::Outline => (Variant::Secondary, ButtonState::Enabled),
        ButtonKind::Ghost => (Variant::Ghost, ButtonState::Enabled),
        ButtonKind::GhostDanger | ButtonKind::Danger => (Variant::Danger, ButtonState::Enabled),
        ButtonKind::GhostOff => (Variant::Ghost, ButtonState::Disabled),
        ButtonKind::Amber => (Variant::Primary, ButtonState::Enabled),
        ButtonKind::DangerOff => (Variant::Danger, ButtonState::Disabled),
    };
    let size = match size {
        ButtonSize::Small => Size::Sm,
        ButtonSize::Medium => Size::Md,
    };
    ui_button(keys(&k), words, ic, variant, size, state, t)
}

/// The key of the tab worded `words` in the switch `n`.
pub fn segment_key(n: usize, words: &str) -> String {
    format!("seg-{n}-{}", slug(words))
}

/// A worded segmented control (`.kit-seg.kit-seg-sm`): `tabs` are (words, chosen). Keys:
/// `seg-<n>`, each tab `seg-<n>-<words>` and `-label`. The caller adds each tab's `on_press`
/// through `each`.
pub fn segments(n: usize, name: &str, tabs: &[(&'static str, bool)], t: &Theme, each: impl Fn(usize, El) -> El) -> El {
    let mut well = row()
        .key(format!("seg-{n}"))
        .role("tablist")
        .label(name)
        .focus_group()
        .padding(crate::theme::scale::space::S2)
        .radius(crate::theme::tokens::radius::LG)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator));
    for (i, (words, on)) in tabs.iter().enumerate() {
        let k = segment_key(n, words);
        let mut tab = el()
            .key(k.clone())
            .role("tab")
            .label(*words)
            .checked(*on)
            .h(24.0)
            // The board's 10 pt, less the quarter point Rune adds to a text on average: it reports a
            // text's width rounded up to the next half point ("With me" 48.0 for 47.59), and three
            // tabs in a row would put the control's left edge 1.2 pt off.
            .px(9.75)
            .center()
            .radius(8.0)
            .focusable(true)
            // One Tab stop for the switch: the chosen tab. Left and Right move between tabs.
            .tab_index(if *on { 0 } else { -1 })
            .child(kit::label(
                format!("{k}-label"),
                *words,
                style::SEG,
                if *on { t.text } else { t.text_secondary },
            ));
        if *on {
            tab = tab
                .fill(t.card)
                .box_shadow(kit::drop(1.0, 2.0, t.shadow_raised))
                .box_shadow(kit::ring(kit::HAIR, t.separator));
        } else {
            tab = tab.hover(t.hover).press(t.press);
        }
        well = well.child(each(i, tab));
    }
    well
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Green,
}

/// A pill: `ui::pill`. Keys: `<key>-pill`, `-pill-dot`, `-pill-label`.
pub fn pill(key: &str, words: &str, tone: Tone, dot: bool, t: &Theme) -> El {
    let tone = match tone {
        Tone::Neutral => crate::ui::pill::Tone::Neutral,
        Tone::Green => crate::ui::pill::Tone::Good,
    };
    crate::ui::pill::pill(&format!("{key}-pill"), words, tone, dot, t)
}
