//! New Word, Excel and PowerPoint documents (DW-1558): Docs, Sheets and Slides, a template for Docs, a name.
//!
//! The dialog lives in the New file dialog's slot (`State::viewer.new_file`, `Step::Document`), so it is drawn over
//! Files, holds the keyboard and goes with the route as that one does. **Nothing is made here**: Create asks the
//! window's source (`ViewerSource::create_document`): the helper builds the document from the template, the engine
//! uploads it encrypted as a new file in the folder shown, and the answer comes by `settled`; the new file is then
//! opened in the editor from that same source. A refusal (or a failed creation) is said in the dialog, which stays
//! open on the same choices, in the words of every failed action ("Couldn't make the file.").
//!
//! Must not: log or keep a name beyond the dialog.

use beebeeb_office_protocol::office::{EditKind, Template};
use rune::{El, Key, KeyEvent, TextStyle, Tx, col, row};

use crate::State;
use crate::frames::files::Toast;
use crate::frames::files::names::{self, NameError};
use crate::frames::files::source::{Action, ActionError};
use crate::frames::viewers::new_file::{self, Kind, NAME_INPUT, NewFile, Step};
use crate::frames::viewers::source::{Settled, ViewerSource};
use crate::theme::tokens::{Theme, type_};
use crate::ui::dialog::{self, Sub};
use crate::ui::field;
use crate::ui::kit;
use crate::ui::settings_dialog::{Weight, button};

/// The name every new document starts with (the part that is selected, so typing replaces it).
pub const UNTITLED: &str = "Untitled document";

pub const TITLE: &str = "New document";
pub const ENCRYPTED: &str = "Everything you create here is encrypted before it leaves this device.";

/// The templates of Docs, with the web editor's descriptions.
pub const TEMPLATES: [(Template, &str, &str); 5] = [
    (Template::Blank, "Blank", "Start from nothing"),
    (Template::Letter, "Letter", "Formal letter, address block"),
    (Template::Invoice, "Invoice", "Line items, tax, totals"),
    (Template::MeetingNotes, "Meeting notes", "Agenda, notes, action items"),
    (Template::Report, "Report", "Title page, sections, summary"),
];

/// The three apps, in the board's order: the type, its tab word, and the kind the helper edits.
const APPS: [(Kind, &str, EditKind); 3] = [
    (Kind::Word, "Docs", EditKind::Docx),
    (Kind::Excel, "Sheets", EditKind::Xlsx),
    (Kind::PowerPoint, "Slides", EditKind::Pptx),
];

const TAB: TextStyle = TextStyle::new(12.0, 600.0).lh(17.0);
const ROW_TITLE: TextStyle = TextStyle::new(13.0, 600.0).lh(17.0);

/// New document's state while its dialog is open.
pub struct Doc {
    pub template: Template,
    /// The source that makes the document and then shows it. `None`: the window has none (nothing can be made).
    source: Option<Box<dyn ViewerSource>>,
    /// Create was taken by the source and has not answered yet.
    pub creating: bool,
    /// Why the last Create did not make a document, in a sentence.
    pub failed: Option<String>,
}

impl Doc {
    pub fn new(source: Option<Box<dyn ViewerSource>>) -> Doc {
        Doc {
            template: Template::Blank,
            source,
            creating: false,
            failed: None,
        }
    }
}

/// The type's app. An office type only; anything else is Docs.
fn app(kind: Kind) -> (Kind, &'static str, EditKind) {
    APPS.iter().copied().find(|a| a.0 == kind).unwrap_or(APPS[0])
}

pub fn edit_kind(kind: Kind) -> EditKind {
    app(kind).2
}

/// The name a new document of `kind` starts with: "Untitled document.docx", `.xlsx`, `.pptx`.
pub fn default_name(kind: Kind) -> String {
    format!("{UNTITLED}{}", app(kind).0.ext())
}

/// New document, the entry of Files' New menu and of New file's Continue on an office type (`kind` `None`: Docs).
/// `folder`: the folder the document is made in (`None`: the top of Files).
pub fn open(s: &mut State, _tx: &mut Tx, kind: Option<Kind>, folder: Option<String>) {
    begin(s, kind, folder);
}

/// [`open`] without the keyboard's handle (the board's scene). `false`: not here (see `new_file::begin_document`).
pub fn begin(s: &mut State, kind: Option<Kind>, folder: Option<String>) -> bool {
    new_file::begin_document(s, app(kind.unwrap_or(Kind::Word)).0, folder)
}

/// Run `f` on the dialog and its New document state, when New document is what is open.
fn with<R>(s: &mut State, f: impl FnOnce(&mut NewFile, &mut Doc) -> R) -> Option<R> {
    let nf = new_file::ui(s).filter(|nf| nf.step == Step::Document)?;
    let mut doc = nf.doc.take()?;
    let out = f(nf, &mut doc);
    nf.doc = Some(doc);
    Some(out)
}

fn app_index(kind: Kind) -> usize {
    APPS.iter().position(|a| a.0 == kind).unwrap_or(0)
}

fn tab_key(i: usize) -> &'static str {
    ["doc-tab-0", "doc-tab-1", "doc-tab-2"]
        .get(i)
        .copied()
        .unwrap_or("doc-tab-0")
}

fn tpl_key(i: usize) -> &'static str {
    ["tpl-0", "tpl-1", "tpl-2", "tpl-3", "tpl-4"]
        .get(i)
        .copied()
        .unwrap_or("tpl-0")
}

/// A tab (Docs, Sheets, Slides): another app; the template goes back to Blank. A name still the default follows.
pub fn choose_kind(s: &mut State, kind: Kind) {
    with(s, |nf, d| {
        if d.creating || nf.kind == kind || !kind.office() {
            return;
        }
        let untouched = nf.name.text == default_name(nf.kind);
        nf.kind = kind;
        d.template = Template::Blank;
        d.failed = None;
        nf.error = None;
        if untouched {
            nf.name = rune::TextEdit::new(default_name(kind));
            nf.name.set_selection(0, UNTITLED.len());
        }
        nf.focus = Some(tab_key(app_index(kind)));
    });
}

/// A template of Docs.
pub fn choose_template(s: &mut State, template: Template) {
    with(s, |nf, d| {
        if d.creating || nf.kind != Kind::Word {
            return;
        }
        d.template = template;
        d.failed = None;
        nf.focus = Some(tpl_key(TEMPLATES.iter().position(|t| t.0 == template).unwrap_or(0)));
    });
}

/// The name as it is made: checked as New file's names are (against what the folder shows), and given the app's
/// extension when it was left off.
fn checked_name(s: &State, typed: &str, kind: Kind) -> Result<String, NameError> {
    let others = || s.listing.items.iter().map(|i| (i.name.as_str(), i.is_folder));
    let ext = kind.ext();
    let name = names::check_on(s.actions.host, typed, others())?;
    let name = if name.to_lowercase().ends_with(ext) {
        name
    } else {
        format!("{name}{ext}")
    };
    // The extension alone is not a name (it would be a hidden file).
    if name.len() <= ext.len() {
        return Err(NameError::Empty);
    }
    names::check_on(s.actions.host, &name, others())
}

/// Create: the name is checked, then asked of the source. The dialog stays until the answer comes ([`poll`]).
pub fn create(s: &mut State) {
    let Some((typed, kind)) = with(s, |nf, d| (!d.creating).then(|| (nf.name.text.clone(), nf.kind))).flatten() else {
        return;
    };
    let checked = checked_name(s, &typed, kind);
    with(s, |nf, d| match checked {
        Err(why) => {
            nf.error = Some((typed, why));
            nf.focus = Some(NAME_INPUT);
        }
        Ok(name) => {
            nf.error = None;
            let template = if kind == Kind::Word {
                d.template
            } else {
                Template::Blank
            };
            let folder = nf.folder.clone();
            let taken = d
                .source
                .as_mut()
                .is_some_and(|src| src.create_document(folder.as_deref(), &name, edit_kind(kind), template));
            if taken {
                d.creating = true;
                d.failed = None;
            } else {
                d.failed = Some(Toast::failure(Action::NewFile, ActionError::Failed));
            }
        }
    });
}

/// The engine's answer, read on the window's poll (`new_file::sync`): the new file opens in the editor, a failure
/// is said in the dialog. Nothing while no creation waits.
pub fn poll(s: &mut State) {
    let made = with(s, |nf, d| {
        if !d.creating {
            return None;
        }
        let settled = d.source.as_mut()?.settled("")?;
        match settled {
            Settled::Saved { id, .. } => d.source.take().map(|source| (source, id)),
            _ => {
                d.creating = false;
                d.failed = Some(Toast::failure(Action::NewFile, ActionError::Failed));
                // The keyboard goes back to the name: that also makes the window draw the sentence.
                nf.focus = Some(NAME_INPUT);
                None
            }
        }
    })
    .flatten();
    if let Some((source, id)) = made {
        new_file::open_made(s, source, &id);
    }
}

/// The arrows on a tab or a template choose the next one (`dialog_key`). Every key is the dialog's.
pub fn arrow(s: &mut State, ev: &KeyEvent, tx: &mut Tx, delta: isize) -> bool {
    if ev.mods.cmd || ev.mods.alt || matches!(ev.key, Key::Tab) {
        return true;
    }
    let focused = tx.focused().map(str::to_string).unwrap_or_default();
    let on_tab = focused.strip_prefix("doc-tab-").and_then(|n| n.parse::<usize>().ok());
    let on_tpl = focused.strip_prefix("tpl-").and_then(|n| n.parse::<usize>().ok());
    if let Some(i) = on_tab {
        let next = (i as isize + delta).rem_euclid(APPS.len() as isize) as usize;
        choose_kind(s, APPS[next].0);
    } else if let Some(i) = on_tpl {
        let next = (i as isize + delta).rem_euclid(TEMPLATES.len() as isize) as usize;
        choose_template(s, TEMPLATES[next].0);
    }
    if let Some(key) = new_file::take_focus(s) {
        tx.focus(key, true);
    }
    true
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

fn tab(i: usize, on: bool, t: &Theme) -> El {
    let (kind, word, _) = APPS[i];
    let key = tab_key(i);
    row()
        .key(key)
        .role("radio")
        .label(word)
        .checked(on)
        .h(28.0)
        .px(14.0)
        .center()
        .radius(14.0)
        .fill(if on { t.text } else { t.tile })
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(kit::label(
            format!("{key}-label"),
            word,
            TAB,
            if on { t.paper } else { t.text_secondary },
        ))
        .on_press(move |s: &mut State| choose_kind(s, kind))
}

fn template_row(i: usize, on: bool, t: &Theme) -> El {
    let (template, title, words) = TEMPLATES[i];
    let key = tpl_key(i);
    let r = col()
        .key(key)
        .role("radio")
        .label(format!("{title}, {words}"))
        .checked(on)
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(1.0)
        .radius(8.0)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(kit::label(format!("{key}-title"), title, ROW_TITLE, t.text).w_full())
        .child(kit::label(format!("{key}-words"), words, type_::BODY, t.text_tertiary).w_full())
        .on_press(move |s: &mut State| choose_template(s, template));
    if on { r.fill(t.selection) } else { r.hover(t.hover) }
}

/// The dialog's card (the registry draws it over Files, under the caption).
pub fn card(nf: &NewFile, t: &Theme) -> El {
    let creating = nf.doc.as_ref().is_some_and(|d| d.creating);
    let template = nf.doc.as_ref().map_or(Template::Blank, |d| d.template);
    let mut tabs = row().key("doc-tabs").role("radiogroup").label("App").gap(6.0);
    for i in 0..APPS.len() {
        tabs = tabs.child(tab(i, APPS[i].0 == nf.kind, t));
    }
    let mut templates = col()
        .key("doc-templates")
        .role("radiogroup")
        .label("Template")
        .w_full()
        .gap(2.0);
    if nf.kind == Kind::Word {
        for i in 0..TEMPLATES.len() {
            templates = templates.child(template_row(i, TEMPLATES[i].0 == template, t));
        }
    } else {
        // Sheets and Slides: Blank only.
        templates = templates.child(template_row(0, true, t));
    }
    let error = nf
        .error
        .as_ref()
        .filter(|(given, _)| given == &nf.name.text)
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
        new_file::name_edit,
        |input| input.on_input_key(new_file::field_key),
        t,
    );
    let mut body = vec![tabs, templates, name_field];
    if let Some(why) = nf.doc.as_ref().and_then(|d| d.failed.as_deref()) {
        body.push(
            kit::label("doc-failed", why, type_::BODY, t.text)
                .role("alert")
                .wrap()
                .w_full(),
        );
    }
    body.push(
        kit::label("doc-encrypted", ENCRYPTED, type_::BODY, t.text_tertiary)
            .wrap()
            .w_full(),
    );
    let foot = row()
        .center_y()
        .justify_end()
        .child(
            button(
                0,
                if creating { "Creating\u{2026}" } else { "Create" },
                Weight::Primary,
                t,
            )
            .default_button()
            .on_press(create),
        )
        .child(button(1, "Cancel", Weight::Quiet, t).on_press(new_file::close));
    dialog::card(
        TITLE,
        460.0,
        TITLE,
        Some(Sub::Wrap("Docs, Sheets or Slides")),
        body,
        foot,
        t,
    )
}

#[cfg(test)]
#[path = "new_document_tests.rs"]
mod tests;
