//! The command search of the editor (DW-1551): what Ctrl+K lists for each kind of document, how typing filters
//! it, and what a result does. The operation lists are ours (`EditOp`s and the Insert tools); the document's own
//! styles and the file names of this PC's list come from the source and are merged in by `office_over`.
//! Nothing here reads a file or sends anything: the search runs entirely on this device.

use beebeeb_office_protocol::office::{EditKind, EditOp, ListKind, Mark, Marks, NumberFormat, ParaStyle, TextAlign};

use crate::frames::office::edit::word_insert::Insert;
use crate::frames::viewers::office_over::{Command, CommandGroup};

/// What a result does when it is run.
#[derive(Debug, Clone, PartialEq)]
pub enum Run {
    /// One edit to the editor.
    Op(EditOp),
    /// One of Insert's tools (Table and Link open their dialog, Image the file picker).
    Insert(Insert),
    /// Open this file (a name from "Files").
    File(String),
    /// Nothing of ours answers to it.
    Unknown,
}

/// The group of the file names.
pub const FILES: &str = "Files";
/// The group of the document's styles.
pub const STYLES: &str = "Styles";

struct Entry {
    group: &'static str,
    name: &'static str,
    shortcut: Option<&'static str>,
    run: Run,
}

fn e(group: &'static str, name: &'static str, shortcut: Option<&'static str>, run: Run) -> Entry {
    Entry {
        group,
        name,
        shortcut,
        run,
    }
}

fn mark(name: &'static str, key: &'static str, mark: Mark) -> Entry {
    // `on` is settled when it runs (it toggles what the selection has).
    e("Format", name, Some(key), Run::Op(EditOp::Mark { mark, on: true }))
}

fn history() -> Vec<Entry> {
    vec![
        e("Edit", "Undo", Some("Ctrl+Z"), Run::Op(EditOp::Undo)),
        e("Edit", "Redo", Some("Ctrl+Y"), Run::Op(EditOp::Redo)),
    ]
}

/// Our operation list for a kind, in the order it is shown.
fn catalogue(kind: EditKind) -> Vec<Entry> {
    let mut all = Vec::new();
    match kind {
        EditKind::Docx => {
            for (name, style) in [
                ("Normal text", ParaStyle::Normal),
                ("Title", ParaStyle::Title),
                ("Heading 1", ParaStyle::Heading1),
                ("Heading 2", ParaStyle::Heading2),
                ("Heading 3", ParaStyle::Heading3),
            ] {
                all.push(e(STYLES, name, None, Run::Op(EditOp::ParaStyle(style))));
            }
            all.push(mark("Bold", "Ctrl+B", Mark::Bold));
            all.push(mark("Italic", "Ctrl+I", Mark::Italic));
            all.push(mark("Underline", "Ctrl+U", Mark::Underline));
            all.push(mark("Strikethrough", "", Mark::Strike));
            all.push(e("Format", "Bulleted list", None, Run::Op(EditOp::List(ListKind::Bullet))));
            all.push(e("Format", "Numbered list", None, Run::Op(EditOp::List(ListKind::Number))));
            for (name, align) in [
                ("Align left", TextAlign::Left),
                ("Align centre", TextAlign::Center),
                ("Align right", TextAlign::Right),
                ("Justify", TextAlign::Justify),
            ] {
                all.push(e("Format", name, None, Run::Op(EditOp::Align(align))));
            }
            all.push(e("Insert", "Insert table", None, Run::Insert(Insert::Table)));
            all.push(e("Insert", "Insert link", None, Run::Insert(Insert::Link)));
            all.push(e("Insert", "Insert image", None, Run::Insert(Insert::Image)));
            all.push(e("Insert", "Page break", None, Run::Insert(Insert::PageBreak)));
            all.extend(history());
            all.push(e("Edit", "Select all", Some("Ctrl+A"), Run::Op(EditOp::SelectAll)));
        }
        EditKind::Xlsx => {
            all.push(mark("Bold", "Ctrl+B", Mark::Bold));
            all.push(mark("Italic", "Ctrl+I", Mark::Italic));
            for (name, f) in [
                ("Currency", NumberFormat::Currency),
                ("Percent", NumberFormat::Percent),
                ("More decimals", NumberFormat::MoreDecimals),
                ("Fewer decimals", NumberFormat::FewerDecimals),
                ("General number format", NumberFormat::General),
            ] {
                all.push(e("Format", name, None, Run::Op(EditOp::NumberFormat(f))));
            }
            all.push(e("Data", "AutoSum", Some("Alt+="), Run::Op(EditOp::AutoSum)));
            all.push(e("Data", "Sort A to Z", None, Run::Op(EditOp::Sort { ascending: true })));
            all.push(e("Data", "Sort Z to A", None, Run::Op(EditOp::Sort { ascending: false })));
            all.push(e("Data", "Filter", None, Run::Op(EditOp::Filter)));
            all.push(e("Data", "Make table", None, Run::Op(EditOp::MakeTable)));
            all.extend(history());
        }
        EditKind::Pptx => {
            all.push(e("Slides", "New slide", None, Run::Op(EditOp::NewSlide { layout: None })));
            all.push(mark("Bold", "Ctrl+B", Mark::Bold));
            all.push(mark("Italic", "Ctrl+I", Mark::Italic));
            all.extend(history());
        }
    }
    all
}

/// Whether `name` matches what was typed: every typed word is somewhere in it, any case.
fn matches(name: &str, typed: &str) -> bool {
    let name = name.to_lowercase();
    typed.split_whitespace().all(|word| name.contains(&word.to_lowercase()))
}

fn push(groups: &mut Vec<CommandGroup>, title: &str, command: Command) {
    match groups.iter_mut().find(|g| g.title == title) {
        Some(group) => group.commands.push(command),
        None => groups.push(CommandGroup {
            title: title.to_string(),
            commands: vec![command],
        }),
    }
}

/// What the search shows for `typed`: our operations for `kind` (when an editor is live), then what the source
/// found (the document's styles, file names), without a name that is already listed in the same group.
pub fn found(kind: Option<EditKind>, typed: &str, from_source: Vec<CommandGroup>) -> Vec<CommandGroup> {
    let mut groups: Vec<CommandGroup> = Vec::new();
    if let Some(kind) = kind {
        for entry in catalogue(kind).into_iter().filter(|x| matches(x.name, typed)) {
            let shortcut = entry.shortcut.filter(|s| !s.is_empty()).map(String::from);
            push(
                &mut groups,
                entry.group,
                Command {
                    name: entry.name.to_string(),
                    shortcut,
                },
            );
        }
    }
    for group in from_source {
        for command in group.commands {
            let listed = groups
                .iter()
                .any(|g| g.title == group.title && g.commands.iter().any(|c| c.name == command.name));
            if !listed && (kind.is_some() || group.title != FILES || matches(&command.name, typed)) {
                push(&mut groups, &group.title, command);
            }
        }
    }
    groups
}

/// What the result `name` of group `group` does. `marks` are the selection's: a mark runs as the opposite of
/// what the selection has.
pub fn resolve(kind: Option<EditKind>, group: &str, name: &str, marks: Option<&Marks>) -> Run {
    if group == FILES {
        return Run::File(name.to_string());
    }
    if let Some(kind) = kind {
        if let Some(entry) = catalogue(kind).into_iter().find(|x| x.group == group && x.name == name) {
            return match entry.run {
                Run::Op(EditOp::Mark { mark, .. }) => {
                    let has = marks.is_some_and(|m| match mark {
                        Mark::Bold => m.bold,
                        Mark::Italic => m.italic,
                        Mark::Underline => m.underline,
                        Mark::Strike => m.strike,
                    });
                    Run::Op(EditOp::Mark { mark, on: !has })
                }
                other => other,
            };
        }
        // A style the document has, under the name the engine gives the five we can set.
        if group == STYLES && kind == EditKind::Docx {
            let style = match name {
                "Normal" | "Normal text" => Some(ParaStyle::Normal),
                "Title" => Some(ParaStyle::Title),
                "Heading 1" => Some(ParaStyle::Heading1),
                "Heading 2" => Some(ParaStyle::Heading2),
                "Heading 3" => Some(ParaStyle::Heading3),
                _ => None,
            };
            if let Some(style) = style {
                return Run::Op(EditOp::ParaStyle(style));
            }
        }
    }
    Run::Unknown
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod tests;
