//! What opens over the Files window: menus and the inline rename field.

use crate::theme::tokens::{Theme, type_};
use rune::{El, TextEdit, text_input};

/// The menu that is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Menu {
    /// The toolbar's sort menu (DW-0404).
    Sort,
    /// A row's menu (DW-0402).
    Context { row: usize },
}

impl Menu {
    pub fn name(&self) -> &'static str {
        match self {
            Menu::Sort => "sort",
            Menu::Context { .. } => "context",
        }
    }

    pub fn named(name: &str, selected: Option<usize>) -> Option<Menu> {
        match name {
            "sort" => Some(Menu::Sort),
            "context" => selected.map(|row| Menu::Context { row }),
            _ => None,
        }
    }
}

/// The name of a row as a field (DW-0410).
pub fn rename_field(k: &str, edit: &TextEdit, _name: &str, t: &Theme) -> El {
    text_input(edit, type_::LIST_ITEM)
        .key(format!("{k}-rename"))
        .label("Name")
        .single_line()
        .color(t.text)
        .grow(1.0)
}
