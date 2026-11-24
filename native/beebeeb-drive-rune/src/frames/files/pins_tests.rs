//! Pinned folders: Pin to sidebar / Unpin from sidebar in the file menu, the sidebar's Pinned list, the limit.

use super::Toast;
use super::source::{Action, ActionError, BoardDrive, Call, FilesFixture};
use super::tests::*;
use crate::State;
use crate::init;
use rune::Runtime;

fn wired(pins: Vec<crate::source::DriveItem>) -> (Runtime<State>, Calls) {
    let mut state = init();
    let mut fixture = FilesFixture::default();
    fixture.pins = pins;
    let calls = fixture.calls.clone();
    state.source = Box::new(BoardDrive::sharing(&fixture.edits));
    state.actions.source = Box::new(fixture);
    (runtime_of(state), calls)
}

fn row_of(rt: &Runtime<State>, folder: bool) -> usize {
    rt.state
        .listing
        .items
        .iter()
        .position(|i| i.is_folder == folder && !i.is_protected)
        .expect("the board has such a row")
}

fn menu_words(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| e.role == "menuitem")
        .filter_map(|e| e.label)
        .collect()
}

fn pick(rt: &mut Runtime<State>, words: &str) {
    let key = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "menuitem" && e.label.as_deref() == Some(words))
        .and_then(|e| e.key)
        .expect("the menu has the entry");
    click(rt, &key);
}

#[test]
fn a_folder_can_be_pinned_from_the_menu_and_shows_in_the_sidebar() {
    let (mut rt, calls) = wired(Vec::new());
    assert!(
        rt.elements().iter().all(|e| e.key.as_deref() != Some("pinned")),
        "no pins: no Pinned section"
    );
    let at = row_of(&rt, true);
    let id = rt.state.listing.items[at].id.clone();
    right_click(&mut rt, &format!("row-{at}-date"));
    assert!(menu_words(&rt).contains(&"Pin to sidebar".to_string()));
    pick(&mut rt, "Pin to sidebar");
    assert_eq!(asked(&calls), [Call::Pin(id.clone(), true)]);
    let row = info(&rt, &format!("pin-{id}-row"));
    assert_eq!(row.role, "treeitem");
    assert!(row.focusable);
    assert_eq!(info(&rt, "pinned-label").text.as_deref(), Some("PINNED"));
    // Now the menu offers the way back.
    right_click(&mut rt, &format!("row-{at}-date"));
    assert!(menu_words(&rt).contains(&"Unpin from sidebar".to_string()));
    pick(&mut rt, "Unpin from sidebar");
    assert_eq!(asked(&calls).last(), Some(&Call::Pin(id, false)));
    assert!(rt.state.actions.source.pinned().is_empty());
}

#[test]
fn a_file_has_no_pin_entry() {
    let (mut rt, _) = wired(Vec::new());
    let at = row_of(&rt, false);
    right_click(&mut rt, &format!("row-{at}-date"));
    let words = menu_words(&rt);
    assert!(!words.iter().any(|w| w.contains("sidebar")), "{words:?}");
}

#[test]
fn the_eleventh_pin_is_refused_with_the_webs_limit() {
    let (probe, _) = wired(Vec::new());
    let folder = probe.state.listing.items[row_of(&probe, true)].clone();
    let full: Vec<_> = (0..super::pins::MAX_PINS)
        .map(|n| crate::source::DriveItem {
            id: format!("other-{n}"),
            ..folder.clone()
        })
        .collect();
    let (mut rt, calls) = wired(full);
    let at = row_of(&rt, true);
    right_click(&mut rt, &format!("row-{at}-date"));
    pick(&mut rt, "Pin to sidebar");
    assert!(asked(&calls).is_empty(), "nothing was sent");
    assert_eq!(
        rt.state.actions.toast,
        Some(Toast::Failed(Action::Pin, ActionError::PinLimit))
    );
    assert_eq!(
        Toast::failure(Action::Pin, ActionError::PinLimit),
        "Couldn't pin to the sidebar. Unpin a folder first."
    );
}
