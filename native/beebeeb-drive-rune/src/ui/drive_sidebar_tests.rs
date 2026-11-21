//! The one Drive sidebar in every window: the same rows and figures, the window's own row lit,
//! every item going where it says, and the closed standings dimmed and inert.

use std::sync::Arc;

use rune::{Env, Runtime};

use crate::frames::files::source::FilesFixture;
use crate::frames::shared::tests::{click, settle};
use crate::shell::actions::FixtureEngine;
use crate::shell::app::{GateState, Screen, Section, SettingsPane};
use crate::shell::host::Shell;
use crate::shell::platform::fake::FakePlatform;
use crate::{State, view};

/// The windows with a Drive sidebar, each on a board frame, and the row it lights.
const WINDOWS: [(&str, fn(&mut State), &str); 8] = [
    ("Files", |_| {}, "Files-row"),
    ("Search", |s| s.frames.find.open = true, "Files-row"),
    ("Shared", crate::frames::shared::scenes::dw_0501, "Shared-row"),
    ("Trash", crate::frames::trash::scenes::dw_0747, "Trash-row"),
    ("Photos", |s| crate::frames::photos::set_scene(s, "DW-0601"), "Photos-row"),
    ("Viewer", |s| crate::frames::viewers::set_scene(s, "DW-1512"), ""),
    ("Sync history", crate::frames::this_pc::scenes::dw_1301, "Sync history-row"),
    ("Available offline", crate::frames::this_pc::scenes::dw_0715, "Available offline-row"),
];

fn window(scene: fn(&mut State)) -> Runtime<State> {
    let engine = Arc::new(FixtureEngine::ready());
    let shell = Shell::fixture_on(engine, Arc::new(FakePlatform::default()));
    let mut state = crate::shell::state::init_with(shell);
    state.actions.source = Box::new(FilesFixture::default());
    scene(&mut state);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    settle(&mut rt);
    rt
}

/// The elements under the `sidebar` element.
fn sidebar(rt: &Runtime<State>) -> Vec<rune::ElementInfo> {
    let all = rt.elements();
    let root = all
        .iter()
        .position(|e| e.key.as_deref() == Some("sidebar") && !e.exiting)
        .expect("a sidebar");
    let under = |mut i: usize| loop {
        match all[i].parent {
            Some(p) if p == root => return true,
            Some(p) => i = p,
            None => return false,
        }
    };
    (0..all.len())
        .filter(|&i| !all[i].exiting && under(i))
        .map(|i| all[i].clone())
        .collect()
}

/// Every keyed element of the sidebar with its words: the rows, the icons, the counts.
fn shape(items: &[rune::ElementInfo]) -> Vec<(String, Option<String>)> {
    let mut v: Vec<_> = items
        .iter()
        .filter_map(|e| e.key.clone().map(|k| (k, e.text.clone())))
        .collect();
    v.sort();
    v
}

/// The rows drawn as selected.
fn lit(items: &[rune::ElementInfo]) -> Vec<String> {
    items
        .iter()
        .filter(|e| e.key.as_deref().is_some_and(|k| k.ends_with("-row")) && e.fill.a > 0.0)
        .filter_map(|e| e.key.clone())
        .collect()
}

#[test]
fn every_window_draws_the_same_sidebar_with_its_own_row_lit() {
    let (_, files, _) = WINDOWS[0];
    let want = shape(&sidebar(&window(files)));
    for key in [
        "Files-icon",
        "Shared-pill",
        "Shared-badge",
        "Trash-badge",
        "Sync history-pill",
        "Available offline-badge",
        "settings-btn",
    ] {
        assert!(want.iter().any(|(k, _)| k == key), "Files draws {key}");
    }
    for (name, scene, row) in WINDOWS {
        let mut rt = window(scene);
        let items = sidebar(&rt);
        let row = if row.is_empty() {
            format!("{}-row", rt.state.sidebar_section.label())
        } else {
            row.to_string()
        };
        assert_eq!(lit(&items), vec![row], "{name}: its own row is lit");
        // Files over the same data (a scene may hold its own This PC): the same rows and figures.
        rt.update(|s, _| {
            crate::shell::host::dispatch(
                s,
                crate::shell::app::Event::Navigate(crate::shell::app::Route::section(Section::Files)),
            );
        });
        settle(&mut rt);
        assert_eq!(shape(&items), shape(&sidebar(&rt)), "{name}: the same sidebar as Files");
    }
}

#[test]
fn each_item_goes_through_the_route_machine_and_the_gear_to_settings() {
    use crate::ui::sidebar::SidebarSection as S;
    for (key, to, lit) in [
        ("Files-row", Section::Files, Some(S::Files)),
        ("Shared-row", Section::Shared, Some(S::Shared)),
        ("Photos-row", Section::Photos, Some(S::Photos)),
        ("Backups-row", Section::Backups, Some(S::Backups)),
        ("Trash-row", Section::Trash, Some(S::Trash)),
        ("Sync history-row", Section::SyncHistory, None),
        ("Available offline-row", Section::AvailableOffline, None),
    ] {
        let mut rt = window(|_| {});
        click(&mut rt, key);
        assert_eq!(rt.state.shell.app.route().section, to, "{key}");
        if let Some(lit) = lit {
            assert_eq!(rt.state.sidebar_section, lit, "{key}: the section is selected");
        }
    }
    let mut rt = window(|_| {});
    click(&mut rt, "settings-btn");
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::General));
}

#[test]
fn sync_problems_show_the_red_pill_in_files_and_in_sync_history() {
    let mut rt = window(crate::frames::this_pc::scenes::dw_0428);
    click(&mut rt, "btn-try-again");
    assert_eq!(rt.state.this_pc.problem_count(), 2);
    let pill = |rt: &Runtime<State>| {
        sidebar(rt)
            .into_iter()
            .find(|e| e.key.as_deref() == Some("Sync history-badge"))
            .and_then(|e| e.text)
    };
    assert_eq!(pill(&rt).as_deref(), Some("2"), "Sync history");
    click(&mut rt, "Files-row");
    assert_eq!(rt.state.shell.app.route().section, Section::Files);
    assert_eq!(pill(&rt).as_deref(), Some("2"), "Files");
    let label = sidebar(&rt)
        .into_iter()
        .find(|e| e.key.as_deref() == Some("Sync history-pill"))
        .and_then(|e| e.label);
    assert_eq!(label.as_deref(), Some("2 items didn't sync"));
}

#[test]
fn locked_and_no_plan_are_dimmed_and_inert() {
    for gate in [GateState::Locked, GateState::NoPlan] {
        let engine = Arc::new(FixtureEngine::ready());
        engine.set_gate(gate);
        let state = crate::shell::state::init_with(Shell::fixture_on(engine, Arc::new(FakePlatform::default())));
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, crate::shell::registry::view, (1278.0, 798.0), env);
        rt.on_key = Some(crate::shell::host::on_key);
        rt.poll = Some(crate::shell::host::poll);
        rt.frame(0.0);
        rt.frame(0.016);
        let items = sidebar(&rt);
        assert!(lit(&items).is_empty(), "{gate:?}: nothing lit");
        for e in items.iter().filter(|e| e.role == "button" || e.role == "treeitem") {
            assert!(!e.focusable, "{gate:?}: {:?} takes no focus", e.key);
        }
        assert!(
            rt.elements().iter().any(|e| e.key.as_deref() == Some("sidebar") && e.opacity < 0.6),
            "{gate:?}: dimmed"
        );
        for key in ["Shared-row", "Sync history-row", "Available offline-row", "settings-btn"] {
            click(&mut rt, key);
        }
        assert_eq!(rt.state.shell.app.screen(), Screen::Gate(gate), "{gate:?}: the window stays");
    }
}
