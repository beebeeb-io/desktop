//! The board scenes of the New file dialog (DW-1520 to DW-1522), moved out of `mod.rs` to keep it under the size limit.

use super::new_file;
use crate::State;

/// DW-1520 to DW-1522: the Files window as the board draws it under New (Contract draft.pdf
/// selected, the details panel open), then New's menu on "New file…", or the dialog.
pub fn set_new_file_scene(s: &mut State, frame: &str) {
    s.select_only(6);
    match frame {
        "DW-1520" => {
            s.actions.menu = Some(crate::frames::files::OpenMenu {
                kind: crate::frames::files::MenuKind::New,
                at: (0.0, 0.0),
                hot: Some(1),
            });
        }
        "DW-1521" => {
            new_file::begin(s);
        }
        "DW-1522" => {
            // Text, Continue, "export_invoices" typed and selected, .py picked.
            new_file::begin(s);
            new_file::choose(s, new_file::Kind::Text);
            new_file::go_on(s);
            new_file::choose_ext(s, ".py");
            if let Some(nf) = s.viewer.new_file.as_mut() {
                nf.name = rune::TextEdit::new("export_invoices");
                nf.name.set_selection(0, nf.name.text.len());
                nf.name.focused = true;
            }
        }
        _ => {}
    }
}
