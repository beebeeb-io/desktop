//! What comment mode draws over the app: the outline and its key path, the note popover, the
//! notes panel and the small "Feedback" entry. Tokens and type styles are the app's own;
//! selection is neutral and Amber appears only on the one primary action.

use rune::{Align, Cx, El, ScrollSpec, Side, Tx, col, el, row, text, text_input};

use super::model::{self, NoteEntry, StatusState};
use super::{Feedback, Mode, ROOT_KEY, app_path};
use crate::theme::tokens::{self, Theme, radius, space, type_};
use crate::{State, TITLEBAR_H};

pub const NOTE_INPUT: &str = "fb-note-input";
const PANEL_W: f32 = 360.0;
const POPOVER_W: f32 = 340.0;

pub const PRIVACY_LINE: &str = "Notes and screenshots stay on this PC, in your own profile. Nothing is sent anywhere. \
    Screenshots show your file names; password and key fields are blanked.";

/// The app, with the feedback chrome over it. During the capture frame there is no chrome at
/// all: the picture and the tree are the app's alone.
pub fn wrap(s: &State, cx: &mut Cx<State>, app: El) -> El {
    let t = tokens::theme(cx.system_dark());
    let fb = &s.feedback;
    let mut root = el().key(ROOT_KEY).w_full().h_full().fill(t.bg).child(app);
    match fb.mode {
        Mode::Capturing => return root,
        Mode::Picking | Mode::Writing => root = root.child(comment_layer(fb, &t)),
        Mode::Off => {}
    }
    if fb.panel_open {
        root = root.child(panel(fb, &t));
    } else if fb.mode == Mode::Off {
        root = root.child(entry(&t));
    }
    root
}

fn mono(s: impl Into<String>, t: &Theme) -> El {
    text(s).font(type_::KBD).color(t.text_tertiary)
}

fn button(key: &'static str, label: &'static str, t: &Theme) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(26.0)
        .px(space::MD)
        .center()
        .radius(radius::MD)
        .border(1.0, t.card_border)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(text(label).font(type_::BODY).color(t.text))
}

/// The one Amber control on a surface.
fn primary(key: &'static str, label: &'static str, t: &Theme) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(26.0)
        .px(space::MD)
        .center()
        .radius(radius::MD)
        .fill(t.accent)
        .focusable(true)
        .child(text(label).font(type_::BODY_SEMI).color(t.accent_text_on))
}

/// The small entry that is always reachable by Tab: opens the notes panel.
fn entry(t: &Theme) -> El {
    row()
        .key("fb-entry")
        .role("button")
        .label("Feedback")
        .pin(None, None, Some(space::MD), Some(36.0))
        .h(24.0)
        .px(space::SM)
        .gap(6.0)
        .center()
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.card_border)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(|s: &mut State| s.feedback.toggle_panel())
        .child(text("Feedback").font(type_::BODY_SM).color(t.text_secondary))
        .child(mono("F8", t))
}

fn comment_layer(fb: &Feedback, t: &Theme) -> El {
    let writing = fb.mode == Mode::Writing;
    let mut layer = el()
        .key("fb-overlay")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        // The outline lands on the element at once: it must never trail the pointer.
        .instant_changes()
        .modal(false, |s: &mut State| s.feedback.back())
        .child(
            el().key("fb-catcher")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .on_pointer_move(|s: &mut State, p| {
                    if s.feedback.mode == Mode::Picking {
                        s.feedback.pointer = Some((p.x, p.y));
                    }
                })
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.feedback.pick(tx)),
        );
    if let Some(target) = fb.target() {
        let [x, y, w, h] = target.visible;
        // The last three keys are enough to recognise it; the note records the whole path.
        let full = app_path(&target.path);
        let tail: Vec<&str> = full.rsplit('/').take(3).collect();
        let path = format!(
            "{}{}",
            if full.split('/').count() > 3 { "…/" } else { "" },
            tail.into_iter().rev().collect::<Vec<_>>().join("/")
        );
        layer = layer
            .child(
                el().key("fb-highlight")
                    .absolute(x, y)
                    .size(w.max(1.0), h.max(1.0))
                    .fill(t.selection)
                    .border(1.5, t.text),
            )
            .child(
                row()
                    .key("fb-path")
                    .absolute(x.max(0.0), if y >= 20.0 { y - 19.0 } else { y + h + 3.0 })
                    .h(16.0)
                    .px(5.0)
                    .center_y()
                    .radius(radius::SM)
                    .fill(t.text)
                    .child(text(path).key("fb-path-text").font(type_::CAPTION_MONO_SM).color(t.bg)),
            );
        if writing {
            layer = layer.child(popover(fb, t));
        }
    }
    layer.child(hint_bar(fb, t))
}

fn hint_bar(fb: &Feedback, t: &Theme) -> El {
    let key_hint = |keys: &'static str, what: &'static str| {
        row()
            .gap(5.0)
            .center_y()
            .child(mono(keys, t))
            .child(text(what).font(type_::BODY_SM).color(t.text_secondary))
    };
    let mut bar = row()
        .key("fb-hint")
        // In the caption bar's empty drag area: the one place that covers none of the app.
        .pin(Some(232.0), Some(4.0), None, None)
        .h(TITLEBAR_H - 8.0)
        .px(space::SM)
        .gap(space::MD)
        .center_y()
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.separator_strong)
        .child(
            text("Comment mode")
                .key("fb-hint-title")
                .font(type_::BODY_SEMI)
                .color(t.text),
        );
    if fb.mode == Mode::Picking {
        bar = bar
            .child(key_hint("Click", "pick"))
            .child(key_hint("Alt+Up", "parent"))
            .child(key_hint("Alt+Down", "child"))
            .child(key_hint("Esc", "leave"));
    }
    if fb.secure_on_screen {
        bar = bar.child(
            text("A password or key field is on screen: it is blanked in the note.")
                .key("fb-secure-line")
                .font(type_::BODY_SM)
                .color(t.text_secondary),
        );
    }
    if let Some(m) = &fb.message {
        bar = bar.child(text(m.clone()).key("fb-message").font(type_::BODY_SM).color(t.text));
    }
    bar
}

fn popover(fb: &Feedback, t: &Theme) -> El {
    let tags = model::TAGS.iter().enumerate().map(|(i, &tag)| {
        let on = fb.tags[i];
        row()
            .key(format!("fb-tag-{i}"))
            .role("button")
            .label(tag)
            .h(22.0)
            .px(space::SM)
            .center()
            .radius(radius::SM)
            .fill(if on { t.selection } else { rune::Color::TRANSPARENT })
            .border(1.0, if on { t.text_secondary } else { t.card_border })
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .on_press(move |s: &mut State| s.feedback.tags[i] = !s.feedback.tags[i])
            .child(
                text(tag)
                    .font(type_::BODY_SM)
                    .color(if on { t.text } else { t.text_secondary }),
            )
    });
    col()
        .key("fb-popover")
        .w(POPOVER_W)
        .anchored("fb-highlight", Side::Bottom, Align::Start, 8.0)
        .padding(space::MD)
        .gap(space::SM)
        .radius(radius::LG)
        .fill(t.card)
        .border(1.0, t.separator_strong)
        .child(text("Note on this element").font(type_::BODY_SEMI).color(t.text))
        .child(
            el().key("fb-note-box")
                .w_full()
                .min_h(76.0)
                .padding(space::SM)
                .radius(radius::MD)
                .fill(t.surface)
                .border(1.0, t.card_border)
                .child(
                    text_input(&fb.note, type_::BODY)
                        .key(NOTE_INPUT)
                        .label("Note")
                        .color(t.text)
                        .placeholder("What should change here?", t.text_tertiary)
                        .on_edit(|s: &mut State| &mut s.feedback.note)
                        .on_submit(|s: &mut State, tx: &mut Tx| s.feedback.save(tx))
                        .w_full(),
                ),
        )
        .child(row().key("fb-tags").gap(6.0).children(tags))
        .child(mono("Shift+Enter new line  Esc cancels", t))
        .child(
            row()
                .key("fb-popover-foot")
                .w_full()
                .center_y()
                .justify_between()
                .child(mono("Enter saves", t))
                .child(
                    row()
                        .gap(6.0)
                        .child(button("fb-cancel", "Cancel", t).on_press(|s: &mut State| s.feedback.back()))
                        .child(
                            primary("fb-save", "Save note", t)
                                .on_press_tx(|s: &mut State, tx: &mut Tx| s.feedback.save(tx)),
                        ),
                ),
        )
}

fn chip(note: &NoteEntry, t: &Theme) -> El {
    let fixed = note.status.state == StatusState::Fixed;
    row()
        .key(format!("fb-chip-{}", note.id))
        .h(18.0)
        .px(6.0)
        .center()
        .radius(radius::SM)
        .fill(if fixed { t.green_bg } else { t.surface })
        .border(1.0, t.card_border)
        .child(
            text(note.status.chip())
                .font(type_::BODY_SM)
                .color(if note.status.state == StatusState::New {
                    t.text_tertiary
                } else {
                    t.text
                }),
        )
}

fn reply(note: &NoteEntry, t: &Theme) -> Option<El> {
    let r = note.status.reply.as_ref()?;
    let by = note.status.by.as_deref().unwrap_or("lead");
    Some(
        col()
            .key(format!("fb-reply-{}", note.id))
            .w_full()
            .padding(space::SM)
            .gap(2.0)
            .radius(radius::MD)
            .fill(t.surface)
            .child(
                text(format!("Reply from {by}"))
                    .font(type_::SECTION_LABEL)
                    .color(t.text_tertiary),
            )
            .child(text(r.clone()).font(type_::BODY).color(t.text).wrap()),
    )
}

fn note_row(fb: &Feedback, note: &NoteEntry, t: &Theme) -> El {
    let id = note.id.clone();
    let mut thumb = el()
        .key(format!("fb-thumb-{}", note.id))
        .size(96.0, 60.0)
        .radius(radius::SM)
        .fill(t.surface)
        .border(1.0, t.card_border);
    if let Some(p) = fb.thumbs.borrow().get(&note.id) {
        thumb = thumb.pixels(p);
    }
    let mut body = col()
        .grow(1.0)
        .gap(3.0)
        .child(
            text(note.comment.clone())
                .key(format!("fb-comment-{}", note.id))
                .font(type_::BODY)
                .color(t.text)
                .wrap()
                .max_lines(3),
        )
        .child(
            text(note.element_path.rsplit('/').next().unwrap_or_default().to_string())
                .font(type_::KBD)
                .color(t.text_tertiary)
                .truncate(),
        )
        .child(
            row().gap(6.0).center_y().child(chip(note, t)).child(
                text(note.created_at.get(..16).unwrap_or(&note.created_at).replace('T', " "))
                    .font(type_::BODY_SM)
                    .color(t.text_quaternary),
            ),
        );
    if let Some(r) = reply(note, t) {
        body = body.child(r);
    }
    row()
        .key(format!("fb-note-{}", note.id))
        .role("button")
        .label("Open note")
        .w_full()
        .padding(space::SM)
        .gap(space::SM)
        .items_start()
        .radius(radius::MD)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(move |s: &mut State| s.feedback.open_note(&id))
        .child(thumb)
        .child(body)
}

fn note_detail(fb: &Feedback, note: &NoteEntry, t: &Theme) -> El {
    let dir = note.dir.display().to_string();
    let mut shot = el()
        .key("fb-detail-shot")
        .w_full()
        .radius(radius::MD)
        .border(1.0, t.card_border)
        .fill(t.surface);
    shot = match &fb.opened_image {
        Some(p) => shot.pixels(p),
        None => shot.h(120.0),
    };
    let mut c = col()
        .key("fb-detail")
        .w_full()
        .gap(space::SM)
        .child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .child(button("fb-back", "All notes", t).on_press(|s: &mut State| {
                    s.feedback.opened = None;
                    s.feedback.opened_image = None;
                }))
                .child(chip(note, t)),
        )
        .child(shot)
        .child(text(note.comment.clone()).font(type_::BODY).color(t.text).wrap())
        .child(
            text(note.element_path.clone())
                .font(type_::KBD)
                .color(t.text_tertiary)
                .wrap_anywhere(),
        );
    if !note.tags.is_empty() {
        c = c.child(text(note.tags.join("  ")).font(type_::BODY_SM).color(t.text_secondary));
    }
    if let Some(r) = reply(note, t) {
        c = c.child(r);
    }
    c.child(
        text(dir.clone())
            .key("fb-detail-path")
            .font(type_::KBD)
            .color(t.text_tertiary)
            .wrap_anywhere(),
    )
    .child(row().child(
        button("fb-copy-path", "Copy path", t).on_press_tx(move |_: &mut State, tx: &mut Tx| tx.copy(dir.clone())),
    ))
}

fn panel(fb: &Feedback, t: &Theme) -> El {
    let opened = fb.opened.as_ref().and_then(|id| fb.notes.iter().find(|n| &n.id == id));
    let inbox = fb.inbox.as_ref().map(|p| p.display().to_string());
    let body = match opened {
        Some(note) => note_detail(fb, note, t),
        None if fb.notes.is_empty() => col().key("fb-empty").w_full().gap(space::XS).py(space::LG).child(
            text("No notes yet. Press F8, click an element, type a note and press Enter.")
                .font(type_::BODY)
                .color(t.text_secondary)
                .wrap(),
        ),
        None => col()
            .key("fb-list")
            .w_full()
            .gap(2.0)
            .children(fb.notes.iter().map(|n| note_row(fb, n, t))),
    };
    let mut foot = col().key("fb-panel-foot").w_full().gap(space::XS).child(
        text(PRIVACY_LINE)
            .key("fb-privacy")
            .font(type_::BODY_SM)
            .color(t.text_tertiary)
            .wrap(),
    );
    if let Some(dir) = inbox {
        foot = foot.child(
            row()
                .w_full()
                .gap(space::SM)
                .center_y()
                .child(
                    text(dir.clone())
                        .key("fb-inbox-path")
                        .font(type_::KBD)
                        .color(t.text_tertiary)
                        .truncate_middle()
                        .grow(1.0),
                )
                .child(
                    button("fb-copy-inbox", "Copy path", t)
                        .on_press_tx(move |_: &mut State, tx: &mut Tx| tx.copy(dir.clone())),
                ),
        );
    }
    col()
        .key("fb-panel")
        .role("group")
        .label("Feedback notes")
        .pin(None, Some(TITLEBAR_H), Some(0.0), Some(0.0))
        .w(PANEL_W)
        .fill(t.card)
        .border(1.0, t.separator_strong)
        .padding(space::MD)
        .gap(space::MD)
        .child(
            row()
                .key("fb-panel-head")
                .w_full()
                .center_y()
                .justify_between()
                .child(
                    row()
                        .gap(space::SM)
                        .center_y()
                        .child(text("Feedback notes").font(type_::LABEL).color(t.text))
                        .child(
                            text(fb.notes.len().to_string())
                                .key("fb-count")
                                .font(type_::KBD)
                                .color(t.text_tertiary),
                        ),
                )
                .child(
                    row()
                        .gap(6.0)
                        .child(
                            primary("fb-add", "Add a note", t)
                                .on_press_tx(|s: &mut State, tx: &mut Tx| s.feedback.enter(tx)),
                        )
                        .child(button("fb-close", "Close", t).on_press(|s: &mut State| s.feedback.toggle_panel())),
                ),
        )
        .child(
            el().key("fb-panel-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(body),
        )
        .child(foot)
}
