//! Power and network policy for the sync tick: pause uploads/downloads on a metered network (when the
//! person set `metered`) and while the OS battery saver is on. Hydration the person asked for goes through the
//! Cloud Files callbacks, not the tick, so it still runs.
//!
//! The OS is read behind [`PowerState`]; tests use [`FakePower`].

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
}

#[cfg(test)]
impl PowerState for FakePower {
    fn network_metered(&self) -> bool {
        self.metered
    }
    fn battery_saver(&self) -> bool {
        self.saver
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_decide_table() {
        let f = |metered, saver| FakePower { metered, saver };
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
}
