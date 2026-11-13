//! Task 2477, PR 0: an editing surface drawn by the app, on Rune as the native app pins it.
//!
//! No Rune text field is involved in the surface: the "canvas" is a plain focusable element whose
//! characters, selection, composition underline and caret are elements the app places itself, one
//! fixed-width cell per character. What is being checked is the INPUT side:
//!
//! | Need | Rune item used here |
//! |---|---|
//! | key presses, typed text | `El::on_key` on a `focusable` element: `Key::Text`, `Key::Command`, arrows |
//! | input-method composition | `App::on_ime`: `Ime::Preedit(text, cursor)`, `Ime::Commit`, `Enabled`, `Disabled` |
//! | candidate-window position | `El::ime_cursor` on the caret element; headless it shows as `Runtime::ime_area` |
//! | clipboard | `Tx::copy_rich`, `Tx::read_clipboard`, `ClipboardContent` |
//! | focus | `focusable`, `Tx::focused` (read in `AppSpec::poll`), `App::on_focus` for the window |
//! | pointer | `El::on_drag`: `Phase`, element-local `x`/`y`, `clicks`; `Tx::mods` for Shift |
//!
//! The model is deliberately small (one line, one cell per `char`); it is a probe, not an editor.

use rune::{
    App, AppSpec, ClipboardContent, Color, Cx, Drag, El, Ime, Key, KeyEvent, Phase, TextEdit, TextStyle, Tx, col, el,
    text, text_input,
};

/// Width of one character cell, the line height and the inset of the first cell, in points.
pub const CELL: f32 = 16.0;
pub const LINE: f32 = 24.0;
pub const PAD: f32 = 16.0;
pub const CANVAS: (f32, f32) = (448.0, 120.0);
pub const WINDOW: (f32, f32) = (480.0, 240.0);

#[derive(Default)]
pub struct Probe {
    pub text: Vec<char>,
    /// The caret and the other end of the selection, as indexes into `text`.
    pub caret: usize,
    pub anchor: usize,
    /// The input method's composition: shown at the caret, not part of `text`.
    pub preedit: String,
    /// The byte range of the input method's own cursor inside `preedit`, as Rune hands it over.
    pub preedit_cursor: Option<(usize, usize)>,
    /// The canvas has keyboard focus. Rune has no per-element focus callback: `poll` reads `Tx::focused`.
    pub has_keyboard: bool,
    pub window_focused: bool,
    /// The caret's drawn rectangle in window points (`on_drawn_layout`), for the real-window check.
    pub caret_drawn: Option<[f32; 4]>,
    /// The canvas's drawn rectangle in window points: where the real-window script aims its pointer.
    pub canvas_drawn: Option<[f32; 4]>,
    /// Every event the probe heard, in order. The tests and the real-window script read this.
    pub log: Vec<String>,
    /// A Rune text field beside the canvas: a focused field must win the input method.
    pub field: TextEdit,
}

impl Probe {
    pub fn text(&self) -> String {
        self.text.iter().collect()
    }
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }
    pub fn selected(&self) -> String {
        let (a, b) = self.selection();
        self.text[a..b].iter().collect()
    }
    /// Characters of the composition before the input method's cursor: where the caret is drawn.
    pub fn preedit_caret_chars(&self) -> usize {
        match self.preedit_cursor {
            Some((start, _)) => self.preedit.char_indices().take_while(|(i, _)| *i < start).count(),
            None => self.preedit.chars().count(),
        }
    }
    /// The cell the caret is drawn in front of, counting the composition.
    pub fn caret_cell(&self) -> usize {
        self.caret + self.preedit_caret_chars()
    }

    fn delete_selection(&mut self) -> bool {
        let (a, b) = self.selection();
        self.text.drain(a..b);
        (self.caret, self.anchor) = (a, a);
        a != b
    }
    fn insert(&mut self, s: &str) {
        self.delete_selection();
        for (i, ch) in s.chars().enumerate() {
            self.text.insert(self.caret + i, ch);
        }
        self.caret += s.chars().count();
        self.anchor = self.caret;
    }
    fn move_to(&mut self, to: usize, extend: bool) {
        self.caret = to.min(self.text.len());
        if !extend {
            self.anchor = self.caret;
        }
    }
    /// The index a point in the canvas's own points lands on.
    pub fn hit(&self, x: f32) -> usize {
        (((x - PAD) / CELL).round().max(0.0) as usize).min(self.text.len())
    }
}

/// `El::on_key` of the canvas: typing, movement, selection and the clipboard chords.
pub fn on_key(s: &mut Probe, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // Rune animates the layout change of a key, input-method or press commit unless the handler says otherwise:
    // a caret that glides takes the candidate window with it. Every handler of an editor says `instant`.
    tx.instant();
    s.log
        .push(format!("key {:?} shift={} cmd={}", ev.key, ev.mods.shift, ev.mods.cmd));
    let shift = ev.mods.shift;
    match &ev.key {
        Key::Text(t) => s.insert(t),
        Key::Backspace => {
            if !s.delete_selection() && s.caret > 0 {
                s.text.remove(s.caret - 1);
                s.move_to(s.caret - 1, false);
            }
        }
        Key::Delete => {
            if !s.delete_selection() && s.caret < s.text.len() {
                s.text.remove(s.caret);
            }
        }
        Key::Left => s.move_to(s.caret.saturating_sub(1), shift),
        Key::Right => s.move_to(s.caret + 1, shift),
        Key::Home => s.move_to(0, shift),
        Key::End => s.move_to(s.text.len(), shift),
        Key::Command('a') => (s.anchor, s.caret) = (0, s.text.len()),
        Key::Command(c @ ('c' | 'x')) => {
            let selected = s.selected();
            if selected.is_empty() {
                return true;
            }
            // Plain text for every app, and an HTML form beside it: what Rune writes of the pair is its answer.
            let html = format!("<b>{}</b>", selected.replace('&', "&amp;").replace('<', "&lt;"));
            tx.copy_rich(ClipboardContent::new(selected.clone()).with_html(html));
            s.log.push(format!("copy {selected:?}"));
            if *c == 'x' {
                s.delete_selection();
            }
        }
        Key::Command('v') => tx.read_clipboard(|s: &mut Probe, content: &ClipboardContent, tx| {
            tx.instant();
            s.log.push(format!(
                "paste text={:?} html={} rtf={} image={} files={}",
                content.text(),
                content.html().is_some(),
                content.rtf().is_some(),
                content.image().is_some(),
                content.files().len()
            ));
            if !content.text().is_empty() {
                s.insert(&content.text().to_owned());
            }
        }),
        // Tab, Escape, function keys and the rest stay Rune's (focus navigation, the window's `on_key`).
        _ => return false,
    }
    true
}

/// `App::on_ime`: the composition of the app-drawn editor.
pub fn on_ime(s: &mut Probe, ev: &Ime, tx: &mut Tx) -> bool {
    tx.instant();
    s.log.push(format!("ime {ev:?}"));
    match ev {
        Ime::Preedit(t, cursor) => {
            // The first non-empty pre-edit is the start: it replaces the selection, as typing does.
            if s.preedit.is_empty() && !t.is_empty() {
                s.delete_selection();
            }
            s.preedit = t.clone();
            s.preedit_cursor = if t.is_empty() { None } else { *cursor };
        }
        Ime::Commit(t) => {
            s.preedit.clear();
            s.preedit_cursor = None;
            s.insert(t);
        }
        Ime::Enabled | Ime::Disabled => {}
    }
    true
}

/// `El::on_drag` of the canvas: press places the caret, a drag extends the selection, Shift extends on press,
/// a second click of a run selects everything.
pub fn on_drag(s: &mut Probe, d: &mut Drag, tx: &mut Tx) {
    tx.instant();
    s.log.push(format!(
        "pointer {:?} x={:.0} y={:.0} clicks={} shift={}",
        d.phase,
        d.x,
        d.y,
        d.clicks,
        tx.mods().shift
    ));
    let at = s.hit(d.x);
    match d.phase {
        Phase::Down if d.clicks >= 2 => (s.anchor, s.caret) = (0, s.text.len()),
        Phase::Down => s.move_to(at, tx.mods().shift),
        Phase::Move => s.move_to(at, true),
        Phase::Up if d.cancelled() => s.anchor = s.caret,
        Phase::Up => {}
    }
}

/// `AppSpec::poll`, at the start of every frame: the only place the app can ask who has the keyboard.
pub fn poll(s: &mut Probe, _: f64, tx: &mut Tx) -> bool {
    let now = tx.focused() == Some("canvas");
    if now == s.has_keyboard {
        return false;
    }
    s.has_keyboard = now;
    tx.instant();
    s.log.push(format!("focus canvas={now}"));
    true
}

pub fn on_window_focus(s: &mut Probe, focused: bool, _: &mut Tx) {
    s.window_focused = focused;
    s.log.push(format!("window focus={focused}"));
}

const INK: Color = Color {
    r: 0.10,
    g: 0.10,
    b: 0.12,
    a: 1.0,
};
const PAPER: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
const DESK: Color = Color {
    r: 0.93,
    g: 0.93,
    b: 0.94,
    a: 1.0,
};
const SELECTION: Color = Color {
    r: 0.70,
    g: 0.80,
    b: 1.0,
    a: 1.0,
};

pub fn view(s: &Probe, _: &mut Cx<Probe>) -> El {
    let x = |cell: usize| PAD + cell as f32 * CELL;
    let preedit: Vec<char> = s.preedit.chars().collect();
    // What is drawn: the text with the composition spliced in at the caret.
    let shown = s.text[..s.caret].iter().chain(&preedit).chain(&s.text[s.caret..]);
    let mut canvas = el()
        .key("canvas")
        .size(CANVAS.0, CANVAS.1)
        .fill(PAPER)
        .focusable(true)
        .on_key(on_key)
        .on_drag(on_drag);
    canvas = canvas.on_drawn_layout(|s: &mut Probe, r| s.canvas_drawn = Some(r));
    let (a, b) = s.selection();
    if a != b {
        canvas = canvas.child(
            el().key("selection")
                .absolute(x(a), PAD)
                .size((b - a) as f32 * CELL, LINE)
                .fill(SELECTION),
        );
    }
    for (i, ch) in shown.enumerate() {
        canvas = canvas.child(
            text(ch.to_string())
                .font(TextStyle::new(16.0, 400.0))
                .color(INK)
                .absolute(x(i), PAD)
                .size(CELL, LINE),
        );
    }
    if !preedit.is_empty() {
        // The composition is underlined, as every platform draws marked text.
        canvas = canvas.child(
            el().key("preedit")
                .absolute(x(s.caret), PAD + LINE - 2.0)
                .size(preedit.len() as f32 * CELL, 2.0)
                .fill(INK),
        );
    }
    if s.has_keyboard {
        // The caret the app draws. `ime_cursor` is the whole request to the platform: input method on, candidate
        // window at this element's drawn rectangle.
        canvas = canvas.child(
            el().key("caret")
                .absolute(x(s.caret_cell()), PAD)
                .size(2.0, LINE)
                .fill(INK)
                .ime_cursor()
                .on_drawn_layout(|s: &mut Probe, r| s.caret_drawn = Some(r)),
        );
    }
    col()
        .w_full()
        .h_full()
        .fill(DESK)
        .child(
            text_input(&s.field, TextStyle::new(14.0, 400.0))
                .single_line()
                .key("field")
                .size(200.0, 32.0)
                .on_edit(|s: &mut Probe| &mut s.field),
        )
        .child(
            el().key("other")
                .size(80.0, 32.0)
                .focusable(true)
                .on_press(|s: &mut Probe| s.log.push("press other".into())),
        )
        .child(canvas)
}

pub fn spec() -> AppSpec<Probe> {
    AppSpec {
        poll: Some(poll),
        ..AppSpec::new("Rune input check", WINDOW, Probe::default, view)
    }
}

/// The app as both the tests and the real window build it. `on_ime` is the registration under test.
pub fn app(spec: AppSpec<Probe>) -> App<Probe> {
    App::new(spec).on_ime(on_ime).on_focus(on_window_focus)
}
