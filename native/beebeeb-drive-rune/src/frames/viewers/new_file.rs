//! New file (DW-1520 to DW-1522): New's menu in the Files window offers "New file…" (Ctrl+N), and
//! this dialog, over the Files window, picks the type (DW-1521) and names the file (DW-1522).
//!
//! **Nothing is made here.** Create file is an [`Intent::CreateFile`] the engine lane takes: the
//! file is made in the folder shown, encrypted on this PC before it uploads, and opened in the
//! editor (DW-1523). Word, Excel and PowerPoint go on to New document (DW-1558) as an
//! [`Intent::NewDocument`]. This group writes no file and keeps no name it was not asked to.
//!
//! The dialog is drawn over whatever the Files window shows ([`over`], the registry's one line)
//! and holds the keyboard while it is open: a key it does not use stops at it, so nothing typed
//! here reaches the list under it (Delete there would move the selected file to Trash).

use rune::{Color, Cx, El, Key, KeyEvent, Shadow, TextEdit, TextStyle, Tx, col, el, row, text_input};

use super::{Intent, style};
use crate::State;
use crate::frames::files::names::{self, Host, NameError};
use crate::shell::app::{Screen, Section};
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::dialog::{self, Sub};
use crate::ui::settings_dialog::{Weight, button};
use crate::ui::{field, kit};

/// What a new file can be (DW-1521), in the board's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Markdown,
    Text,
    Word,
    Excel,
    PowerPoint,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Markdown, Kind::Text, Kind::Word, Kind::Excel, Kind::PowerPoint];

    /// The type's word under its tile.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Markdown => "Markdown",
            Kind::Text => "Text",
            Kind::Word => "Word",
            Kind::Excel => "Excel",
            Kind::PowerPoint => "PowerPoint",
        }
    }

    /// The extension a new file of this type gets.
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Markdown => ".md",
            Kind::Text => ".txt",
            Kind::Word => ".docx",
            Kind::Excel => ".xlsx",
            Kind::PowerPoint => ".pptx",
        }
    }

    /// An office document: made from a template in New document (DW-1558), not named here.
    pub fn office(self) -> bool {
        matches!(self, Kind::Word | Kind::Excel | Kind::PowerPoint)
    }

    /// The naming dialog's title (DW-1522).
    fn title(self) -> &'static str {
        match self {
            Kind::Markdown => "New Markdown file",
            _ => "New text file",
        }
    }

    fn index(self) -> usize {
        Kind::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

/// The extensions a text file is offered (DW-1522), in the board's order.
pub const TEXT_EXTENSIONS: [&str; 7] = [".txt", ".py", ".js", ".json", ".sh", ".log", ".csv"];

/// The name a new file starts with, selected so typing replaces it.
pub const UNTITLED: &str = "Untitled";

/// The key of the name's text field, and of the extension's.
pub const NAME_INPUT: &str = "new-file-name";
pub const EXT_INPUT: &str = "new-file-ext";

/// Which question the dialog asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// DW-1521: what to make.
    Pick,
    /// DW-1522: its name, and its extension as a field of its own.
    Name,
}

/// The New file dialog while it is open (`State::viewer`).
pub struct NewFile {
    pub step: Step,
    /// The type picked (outlined in ink: selection is neutral, never Amber).
    pub kind: Kind,
    /// The folder the file is made in: the one Files showed when New was pressed (`None`: the
    /// top of Files).
    pub folder: Option<String>,
    pub name: TextEdit,
    pub ext: TextEdit,
    /// Why the name as it was when Create file was pressed can't be given. Typing takes it away.
    pub error: Option<(String, NameError)>,
    /// The control that takes the keyboard once this change is on screen.
    focus: Option<&'static str>,
}

impl NewFile {
    fn new(folder: Option<String>) -> NewFile {
        NewFile {
            step: Step::Pick,
            kind: Kind::Markdown,
            folder,
            name: TextEdit::new(""),
            ext: TextEdit::new(""),
            error: None,
            focus: Some(type_key(0)),
        }
    }

    /// The name as it would be made: the name, then the extension (a dot added if it has none).
    pub fn full_name(&self) -> String {
        let ext = self.ext.text.trim();
        let ext = if ext.is_empty() || ext.starts_with('.') {
            ext.to_string()
        } else {
            format!(".{ext}")
        };
        format!("{}{ext}", self.name.text.trim())
    }
}

/// The key of a type's tile (the focus target is a `&'static str`).
fn type_key(i: usize) -> &'static str {
    ["type-0", "type-1", "type-2", "type-3", "type-4"]
        .get(i)
        .copied()
        .unwrap_or("type-0")
}

fn chip_key(i: usize) -> &'static str {
    ["chip-0", "chip-1", "chip-2", "chip-3", "chip-4", "chip-5", "chip-6"]
        .get(i)
        .copied()
        .unwrap_or("chip-0")
}

fn ui(s: &mut State) -> Option<&mut NewFile> {
    s.viewer.new_file.as_mut()
}

/// The Files window is on screen, behind no settings and no open file: where the dialog belongs.
fn in_files(s: &State) -> bool {
    matches!(s.shell.app.screen(), Screen::Drive(route) if route.item.is_none() && route.section == Section::Files)
}

/// New file… (DW-1520's item, Ctrl+N): the dialog on the type, Markdown chosen. `false`: the
/// folder shown takes no new files (Backups, a service's folder: New is dimmed there).
pub fn begin(s: &mut State) -> bool {
    if !in_files(s) || !s.can_add() {
        return false;
    }
    s.actions.menu = None;
    s.viewer.new_file = Some(NewFile::new(s.listing.folder.id.clone()));
    true
}

/// Cancel, Esc, a press on the scrim: the dialog closes; nothing was made.
pub fn close(s: &mut State) {
    s.viewer.new_file = None;
}

/// Choose a type (a press on its tile, or the arrows).
pub fn choose(s: &mut State, kind: Kind) {
    if let Some(nf) = ui(s) {
        nf.kind = kind;
        nf.focus = Some(type_key(kind.index()));
    }
}

fn step_kind(s: &mut State, delta: isize) {
    let Some(kind) = s.viewer.new_file.as_ref().map(|nf| nf.kind) else {
        return;
    };
    let n = Kind::ALL.len() as isize;
    let next = (kind.index() as isize + delta).rem_euclid(n) as usize;
    choose(s, Kind::ALL[next]);
}

/// Continue (DW-1521): Markdown and Text are named next (DW-1522); an office type goes on to New
/// document with that app's templates (DW-1558), which the shell opens.
pub fn go_on(s: &mut State) {
    let Some(nf) = ui(s) else { return };
    if nf.kind.office() {
        let kind = nf.kind;
        close(s);
        s.viewer.asked.push(Intent::NewDocument(kind));
        return;
    }
    nf.step = Step::Name;
    nf.name = TextEdit::new(UNTITLED);
    nf.name.set_selection(0, UNTITLED.len());
    nf.ext = TextEdit::new(nf.kind.ext());
    nf.error = None;
    nf.focus = Some(NAME_INPUT);
}

/// A suggested extension (DW-1522's chips): it goes into the extension's field.
pub fn choose_ext(s: &mut State, ext: &str) {
    if let Some(nf) = ui(s) {
        nf.ext = TextEdit::new(ext);
        nf.error = None;
    }
}

/// Create file (DW-1522): the name is checked by the rules of Rename (DW-0410b) against what the
/// folder holds; a name that can be given is asked of the engine ([`Intent::CreateFile`]) and the
/// dialog closes. Nothing is written here.
pub fn create(s: &mut State) {
    let Some(nf) = s.viewer.new_file.as_ref() else { return };
    if nf.step != Step::Name {
        return;
    }
    let full = nf.full_name();
    let typed = nf.name.text.clone();
    let others = s.listing.items.iter().map(|i| (i.name.as_str(), i.is_folder));
    let checked = names::check_on(Host::this(), &full, others);
    match checked {
        Ok(name) => {
            let folder = nf.folder.clone();
            close(s);
            s.viewer.asked.push(Intent::CreateFile { folder, name });
        }
        Err(why) => {
            if let Some(nf) = ui(s) {
                nf.error = Some((typed, why));
                nf.focus = Some(NAME_INPUT);
            }
        }
    }
}

/// The control the dialog wants focused once its change is on screen (for `viewers::poll`).
pub fn take_focus(s: &mut State) -> Option<&'static str> {
    ui(s).and_then(|nf| nf.focus.take())
}

/// The route left Files (another section, settings, a file): the dialog goes with it.
pub fn sync(s: &mut State) {
    if s.viewer.new_file.is_some() && !in_files(s) {
        close(s);
    }
}

/// The dialog's keys while focus is on one of its controls (a tile, a chip, a button): the arrows
/// choose, Esc closes; Tab, Space and Enter are Rune's (move focus, press). Every other key stops
/// here: none reaches the Files window under the dialog.
fn dialog_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(step) = s.viewer.new_file.as_ref().map(|nf| nf.step) else {
        return false;
    };
    let delta = match &ev.key {
        Key::Tab | Key::Enter => return false,
        Key::Text(t) if t == " " && !ev.mods.cmd && !ev.mods.alt => return false,
        Key::Escape => {
            close(s);
            return true;
        }
        Key::Left | Key::Up => -1,
        Key::Right | Key::Down => 1,
        _ => return true,
    };
    if ev.mods.cmd || ev.mods.alt {
        return true;
    }
    let focused = tx.focused().map(str::to_string);
    match step {
        Step::Pick if focused.as_deref().is_some_and(|k| k.starts_with("type-")) => step_kind(s, delta),
        Step::Name => {
            if let Some(i) = focused
                .as_deref()
                .and_then(|k| k.strip_prefix("chip-"))
                .and_then(|n| n.parse::<usize>().ok())
            {
                let n = TEXT_EXTENSIONS.len() as isize;
                let next = (i as isize + delta).rem_euclid(n) as usize;
                choose_ext(s, TEXT_EXTENSIONS[next]);
                if let Some(nf) = ui(s) {
                    nf.focus = Some(chip_key(next));
                }
            }
        }
        Step::Pick => {}
    }
    if let Some(key) = take_focus(s) {
        tx.focus(key, true);
    }
    true
}

/// A field's keys before the field edits (Rune gives a focused field's keys to no ancestor): Esc
/// closes; what a one-line field edits, Tab and Enter (Create file, the default button) go on;
/// anything else (F2, Ctrl+N, Up, Page Down …) stops here, so it never reaches the list behind.
fn field_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    match &ev.key {
        Key::Escape => {
            close(s);
            true
        }
        Key::Text(_)
        | Key::Left
        | Key::Right
        | Key::Home
        | Key::End
        | Key::Backspace
        | Key::Delete
        | Key::Tab
        | Key::Enter => false,
        Key::Command(c) => !matches!(c, 'a' | 'c' | 'x' | 'v' | 'z' | 'y'),
        _ => true,
    }
}

fn name_edit(s: &mut State) -> &mut TextEdit {
    match s.viewer.new_file.as_mut() {
        Some(nf) => &mut nf.name,
        None => &mut s.search,
    }
}

fn ext_edit(s: &mut State) -> &mut TextEdit {
    match s.viewer.new_file.as_mut() {
        Some(nf) => &mut nf.ext,
        None => &mut s.search,
    }
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// The extension inside a type's document outline: mono 10px on a 17px line.
const DOC_EXT: TextStyle = TextStyle::new(10.0, 400.0).family(rune::tok::Family::Mono).lh(17.0);
/// The extension's field (`.kit-field-value.font-mono`): mono 11.5px on a 17px line.
const EXT_VALUE: TextStyle = TextStyle::new(11.5, 400.0).family(rune::tok::Family::Mono).lh(17.0);
/// A suggested extension (`.kit-chip`): 12px/600 on a 17px line.
const CHIP: TextStyle = TextStyle::new(12.0, 600.0).lh(17.0);
/// A type's tile: the dialog's 512 across five, 12 apart.
const TILE_W: f32 = (512.0 - 4.0 * 12.0) / 5.0;
/// The outline of a document in a type's tile.
const DOC_W: f32 = 44.0;

/// Where the browser paints a box laid out at `x`, `w` wide (points, from a parent on a whole
/// device pixel): both edges rounded to the device pixel of a 200 % display, each on its own. The
/// dialog's card is drawn at its exact place, a quarter point down as the board's
/// (`pixel_snap(false)`), so the boxes in it are put on the board's edges here: across, at whole
/// device pixels; down, where the card's half pixel puts them, as there.
fn snapped(x: f32, w: f32) -> (f32, f32) {
    let left = field::snap(x);
    (left, field::snap(x + w) - left)
}

/// A row of boxes at the browser's painted edges: `boxes` are (laid-out x, laid-out width, the
/// box); each is placed by its margin from the last one's painted right edge. The words in a box
/// stay where the browser draws them (text is not snapped): the caller offsets them by [`drift`].
fn snapped_row(key: &'static str, boxes: Vec<(f32, f32, El)>) -> El {
    let mut r = row().key(key);
    let mut right = 0.0;
    for (x, w, b) in boxes {
        let (left, width) = snapped(x, w);
        r = r.child(b.w(width).ml(left - right));
        right = left + width;
    }
    r
}

/// How far the middle of a box laid out at `x`, `w` wide is from the middle of where it is
/// painted: the words centred in it are moved back by this, to the browser's place.
fn drift(x: f32, w: f32) -> f32 {
    let (left, width) = snapped(x, w);
    (x + w / 2.0) - (left + width / 2.0)
}

/// An inset ring `width` wide (CSS `box-shadow: inset 0 0 0 <width>`).
fn inset(width: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y: 0.0,
        blur: 0.0,
        spread: width,
        color,
        inset: true,
    }
}

/// One type (DW-1521): a tile with the outline of a document holding the extension, the word under
/// it. `x`: where the board lays the tile out, from the body's left edge.
fn type_tile(kind: Kind, on: bool, x: f32, t: &Theme) -> El {
    let i = kind.index();
    let key = type_key(i);
    let (tile_left, _) = snapped(x, TILE_W);
    let doc_x = x + (TILE_W - DOC_W) / 2.0;
    let (doc_left, _) = snapped(doc_x, DOC_W);
    let doc = el()
        .key(format!("{key}-doc"))
        .size(DOC_W, 54.0)
        .ml(doc_left - tile_left)
        .radius(4.0)
        .box_shadow(inset(1.5, t.text_tertiary))
        .center()
        .child(kit::label(format!("{key}-ext"), kind.ext(), DOC_EXT, t.text_secondary).offset(doc_x - doc_left, 0.0));
    let tile = col()
        .key(key)
        .role("radio")
        .label(format!("{}, {}", kind.word(), kind.ext()))
        .checked(on)
        .h(101.0)
        .py(12.0)
        .gap(8.0)
        .items_start()
        .radius(10.0)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(doc)
        .child(
            row().key(format!("{key}-word")).w_full().justify_center().child(
                kit::label(
                    format!("type-{i}-label"),
                    kind.word(),
                    style::SUB,
                    if on { t.text_secondary } else { t.text_tertiary },
                )
                .offset(drift(x, TILE_W), 0.0),
            ),
        )
        .on_press(move |s: &mut State| choose(s, kind))
        .on_double_press(move |s: &mut State| {
            choose(s, kind);
            go_on(s);
        });
    if on {
        tile.fill(t.selection).box_shadow(inset(2.0, t.text))
    } else {
        tile.box_shadow(kit::ring_inset(t.separator)).hover(t.hover)
    }
}

/// DW-1521: what to make.
fn pick(nf: &NewFile, t: &Theme) -> El {
    let label = kit::label("dlg-label", "PICK A TYPE", type_::SECTION_LABEL, t.text_tertiary).w_full();
    let tiles = snapped_row(
        "types",
        Kind::ALL
            .into_iter()
            .enumerate()
            .map(|(i, kind)| {
                let x = i as f32 * (TILE_W + 12.0);
                (x, TILE_W, type_tile(kind, kind == nf.kind, x, t))
            })
            .collect(),
    )
    .role("radiogroup")
    .label("Pick a type")
    .w_full();
    let foot = row()
        .center_y()
        .justify_end()
        .child(
            button(0, "Continue", Weight::Primary, t)
                .default_button()
                .on_press(go_on),
        )
        .child(button(1, "Cancel", Weight::Quiet, t).on_press(close));
    dialog::card(
        "New file",
        560.0,
        "New file",
        Some(Sub::Wrap("Encrypted on this PC before it leaves")),
        vec![label, tiles],
        foot,
        t,
    )
}

/// The extension's field: its own, beside the name (`.kit-field`, recessed at rest).
fn ext_field(nf: &NewFile, t: &Theme) -> El {
    let focused = nf.ext.focused;
    kit::note_text("field-1-value", "field-1-value-box", &nf.ext.text, EXT_VALUE, t.text);
    let input = text_input(&nf.ext, EXT_VALUE)
        .key(EXT_INPUT)
        .label("Extension")
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, field::selection(t))
        .selection_style(rune::SelectionStyle::new(field::selection(t)))
        .on_edit(ext_edit)
        .on_input_key(field_key)
        .grow(1.0)
        .min_w(0.0);
    let main = row()
        .key("field-1-main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center_y()
        .child(input)
        // The value's box, for the gate: the board's value span is as wide as the field's inside.
        .child(
            el().key("field-1-value-box")
                .a11y_hidden()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full(),
        );
    let well = row()
        .key("field-1")
        .w(90.0)
        .h(28.0)
        .px(8.0)
        .center_y()
        .radius(6.0)
        .fill(if focused { t.card } else { t.field })
        .border(1.0, if focused { t.accent_deep } else { Color::TRANSPARENT })
        .border_layout()
        .child(main);
    if focused {
        field::ringed(well, "field-1-ring", 3.0, 6.0, t.accent.alpha(0.30))
    } else {
        well.box_shadow(kit::ring_inset(t.separator))
    }
}

/// A suggested extension (`.kit-chip.tone-neutral`); the chosen one is ink with paper words. `x`, `w`:
/// where the board lays it out (its words' width and 10 on each side).
fn chip(i: usize, ext: &'static str, on: bool, x: f32, w: f32, t: &Theme) -> El {
    let key = chip_key(i);
    row()
        .key(key)
        .role("radio")
        .label(ext)
        .checked(on)
        .h(26.0)
        .center()
        .radius(13.0)
        .fill(if on { t.text } else { t.tile })
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(
            style::fitted(
                format!("{key}-label"),
                ext,
                CHIP,
                if on { t.paper } else { t.text_secondary },
            )
            .offset(drift(x, w), 0.0),
        )
        .on_press(move |s: &mut State| choose_ext(s, ext))
}

/// DW-1522: the name, the extension as its own field, and the extensions a text file is offered.
fn name(nf: &NewFile, t: &Theme) -> El {
    let error = nf
        .error
        .as_ref()
        .filter(|(typed, _)| typed == &nf.name.text)
        .map(|(_, why)| why.message());
    let name_field = field::bare_field(
        field::Bare {
            key: "field-0",
            name: "Name",
            message: error.as_deref(),
            error: error.is_some(),
        },
        &nf.name,
        NAME_INPUT,
        name_edit,
        |input| input.on_input_key(field_key),
        t,
    );
    let fields = row()
        .key("name-row")
        .w_full()
        .gap(8.0)
        .items_start()
        .child(el().key("name-cell").grow(1.0).min_w(0.0).child(name_field))
        .child(ext_field(nf, t));
    let mut body = vec![fields];
    if nf.kind == Kind::Text {
        // The seven fit on one line of the dialog's 412.
        let mut x = 0.0;
        let mut boxes = Vec::new();
        for (i, ext) in TEXT_EXTENSIONS.into_iter().enumerate() {
            let w = rune::text::advance(ext, CHIP) + 20.0;
            boxes.push((x, w, chip(i, ext, nf.ext.text == ext, x, w, t)));
            x += w + 6.0;
        }
        body.push(
            snapped_row("chips", boxes)
                .role("radiogroup")
                .label("Extension")
                .w_full(),
        );
    }
    let foot = row()
        .center_y()
        .justify_end()
        .child(
            button(0, "Create file", Weight::Primary, t)
                .default_button()
                .on_press(create),
        )
        .child(button(1, "Cancel", Weight::Quiet, t).on_press(close));
    dialog::card(nf.kind.title(), 460.0, nf.kind.title(), None, body, foot, t)
}

/// The Files window with the New file dialog over it, while it is open; else `behind` as it is.
/// The registry's one line (`shell/registry.rs`, `view`).
pub fn over(s: &State, cx: &mut Cx<State>, behind: El) -> El {
    let Some(nf) = s.viewer.new_file.as_ref().filter(|_| in_files(s)) else {
        return behind;
    };
    let t = tokens::theme(cx.system_dark());
    let card = match nf.step {
        Step::Pick => pick(nf, &t),
        Step::Name => name(nf, &t),
    };
    // Under the caption: the window can still be moved and closed (`.mk-scrim`).
    let layer = el()
        .key("dialog-new-file")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(dialog::scrim(crate::TITLEBAR_H, card, &t))
        .on_key(dialog_key)
        .modal_tx(false, |s: &mut State, _tx: &mut Tx| close(s));
    el().key("window-with-new-file")
        .w_full()
        .h_full()
        .child(behind)
        .child(layer)
}
