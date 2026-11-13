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
    /// Where the check for a newer version stands (the row's second line).
    pub check: UpdateCheck,
    pub channel: Channel,
    /// The chosen channel's newest version is older than the one running (DW-0789, Rules).
    pub older: Option<OlderChannel>,
    pub notes: ReleaseNotes,
    /// The foot line under More: the company.
    pub company: String,
}

/// Where the check for a newer version stands (DW-0751, Rules: "while checking the row says
/// Checking…; a check that fails says so with Try again").
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UpdateCheck {
    /// The last check's answer, in the row's words.
    Checked(String),
    /// Asked, not answered yet.
    Checking,
    /// The last check did not get an answer.
    Failed,
}

impl UpdateCheck {
    /// The row's second line. The two fixed sentences are the 0.8 desktop app's
    /// (`src/macSettingsModel.ts`, `updateRow`), which the board's rule cites.
    pub fn says(&self) -> &str {
        match self {
            Self::Checked(words) => words,
            Self::Checking => "Checking…",
            Self::Failed => "Couldn’t check for updates. Try again in a moment.",
        }
    }
}

/// The chosen channel carries an older version than the one this PC runs (DW-0789, Rules).
/// The row informs and links the notes; nothing is installed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OlderChannel {
    /// The chosen channel and its newest version.
    pub channel: Channel,
    pub version: String,
    /// What this PC runs, and the channel it came from.
    pub runs: String,
    pub from: Channel,
}

impl OlderChannel {
    pub fn title(&self) -> String {
        format!("The {} channel is at version {}", self.channel.label(), self.version)
    }
    pub fn says(&self) -> String {
        format!(
            "This PC runs {}, from the {} channel. Read the release notes before you go back to an older version.",
            self.runs,
            self.from.label()
        )
    }
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
    /// The stored choice. `None`: the list is empty, which no pane may draw as a list
    /// (an answer without choices is shown as a list that could not load).
    pub fn chosen(&self) -> Option<&Location> {
        self.choices.get(self.chosen).or_else(|| self.choices.last())
    }
}

/// One alert Beebeeb can send on this PC, and whether it does (DW-0750).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Alert {
    pub title: String,
    pub says: String,
    pub on: bool,
}

/// How far a source is with something it reads from the account: what a pane draws in its place.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ReadState {
    /// Asked for, no answer yet.
    Loading,
    /// The answer did not come, or did not read: nothing of the account is known.
    Failed,
    /// Read: what the source hands out is what the account holds.
    #[default]
    Ready,
}

/// The Notifications pane's values.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Notifications {
    /// `false`: notifications are off for Beebeeb in Windows Settings; nothing arrives (DW-0750b).
    pub allowed_in_windows: bool,
    /// The app's own switch.
    pub allow: bool,
    /// How many recent alerts have not been seen.
    pub new_recent: usize,
    /// The alerts a person can turn on and off, in titled groups.
    pub groups: Vec<(String, Vec<Alert>)>,
}

/// One recent alert from the account (DW-0791).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecentAlert {
    pub kind: AlertKind,
    pub title: String,
    pub says: String,
    /// The time of day, "08:12".
    pub time: String,
    /// Not seen yet: marked "New" until this page has been opened.
    pub new: bool,
    /// Where a press on the alert leads.
    pub leads_to: AlertTarget,
}

/// Where a recent alert leads: a place in the app, never a board frame id (the panes map it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlertTarget {
    /// Settings › Devices and sessions.
    DevicesAndSessions,
    /// Shared.
    Shared,
    /// Shared › Requests.
    SharedRequests,
    /// Settings › Security › the security log.
    SecurityLog,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlertKind {
    SignIn,
    Share,
    Request,
    Security,
}

/// What a source's workers answered since the last frame (`SettingsSource::pump`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Pumped {
    /// Something a pane draws changed: the window draws again.
    pub changed: bool,
    /// What the toast over the pane says now (a change taken, or refused).
    pub toast: Option<&'static str>,
    /// An answer is still out: the window looks again soon.
    pub busy: bool,
}

pub trait SettingsSource {
    /// `true` only for a source whose values are sample content (`BoardSettings`): a build
    /// that ships must never open settings on one.
    fn sample(&self) -> bool {
        false
    }
    /// `false` for a pane this source does not answer for (`Unwired`: none): settings draws the
    /// shell's "not built yet" frame in its place, never a value of it.
    fn wired(&self, pane: super::Pane) -> bool {
        let _ = pane;
        true
    }
    /// `false` for a frame over a wired pane (a dialog) that this source has nothing real behind
    /// yet: settings asks the shell for it, as for a frame it has not built.
    fn opens(&self, frame: &str) -> bool {
        let _ = frame;
        true
    }
    /// The window shows `pane` now: a source that reads from elsewhere asks again if what it
    /// holds is old. Never waits.
    fn opened(&mut self, pane: super::Pane) {
        let _ = pane;
    }
    /// Take what the source's workers answered since the last call. Never waits.
    fn pump(&mut self) -> Pumped {
        Pumped::default()
    }
    /// The list of storage locations is being asked for and nothing is known yet.
    fn locations_loading(&self) -> bool {
        false
    }
    /// Whether `recent_alerts` is what the account holds. Not `Ready`: its empty answer means "not known", and the
    /// page says loading or couldn't load, never "No notifications".
    fn recent_state(&self) -> ReadState {
        ReadState::Ready
    }
    /// Whether the switches of `Notifications::groups[group]` are read. Not `Ready`: its rows are there (so nothing
    /// below them moves when the answer lands) but each `Alert::on` is a filler, never drawn and never pressed.
    fn alerts_state(&self, group: usize) -> ReadState {
        let _ = group;
        ReadState::Ready
    }
    /// Ask again for what `recent_state` and `alerts_state` speak of. Never waits.
    fn reload_notifications(&mut self) {}
    /// The theme preference alone (every window reads it on every frame).
    fn theme(&self) -> ThemeChoice {
        self.general().theme
    }
    /// Engine: the Recent notifications page has been opened and left: nothing on it is new
    /// any more, and the count on the Notifications row is 0.
    fn mark_recent_seen(&mut self);
    /// Engine: ask the updater whether a newer version exists, now. The check stands at
    /// `UpdateCheck::Checking` until it answers.
    fn check_for_updates(&mut self);
    /// Engine: the account's notifications, newest first, grouped by day ("Today",
    /// "Yesterday", "3 Nov"). Empty: nothing to show.
    fn recent_alerts(&self) -> Vec<(String, Vec<RecentAlert>)>;
    /// Engine: the stored alert preferences, the count of unseen account alerts, and what
    /// Windows says about notifications for this app.
    fn notifications(&self) -> Notifications;
    /// Engine: store the app's own "Allow notifications".
    fn set_allow_notifications(&mut self, on: bool);
    /// Engine: store one alert's switch (`group`, `index` as in `Notifications::groups`).
    fn set_alert(&mut self, group: usize, index: usize, on: bool);
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
    notifications: Notifications,
    recent: Vec<(String, Vec<RecentAlert>)>,
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
                check: UpdateCheck::Checked("0.9.1 is the newest version · checked today, 09:12".into()),
                channel: Channel::Stable,
                older: None,
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
            notifications: board_notifications(),
            recent: board_recent(),
        }
    }

    /// DW-0751, Rules: the last check for a newer version failed.
    #[cfg(test)]
    pub fn with_check_failed(mut self) -> Self {
        self.about.check = UpdateCheck::Failed;
        self
    }

    /// DW-0789, Rules: this PC runs 0.9.1 from Beta, and Stable (chosen) is at 0.9.0.
    #[cfg(test)]
    pub fn with_older_channel(mut self) -> Self {
        self.about.channel = Channel::Stable;
        self.about.older = Some(OlderChannel {
            channel: Channel::Stable,
            version: "0.9.0".into(),
            runs: "0.9.1".into(),
            from: Channel::Beta,
        });
        self
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

/// DW-0750: the alerts and their switches as the board draws them.
fn board_notifications() -> Notifications {
    let alert = |title: &str, says: &str, on: bool| Alert {
        title: title.into(),
        says: says.into(),
        on,
    };
    Notifications {
        allowed_in_windows: true,
        allow: true,
        new_recent: 2,
        groups: vec![
            (
                "Your account".into(),
                vec![
                    alert("Shared with you", "When someone shares a file with you", true),
                    alert("File updates", "When a file changes on another device", false),
                    alert("Storage warnings", "When you're close to your limit", true),
                ],
            ),
            (
                "Sync on this PC".into(),
                vec![
                    alert("Sync stopped", "When files couldn't upload", true),
                    alert("Files need a decision", "When a file changed in two places", true),
                    alert(
                        "Sync finished",
                        "When everything on this PC has synced. Off by default",
                        false,
                    ),
                    alert("Space on this PC", "When downloaded files near their limit", true),
                ],
            ),
            (
                "Backups on this PC".into(),
                vec![
                    alert("Backup finished", "When a backup completes", false),
                    alert("Backup needs attention", "Such as a folder that can't be read", true),
                ],
            ),
        ],
    }
}

impl BoardSettings {
    /// DW-0750b: notifications are off for Beebeeb in Windows Settings.
    pub fn with_notifications_off_in_windows(mut self) -> Self {
        self.notifications.allowed_in_windows = false;
        self
    }
}

/// DW-0791: the four alerts the board draws.
fn board_recent() -> Vec<(String, Vec<RecentAlert>)> {
    let alert = |kind, title: &str, says: &str, time: &str, new, leads_to| RecentAlert {
        kind,
        title: title.into(),
        says: says.into(),
        time: time.into(),
        new,
        leads_to,
    };
    vec![
        (
            "Today".into(),
            vec![
                alert(
                    AlertKind::SignIn,
                    "New sign-in on Anna's iPad",
                    "From Germany. Not you? Sign it out in Devices and sessions.",
                    "08:12",
                    true,
                    AlertTarget::DevicesAndSessions,
                ),
                alert(
                    AlertKind::Share,
                    "Lotte de Wit shared Harbourlight shoot with you",
                    "6 files, in Shared.",
                    "07:40",
                    true,
                    AlertTarget::Shared,
                ),
            ],
        ),
        (
            "Yesterday".into(),
            vec![alert(
                AlertKind::Request,
                "Priya Shah asked for a file in Invoices 2026",
                "Approve or decline it in Shared, Requests.",
                "16:05",
                false,
                AlertTarget::SharedRequests,
            )],
        ),
        (
            "3 Nov".into(),
            vec![alert(
                AlertKind::Security,
                "Security logging is on",
                "Sign-ins and security changes are recorded from now on.",
                "10:31",
                false,
                AlertTarget::SecurityLog,
            )],
        ),
    ]
}

impl BoardSettings {
    /// DW-0791b: no recent alerts.
    pub fn with_no_recent_alerts(mut self) -> Self {
        self.recent.clear();
        self.notifications.new_recent = 0;
        self
    }
}

impl SettingsSource for BoardSettings {
    fn sample(&self) -> bool {
        true
    }
    fn mark_recent_seen(&mut self) {
        for (_, alerts) in &mut self.recent {
            for alert in alerts {
                alert.new = false;
            }
        }
        self.notifications.new_recent = 0;
    }
    fn check_for_updates(&mut self) {
        self.about.check = UpdateCheck::Checking;
    }
    fn recent_alerts(&self) -> Vec<(String, Vec<RecentAlert>)> {
        self.recent.clone()
    }
    fn notifications(&self) -> Notifications {
        self.notifications.clone()
    }
    fn set_allow_notifications(&mut self, on: bool) {
        self.notifications.allow = on;
    }
    fn set_alert(&mut self, group: usize, index: usize, on: bool) {
        if let Some(a) = self
            .notifications
            .groups
            .get_mut(group)
            .and_then(|g| g.1.get_mut(index))
        {
            a.on = on;
        }
    }
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
        if let Some(place) = self
            .locations
            .as_ref()
            .and_then(Locations::chosen)
            .filter(|c| !c.default)
        {
            g.storage_location = place.name.clone();
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
        // DW-0789, Rules: another channel forgets the last check's answer and checks again.
        if channel != self.about.channel {
            self.about.channel = channel;
            self.about.older = None;
            self.about.check = UpdateCheck::Checking;
        }
    }
}

/// The source of a build that ships before the engine lane has wired settings: it answers
/// nothing, and `frames::settings::view` draws the shell's "not built yet" frame instead of a
/// pane. No value of it is ever shown.
pub struct Unwired;

impl SettingsSource for Unwired {
    fn wired(&self, _pane: super::Pane) -> bool {
        false
    }
    fn mark_recent_seen(&mut self) {}
    fn check_for_updates(&mut self) {}
    fn recent_alerts(&self) -> Vec<(String, Vec<RecentAlert>)> {
        Vec::new()
    }
    fn notifications(&self) -> Notifications {
        Notifications {
            allowed_in_windows: true,
            allow: false,
            new_recent: 0,
            groups: Vec::new(),
        }
    }
    fn set_allow_notifications(&mut self, _on: bool) {}
    fn set_alert(&mut self, _group: usize, _index: usize, _on: bool) {}
    fn locations(&self) -> Option<Locations> {
        None
    }
    fn choose_location(&mut self, _index: usize) -> bool {
        false
    }
    fn set_only_here(&mut self, _on: bool) {}
    fn reload_locations(&mut self) {}
    fn about(&self) -> About {
        About {
            product: String::new(),
            tagline: String::new(),
            version: String::new(),
            build: String::new(),
            core: String::new(),
            check: UpdateCheck::Failed,
            channel: Channel::Stable,
            older: None,
            notes: ReleaseNotes {
                version: String::new(),
                channel: Channel::Stable,
                released: String::new(),
                lines: Vec::new(),
            },
            company: String::new(),
        }
    }
    fn set_channel(&mut self, _channel: Channel) {}
    fn general(&self) -> General {
        General {
            theme: ThemeChoice::System,
            storage_location: String::new(),
            version: String::new(),
            update: Update::Current,
        }
    }
    fn set_theme(&mut self, _theme: ThemeChoice) {}
}

/// The source settings starts on. A development build (and every test and headless render)
/// starts on the board's sample content; a build that ships starts on `Unwired`, so sample
/// values are never shown as a person's own. The engine lane replaces this.
pub fn first(dev_build: bool) -> Box<dyn SettingsSource> {
    if dev_build {
        Box::new(BoardSettings::new())
    } else {
        Box::new(Unwired)
    }
}
