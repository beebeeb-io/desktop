//! The honest exits, drawn: "Trouble signing in?" (DW-A45), "We cannot recover your account"
//! (DW-A46) and "We can't restore your keys" (DW-A25). No screen here says "reset": Beebeeb
//! cannot reset a password, and the copy never hints it can.

use rune::{El, col, el, row};

use super::view::actions;
use super::{Intent, Ui, act, icons};
use crate::theme::tokens::{Theme, type_};
use crate::ui::au::{self, ButtonKind, ButtonSize, ButtonState, Keys, Tone, type_au};
use crate::ui::{au_entry, kit};

fn back_to(label: &str, to: fn(&mut Ui), t: &Theme) -> El {
    au::back_link(label, &icons::BACK, t).on_press_tx(act(to))
}

fn outline(k: &mut Keys, label: &str, to: fn(&mut Ui), t: &Theme) -> El {
    au::button(
        k,
        label,
        None,
        ButtonKind::Outline,
        ButtonSize::Lg,
        ButtonState::Enabled,
        t,
    )
    .on_press_tx(act(to))
}

// ---------------------------------------------------------------------------
// DW-A45
// ---------------------------------------------------------------------------

/// One option card: an icon and a title, a sentence, and the button that takes it. The button
/// is the card's one control; its description is the card's title.
#[allow(clippy::too_many_arguments)]
fn option(
    k: &mut Keys,
    icon: &'static rune::Icon,
    title: &str,
    words: &str,
    label: &str,
    kind: ButtonKind,
    to: fn(&mut Ui),
    t: &Theme,
) -> El {
    let (key, card) = au::card(k, (14.0, 12.0), 8.0, t);
    card.role("group")
        .label(title)
        .child(
            row()
                .gap(10.0)
                .center_y()
                .child(kit::glyph(format!("{key}-icon"), icon, 16.0, t.text))
                .child(kit::label(format!("{key}-title"), title, type_::HEAD, t.text)),
        )
        .child(
            kit::label(format!("{key}-text"), words, type_au::SUB, t.text_tertiary)
                .wrap()
                .w_full(),
        )
        .child(
            au::button(k, label, None, kind, ButtonSize::Md, ButtonState::Enabled, t)
                .description(title)
                .on_press_tx(act(to)),
        )
}

pub fn forgot(k: &mut Keys, t: &Theme) -> El {
    au::column()
        .child(back_to("Sign in", Ui::open_sign_in, t))
        .child(au::heading(
            "Trouble signing in?",
            Some("Choose the option that applies to you."),
            false,
            t,
        ))
        .child(option(
            k,
            &icons::KEY,
            "I have my recovery phrase",
            "The 12 words from account setup. Use them to set a new password.",
            "Recover with phrase",
            ButtonKind::Primary,
            Ui::recover_with_phrase,
            t,
        ))
        .child(option(
            k,
            &icons::PHONE,
            "I have another device signed in",
            "Approve this PC from a device where you're already signed in. No phrase needed.",
            "Sign in with another device",
            ButtonKind::Outline,
            Ui::ask_other_device,
            t,
        ))
        .child(option(
            k,
            &icons::CROSS,
            "I have neither",
            "We'll explain what that means for your data.",
            "Show my options",
            ButtonKind::Outline,
            |ui| ui.cannot_recover(true),
            t,
        ))
}

// ---------------------------------------------------------------------------
// DW-A46
// ---------------------------------------------------------------------------

fn paragraph(k: &mut Keys, words: &str, t: &Theme) -> El {
    kit::label(k.next("note"), words, type_au::SUB, t.text_secondary)
        .wrap()
        .w_full()
}

pub fn cannot_recover(k: &mut Keys, t: &Theme) -> El {
    let banner = au::banner(
        k,
        Tone::Neutral,
        &icons::SHIELD,
        Some("Beebeeb is zero-knowledge by design"),
        Some(
            "We never see your password or your encryption keys. Without your recovery phrase or a signed-in device, we have no way to restore access to your data.",
        ),
        false,
        t,
    );
    let why = paragraph(
        k,
        "Your files are encrypted with keys only you hold. Even with a court order we could not hand over your data in readable form.",
        t,
    );
    let head = kit::label(k.next("note"), "What we can still do", type_::HEAD, t.text)
        .role("heading")
        .level(2)
        .w_full();
    let what = paragraph(
        k,
        "We can't decrypt your files or reset your vault. We can help you close the account, confirm what was stored, and answer billing questions.",
        t,
    );
    // The support address opens a message to it in the person's mail app.
    let address = row()
        .h(15.0)
        .gap(6.0)
        .center_y()
        .child(kit::glyph("mail-icon", &icons::MAIL, 14.0, t.text_tertiary))
        // 12px Inter on a 15px line, as `au::link` draws one; the at sign dips below the line.
        .child(
            au_entry::link_in(k, "support@beebeeb.io", type_au::SUB, 13.875, 0.875, t)
                .on_press_tx(act(|ui| ui.exit(Intent::WriteToSupport))),
        );
    let warning = kit::label(
        k.next("note"),
        "Include the account email. Do not send us your recovery phrase: it is useless to us and dangerous to send.",
        type_::CAPTION,
        t.text_tertiary,
    )
    .wrap()
    .w_full();
    au::column()
        .child(back_to("Recovery options", |ui| ui.cannot_recover(false), t))
        .child(au::heading("We cannot recover your account", None, false, t))
        .child(banner)
        .child(why)
        .child(
            col()
                .key("au-stack")
                .w_full()
                .gap(8.0)
                .child(head)
                .child(what)
                .child(address)
                .child(warning),
        )
        .child(actions(6.0).child(outline(k, "Back to recovery options", |ui| ui.cannot_recover(false), t)))
}

// ---------------------------------------------------------------------------
// DW-A25
// ---------------------------------------------------------------------------

/// One exit of DW-A25: a row that is a button. Drive's own dialog for it (each with its own
/// confirmation) is the shell's to open.
fn exit_row(
    k: &mut Keys,
    icon: &'static rune::Icon,
    title: &str,
    sub: &str,
    danger: bool,
    to: fn(&mut Ui),
    t: &Theme,
) -> El {
    // `au::item` sets a second line in the small caption style; this list's is `.kit-item-sub`
    // at 12px on 15px, so the row is laid out here, with the same keys.
    let key = k.next("item");
    let mut r = row()
        .key(key.clone())
        .w_full()
        .h(40.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .child(
            el().w(30.0)
                .center()
                .child(kit::glyph(format!("{key}-icon"), icon, 18.0, t.text_secondary)),
        )
        .child(
            col()
                .gap(1.0)
                .grow(1.0)
                .min_w(0.0)
                .child(kit::label(
                    format!("{key}-title"),
                    title,
                    type_::ITEM_TITLE,
                    if danger { au::red_text(t) } else { t.text },
                ))
                .child(kit::label(format!("{key}-sub"), sub, type_au::SUB, t.text_tertiary)),
        )
        .child(kit::glyph(
            format!("{key}-chev"),
            &icons::CHEVRON,
            14.0,
            t.text_quaternary,
        ));
    if !key.ends_with("-0") {
        r = r.child(
            el().key(format!("{key}-rule"))
                .absolute(48.0, 0.0)
                .w(296.0)
                .h(kit::HAIR)
                .fill(t.separator),
        );
    }
    let r = r
        .role("button")
        .label(title)
        .focusable(true)
        .focus_ring(2.0, -2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .hover(t.hover)
        .press(t.press)
        .on_press_tx(act(to));
    if danger {
        // "Destructive rows are marked destructive to assistive tech."
        r.description(format!("{sub} Destructive."))
    } else {
        r.description(sub)
    }
}

pub fn no_phrase(k: &mut Keys, t: &Theme) -> El {
    let rows = vec![
        exit_row(
            k,
            &icons::CLOCK,
            "Cancel billing",
            "Files stay, still locked.",
            false,
            |ui| ui.exit(Intent::CancelBilling),
            t,
        ),
        exit_row(
            k,
            &icons::TRASH,
            "Erase all files",
            "Takes 72 hours; cancel any time.",
            true,
            |ui| ui.exit(Intent::EraseAllFiles),
            t,
        ),
        exit_row(
            k,
            &icons::CROSS,
            "Delete the account",
            "Removes the account and its files.",
            true,
            |ui| ui.exit(Intent::DeleteAccount),
            t,
        ),
    ];
    au::column()
        .child(back_to("Set up this device", |ui| ui.lost_phrase(false), t))
        .child(au::heading(
            "We can't restore your keys",
            Some(
                "Beebeeb never holds your keys, so without the phrase nothing here can be opened. This is what we can do.",
            ),
            false,
            t,
        ))
        .child(au::list(k, rows, t))
        .child(au::banner(
            k,
            Tone::Neutral,
            &icons::MAIL,
            Some("Questions about billing or closing the account"),
            Some("Write to support@beebeeb.io with the account email. Never send your phrase."),
            false,
            t,
        ))
        .child(actions(6.0).child(outline(k, "I found my phrase", |ui| ui.lost_phrase(false), t)))
}
