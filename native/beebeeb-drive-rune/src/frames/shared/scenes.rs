//! The board frames of Shared as states of a fresh `State` (`--scene DW-05xx`, the frame gate).
//! The section's frames are in the shell's table; the lane's dialogs are in [`FRAMES`].

use super::dialogs::{Dialog, SafetyNumber};
use super::parts::menu::Open;
use super::source::{Access, BoardShared};
use super::{SharedUi, Tab};
use crate::State;
use crate::shell::app::Section;

fn scene(s: &mut State, tab: Tab, source: BoardShared) {
    crate::frames::scene(s, Section::Shared);
    s.frames.shared = SharedUi {
        source: Box::new(source),
        tab,
        selected: None,
        menu: None,
        dialog: None,
        fields: Default::default(),
    };
}

pub fn dw_0501(s: &mut State) {
    scene(s, Tab::WithMe, BoardShared::new());
}
pub fn dw_0502(s: &mut State) {
    scene(s, Tab::ByMe, BoardShared::new());
}
pub fn dw_0503(s: &mut State) {
    scene(s, Tab::Requests, BoardShared::new());
}
pub fn dw_0508(s: &mut State) {
    scene(s, Tab::WithMe, BoardShared::empty());
}
pub fn dw_0509(s: &mut State) {
    scene(s, Tab::ByMe, BoardShared::empty());
}
pub fn dw_0510(s: &mut State) {
    scene(s, Tab::Requests, BoardShared::empty());
}

fn menu(s: &mut State, selected: usize, at: (f32, f32), hot: usize) {
    s.frames.shared.selected = Some(selected);
    s.frames.shared.menu = Some(Open::at(at.0, at.1).lit(hot));
}

/// The menu on Brand review.pdf, Leave… lit.
pub fn dw_0505(s: &mut State) {
    dw_0501(s);
    menu(s, 1, (430.0, 262.0), 5);
}
/// The menu on the link to Budget 2026.xlsx, Copy link lit.
pub fn dw_0506(s: &mut State) {
    dw_0502(s);
    menu(s, 3, (560.0, 332.0), 0);
}
/// The menu on the request Shoot selects, View files lit.
pub fn dw_0507(s: &mut State) {
    dw_0503(s);
    menu(s, 1, (520.0, 222.0), 1);
}

/// The share the board's People dialogs are about (Lisbon trip), and the person in them.
const LISBON: &str = "p-lisbon";
const JONAS: &str = "m-jonas";

/// The board's sample safety number (DW-0513, DW-0513b). Crypto material is never made up in
/// the app: this is the board's drawing, shown only in these scenes. The real number is the
/// engine's, in the security-reviewed wiring.
fn board_safety_number() -> SafetyNumber {
    SafetyNumber(
        [
            "37195", "04826", "91730", "62458", "18094", "75263", "40917", "86352", "29641", "05738", "61294", "83570",
        ]
        .iter()
        .map(|g| g.to_string())
        .collect(),
    )
}

/// New request over Requests, the Title in focus.
pub fn dw_0504(s: &mut State) {
    dw_0503(s);
    s.frames.shared.open_dialog(Dialog::NewRequest);
    s.frames.shared.fields.title.focused = true;
}

/// People with access to Lisbon trip, over By me with it selected.
pub fn dw_0511(s: &mut State) {
    dw_0502(s);
    s.frames.shared.selected = Some(0);
    s.frames.shared.show_people(LISBON);
}

/// Jonas's access.
pub fn dw_0512(s: &mut State) {
    dw_0511(s);
    s.frames.shared.show_person(LISBON, JONAS);
}

/// Remove Jonas's access?
pub fn dw_0512b(s: &mut State) {
    dw_0511(s);
    s.frames.shared.open_dialog(Dialog::RemoveAccess {
        share: LISBON.into(),
        person: JONAS.into(),
    });
}

/// The safety number to compare with Jonas.
pub fn dw_0513(s: &mut State) {
    dw_0511(s);
    s.frames.shared.open_dialog(Dialog::SafetyNumber {
        share: LISBON.into(),
        person: JONAS.into(),
        number: board_safety_number(),
    });
}

/// The numbers did not match: sharing with Jonas is stopped.
pub fn dw_0513b(s: &mut State) {
    dw_0511(s);
    s.frames.shared.open_dialog(Dialog::Stopped {
        share: LISBON.into(),
        person: JONAS.into(),
        number: board_safety_number(),
    });
}

/// Invite someone to Lisbon trip: an address typed, the field in focus, Can view.
pub fn dw_0514(s: &mut State) {
    dw_0511(s);
    s.frames.shared.start_invite(LISBON);
    s.frames
        .shared
        .fields
        .email
        .replace_text("jonas@northwind.studio".to_string());
    if let Some(Dialog::Invite { access, .. }) = s.frames.shared.dialog.as_mut() {
        *access = Access::View;
    }
}

/// Jonas's key changed: the alert over People.
pub fn dw_0515(s: &mut State) {
    dw_0511(s);
    s.frames.shared.open_dialog(Dialog::People {
        share: LISBON.into(),
        key_changed: Some(JONAS.into()),
    });
}

/// Empty Trash? over Trash, two digits of the code typed.
pub fn dw_0749(s: &mut State) {
    crate::frames::trash::scenes::dw_0747(s);
    s.frames.shared.open_dialog(Dialog::EmptyTrash);
    s.frames.shared.type_code("51");
}

/// The lane's dialogs, by frame id (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-0504", dw_0504),
    ("DW-0511", dw_0511),
    ("DW-0512", dw_0512),
    ("DW-0512b", dw_0512b),
    ("DW-0513", dw_0513),
    ("DW-0513b", dw_0513b),
    ("DW-0514", dw_0514),
    ("DW-0515", dw_0515),
    ("DW-0749", dw_0749),
];
