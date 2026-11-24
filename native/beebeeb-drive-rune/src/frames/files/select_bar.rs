//! Several rows selected (DW-0403): the bar under the toolbar with what can be done to all of them,
//! and the details panel counting them (two pages, how many, how big, which, how many on this PC).

use crate::source::DriveItem;
use crate::theme::icons;
use crate::theme::scale;
use crate::theme::tokens::{radius, type_, Theme};
use crate::ui::detail_pane::{action_button, encrypted_badge, meta_row, pane, preview};
use crate::ui::{button, kit};
use crate::State;
use rune::{col, el, row, Color, El, Icon, Span, TextStyle, Tx};

/// `.mk-selbar` under the toolbar: 34 pt and its hairline.
const BAR_H: f32 = 34.5;

/// Move to a folder: a folder with an arrow going into it.
static MOVE_TO: Icon = Icon {
    name: "folder-input",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M2 9V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-1"></path>"#,
        r#"<path d="M2 13h10"></path><path d="m9 16 3-3-3-3"></path>"#,
        "</g>"
    ),
};

impl State {
    /// Is more than one row selected? The bar and the counting panel are for that.
    pub fn selects_several(&self) -> bool {
        self.selected_rows().len() > 1
    }

    /// Done: nothing selected any more.
    pub fn done_selecting(&mut self) {
        self.clear_selection();
    }

    /// The bar's Move to Trash. The message of Undo follows as it does for one row.
    fn trash_from_bar(&mut self, tx: &mut Tx) {
        self.trash_selection(tx);
    }

    /// The bar's Share…: the link dialog for the rows, as the menu opens it.
    fn share_from_bar(&mut self, tx: &mut Tx) {
        self.ask_share(tx);
    }
}

/// The bar. `None` while fewer than two rows are selected.
pub fn bar(s: &State, t: &Theme) -> Option<El> {
    if !s.selects_several() {
        return None;
    }
    let count = s.selected_rows().len();
    let words = format!("{count} selected");
    let all = "Select all";
    let link = kit::label(
        "selbar-all",
        all,
        TextStyle::new(13.0, 400.0).lh(17.0).tracking(-0.065),
        t.text_secondary,
    )
    .spans(vec![Span::new(0, all.len()).underline(t.text_secondary)])
    .role("button")
    .focusable(true)
    .on_press(|s: &mut State| s.select_all());
    let kept = s.selection_is_kept();
    let keep_words = if kept {
        concat!("Remove from ", beebeeb_engine::device_word!())
    } else {
        "Make available offline"
    };
    Some(
        row()
            .key("selbar")
            .role("toolbar")
            .label("Selection")
            .w_full()
            .h(BAR_H)
            .px(scale::space::S16)
            .gap(scale::space::S8)
            .center_y()
            .fill(t.paper2)
            .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
            .border_layout()
            .child(kit::label("selbar-count", words, type_::HEAD, t.text))
            .child(link)
            .child(el().key("selbar-space").grow(1.0))
            .child(
                action_button("selbar-share", "Share\u{2026}", &icons::SHARE, t)
                    .on_press_tx(|s: &mut State, tx: &mut Tx| s.share_from_bar(tx)),
            )
            .child(
                action_button("selbar-keep", keep_words, &icons::AVAILABLE_OFFLINE, t).on_press_tx(
                    |s: &mut State, tx: &mut Tx| {
                        s.toggle_keep_offline(tx);
                    },
                ),
            )
            .child(action_button("selbar-move", "Move\u{2026}", &MOVE_TO, t).on_press_tx(
                |s: &mut State, tx: &mut Tx| {
                    s.begin_move(tx);
                },
            ))
            .child(
                action_button("selbar-trash", "Move to Trash", &icons::TRASH, t)
                    .on_press_tx(|s: &mut State, tx: &mut Tx| s.trash_from_bar(tx)),
            )
            .child(
                button::button(
                    button::keys("selbar-done"),
                    "Done",
                    None,
                    button::Variant::Ghost,
                    button::Size::Sm,
                    button::ButtonState::Enabled,
                    t,
                )
                .on_press(|s: &mut State| s.done_selecting()),
            ),
    )
}

/// A page of the stack (`.mk-page`): 96 x 124, a heading and `lines` lines.
fn page(key: &str, first: usize, lines: usize, t: &Theme) -> El {
    let inner = 96.0 - 2.0 * 10.0;
    let mut page = col()
        .key(key.to_string())
        .size(96.0, 124.0)
        .radius(radius::SM)
        .fill(t.white)
        .box_shadow(kit::drop(2.0, 10.0, t.shadow_page))
        .py(scale::space::S12)
        .px(scale::space::S10)
        .gap(5.0)
        .child(
            el().key(format!("preview-line-{first}"))
                .h(6.0)
                .w(inner * 0.6)
                .radius(2.0)
                .fill(Color::BLACK.alpha(0.30))
                .mb(3.0),
        );
    for n in 0..lines {
        page = page.child(
            el().key(format!("preview-line-{}", first + 1 + n))
                .h(3.0)
                .w(inner)
                .radius(2.0)
                .fill(t.shadow_page),
        );
    }
    page
}

/// Two pages, the one behind turned a little (the board's `.mk-stack`).
fn stack(t: &Theme) -> El {
    let back = page("preview-page", 0, 2, t).rotate(-6.0_f32.to_radians());
    let front = page("front-page", 3, 3, t);
    el().key("preview-stack")
        .size(251.5, 152.0)
        .child(back.absolute(125.75 - 12.0 - 48.0, 76.0 + 15.0 - 62.0))
        .child(front.absolute(125.75 + 12.0 - 48.0, 76.0 + 7.0 - 62.0))
}

fn fact(key: &'static str, label: &'static str, value: String, t: &Theme) -> El {
    row()
        .key(format!("meta-row-{key}"))
        .role("listitem")
        .label(format!("{label}: {value}"))
        .w_full()
        .gap(scale::space::S8)
        .child(kit::label(key, label, type_::BODY, t.text_tertiary).w(92.0))
        .child(
            kit::label(format!("{key}-value"), value, type_::BODY, t.text)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

/// The panel for several rows: how many, how big, which ones, how many are on this PC.
pub fn panel(items: &[&DriveItem], t: &Theme) -> El {
    let bytes: i64 = items.iter().filter_map(|i| i.size).sum();
    let names = items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ");
    let on_pc = items
        .iter()
        .filter(|i| matches!(i.pc_status.as_deref(), Some("on_this_pc" | "available_offline")))
        .count();
    let _ = meta_row;
    pane(t)
        .child(preview(stack(t), t))
        .child(
            col()
                .key("detail-name-group")
                .child(kit::label("detail-name", format!("{} items", items.len()), type_::HEAD, t.text).truncate())
                .child({
                    // The size is a figure: JetBrains Mono, as the one item's line has it.
                    let size = crate::source::format_size(bytes);
                    let mono = Span {
                        start: 0,
                        end: size.len() as u32,
                        mono: true,
                        ..Span::default()
                    };
                    kit::label("detail-kind", size, type_::CAPTION, t.text_tertiary)
                        .spans(vec![mono])
                        .mt(2.0)
                }),
        )
        .child(encrypted_badge(t))
        .child(
            col()
                .key("detail-meta")
                .role("list")
                .gap(scale::space::S6)
                .child(fact("detail-selected", "Selected", names, t))
                .child(fact(
                    "detail-on-this-pc",
                    concat!("On ", beebeeb_engine::device_word!()),
                    format!("{on_pc} of {}", items.len()),
                    t,
                )),
        )
}
