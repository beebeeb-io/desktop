//! The app shell: lifetime, windows, tray, single instance, autostart, deep links, routing.
//! See `README.md` in this directory.

pub mod files_frame;
pub mod state;

use rune::{App, AppSpec, Titlebar};

use crate::source::{DriveSource, LargeFixtureSource};
use state::State;

/// Titlebar height in points — the single source of truth used by both
/// `App::titlebar` and `ui::caption::caption_bar`.
/// Board: brand/caption area is 33px (sidebar starts at y=33, toolbar at y=33).
pub const TITLEBAR_H: f32 = 33.0;

/// Inter (sans) + JetBrains Mono (mono), bundled as TTF.
pub static FONTS: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/inter-latin-400-normal.ttf"),
    include_bytes!("../../assets/fonts/inter-latin-500-normal.ttf"),
    include_bytes!("../../assets/fonts/inter-latin-600-normal.ttf"),
    include_bytes!("../../assets/fonts/inter-latin-700-normal.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-400-normal.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-500-normal.ttf"),
    include_bytes!("../../assets/fonts/jetbrains-mono-latin-600-normal.ttf"),
];

/// Starts the app and returns when it quits.
pub fn run() {
    let spec = AppSpec::<State> {
        fonts: FONTS,
        families: ("Inter", "JetBrains Mono"),
        scenes: &[
            ("light", |_| {}),
            ("dark", |_| {}),
            ("large-folder", |s| {
                let source = Box::new(LargeFixtureSource::new(5000));
                s.listing = source.list(None);
                s.source = source;
                s.selected = None;
                s.show_detail_pane = false;
            }),
        ],
        on_key: Some(files_frame::on_key),
        ..AppSpec::new("Beebeeb Drive", (1280.0, 800.0), state::init, files_frame::view)
    };

    App::new(spec).titlebar(Titlebar::new(TITLEBAR_H)).run();
}
