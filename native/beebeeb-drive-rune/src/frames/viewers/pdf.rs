//! A PDF (DW-1505, DW-1506): the page thumbnails in a column, the page on grey, zoom and Find in
//! the toolbar; the find bar under the toolbar with the matches marked on the page (neutral, the
//! current one darker; never Amber, task 2321).
//!
//! The page's pixels are the source's (rendered on this PC from the decrypted file). A source
//! with no picture of a page gets a blank page, never a sample.

use rune::tok::Family;
use rune::{Color, El, SurfaceFit, TextStyle, col, el, row, text_input};

use super::chrome::{ButtonKind, Dot, Head, Status, button};
use super::media::{FIND_KEY, ZOOMS};
use super::source::{PdfDoc, Picture};
use super::{Intent, Ui, details, media_icons, style, ui};
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::toolbar::{ToolState, tool_button};

/// The page at 100 %: 420 points wide (the board's `MPage`).
const PAGE_W: f32 = 420.0;
/// The zoom's figure: mono 11px on 17px.
const ZOOM: TextStyle = TextStyle::new(11.0, 400.0).family(Family::Mono).lh(17.0);

fn with_ui(act: impl Fn(&mut Ui) + 'static) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(ui) = ui(s) {
            act(ui);
        }
    }
}

/// The line under the path: "Page 2 of 6 · 2.3 MB".
pub fn head(ui: &Ui, doc: &PdfDoc) -> Head {
    Head {
        facts: Some(format!("Page {} of {} \u{00B7} {}", ui.media.page, doc.pages, doc.size)),
        unsaved: false,
    }
}

pub fn status() -> Status {
    Status {
        dot: Dot::Green,
        text: details::DECRYPTED_HERE.into(),
        details: "Space for Preview".into(),
    }
}

/// A second key on the same box (the golden keys the zoom's three controls twice): a clear box
/// over the control that takes no press.
fn also(key: &'static str, inner: El, w: f32, h: f32) -> El {
    el().key(key).size(w, h).child(inner)
}

/// The toolbar's controls: zoom (minus, the figure, plus), Find, Share.
pub fn tools(ui: &Ui, t: &Theme) -> Vec<El> {
    let zoom = ui.media.zoom;
    let step = |key: &'static str, name: &'static str, icon: &'static rune::Icon, by: i32, can: bool| {
        tool_button(
            key,
            name,
            icon,
            13.0,
            if can { ToolState::Idle } else { ToolState::Off },
            t,
        )
        .h(24.0)
        .w(28.0)
        .on_press(with_ui(move |ui| ui.zoom_by(by)))
    };
    let figure = format!("{zoom}%");
    let group = row()
        .key("zoom-group")
        .role("group")
        .label("Zoom")
        .padding(2.0)
        .gap(1.0)
        .radius(9.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator))
        .child(also(
            "tool-0",
            step("zoom-minus", "Zoom out", &media_icons::MINUS, -1, zoom > ZOOMS[0])
                .shortcut_hint("Ctrl+Minus")
                .child(
                    kit::glyph("tool-0-icon", &media_icons::MINUS, 13.0, t.text_secondary)
                        .absolute(7.5, 5.5)
                        .opacity(0.0),
                ),
            28.0,
            24.0,
        ))
        .child(also(
            "tool-1",
            kit::label("zoom-pct", figure, ZOOM, t.text_secondary)
                .size(40.0, 24.0)
                .pt(3.5)
                .align(rune::Align::Center),
            40.0,
            24.0,
        ))
        .child(also(
            "tool-2",
            step(
                "zoom-plus",
                "Zoom in",
                &media_icons::PLUS_SMALL,
                1,
                zoom < ZOOMS[ZOOMS.len() - 1],
            )
            .shortcut_hint("Ctrl+Plus")
            .child(
                kit::glyph("tool-2-icon", &media_icons::PLUS_SMALL, 13.0, t.text_secondary)
                    .absolute(7.5, 5.5)
                    .opacity(0.0),
            ),
            28.0,
            24.0,
        ));
    let finding = ui.media.find.is_some();
    vec![
        group,
        tool_button(
            "tool-3",
            "Find",
            &media_icons::FIND,
            14.0,
            if finding { ToolState::On } else { ToolState::Idle },
            t,
        )
        .shortcut_hint("Ctrl+F")
        .checked(finding)
        .on_press(with_ui(|ui| {
            if ui.media.find.is_some() {
                ui.close_find();
            } else {
                ui.open_find();
            }
        })),
        tool_button("tool-4", "Share", &media_icons::SHARE, 15.0, ToolState::Idle, t)
            .on_press(with_ui(|ui| ui.intent(Intent::Share))),
    ]
}

/// The edit the find field types into. (With no find bar open nothing reaches this.)
pub fn find_edit(s: &mut State) -> &mut rune::TextEdit {
    match s.viewer.ui.as_mut().and_then(|ui| ui.media.find.as_mut()) {
        Some(find) => &mut find.edit,
        None => &mut s.search,
    }
}

/// The find bar (`.mk-bulk`), under the toolbar.
pub fn find_bar(ui: &Ui, t: &Theme) -> Option<El> {
    let find = ui.media.find.as_ref()?;
    let words = &find.edit.text;
    kit::note_text("find-text", FIND_KEY, words, type_::SEARCH, t.text);
    // The caret stands one point after the words (`.kit-caret`).
    let caret_x = rune::text::advance(words, type_::SEARCH) + 1.0;
    let field = row()
        .key("find-field")
        .role("search")
        .size(260.0, 28.0)
        .pl(9.0)
        .pr(6.0)
        .center_y()
        .radius(8.0)
        .fill(t.card)
        // The focused field: Amber's ring at 30 %, the deep Amber line inside.
        .box_shadow(kit::ring(3.0, t.accent.alpha(0.3)))
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: t.accent_deep,
            inset: true,
        })
        .child(
            el().key("find-field-inner")
                .size(245.0, 17.0)
                .child(
                    text_input(&find.edit, type_::SEARCH)
                        .key(FIND_KEY)
                        .label("Find in this document")
                        .single_line()
                        .color(t.text)
                        .caret(Color::TRANSPARENT, t.selection)
                        .on_edit(find_edit)
                        // Enter in the field: the next match. Shift+Enter is the previous one only while the
                        // keyboard is outside the field (`media::on_key`): a field's submit carries no
                        // modifiers in Rune, and its key hook is not asked for Enter, so in the field
                        // Shift+Enter steps on like Enter. The previous-match chevron is the way back
                        // (named in FRAMES.md and the PR body as a toolkit limit).
                        .on_submit(|s: &mut State, _: &mut rune::Tx| {
                            if let Some(ui) = super::ui(s) {
                                ui.find_step(1);
                            }
                        })
                        .size(245.0, 17.0),
                )
                .child(
                    el().key("find-caret")
                        .absolute(caret_x, 1.5)
                        .size(1.5, 14.0)
                        .radius(1.0)
                        .fill(t.accent_deep),
                ),
        );
    let nav = |key: &'static str, name: &'static str, by: i32, hint: &'static str, turned: bool| {
        let mut icon = kit::glyph(format!("{key}-icon"), &media_icons::CHEV_DOWN, 14.0, t.text_secondary);
        if turned {
            icon = icon.rotate(std::f32::consts::PI);
        }
        row()
            .key(key)
            .role("button")
            .label(name)
            .shortcut_hint(hint)
            .size(28.0, 28.0)
            .center()
            .radius(7.0)
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .on_press(with_ui(move |ui| ui.find_step(by)))
            .child(icon)
    };
    let count = ui.find_count();
    Some(
        row()
            .key("find-bar")
            .role("toolbar")
            .label("Find")
            .w_full()
            .h(40.5)
            .px(14.0)
            .gap(8.0)
            .center_y()
            .fill(bar_ground(t))
            .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
            .border_layout()
            .child(kit::glyph("find-bar-icon", &media_icons::FIND, 13.0, t.text_secondary))
            .child(field)
            .child(style::fitted("find-count", &count, style::SUB_MONO, t.text_tertiary))
            .child(nav("find-nav-0", "Next match", 1, "Enter", false))
            .child(nav("find-nav-1", "Previous match", -1, "Shift+Enter", true))
            .child(el().key("find-spacer").grow(1.0))
            .child(button("find-done", "Done", ButtonKind::Ghost, true, t).on_press(with_ui(Ui::close_find))),
    )
}

/// `.mk-bulk`'s ground.
fn bar_ground(t: &Theme) -> Color {
    rune::color::hex(if t.dark { "#201C17" } else { "#F3F2F1" }).unwrap_or(t.bg)
}

fn thumbs(ui: &Ui, doc: &PdfDoc, t: &Theme) -> El {
    let mut column = col()
        .key("thumb-sidebar")
        .role("list")
        .label("Pages")
        .w(120.0)
        .h_full()
        .py(16.0)
        .gap(12.0)
        .items_center()
        .fill(t.sidebar_bg)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .clip_rounded(0.0);
    for n in 1..=doc.pages {
        let i = n - 1;
        let current = n == ui.media.page;
        let page = el()
            .key(format!("thumb-{i}-page"))
            .size(64.0, 84.0)
            .radius(2.0)
            .fill(Color::WHITE);
        let page = if current {
            page.box_shadow(kit::ring(2.0, t.text))
        } else {
            page.box_shadow(kit::drop(1.0, 4.0, Color::BLACK.alpha(0.14)))
        };
        column = column.child(
            col()
                .key(format!("thumb-{i}"))
                .role("button")
                .label(format!("Page {n}"))
                .checked(current)
                .items_center()
                .gap(4.0)
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(with_ui(move |ui| ui.go_page(n)))
                .child(page)
                .child(style::fitted(
                    format!("thumb-{i}-num"),
                    &n.to_string(),
                    style::CAP_MONO,
                    t.text_tertiary,
                )),
        );
    }
    column
}

fn page(ui: &Ui) -> El {
    let scale = ui.media.zoom as f32 / 100.0;
    let width = PAGE_W * scale;
    let sheet = el()
        .key("page")
        .role("image")
        .label(format!("Page {}", ui.media.page))
        .w(width)
        .radius(2.0)
        .fill(Color::WHITE)
        .box_shadow(kit::drop(4.0, 18.0, Color::BLACK.alpha(0.16)));
    let mut sheet = match ui.source.picture(&ui.file.id, Picture::Page(ui.media.page)) {
        Some(pixels) => {
            let (w, h) = pixels.size();
            let height = if w == 0 { 0.0 } else { width * h as f32 / w as f32 };
            sheet.h(height).pixels_fit(&pixels, SurfaceFit::Fill)
        }
        // No picture of this page: a blank page of the board's shape.
        None => sheet.h(560.0 * scale),
    };
    if let Some(find) = &ui.media.find {
        let ink = |alpha: f32| Color::BLACK.alpha(alpha);
        for (i, m) in find.matches.iter().enumerate() {
            if m.page != ui.media.page {
                continue;
            }
            sheet = sheet.child(
                el().key(format!("mark-{i}"))
                    .absolute(m.x * scale, m.y * scale)
                    .size(m.w * scale, m.h * scale)
                    .radius(2.0)
                    .fill(ink(if i == find.current { 0.32 } else { 0.13 })),
            );
        }
    }
    sheet
}

pub fn body(ui: &Ui, doc: &PdfDoc, t: &Theme) -> El {
    row()
        .key("viewer-body")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(thumbs(ui, doc, t))
        .child(
            row()
                .key("page-area")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .pt(24.0)
                .justify_center()
                .items_start()
                .fill(style::stage(t))
                .clip_rounded(0.0)
                .child(page(ui)),
        )
}
