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

/// Which releases this PC is offered (DW-0789).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Channel {
    #[default]
    Stable,
    Beta,
    Alpha,
}

impl Channel {
    pub const ALL: [Channel; 3] = [Channel::Stable, Channel::Beta, Channel::Alpha];

    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Beta => "Beta",
            Self::Alpha => "Alpha",
        }
    }

    /// What the channel offers, as the Release channel row says it.
    pub fn says(self) -> &'static str {
        match self {
            Self::Stable => "Regular releases after validation.",
            Self::Beta => "Earlier builds for release testing.",
            Self::Alpha => "Earliest builds for rapid feedback.",
        }
    }
}

/// A version's release notes (DW-0789b).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReleaseNotes {
    pub version: String,
    pub channel: Channel,
    /// The day it was released, as the dialog says it ("4 Nov 2026").
    pub released: String,
    /// The release's own notes, a line each. Empty: the release came without notes.
    pub lines: Vec<String>,
}

/// The About pane's values (DW-0751).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct About {
    pub product: String,
    pub tagline: String,
    /// The running version and its build number: "0.9.1", "311".
    pub version: String,
    pub build: String,
    /// The encryption core's revision, as short as the board shows it.
    pub core: String,
    /// What the last check found, in the row's words.
    pub last_check: String,
    pub channel: Channel,
    pub notes: ReleaseNotes,
    /// The foot line under More: the company.
    pub company: String,
}

/// One place new files can be stored (DW-0788): the account's default, or a country.
/// A country, never a city, never a provider.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Location {
    pub name: String,
    /// The default's second line.
    pub says: Option<String>,
    /// `false`: not live yet. Shown as "Coming soon", never choosable, never presented as live.
    pub live: bool,
    /// `true` for the account's default (Europe: Beebeeb places each file).
    pub default: bool,
}

/// Where new files are stored, and what happens when that place cannot take one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Locations {
    pub choices: Vec<Location>,
    /// Index into `choices`.
    pub chosen: usize,
    /// Only store in the chosen country: an upload it cannot take waits (off: another EU country).
    pub only_here: bool,
    /// The chosen country cannot take files right now.
    pub unavailable: bool,
}

impl Locations {
    pub fn chosen(&self) -> &Location {
        &self.choices[self.chosen.min(self.choices.len() - 1)]
    }
}

pub trait SettingsSource {
    /// Engine: the server's list of storage locations for this account and the stored
    /// choice. `None`: the list could not load.
    fn locations(&self) -> Option<Locations>;
    /// Engine: store the choice (new files only; existing files never move). `false`: not choosable.
    fn choose_location(&mut self, index: usize) -> bool;
    /// Engine: store "only store in the chosen country".
    fn set_only_here(&mut self, on: bool);
    /// Engine: ask the server for the list again.
    fn reload_locations(&mut self);
    /// Engine: the app's own version and build, the pinned core revision, the updater's
    /// last check and channel, and the newest release's notes.
    fn about(&self) -> About;
    /// Engine: store the release channel; the updater forgets its last answer and checks again.
    fn set_channel(&mut self, channel: Channel);
    /// Engine: the stored theme preference, the account's storage region, the app's own
    /// version, and the updater's last known state.
    fn general(&self) -> General;
    /// Engine: store the theme preference (it applies at once, to every window).
    fn set_theme(&mut self, theme: ThemeChoice);
}

/// The board's sample content, frame by frame.
pub struct BoardSettings {
    general: General,
    about: About,
    locations: Option<Locations>,
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
            about: About {
                product: "Beebeeb Drive".into(),
                tagline: "End-to-end encrypted storage".into(),
                version: "0.9.1".into(),
                build: "311".into(),
                core: "2084927".into(),
                last_check: "0.9.1 is the newest version · checked today, 09:12".into(),
                channel: Channel::Stable,
                notes: ReleaseNotes {
                    version: "0.9.2".into(),
                    channel: Channel::Stable,
                    released: "4 Nov 2026".into(),
                    lines: vec![
                        "Bandwidth limits apply straight away, without pausing sync.".into(),
                        "Sync history keeps 30 days on every PC.".into(),
                        "A file badge no longer stays on Uploading after the upload finished.".into(),
                    ],
                },
                company: "Initlabs B.V.".into(),
            },
            locations: Some(board_locations()),
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

/// DW-0788: the default (Europe) chosen; Germany and Finland live; the Netherlands coming soon.
fn board_locations() -> Locations {
    let country = |name: &str, live: bool| Location {
        name: name.into(),
        says: None,
        live,
        default: false,
    };
    Locations {
        choices: vec![
            Location {
                name: "Default".into(),
                says: Some("Europe: Beebeeb places each new file in the EU".into()),
                live: true,
                default: true,
            },
            country("Germany", true),
            country("Finland", true),
            country("Netherlands", false),
        ],
        chosen: 0,
        only_here: false,
        unavailable: false,
    }
}

impl BoardSettings {
    /// DW-0788b / DW-0788c: country `index` is the stored choice; `unavailable`: it cannot
    /// take files right now.
    pub fn with_location(mut self, index: usize, unavailable: bool) -> Self {
        if let Some(l) = &mut self.locations {
            l.chosen = index;
            l.unavailable = unavailable;
        }
        self
    }

    /// DW-0788d: the list of locations could not load.
    pub fn with_locations_failed(mut self) -> Self {
        self.locations = None;
        self
    }
}

impl SettingsSource for BoardSettings {
    fn locations(&self) -> Option<Locations> {
        self.locations.clone()
    }
    fn choose_location(&mut self, index: usize) -> bool {
        match &mut self.locations {
            Some(l) if l.choices.get(index).is_some_and(|c| c.live) => {
                l.chosen = index;
                l.unavailable = false;
                if l.choices[index].default {
                    l.only_here = false;
                }
                true
            }
            _ => false,
        }
    }
    fn set_only_here(&mut self, on: bool) {
        if let Some(l) = &mut self.locations {
            l.only_here = on;
        }
    }
    fn reload_locations(&mut self) {
        self.locations = Some(board_locations());
    }
    fn general(&self) -> General {
        let mut g = self.general.clone();
        // General names a country only once the account has chosen one.
        if let Some(l) = self.locations.as_ref().filter(|l| !l.chosen().default) {
            g.storage_location = l.chosen().name.clone();
        }
        g
    }
    fn set_theme(&mut self, theme: ThemeChoice) {
        self.general.theme = theme;
    }
    fn about(&self) -> About {
        self.about.clone()
    }
    fn set_channel(&mut self, channel: Channel) {
        self.about.channel = channel;
    }
}
