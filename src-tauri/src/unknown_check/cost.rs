//! A larger migration allowance requires positive Windows cost evidence.
#[derive(Clone, Copy)]
pub(super) struct Cost {
    pub unrestricted: bool,
    pub roaming: bool,
    pub over: bool,
    pub approaching: bool,
    pub restricted: bool,
}
pub(super) fn confirmed(profiles: Option<&[Cost]>) -> bool {
    profiles.is_some_and(|p| {
        !p.is_empty()
            && p.iter()
                .all(|c| c.unrestricted && !c.roaming && !c.over && !c.approaching && !c.restricted)
    })
}
pub(super) fn unmetered() -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkCostType, NetworkInformation};
        let read = || -> windows::core::Result<Vec<Cost>> {
            let profile = NetworkInformation::GetInternetConnectionProfile()?;
            if profile.GetNetworkConnectivityLevel()? != NetworkConnectivityLevel::InternetAccess {
                return Ok(vec![]);
            }
            let profiles = NetworkInformation::GetConnectionProfiles()?;
            let mut found = Vec::new();
            for p in profiles {
                if p.GetNetworkConnectivityLevel()? == NetworkConnectivityLevel::InternetAccess {
                    let c = p.GetConnectionCost()?;
                    found.push(Cost {
                        unrestricted: c.NetworkCostType()? == NetworkCostType::Unrestricted,
                        roaming: c.Roaming()?,
                        over: c.OverDataLimit()?,
                        approaching: c.ApproachingDataLimit()?,
                        restricted: c.BackgroundDataUsageRestricted()?,
                    });
                }
            }
            Ok(found)
        };
        confirmed(read().ok().as_deref())
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_2191_s2_cost_errors_unknown_and_conflicting_profiles_stay_conservative() {
        let free = Cost {
            unrestricted: true,
            roaming: false,
            over: false,
            approaching: false,
            restricted: false,
        };
        assert!(confirmed(Some(&[free])));
        assert!(!confirmed(None));
        assert!(!confirmed(Some(&[])));
        for constrained in [
            Cost {
                unrestricted: false,
                ..free
            },
            Cost { roaming: true, ..free },
            Cost { over: true, ..free },
            Cost {
                approaching: true,
                ..free
            },
            Cost {
                restricted: true,
                ..free
            },
        ] {
            assert!(!confirmed(Some(&[constrained])));
            assert!(!confirmed(Some(&[free, constrained])));
        }
    }
}
