//! What lies in front of Photos: the two menus (`.wn-menu`: DW-0603, DW-0606), the three
//! dialogs on their scrim (`.wn-dlg`: DW-0604, DW-0607, DW-0609) and the message (`.kit-toast`:
//! DW-0608, DW-0610).

use rune::{Color, El, Icon, col, el, row};

use super::chrome::{self, Tone, style};
use super::grid::megabytes;
use super::source::{DeviceKind, Shape, SourceRef};
use super::{ACROSS, Dialog, Intent, Menu, OpenMenu, ShareDraft, SourcesDraft, Toast, Ui, icons};
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;

/// The caption bar's height: menus and dialogs sit in the window under it.
const CAPTION: f32 = 32.0;

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

struct Row {
    label: String,
    keys: Option<&'static str>,
    head: bool,
    checked: bool,
}

fn rows(ui: &Ui, which: Menu) -> Vec<Row> {
    let row = |label: &str, keys| Row {
        label: label.into(),
        keys,
        head: false,
        checked: false,
    };
    match which {
        Menu::Across => {
            let mut out = vec![Row {
                head: true,
                ..row("Photos across", None)
            }];
            // The two shortcuts sit on the choices they lead to from here.
            let (larger, smaller) = (ui.across_after(true), ui.across_after(false));
            for n in ACROSS {
                let label = match n {
                    3 => "3, larger".to_string(),
                    7 => "7, smallest".to_string(),
                    n => n.to_string(),
                };
                out.push(Row {
                    checked: n == ui.across,
                    ..row(
                        &label,
                        if n == smaller {
                            Some("Ctrl\u{2212}")
                        } else if n == larger {
                            Some("Ctrl+Plus")
                        } else {
                            None
                        },
                    )
                });
            }
            out
        }
        Menu::More => vec![
            row("Save to PC\u{2026}", None),
            row("Move\u{2026}", None),
            row("Rename", Some("F2")),
            row("Duplicate", None),
        ],
    }
}

pub fn menu(ui: &Ui, open: OpenMenu, t: &Theme) -> El {
    // Where the board puts each: from the window's right edge, under the toolbar's buttons.
    let (name, top, right) = match open.which {
        Menu::Across => ("Photos across", CAPTION + 80.0, 58.0),
        Menu::More => ("More", CAPTION + 44.0, 398.0),
    };
    let which = open.which;
    let mut menu = col()
        .key("ph-menu")
        .role("menu")
        .label(name)
        .pin(None, Some(top), Some(right), None)
        .min_w(180.0)
        .padding(4.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(8.0, 28.0, Color::BLACK.alpha(0.2)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .modal(true, |s: &mut State| s.photos.menu = None);
    for (i, r) in rows(ui, which).into_iter().enumerate() {
        let hot = open.hot == Some(i);
        let key = format!("ph-menu-{i}");
        let mut item = row()
            .key(key.clone())
            .h(if r.head { 26.0 } else { 30.0 })
            .pl(8.0)
            .pr(12.0)
            .gap(10.0)
            .center_y()
            .radius(4.0);
        if r.head {
            item = item.child(kit::label(format!("{key}-label"), r.label, style::MENU_HEAD, t.text_tertiary).grow(1.0));
        } else {
            let mut check = el().key(format!("{key}-ck")).w(14.0).center();
            if r.checked {
                check = check.child(kit::glyph(format!("{key}-check"), &icons::CHECK, 12.0, t.text));
            }
            item = item
                .role(if which == Menu::Across {
                    "menuitemradio"
                } else {
                    "menuitem"
                })
                .label(r.label.clone())
                .focusable(true)
                .fill(if hot { t.press } else { Color::TRANSPARENT })
                .on_hover(move |s: &mut State| {
                    if let Some(open) = &mut s.photos.menu {
                        open.hot = Some(i);
                    }
                })
                .on_press(move |s: &mut State| s.photos.pick(which, i))
                .child(check)
                .child(kit::label(format!("{key}-label"), r.label, style::MENU, t.text).grow(1.0));
            if which == Menu::Across {
                item = item.checked(r.checked);
            }
            if let Some(keys) = r.keys {
                item = item.shortcut_hint(keys).child(
                    kit::label(
                        format!("{key}-sc"),
                        keys,
                        style::MENU_KEYS,
                        if hot { t.text_secondary } else { t.text_tertiary },
                    )
                    .pl(24.0),
                );
            }
        }
        menu = menu.child(item);
    }
    menu
}

// ---------------------------------------------------------------------------
// The message
// ---------------------------------------------------------------------------

/// A message over the content, `bottom` points above its lower edge.
pub fn toast_layer(toast: &Toast, bottom: f32, over_photos: bool, t: &Theme) -> El {
    let (ic, words): (&'static Icon, &str) = match toast {
        Toast::Kept => (&icons::OFFLINE, "Keeping the original on this PC"),
        Toast::Trashed(_) => (&icons::TRASH, "Moved to Trash"),
    };
    let glass = if t.dark {
        chrome::rgb(25, 21, 16).alpha(173.0 / 255.0)
    } else {
        chrome::rgb(251, 249, 247).alpha(184.0 / 255.0)
    };
    let mut pill = row()
        .key("ph-toast")
        .role("status")
        .h(38.0)
        .px(14.0)
        .gap(8.0)
        .center_y()
        .radius(19.0)
        .fill(glass)
        .border(kit::HAIR, t.separator)
        .box_shadow(kit::drop(6.0, 22.0, Color::BLACK.alpha(0.1)))
        // `.glass`: the half-point highlight along the top, inside.
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: kit::HAIR,
            blur: 0.0,
            spread: 0.0,
            color: t.highlight,
            inset: true,
        })
        .child(kit::glyph("ph-toast-icon", ic, 15.0, t.text_secondary))
        .child(kit::label("ph-toast-text", words, style::TOAST, t.text));
    // Rune's backdrop blur also takes in what lies beside the element (on the stage: the strip
    // just under the message, which lit its lower edge), where the board's takes only what is
    // behind it. Over the dark stage there is nothing behind to blur; over the grid there is.
    if over_photos {
        pill = pill.backdrop_blur(22.0);
    }
    if matches!(toast, Toast::Trashed(_)) {
        pill = pill.child(
            el().key("ph-toast-action")
                .role("button")
                .label("Undo")
                .shortcut_hint("Ctrl+Z")
                // `ml-2` and the space before it in the board's sentence.
                .ml(3.37)
                .h(15.5)
                .center()
                .focusable(true)
                .on_press(|s: &mut State| {
                    s.photos.undo();
                })
                .child(kit::label(
                    "ph-toast-action-label",
                    "Undo Ctrl+Z",
                    style::TOAST_ACTION,
                    t.text,
                )),
        );
    }
    row()
        .key("ph-toast-layer")
        .pin(Some(0.0), None, Some(0.0), Some(bottom))
        .center()
        .child(pill)
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

pub fn scrim(t: &Theme) -> El {
    el().key("ph-scrim")
        .pin(Some(0.0), Some(CAPTION), Some(0.0), Some(0.0))
        .fill(crate::shell::registry::scrim(t.dark))
}

/// A switch (36 x 20): ink when on, as the board draws it on the desktop.
fn switch(key: &str, name: &str, on: bool, t: &Theme) -> El {
    el().key(format!("{key}-switch"))
        .role("switch")
        .label(name)
        .checked(on)
        .size(36.0, 20.0)
        .radius(10.0)
        .fill(if on { t.text } else { t.paper3 })
        .border(1.0, if on { t.text } else { t.separator_strong })
        .focusable(true)
        .child(
            el().key(format!("{key}-knob"))
                .absolute(if on { 18.0 } else { 2.0 }, 2.0)
                .size(14.0, 14.0)
                .radius(7.0)
                .fill(if on { t.paper } else { Color::WHITE })
                .box_shadow(kit::drop(1.0, 2.0, t.shadow)),
        )
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

fn sources(ui: &Ui, draft: &SourcesDraft, t: &Theme) -> (f32, Vec<El>, Vec<El>) {
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
                    switch(&key, &d.name, on, t)
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
                    switch(&key, &f.name, on, t)
                        .on_press(move |s: &mut State| s.photos.toggle_source(SourceRef::Folder(m))),
                ],
                t,
            )
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
        chrome::button("ph-dlg-btn-0", "Done", None, Tone::Primary, 28.0, style::BUTTON, t)
            .default_button()
            .on_press(|s: &mut State| s.photos.confirm_dialog()),
        chrome::button("ph-dlg-btn-1", "Cancel", None, Tone::Quiet, 28.0, style::BUTTON, t)
            .on_press(|s: &mut State| s.photos.close_dialog()),
    ];
    (460.0, body, foot)
}

fn share(ui: &Ui, draft: &ShareDraft, t: &Theme) -> (f32, Vec<El>, Vec<El>) {
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
            .child(kit::label("ph-dlg-name", name, type_::HEAD, t.text))
            .child(kit::label(
                "ph-dlg-size",
                megabytes(bytes, true),
                style::SIZE_MONO,
                t.text_tertiary,
            )),
    );
    let expiry = match draft.days {
        Some(days) => format!("In {days} days"),
        None => "Never".into(),
    };
    let items = vec![
        item(
            0,
            0,
            Some(&icons::LINK),
            "Anyone with the link",
            None,
            vec![
                pop(
                    "ph-item-0-0",
                    "Anyone with the link",
                    if draft.can_edit { "Can edit" } else { "Can view" },
                    t,
                )
                .on_press(|s: &mut State| {
                    if let Some(Dialog::Share(draft)) = &mut s.photos.dialog {
                        draft.can_edit = !draft.can_edit;
                    }
                }),
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
                pop("ph-item-0-1", "Link expires", &expiry, t).on_press(|s: &mut State| {
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
                switch("ph-item-0-2", "Password", draft.password, t).on_press(|s: &mut State| {
                    if let Some(Dialog::Share(draft)) = &mut s.photos.dialog {
                        draft.password = !draft.password;
                    }
                }),
            ],
            t,
        ),
    ];
    let body = vec![
        what,
        list(0, None, items, t),
        note(
            "Copy link keeps it encrypted. Send a copy decrypts it, so it leaves Beebeeb's protection.",
            t,
        ),
    ];
    let foot = vec![
        chrome::button(
            "ph-dlg-btn-0",
            "Send a copy\u{2026}",
            None,
            Tone::Plain,
            28.0,
            style::BUTTON,
            t,
        )
        .on_press(|s: &mut State| {
            if let Some(id) = s.photos.viewer.as_ref().map(|v| v.id) {
                s.photos.ask(Intent::SendCopy(id));
                s.photos.close_dialog();
            }
        }),
        el().key("ph-dlg-foot-spacer").grow(1.0),
        chrome::button(
            "ph-dlg-btn-1",
            "Copy link",
            Some(&icons::LINK),
            Tone::Primary,
            28.0,
            style::BUTTON,
            t,
        )
        .default_button()
        .on_press(|s: &mut State| s.photos.confirm_dialog()),
        chrome::button("ph-dlg-btn-2", "Cancel", None, Tone::Quiet, 28.0, style::BUTTON, t)
            .on_press(|s: &mut State| s.photos.close_dialog()),
    ];
    (440.0, body, foot)
}

fn details(ui: &Ui, t: &Theme) -> (f32, Vec<El>, Vec<El>) {
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
    let foot = vec![
        chrome::button("ph-dlg-btn-0", "Done", None, Tone::Plain, 28.0, style::BUTTON, t)
            .default_button()
            .on_press(|s: &mut State| s.photos.close_dialog()),
    ];
    (420.0, vec![list(0, None, items, t)], foot)
}

/// A dialog (`.wn-dlg`), centred in the window under the caption.
pub fn dialog(ui: &Ui, dialog: &Dialog, t: &Theme) -> El {
    let (title, (width, body, foot)) = match dialog {
        Dialog::Sources(draft) => ("Photo sources", sources(ui, draft, t)),
        Dialog::Share(draft) => ("Share", share(ui, draft, t)),
        Dialog::Details => ("Details", details(ui, t)),
    };
    let card = col()
        .key("ph-dlg")
        .role("dialog")
        .label(title)
        .w(width)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            el().key("ph-dlg-head").w_full().px(24.0).pt(20.0).pb(4.0).child(
                kit::label("ph-dlg-title", title, style::TITLE, t.text)
                    .role("heading")
                    .w_full(),
            ),
        )
        .child(
            col()
                .key("ph-dlg-body")
                .w_full()
                .px(24.0)
                .pt(10.0)
                .pb(18.0)
                .gap(12.0)
                .children(body),
        )
        .child(
            row()
                .key("ph-dlg-foot")
                .w_full()
                .px(24.0)
                .py(16.0)
                .gap(8.0)
                .center_y()
                .justify_end()
                .corners([0.0, 0.0, 8.0, 8.0])
                .fill(t.surface)
                .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
                .border_layout()
                .children(foot),
        );
    col()
        .key("ph-dlg-layer")
        .pin(Some(0.0), Some(CAPTION), Some(0.0), Some(0.0))
        .center()
        // The window behind is inert while the dialog is open.
        .modal(false, |s: &mut State| s.photos.close_dialog())
        .child(card)
}
