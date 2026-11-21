//! Power and network policy for the sync tick: pause uploads/downloads on a metered network (when the
//! person set `metered`) and while the OS battery saver is on. Hydration the person asked for goes through the
//! Cloud Files callbacks, not the tick, so it still runs.
//!
//! "Sync anyway" ([`sync_anyway`]) lifts the hold that stands until its condition changes: for a metered network,
//! until the connection changes (another network, or one that is not metered); for battery saver, until it goes
//! off. The next hold after that applies again.
//!
//! The OS is read behind [`PowerState`]; tests use [`FakePower`].

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

/// Why the tick is held. The status shows `code()` as `reason.code`, with `words()` as the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PausedFor {
    Metered,
    BatterySaver,
}

impl PausedFor {
    pub fn code(self) -> &'static str {
        match self {
            Self::Metered => "metered",
            Self::BatterySaver => "battery_saver",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Self::Metered => "Paused on a metered network",
            Self::BatterySaver => "Paused while battery saver is on",
        }
    }
}

/// What the OS says right now.
pub trait PowerState: Send + Sync {
    fn network_metered(&self) -> bool;
    fn battery_saver(&self) -> bool;
    /// Which connection this is (the OS's profile name), so a Sync anyway ends when it changes. `None`: not known.
    fn connection(&self) -> Option<String> {
        None
    }
}

/// The real OS. Anything it cannot read counts as "not held": sync is never stopped by a failed probe.
pub struct SystemPower;

impl PowerState for SystemPower {
    fn network_metered(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
            let cost = NetworkInformation::GetInternetConnectionProfile()
                .and_then(|p| p.GetConnectionCost())
                .and_then(|c| c.NetworkCostType());
            matches!(cost, Ok(NetworkCostType::Fixed | NetworkCostType::Variable))
        }
        #[cfg(not(target_os = "windows"))]
        false
    }

    fn battery_saver(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
            let mut status = SYSTEM_POWER_STATUS::default();
            // SAFETY: `status` is a valid out-pointer for the call.
            unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.SystemStatusFlag == 1
        }
        #[cfg(not(target_os = "windows"))]
        false
    }

    fn connection(&self) -> Option<String> {
        #[cfg(target_os = "windows")]
        {
            use windows::Networking::Connectivity::NetworkInformation;
            NetworkInformation::GetInternetConnectionProfile()
                .and_then(|p| p.ProfileName())
                .ok()
                .map(|name| name.to_string())
        }
        #[cfg(not(target_os = "windows"))]
        None
    }
}

/// The hold for this tick. `metered_setting` is `DesktopConfig::metered`.
pub fn decide(metered_setting: Option<bool>, os: &dyn PowerState) -> Option<PausedFor> {
    if os.battery_saver() {
        Some(PausedFor::BatterySaver)
    } else if metered_setting == Some(true) && os.network_metered() {
        Some(PausedFor::Metered)
    } else {
        None
    }
}

/// A Sync anyway: asked and not yet applied, or lifting one hold on one connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Anyway {
    #[default]
    Off,
    Asked,
    Lifting {
        hold: PausedFor,
        connection: Option<String>,
    },
}

impl Anyway {
    /// The hold this tick keeps, after a Sync anyway: `hold` is what [`decide`] said. A lift stands while the same
    /// hold stands on the same connection; anything else ends it, and a later hold applies again.
    pub fn apply(&mut self, hold: Option<PausedFor>, os: &dyn PowerState) -> Option<PausedFor> {
        let Some(hold) = hold else {
            *self = Anyway::Off;
            return None;
        };
        let connection = match hold {
            PausedFor::Metered => os.connection(),
            PausedFor::BatterySaver => None,
        };
        match self {
            Anyway::Asked => {
                *self = Anyway::Lifting { hold, connection };
                None
            }
            Anyway::Lifting { hold: lifted, connection: on } if *lifted == hold && *on == connection => None,
            _ => {
                *self = Anyway::Off;
                Some(hold)
            }
        }
    }
}

static ANYWAY: Mutex<Anyway> = Mutex::new(Anyway::Off);

/// The person pressed Sync anyway: the next tick lifts the hold that stands (if one does), until it changes.
pub fn sync_anyway() {
    let mut anyway = ANYWAY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if *anyway == Anyway::Off {
        *anyway = Anyway::Asked;
    }
}

/// The tick's hold: [`decide`], less what a Sync anyway lifts.
pub fn hold_now(metered_setting: Option<bool>, os: &dyn PowerState) -> Option<PausedFor> {
    let hold = decide(metered_setting, os);
    ANYWAY.lock().unwrap_or_else(std::sync::PoisonError::into_inner).apply(hold, os)
}

// The hold the last tick decided, so the one-second status pulse does not report `idle` over it.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_current(hold: Option<PausedFor>) {
    CURRENT.store(
        match hold {
            None => 0,
            Some(PausedFor::Metered) => 1,
            Some(PausedFor::BatterySaver) => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn current() -> Option<PausedFor> {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Some(PausedFor::Metered),
        2 => Some(PausedFor::BatterySaver),
        _ => None,
    }
}

#[cfg(test)]
pub struct FakePower {
    pub metered: bool,
    pub saver: bool,
    pub connection: Option<String>,
}

#[cfg(test)]
impl PowerState for FakePower {
    fn network_metered(&self) -> bool {
        self.metered
    }
    fn battery_saver(&self) -> bool {
        self.saver
    }
    fn connection(&self) -> Option<String> {
        self.connection.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_decide_table() {
        let f = |metered, saver| FakePower {
            metered,
            saver,
            connection: None,
        };
        assert_eq!(decide(Some(true), &f(true, false)), Some(PausedFor::Metered));
        assert_eq!(decide(Some(false), &f(true, false)), None);
        assert_eq!(decide(None, &f(true, false)), None);
        assert_eq!(decide(Some(true), &f(false, false)), None);
        assert_eq!(decide(None, &f(false, true)), Some(PausedFor::BatterySaver));
        assert_eq!(decide(Some(true), &f(true, true)), Some(PausedFor::BatterySaver));
    }

    #[test]
    fn power_current_roundtrip_resumes_on_change() {
        set_current(Some(PausedFor::Metered));
        assert_eq!(current(), Some(PausedFor::Metered));
        set_current(None);
        assert_eq!(current(), None);
    }

    fn on(metered: bool, saver: bool, connection: &str) -> FakePower {
        FakePower {
            metered,
            saver,
            connection: Some(connection.to_string()),
        }
    }

    #[test]
    fn metered_sync_anyway_lifts_until_the_connection_changes() {
        let mut anyway = Anyway::Off;
        let phone = on(true, false, "Phone hotspot");
        // Paused on a metered connection, and the status says why.
        let hold = anyway.apply(decide(Some(true), &phone), &phone);
        assert_eq!(hold, Some(PausedFor::Metered));
        assert_eq!(hold.map(PausedFor::code), Some("metered"));
        // Sync anyway: the tick runs, and keeps running on the same connection.
        anyway = Anyway::Asked;
        assert_eq!(anyway.apply(decide(Some(true), &phone), &phone), None);
        assert_eq!(anyway.apply(decide(Some(true), &phone), &phone), None);
        // Another metered connection: the pause applies again, and stays.
        let other = on(true, false, "Train wifi");
        assert_eq!(anyway.apply(decide(Some(true), &other), &other), Some(PausedFor::Metered));
        assert_eq!(anyway.apply(decide(Some(true), &phone), &phone), Some(PausedFor::Metered));
    }

    #[test]
    fn metered_sync_anyway_ends_when_the_hold_ends() {
        let mut anyway = Anyway::Asked;
        let phone = on(true, false, "Phone hotspot");
        assert_eq!(anyway.apply(decide(Some(true), &phone), &phone), None);
        // Back home (not metered), then the hotspot again: paused again.
        let home = on(false, false, "Home");
        assert_eq!(anyway.apply(decide(Some(true), &home), &home), None);
        assert_eq!(anyway, Anyway::Off);
        assert_eq!(anyway.apply(decide(Some(true), &phone), &phone), Some(PausedFor::Metered));
        // A Sync anyway asked while nothing held does not lift a later hold.
        let mut early = Anyway::Asked;
        assert_eq!(early.apply(None, &home), None);
        assert_eq!(early.apply(decide(Some(true), &phone), &phone), Some(PausedFor::Metered));
        // Battery saver: lifted until it goes off; a metered hold under it is a change.
        let mut saver = Anyway::Asked;
        let low = on(false, true, "Home");
        assert_eq!(saver.apply(decide(None, &low), &low), None);
        assert_eq!(saver.apply(decide(None, &low), &low), None);
        assert_eq!(saver.apply(decide(Some(true), &phone), &phone), Some(PausedFor::Metered));
    }
}
