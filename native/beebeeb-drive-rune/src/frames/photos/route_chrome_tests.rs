//! Photos' Search box and New button reach Files (DW-0601, DW-0801, DW-0414).

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::{Intent, Ui};
use crate::shell::app::{Route, Section};
use crate::shell::registry::view;
use crate::shell::state::init;
use crate::State;

fn photos_window() -> Runtime<State> {
    let mut state = init();
    crate::shell::host::dispatch(&mut state, crate::shell::app::Event::Navigate(Route::section(Section::Photos)));
    state.photos = Ui::new(Box::new(super::source::FixturePhotos::new()));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt.frame(0.0);
    rt
}

fn click(rt: &mut Runtime<State>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

#[test]
fn enter_in_photos_search_box_shows_searchs_results_for_that_text() {
    let mut rt = photos_window();
    click(&mut rt, "search-input");
    rt.key(KeyEvent {
        key: Key::Text("harbour".into()),
        mods: Mods::default(),
    });
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    });
    // A fixture library answers at once, so the window's poll leaves the intent; the router takes it as the poll does.
    let intents = rt.state.photos.take_intents();
    assert_eq!(intents, vec![Intent::Search("harbour".into())]);
    let mut tx = rune::Tx::default();
    for intent in intents {
        super::route::route(&mut rt.state, &mut tx, intent);
    }
    rt.frame(0.0);
    assert_eq!(crate::shell::registry::group_for(&rt.state.shell.app), crate::shell::registry::FrameGroup::Section(Section::Files));
    assert_eq!(rt.state.search.text.to_string(), "harbour");
    assert!(crate::frames::search::shows_over_files(&rt.state), "Search's results are what the window shows");
    assert!(rt.state.photos.toast.is_none(), "not 'not built yet'");
}

#[test]
fn new_in_photos_opens_files_new_menu_only_where_files_can_add() {
    let mut rt = photos_window();
    let mut tx = rune::Tx::default();
    super::route::route(&mut rt.state, &mut tx, Intent::NewInFiles);
    rt.frame(0.0);
    assert_eq!(crate::shell::registry::group_for(&rt.state.shell.app), crate::shell::registry::FrameGroup::Section(Section::Files));
    assert_eq!(rt.state.actions.menu.is_some(), rt.state.can_add());
    assert!(rt.state.photos.toast.is_none());
}
