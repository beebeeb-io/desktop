//! What lies in front of Photos: the two menus (`.wn-menu`: DW-0603, DW-0606), the three
//! dialogs on their scrim (`.wn-dlg`: DW-0604, DW-0607, DW-0609) and the message (`.kit-toast`:
//! DW-0608, DW-0610).

use rune::{Color, El, Icon, Tx, col, el, row, text_input};

use super::route_items::rename::Draft;
use super::chrome::{self, Tone, style};
use super::grid::megabytes;
use super::source::{DeviceKind, Shape, SourceRef};
use super::{ACROSS, Dialog, Intent, Menu, OpenMenu, ShareDraft, SourcesDraft, Toast, Ui, icons};
use crate::State;
use crate::frames::files::actions_data::{LinkAccess, LinkExpiry};
use crate::theme::tokens::{Theme, type_};
use crate::ui::dialog as ui_dialog;
use crate::ui::menu::{self as ui_menu, Entry};
use crate::ui::switch::switch;
use crate::ui::toast::{self, Lead, toast_with};
use crate::ui::{field, kit};

/// The caption bar's height: menus and dialogs sit in the window under it.
const CAPTION: f32 = 32.0;

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

fn entries(ui: &Ui, which: Menu) -> Vec<Entry> {
    match which {
        Menu::Across => {
            let mut out = vec![Entry::Head("Photos across")];
            // The two shortcuts sit on the choices they lead to from here.
            let (larger, smaller) = (ui.across_after(true), ui.across_after(false));
            for n in ACROSS {
                let label = match n {
                    3 => "3, larger",
                    4 => "4",
                    5 => "5",
                    7 => "7, smallest",
                    _ => "",
                };
                let checked = n == ui.across;
                out.push(if n == smaller {
                    Entry::Item {
                        id: label,
                        label,
                        shortcut: Some("Ctrl\u{2212}"),
                        checked,
                        danger: false,
                        off: false,
                    }
                } else if n == larger {
                    Entry::Item {
                        id: label,
                        label,
                        shortcut: Some("Ctrl+Plus"),
                        checked,
                        danger: false,
                        off: false,
                    }
                } else {
                    Entry::check(label, label, checked)
                });
            }
            out
        }
        Menu::More => vec![
            Entry::item("save", "Save to PC\u{2026}"),
            Entry::item("move", "Move\u{2026}"),
            Entry::keyed("rename", "Rename", "F2"),
            Entry::item("duplicate", "Duplicate"),
        ],
    }
}

pub fn menu(ui: &Ui, open: OpenMenu, t: &Theme) -> El {
    // Where the board puts each: from the window's right edge, under the toolbar's buttons.
    let (name, top, right) = match open.which {
        Menu::Across => ("Photos across", CAPTION + 80.0, 58.0),
        Menu::More => ("More", CAPTION + 44.0, 398.0),
    };
    ui_menu::menu(
        "ph-menu",
        name,
        180.0,
        &entries(ui, open.which),
        open.hot,
        t,
        |s: &mut State, i, _| {
            if let Some(open) = s.photos.menu {
                s.photos.pick(open.which, i);
            }
        },
        |s: &mut State, i| {
            if let Some(open) = &mut s.photos.menu {
                open.hot = Some(i);
            }
        },
        |s: &mut State, _| s.photos.menu = None,
    )
    .pin(None, Some(top), Some(right), None)
}

// ---------------------------------------------------------------------------
// The message
// ---------------------------------------------------------------------------

/// A message over the content, `bottom` points above its lower edge.
pub fn toast_layer(toast: &Toast, bottom: f32, _over_photos: bool, t: &Theme) -> El {
    let left_words;
    let (lead, words): (Lead, &str) = match toast {
        Toast::Kept => (Lead::Lock, "Keeping the original on this PC"),
        Toast::Trashed(_) => (Lead::Check, "Moved to Trash"),
        Toast::NotYet => (Lead::None, "Not available in this version yet"),
        Toast::Words(words) => (Lead::None, words),
        Toast::Said(words) => (Lead::None, words.as_str()),
        Toast::Left { what, why, count, .. } => {
            left_words = Toast::left_words(*what, *why, *count);
            (Lead::Warn, left_words.as_str())
        }
    };
    let action = if matches!(toast, Toast::Trashed(_)) {
        Some(toast_action("Undo", Some("Ctrl+Z"), |s: &mut State| {
            s.photos.undo();
        }, t))
    } else if toast.can_retry() {
        Some(toast_action("Try again", None, |s: &mut State| {
            s.photos.try_again();
        }, t))
    } else {
        None
    };
    row()
        .key("ph-toast-layer")
        .pin(Some(0.0), None, Some(0.0), Some(bottom))
        .center()
        .child(toast_with("ph-toast", words, lead, action, t))
}

/// The toast's button: its words, and the shortcut where there is one.
fn toast_action(name: &'static str, shortcut: Option<&'static str>, press: fn(&mut State), t: &Theme) -> El {
    let words = match shortcut {
        Some(keys) => format!("{name} {keys}"),
        None => name.to_string(),
    };
    let mut button = row()
        .key("ph-toast-action")
        .role("button")
        .label(name)
        .center_y()
        .focusable(true)
        .on_press(press)
        .child(kit::label("ph-toast-action-label", words, toast::ACTION, t.text));
    if let Some(keys) = shortcut {
        button = button.shortcut_hint(keys);
    }
    button
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

pub fn scrim(t: &Theme) -> El {
    el().key("ph-scrim")
        .pin(Some(0.0), Some(CAPTION), Some(0.0), Some(0.0))
        .fill(crate::shell::registry::scrim(t.dark))
}

/// A pop-up button (`.mk-pop2`): its value and a caret.
fn pop(key: &str, name: &str, value: &str, t: &Theme) -> El {
    row()
        .key(format!("{key}-pop"))
        .role("combobox")
        .label(name)
        .a11y_value(value)
        .h(24.0)
        .pl(9.0)
        .pr(6.0)
        .gap(4.0)
        .center_y()
        .radius(6.0)
        .fill(t.card)
        .box_shadow(kit::drop(0.5, 1.5, Color::BLACK.alpha(0.18)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .focusable(true)
        .child(kit::label(format!("{key}-pop-label"), value, style::POP, t.text))
        .child(kit::glyph(
            format!("{key}-pop-caret"),
            &icons::CHEVRON_DOWN,
            12.0,
            t.text_tertiary,
        ))
}

/// One row of a list (`.kit-item`): icon, title and sub line, and what sits at its end.
fn item(n: usize, m: usize, ic: Option<&'static Icon>, title: &str, sub: Option<&str>, end: Vec<El>, t: &Theme) -> El {
    let key = format!("ph-item-{n}-{m}");
    let mut words = col()
        .key(format!("{key}-text"))
        .grow(1.0)
        .min_w(0.0)
        .gap(1.0)
        .child(kit::label(format!("{key}-title"), title, style::ITEM_TITLE, t.text).truncate());
    if let Some(sub) = sub {
        words = words.child(kit::label(format!("{key}-sub"), sub, style::ITEM_SUB, t.text_tertiary).truncate());
    }
    let mut item = row()
        .key(key.clone())
        .w_full()
        .h(if sub.is_some() { 40.0 } else { 28.0 })
        .px(10.0)
        .gap(8.0)
        .center_y();
    if m > 0 {
        item = item.child(
            el().key(format!("{key}-line"))
                .pin(Some(10.0), Some(0.0), Some(0.0), None)
                .h(kit::HAIR)
                .fill(t.separator),
        );
    }
    if let Some(ic) = ic {
        item = item.child(el().key(format!("{key}-icon-box")).w(30.0).center().child(kit::glyph(
            format!("{key}-icon"),
            ic,
            18.0,
            t.text_secondary,
        )));
    }
    item.child(words).children(end)
}

/// A titled list (`.kit-list`).
fn list(n: usize, title: Option<&str>, items: Vec<El>, t: &Theme) -> El {
    let mut out = col().key(format!("ph-list-{n}-block")).w_full().gap(6.0);
    if let Some(title) = title {
        out = out.child(
            kit::label(format!("ph-list-{n}-title"), title, style::LIST_TITLE, t.text_tertiary)
                .role("heading")
                // `.kit-list-title { padding: 0 var(--pad) }`
                .px(14.0)
                .w_full(),
        );
    }
    out.child(
        col()
            .key(format!("ph-list-{n}"))
            .role("list")
            .w_full()
            .radius(10.0)
            .fill(t.card)
            .box_shadow(kit::ring(kit::HAIR, t.separator))
            .children(items),
    )
}

fn note(words: &str, t: &Theme) -> El {
    kit::label("ph-dlg-note", words, style::CAP, t.text_tertiary)
        .w_full()
        .wrap()
}

fn sources(ui: &Ui, draft: &SourcesDraft, t: &Theme) -> (f32, Vec<El>, Foot) {
    let devices = ui.source.devices();
    // The dialog lists the tablets between the phones as the board does: by name is not its
    // order, so the board's is kept: the newest phone, the tablets, the older phones.
    let mut order: Vec<usize> = (0..devices.len()).collect();
    order.sort_by_key(|&i| match (i, devices[i].kind) {
        (0, _) => 0,
        (_, DeviceKind::Tablet) => 1,
        _ => 2,
    });
    let device_items = order
        .into_iter()
        .enumerate()
        .map(|(m, i)| {
            let d = &devices[i];
            let on = draft.devices.get(i).copied().unwrap_or(false);
            let ic = match d.kind {
                DeviceKind::Phone => &icons::PHONE,
                DeviceKind::Tablet => &icons::TABLET,
            };
            let sub = format!("{} photos", super::grid::thousands(d.photos));
            let key = format!("ph-item-0-{m}");
            item(
                0,
                m,
                Some(ic),
                &d.name,
                Some(&sub),
                vec![
                    switch(&format!("{key}-switch"), &d.name, on, true, t)
                        .on_press(move |s: &mut State| s.photos.toggle_source(SourceRef::Device(i))),
                ],
                t,
            )
        })
        .collect();
    let folders = ui.source.folders();
    let mut folder_items: Vec<El> = folders
        .iter()
        .enumerate()
        .map(|(m, f)| {
            let on = draft.folders.get(m).copied().unwrap_or(false);
            let key = format!("ph-item-1-{m}");
            item(
                1,
                m,
                Some(&icons::FOLDER),
                &f.name,
                Some(&f.detail),
                vec![
                    // Takes the folder out of Photo sources (nothing is moved or deleted); the switch only hides it.
                    // Shown on hover and on keyboard focus only (DW-0604); at rest it is invisible but Tab reaches it.
                    kit::label(format!("ph-item-1-{m}-remove"), "Remove", style::BULK, t.text_secondary)
                        .group_opacity(0.0, 1.0)
                        .spans(vec![rune::Span::new(0, "Remove".len()).underline(t.text_secondary)])
                        .role("button")
                        .label(format!("Remove {}", f.name))
                        .focusable(true)
                        .on_press(move |s: &mut State| s.photos.remove_folder(m)),
                    switch(&format!("{key}-switch"), &f.name, on, true, t)
                        .on_press(move |s: &mut State| s.photos.toggle_source(SourceRef::Folder(m))),
                ],
                t,
            )
            .hover_group_focus()
        })
        .collect();
    folder_items.push(
        item(
            1,
            folders.len(),
            Some(&icons::PLUS),
            "Add a folder\u{2026}",
            None,
            Vec::new(),
            t,
        )
        .role("button")
        .label("Add a folder\u{2026}")
        .focusable(true)
        .hover(t.hover)
        .on_press(|s: &mut State| s.photos.ask(Intent::AddFolder)),
    );
    let body = vec![
        list(0, Some("Backups from your devices"), device_items, t),
        list(1, Some("Folders from Files"), folder_items, t),
        note(
            "This only changes what Photos shows. Nothing is moved or deleted, and backups keep running.",
            t,
        ),
    ];
    let foot = vec![
        foot_button("ph-dlg-btn-0", "Done", None, Tone::Primary, t, |b| {
            b.default_button().on_press(|s: &mut State| s.photos.confirm_dialog())
        }),
        foot_button("ph-dlg-btn-1", "Cancel", None, Tone::Quiet, t, |b| {
            b.on_press(|s: &mut State| s.photos.close_dialog())
        }),
    ];
    (460.0, body, foot)
}

/// The key of the link password's text input.
pub const SHARE_PASSWORD: &str = "ph-share-password";

/// The expiry choices of the Share dialog (Files' four), under the Link expires button.
pub fn expiry_menu(ui: &Ui, t: &Theme) -> Option<El> {
    let Some(Dialog::Share(draft)) = &ui.dialog else { return None };
    if !draft.menu {
        return None;
    }
    let entries: Vec<Entry> = LinkExpiry::ALL
        .iter()
        .map(|choice| Entry::check(choice.choice(), choice.choice(), *choice == draft.expires))
        .collect();
    Some(
        ui_menu::menu(
            "ph-expiry-menu",
            "Link expires",
            130.0,
            &entries,
            None,
            t,
            |s: &mut State, i, _| {
                if let Some(Dialog::Share(draft)) = &mut s.photos.dialog {
                    draft.expires = LinkExpiry::ALL[i];
                    draft.menu = false;
                }
            },
            |_: &mut State, _| {},
            |s: &mut State, _| {
                if let Some(Dialog::Share(draft)) = &mut s.photos.dialog {
                    draft.menu = false;
                }
            },
        )
        .anchored("ph-item-0-1-pop", rune::Side::Bottom, rune::Align::End, 4.0),
    )
}

fn share(ui: &Ui, draft: &ShareDraft, t: &Theme) -> (f32, Vec<El>, Foot) {
    let viewing = ui.viewing();
    let (name, bytes, id) = viewing.as_ref().map_or((String::new(), 0, None), |v| {
        (v.name.clone(), v.original_bytes, Some(v.id))
    });
    let mut thumb = el().key("ph-dlg-thumb").size(36.0, 36.0).radius(9.0);
    thumb = match id.and_then(|id| ui.source.thumbnail(id, Shape::Square)) {
        Some(pixels) => thumb.pixels(&pixels),
        None => thumb.fill(t.tile),
    };
    let what = row().key("ph-dlg-what").gap(12.0).center_y().child(thumb).child(
        col()
            .key("ph-dlg-what-text")
            .items_start()
            .child(kit::label("ph-dlg-name", name, type_::HEAD, t.text))
            .child(kit::label(
                "ph-dlg-size",
                megabytes(bytes, true),
                style::SIZE_MONO,
                t.text_tertiary,
            )),
    );
    let expiry = draft.expires.words();
    let items = vec![
        item(
            0,
            0,
            Some(&icons::LINK),
            "Anyone with the link",
            None,
            vec![
                // A link is a download link (decision 11-17): no "Can edit", and nothing to choose here.
                pop("ph-item-0-0", "Anyone with the link", LinkAccess::Download.words(), t),
            ],
            t,
        ),
        item(
            0,
            1,
            Some(&icons::CLOCK),
            "Link expires",
            None,
            vec![
                pop("ph-item-0-1", "Link expires", expiry, t).on_press(|s: &mut State| {
                    if let Some(id) = s.photos.viewer.as_ref().map(|v| v.id) {
                        s.photos.ask(Intent::ChooseExpiry(id));
                    }
                }),
            ],
            t,
        ),
        item(
            0,
            2,
            Some(&icons::LOCK),
            "Password",
            None,
            vec![
                switch("ph-item-0-2-switch", "Password", draft.password, true, t).on_press_tx(|s: &mut State, tx: &mut Tx| {
                    s.photos.toggle_share_password();
                    if matches!(&s.photos.dialog, Some(Dialog::Share(d)) if d.password) {
                        tx.focus(SHARE_PASSWORD, false);
                    }
                }),
            ],
            t,
        ),
    ];
    let mut body = vec![what, list(0, None, items, t)];
    if draft.password {
        let message = if draft.password_missing {
            "Type a password, or turn Password off."
        } else {
            "Whoever opens the link needs this password. Send it another way than the link."
        };
        body.push(field::bare_field(
            field::Bare {
                key: "ph-share-field",
                name: "Password for the link",
                message: Some(message),
                error: draft.password_missing,
            },
            &ui.share_password,
            SHARE_PASSWORD,
            |s: &mut State| &mut s.photos.share_password,
            |input| input.autofocus().on_submit(|s: &mut State, _: &mut Tx| s.photos.confirm_dialog()),
            t,
        ));
    }
    body.push(
        note(
            "Copy link keeps it encrypted. Send a copy decrypts it, so it leaves Beebeeb's protection.",
            t,
        ),
    );
    let foot = vec![
        foot_button("ph-dlg-btn-0", "Send a copy\u{2026}", None, Tone::Plain, t, |b| {
            b.on_press(|s: &mut State| {
                if let Some(id) = s.photos.viewer.as_ref().map(|v| v.id) {
                    s.photos.ask(Intent::SendCopy(id));
                    s.photos.close_dialog();
                }
            })
        }),
        (el(), None),
        foot_button("ph-dlg-btn-1", "Copy link", Some(&icons::LINK), Tone::Primary, t, |b| {
            b.default_button().on_press(|s: &mut State| s.photos.confirm_dialog())
        }),
        foot_button("ph-dlg-btn-2", "Cancel", None, Tone::Quiet, t, |b| {
            b.on_press(|s: &mut State| s.photos.close_dialog())
        }),
    ];
    (440.0, body, foot)
}

fn details(ui: &Ui, t: &Theme) -> (f32, Vec<El>, Foot) {
    let id = ui.viewing().map(|v| v.id);
    let lines = id.map(|id| ui.source.details(id)).unwrap_or_default();
    let items = lines
        .iter()
        .enumerate()
        .map(|(m, line)| {
            let key = format!("ph-item-0-{m}");
            let mut end = vec![kit::label(
                format!("{key}-value"),
                line.value.clone(),
                if line.mono {
                    style::ITEM_VALUE_MONO
                } else {
                    style::ITEM_VALUE
                },
                t.text_tertiary,
            )];
            if line.opens {
                end.push(kit::glyph(
                    format!("{key}-chev"),
                    &icons::CHEVRON_ROW,
                    14.0,
                    t.text_quaternary,
                ));
            }
            let mut row = item(0, m, None, line.label, None, end, t);
            if line.opens {
                row = row
                    .role("button")
                    .label(format!("{}, {}", line.label, line.value))
                    .focusable(true)
                    .hover(t.hover)
                    .on_press(move |s: &mut State| {
                        if let Some(id) = id {
                            s.photos.ask(Intent::VersionHistory(id));
                        }
                    });
            }
            row
        })
        .collect();
    let foot = vec![foot_button("ph-dlg-btn-0", "Done", None, Tone::Plain, t, |b| {
        b.default_button().on_press(|s: &mut State| s.photos.close_dialog())
    })];
    (420.0, vec![list(0, None, items, t)], foot)
}

/// The name dialog of Rename: Files' field, its words under it when the name cannot be given.
fn rename(draft: &Draft, t: &Theme) -> (f32, Vec<El>, Foot) {
    let edit = &draft.edit.0;
    let error = draft.error_now();
    let mut input = text_input(edit, field::VALUE)
        .key("ph-rename-input")
        .label("Name")
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, field::selection(t))
        .selection_style(rune::SelectionStyle::new(field::selection(t)))
        .invalid(error.is_some())
        .on_edit(rename_edit)
        .on_submit(|s: &mut State, tx: &mut Tx| rename_commit(s, tx))
        .grow(1.0)
        .min_w(0.0);
    if let Some(why) = error {
        input = input.description(why.message());
    }
    let ring = if error.is_some() {
        kit::ring(2.0, t.red)
    } else {
        kit::ring(1.0, t.separator_strong)
    };
    let well = row()
        .key("ph-rename-field")
        .w_full()
        .h(30.0)
        .px(8.0)
        .center_y()
        .radius(6.0)
        .fill(t.card)
        .box_shadow(ring)
        .child(input);
    let mut body = vec![well];
    if let Some(why) = error {
        body.push(kit::label("ph-rename-error", why.message(), style::CAP, t.red).w_full().wrap());
    }
    let foot = vec![
        foot_button("ph-dlg-btn-0", "Rename", None, Tone::Primary, t, |b| {
            b.default_button()
                .on_press_tx(|s: &mut State, tx: &mut Tx| rename_commit(s, tx))
        }),
        foot_button("ph-dlg-btn-1", "Cancel", None, Tone::Quiet, t, |b| {
            b.on_press(|s: &mut State| s.photos.close_dialog())
        }),
    ];
    (400.0, body, foot)
}

fn rename_edit(s: &mut State) -> &mut rune::TextEdit {
    match &mut s.photos.dialog {
        Some(Dialog::Rename(draft)) => &mut draft.edit.0,
        _ => &mut s.actions.name_edit,
    }
}

fn rename_commit(s: &mut State, tx: &mut Tx) {
    super::route_items::rename::commit(s, tx);
}

/// A dialog's foot, left to right: its buttons, each with its width on the board, and `None`
/// for the space that pushes what follows to the right.
type Foot = Vec<(El, Option<f32>)>;

/// A button of a dialog's foot (28 pt) and its width on the board; `with` gives it what it does.
fn foot_button(
    key: &str,
    label: &str,
    ic: Option<&'static Icon>,
    tone: Tone,
    t: &Theme,
    with: impl FnOnce(El) -> El,
) -> (El, Option<f32>) {
    let w = chrome::button_width(label, ic.is_some(), tone, 28.0, style::BUTTON);
    (
        with(chrome::button(key, label, ic, tone, 28.0, style::BUTTON, t)),
        Some(w),
    )
}

/// A dialog (`.wn-dlg`), centred in the window under the caption. `scale`: device pixels a point.
pub fn dialog(ui: &Ui, dialog: &Dialog, _scale: f32, t: &Theme) -> El {
    let (title, (width, body, foot)) = match dialog {
        Dialog::Sources(draft) => ("Photo sources", sources(ui, draft, t)),
        Dialog::Share(draft) => ("Share", share(ui, draft, t)),
        Dialog::Details => ("Details", details(ui, t)),
        Dialog::Rename(draft) => ("Rename", rename(draft, t)),
    };
    // The foot's buttons at their natural widths; `None` is the space that pushes what follows right.
    let foot: Vec<El> = foot
        .into_iter()
        .map(|(button, w)| if w.is_none() { el().grow(1.0) } else { button })
        .collect();
    let card = ui_dialog::dialog(
        ui_dialog::Dialog {
            head: ui_dialog::Head::Title { title, sub: None },
            body,
            foot: ui_dialog::Foot::Row(foot),
            ..ui_dialog::Dialog::new("ph-dlg", title, width)
        },
        t,
    );
    col()
        .key("ph-dlg-layer")
        .pin(Some(0.0), Some(CAPTION), Some(0.0), Some(0.0))
        .center()
        // The window behind is inert while the dialog is open.
        .modal(false, |s: &mut State| s.photos.close_dialog())
        .child(card)
}
