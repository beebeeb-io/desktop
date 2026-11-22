//! Toolbar (`.mk-tb`): back and forward, the folder's title and item count, the list/grid
//! switch, sort, the details-panel toggle, New (the one Amber control) and the search field.

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, Icon, col, el, row, text_input};

use super::kit;

/// A press on a side of the view switch; the flag is "the grid".
type ViewPress = Box<dyn Fn(&mut State, &mut rune::Tx, bool)>;
/// The Open in File Explorer press, shared by the button and its reveal key.
type ExplorerPress = std::rc::Rc<dyn Fn(&mut State, &mut rune::Tx)>;

/// The search field's placeholder: one string for the field and for the gate's note.
const SEARCH_PLACEHOLDER: &str = "Search names";

/// How a toolbar button (`.mk-tbtn`) shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    /// Ready (ink-2).
    Idle,
    /// Pressed in: a toggle that is on, a menu that is open (neutral wash, ink).
    On,
    /// Nothing to do (ink-4): back with no history.
    Off,
}

/// A toolbar row as one Tab stop (role `toolbar`, Rune's focus group): Tab lands on its current
/// item and leaves at the next, Left/Right move between its buttons, Home and End jump to the
/// ends. `label` is what a screen reader calls the toolbar ("Viewer controls").
pub fn roving(bar: El, label: &str) -> El {
    bar.role("toolbar").label(label).focus_group()
}

/// A toolbar button: 28 pt tall, at least 28 wide, one or more 15/16 pt glyphs.
/// Keys: `<key>` and `<key>-icon` (further glyphs are added by the caller).
pub fn tool_button(key: &str, name: &str, ic: &'static Icon, icon_size: f32, state: ToolState, t: &Theme) -> El {
    let ink = match state {
        ToolState::Idle => t.text_secondary,
        ToolState::On => t.text,
        ToolState::Off => t.text_quaternary,
    };
    let icon_key = match key {
        "panel-toggle-btn" => "panel-icon".to_string(),
        "sort-btn" => "sort-icon".to_string(),
        _ => format!("{key}-icon"),
    };
    row()
        .key(key)
        .role("button")
        .label(name)
        .disabled(state == ToolState::Off)
        .h(28.0)
        .min_w(28.0)
        .px(6.0)
        .gap(4.0)
        .center()
        .radius(radius::CONTROL)
        .fill(if state == ToolState::On {
            t.selection
        } else {
            Color::TRANSPARENT
        })
        .hover(t.hover)
        .press(t.press)
        .focusable(state != ToolState::Off)
        .child(kit::glyph(icon_key, ic, icon_size, ink))
}

/// One segment of a segmented control (`.mk-tgroup .mk-tbtn`): the chosen one is raised.
pub fn segment(key: &'static str, name: &'static str, ic: &'static Icon, on: bool, t: &Theme) -> El {
    let mut seg = el()
        .key(key)
        .role("radio")
        .label(name)
        .checked(on)
        .h(24.0)
        .min_w(28.0)
        .px(6.0)
        .center()
        .radius(radius::CONTROL)
        .focusable(true)
        .child(kit::glyph(
            format!("{key}-icon"),
            ic,
            15.0,
            if on { t.text } else { t.text_secondary },
        ));
    if on {
        seg = seg
            .fill(t.card)
            .box_shadow(kit::drop(1.0, 2.0, t.shadow_raised))
            .box_shadow(kit::ring(kit::HAIR, t.separator));
    } else {
        seg = seg.hover(t.hover).press(t.press);
    }
    seg
}

/// A segmented control's well (`.mk-tgroup`).
pub fn segmented(key: &'static str, name: &'static str, segments: Vec<El>, t: &Theme) -> El {
    row()
        .key(key)
        .role("radiogroup")
        .label(name)
        .padding(2.0)
        .gap(1.0)
        .radius(9.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator))
        .children(segments)
}

/// What a press does: the window's own state change, with the transaction it may need.
pub type Press = Box<dyn Fn(&mut State, &mut rune::Tx)>;

/// A [`Press`] from a closure.
pub fn press(f: impl Fn(&mut State, &mut rune::Tx) + 'static) -> Option<Press> {
    Some(Box::new(f))
}

/// The list/grid switch (the board's `view`): a window with one layout draws it dimmed, neither side chosen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    NotHere,
    /// As `NotHere`, with the dimming on a well of its own around the switch (key
    /// `view-toggle-dim`, which the Viewers goldens hold).
    NotHereWell,
    List,
    Grid,
}

/// Sort (the board's `sort` and `sortOpen`): dimmed where nothing is sorted, pressed while its menu is open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    NotHere,
    Idle,
    Open,
}

/// The details panel's button (the board's `details`): dimmed where there is no panel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Details {
    NotHere,
    Closed,
    Open,
}

/// New (`.mk-new`): at 40 % where nothing can be added, ringed while its menu is open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum New {
    Off,
    Idle,
    Open,
}

/// What ONE window puts in the window toolbar ([`bar`]). The controls are the same in every window
/// (the board's `MacToolbar`: "a control that doesn't apply is dimmed, never removed"); a window
/// says which state each is in and what a press does there.
pub struct Bar {
    /// The toolbar's name for a screen reader: the window's section.
    pub name: String,
    /// Back and Forward: whether there is somewhere to go, and what goes there.
    pub back: (bool, Option<Press>),
    pub forward: (bool, Option<Press>),
    /// The title block (`.mk-title`, key `folder-info`): the window's own.
    pub title: El,
    /// The window's own controls, between the title and the standard ones (the board's `extra`).
    pub slot: Option<El>,
    /// The slot holds a hairline of its own (key `toolbar-sep`): the toolbar's is then `toolbar-sep-2`.
    pub slot_sep: bool,
    pub view: View,
    /// A press on a side of the switch; the flag is "the grid".
    pub on_view: Option<ViewPress>,
    pub sort: Sort,
    pub on_sort: Option<Press>,
    pub details: Details,
    pub on_details: Option<Press>,
    /// The switch, Sort and the details button are drawn in their states and take nothing: no
    /// focus, no press, disabled to assistive technology (Sync history draws them as Files does).
    pub inert: bool,
    /// The gate is closed: Open in File Explorer is drawn but out of reach too.
    pub gate_closed: bool,
    /// Open in File Explorer: live in every window.
    pub on_explorer: Press,
    pub new: New,
    pub on_new: Option<Press>,
    /// The search field ([`search_box`] around the window's own input).
    pub search: El,
    /// Room kept free at the trailing edge (at least the toolbar's own 12).
    pub trailing_inset: f32,
}

/// Open in File Explorer where the window shows no folder of its own (Shared, Photos, Trash, This
/// PC): the Beebeeb location, asked of the same source Files asks for its folder (`None` is the
/// location itself). A refusal is logged: only the Files window has a toast to say it in.
pub fn open_beebeeb_location(s: &mut State, _: &mut rune::Tx) {
    if let Err(why) = s.actions.source.open_in_file_explorer(None) {
        eprintln!("beebeeb: the toolbar's Open in File Explorer did not run ({why:?})");
    }
}

/// The search field's box (`.mk-search`): the glass, the window's own input (key `search-input`
/// or the window's), the shortcut. The gate reads the placeholder as the input's text.
pub fn search_box(input_key: &'static str, placeholder: &'static str, input: El, t: &Theme) -> El {
    kit::note_text(
        "search-placeholder",
        input_key,
        placeholder,
        type_::SEARCH,
        t.text_tertiary,
    );
    search_field_box(input, t)
}

fn search_field_box(input: El, t: &Theme) -> El {
    row()
        .key("search-box")
        .role("search")
        .w(210.0)
        .h(28.0)
        .gap(6.0)
        .pl(9.0)
        .pr(6.0)
        .radius(8.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator))
        .center_y()
        .child(kit::glyph("search-icon", &icons::SEARCH, 13.0, t.text_tertiary))
        .child(input)
        .child(kbd("kbd-hint", "Ctrl+F", t))
}

/// The title block (`.mk-title`, key `folder-info`): the title, and under it the info line
/// (item count, "No plan yet") where there is one.
pub fn title_block(title: &str, info: Option<&str>, t: &Theme) -> El {
    let mut block = col()
        .key("folder-info")
        .px(4.0)
        .min_w(0.0)
        .child(kit::label("folder-title", title.to_string(), type_::TITLE, t.text).truncate());
    if let Some(info) = info {
        block = block.child(kit::label(
            "item-count",
            info.to_string(),
            type_::TITLE_SUB,
            t.text_tertiary,
        ));
    }
    block
}

/// The one tool button: 28 pt, one 16 pt glyph, ready. Keys `<key>` and `<key>-icon`.
pub fn tool(key: &str, label: &str, icon: &'static Icon, t: &Theme) -> El {
    tool_button(key, label, icon, 16.0, ToolState::Idle, t)
}

/// THE window toolbar (`.mk-tb`, the board's `MacToolbar`): Back and Forward, the title, the
/// window's own controls, then the same five in every window (the list/grid switch, Sort, the
/// details panel, Open in File Explorer, New) and the search field.
pub fn bar(bar: Bar, t: &Theme) -> El {
    bar_for(bar, false, t)
}

/// The toolbar of an open file (preview or editor): only what applies to that file. No Forward,
/// no list/grid switch, no Sort, no New, no folder search; Back closes the file.
pub fn file_bar(bar: Bar, t: &Theme) -> El {
    bar_for(bar, true, t)
}

fn bar_for(bar: Bar, file: bool, t: &Theme) -> El {
    let nav_button = |key, name, ic, hint: &'static str, (live, press): (bool, Option<Press>)| {
        let button = tool_button(
            key,
            name,
            ic,
            16.0,
            if live { ToolState::Idle } else { ToolState::Off },
            t,
        )
        .shortcut_hint(hint);
        match press {
            Some(press) => button.on_press_tx(move |s: &mut State, tx: &mut rune::Tx| press(s, tx)),
            None => button,
        }
    };
    let mut nav = row()
        .key("nav")
        .child(nav_button("back", "Back", &icons::CHEVRON_LEFT, "Alt+Left", bar.back));
    if !file {
        nav = nav.child(nav_button(
            "forward",
            "Forward",
            &icons::CHEVRON_RIGHT,
            "Alt+Right",
            bar.forward,
        ));
    }

    let on_view = bar.on_view.map(std::rc::Rc::new);
    let side = |key, name, ic, grid: bool| {
        let seg = segment(key, name, ic, bar.view == if grid { View::Grid } else { View::List }, t);
        match (bar.view, &on_view) {
            // One layout: the board draws the switch dimmed, and neither side takes a press.
            (View::NotHere | View::NotHereWell, _) => seg.disabled(true).focusable(false),
            _ if bar.inert => seg.disabled(true).focusable(false),
            (_, Some(on_view)) => {
                let on_view = on_view.clone();
                seg.on_press_tx(move |s: &mut State, tx: &mut rune::Tx| on_view(s, tx, grid))
            }
            (_, None) => seg,
        }
    };
    let mut view_toggle = segmented(
        "view-toggle",
        "View",
        vec![
            side("view-list", "as List", &icons::VIEW_LIST, false),
            side("view-grid", "as Grid", &icons::VIEW_GRID, true),
        ],
        t,
    );
    if bar.view == View::NotHere {
        // `.mk-tgroup` at 45 %.
        view_toggle = view_toggle.disabled(true).opacity(0.45);
    } else if bar.view == View::NotHereWell {
        view_toggle = el()
            .key("view-toggle-dim")
            .radius(9.0)
            .fill(t.field)
            .opacity(0.45)
            .child(view_toggle);
    } else if bar.inert {
        view_toggle = view_toggle.disabled(true).focusable(false);
    }

    let mut sort_btn = tool_button(
        "sort-btn",
        "Sort",
        &icons::SORT,
        15.0,
        match bar.sort {
            Sort::NotHere => ToolState::Off,
            Sort::Idle => ToolState::Idle,
            Sort::Open => ToolState::On,
        },
        t,
    );
    if bar.inert {
        sort_btn = sort_btn.disabled(true).focusable(false);
    } else if bar.sort != Sort::NotHere {
        sort_btn = sort_btn.expanded(bar.sort == Sort::Open);
        if let Some(press) = bar.on_sort {
            sort_btn = sort_btn.on_press_tx(move |s: &mut State, tx: &mut rune::Tx| press(s, tx));
        }
    }
    let sort_btn = sort_btn.child(kit::glyph(
        "sort-caret",
        &icons::CHEVRON_DOWN,
        11.0,
        match bar.sort {
            Sort::NotHere => t.text_quaternary,
            Sort::Idle => t.text_secondary,
            Sort::Open => t.text,
        },
    ));

    let mut panel_btn = tool_button(
        "panel-toggle-btn",
        "Details panel",
        &icons::PANEL_TOGGLE,
        16.0,
        match bar.details {
            Details::NotHere => ToolState::Off,
            Details::Closed => ToolState::Idle,
            Details::Open => ToolState::On,
        },
        t,
    );
    if bar.inert {
        panel_btn = panel_btn.disabled(true).focusable(false);
    } else if bar.details != Details::NotHere {
        panel_btn = panel_btn
            .shortcut_hint("Alt+Shift+P")
            .checked(bar.details == Details::Open);
        if let Some(press) = bar.on_details {
            panel_btn = panel_btn.on_press_tx(move |s: &mut State, tx: &mut rune::Tx| press(s, tx));
        }
    }

    // Open in File Explorer (board b4ffef881): the last icon button, never dimmed.
    let on_explorer: ExplorerPress = std::rc::Rc::from(bar.on_explorer);
    let on_reveal = on_explorer.clone();
    let mut explorer_btn = tool_button(
        "open-explorer",
        "Open in File Explorer",
        &crate::theme::icons_settings::FOLDER,
        16.0,
        ToolState::Idle,
        t,
    )
    .help("Open in File Explorer")
    .on_press_tx(move |s: &mut State, tx: &mut rune::Tx| on_explorer(s, tx));
    if bar.gate_closed {
        explorer_btn = explorer_btn.disabled(true).focusable(false);
    }

    let sep = el()
        .key(if bar.slot_sep { "toolbar-sep-2" } else { "toolbar-sep" })
        .w(kit::HAIR)
        .h(18.0)
        .mx(2.0)
        .fill(t.separator);

    let new_btn = row()
        .key("new-btn")
        .expanded(bar.new == New::Open)
        // Ringed while its menu is open (DW-0414: `box-shadow: 0 0 0 3px` amber at 32 %).
        .box_shadow(kit::ring(
            if bar.new == New::Open { 3.0 } else { 0.0 },
            t.accent.alpha(0.32),
        ))
        .role("button")
        .label("New")
        .h(28.0)
        .pl(8.0)
        .pr(9.0)
        .gap(5.0)
        .center_y()
        .radius(radius::CONTROL)
        .fill(t.accent)
        .child(kit::glyph("plus-icon", &icons::PLUS, 14.0, t.accent_text_on))
        .child(kit::label("new-label", "New", type_::LABEL, t.accent_text_on))
        .child(kit::glyph("new-caret", &icons::CHEVRON_DOWN, 11.0, t.accent_text_on));
    let new_btn = match (bar.new, bar.on_new) {
        // `.mk-new.is-off`: 40 %, out of reach.
        (New::Off, _) => new_btn.opacity(0.4).disabled(true),
        (_, on_new) => {
            let live = new_btn.hover(t.press).press(t.press).focusable(true);
            match on_new {
                Some(press) => live.on_press_tx(move |s: &mut State, tx: &mut rune::Tx| press(s, tx)),
                None => live,
            }
        }
    };

    let out = row()
        .key("toolbar")
        .role("toolbar")
        .label(bar.name)
        .h(46.0)
        .w_full()
        .fill(t.toolbar_bg)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .pl(10.0)
        .pr(bar.trailing_inset.max(12.0))
        .center_y()
        .gap(8.0)
        .child(nav)
        .child(bar.title)
        .child(el().key("toolbar-spacer").grow(1.0))
        .children(bar.slot);
    if file {
        // One action for the file's folder: Show in File Explorer (Finder), the file selected.
        let reveal = tool_button(
            "show-in-folder",
            crate::frames::show_in_folder::label(),
            &crate::theme::icons_settings::FOLDER,
            16.0,
            ToolState::Idle,
            t,
        )
        .help(crate::frames::show_in_folder::label())
        .on_press_tx(move |s: &mut State, tx: &mut rune::Tx| on_reveal(s, tx));
        return out.child(panel_btn).child(reveal);
    }
    out.child(view_toggle)
        .child(sort_btn)
        .child(panel_btn)
        .child(explorer_btn)
        .child(sep)
        .child(new_btn)
        .child(bar.search)
}

/// A keyboard hint (`.kit-kbd`): "Ctrl+F".
pub fn kbd(key: &'static str, keys: &str, t: &Theme) -> El {
    el().key(format!("{key}-box"))
        .h(20.0)
        .px(6.0)
        .radius(6.0)
        .fill(t.card)
        .box_shadow(kit::ring_inset(t.separator))
        .box_shadow(kit::drop(1.0, 0.0, t.separator))
        .center()
        .child(kit::label(key, keys, type_::KBD, t.text_tertiary))
}

/// What the toolbar says while the session's gate is closed (DW-0009 locked, DW-0014 no plan
/// yet): its own heading, and every control drawn but out of reach (the board dims them).
pub struct Closed<'a> {
    pub title: &'a str,
    /// The line under the title ("No plan yet"); none while locked.
    pub sub: Option<&'a str>,
}

pub fn toolbar(s: &State, t: &Theme, trailing_inset: f32) -> El {
    toolbar_in(s, t, trailing_inset, None)
}

/// The toolbar; with `closed`, as it stands while the gate is closed: the one [`bar`], every
/// control drawn in its dimmed state.
pub fn toolbar_in(s: &State, t: &Theme, trailing_inset: f32, closed: Option<&Closed<'_>>) -> El {
    let off = closed.is_some();
    let item_count = s.listing.items.len();
    let (title, info) = match closed {
        Some(c) => (c.title.to_string(), c.sub.map(str::to_string)),
        None if item_count == 1 => (s.listing.folder.name.clone(), Some("1 item".to_string())),
        None => (s.listing.folder.name.clone(), Some(format!("{item_count} items"))),
    };
    let search = if off {
        // Nothing is searched while the gate is closed: the field's words, and no input.
        kit::note_text(
            "search-placeholder",
            "search-input",
            SEARCH_PLACEHOLDER,
            type_::SEARCH,
            t.text_tertiary,
        );
        search_field_box(
            row().key("search-input").a11y_hidden().grow(1.0).center_y().child(
                rune::text(SEARCH_PLACEHOLDER)
                    .font(type_::SEARCH)
                    .color(t.text_tertiary),
            ),
            t,
        )
    } else {
        search_box(
            "search-input",
            SEARCH_PLACEHOLDER,
            text_input(&s.search, type_::SEARCH)
                .key("search-input")
                .label("Search names")
                .shortcut_hint("Ctrl+F")
                .single_line()
                .color(t.text)
                .placeholder(SEARCH_PLACEHOLDER, t.text_tertiary)
                .on_edit(|s: &mut State| &mut s.search)
                .grow(1.0),
            t,
        )
    };
    let back: Option<Press> = press(|s: &mut State, _| {
        s.go_back();
    });
    let on_explorer: Press = if off {
        Box::new(|_, _| {})
    } else {
        Box::new(open_beebeeb_location)
    };
    bar(
        Bar {
            name: "Files".to_string(),
            back: (s.can_go_back() && !off, back),
            forward: (false, None),
            title: title_block(&title, info.as_deref(), t),
            slot: None,
            slot_sep: false,
            view: if off { View::NotHere } else { View::List },
            on_view: None,
            sort: if off { Sort::NotHere } else { Sort::Idle },
            on_sort: None,
            details: if off {
                Details::NotHere
            } else if s.show_detail_pane {
                Details::Open
            } else {
                Details::Closed
            },
            on_details: if off {
                None
            } else {
                press(|s: &mut State, _| s.show_detail_pane = !s.show_detail_pane)
            },
            inert: off,
            gate_closed: off,
            on_explorer,
            new: if off { New::Off } else { New::Idle },
            on_new: None,
            search,
            trailing_inset,
        },
        t,
    )
}
