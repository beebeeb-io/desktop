//! What the settings panes show: `SettingsSource`, and the fixture that holds exactly the
//! board's sample content.
//!
//! A pane never invents a value: every figure, name and state it draws comes from here. The
//! engine lane replaces `BoardSettings` with a source that asks the engine (which call each
//! method needs is listed in the pull request and beside each method).

/// Light, dark, or follow the system (DW-0787).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// What the choice does, as the Theme row says it (DW-0787).
    pub fn says(self) -> &'static str {
        match self {
            Self::System => "Follows Windows.",
            Self::Light => "Always light.",
            Self::Dark => "Always dark.",
        }
    }
}

/// Whether a newer version is known (DW-0701, DW-0781).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Update {
    /// This is the newest version known.
    Current,
    /// A newer one can be downloaded.
    Available { version: String },
}

/// The General pane's values.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct General {
    pub theme: ThemeChoice,
    /// Where new files are stored, as a place name: "Europe", or a country. Never a city.
    pub storage_location: String,
    /// The running version, "0.9.1".
    pub version: String,
    pub update: Update,
}

pub trait SettingsSource {
    /// Engine: the stored theme preference, the account's storage region, the app's own
    /// version, and the updater's last known state.
    fn general(&self) -> General;
    /// Engine: store the theme preference (it applies at once, to every window).
    fn set_theme(&mut self, theme: ThemeChoice);
}

/// The board's sample content, frame by frame.
pub struct BoardSettings {
    general: General,
}

impl BoardSettings {
    /// DW-0701: System theme, Europe, version 0.9.1, nothing newer.
    pub fn new() -> Self {
        Self {
            general: General {
                theme: ThemeChoice::System,
                storage_location: "Europe".into(),
                version: "0.9.1".into(),
                update: Update::Current,
            },
        }
    }

    /// DW-0781: version 0.9.2 can be downloaded.
    pub fn with_update_available(mut self) -> Self {
        self.general.update = Update::Available {
            version: "0.9.2".into(),
        };
        self
    }
}

impl SettingsSource for BoardSettings {
    fn general(&self) -> General {
        self.general.clone()
    }
    fn set_theme(&mut self, theme: ThemeChoice) {
        self.general.theme = theme;
    }
}
