//! Release versions: plain `MAJOR.MINOR.PATCH`, the only shape a desktop release has.
//!
//! The release workflow refuses anything else (`release.yml`, "Validate version and channel"), so a manifest that
//! carries a pre-release or build suffix is not one of ours and is refused instead of being ordered by guesswork.

use std::fmt;

/// A plain semver release version. Ordered numerically, field by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self { major, minor, patch }
    }

    /// Parses `1.2.3`. No sign, no leading zero, no suffix, no fourth part.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let major = number(parts.next()?)?;
        let minor = number(parts.next()?)?;
        let patch = number(parts.next()?)?;
        parts.next().is_none().then_some(Self { major, minor, patch })
    }
}

fn number(part: &str) -> Option<u64> {
    let plain = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) && (part == "0" || !part.starts_with('0'));
    if plain { part.parse().ok() } else { None }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn parses_plain_semver_only() {
        assert_eq!(Version::parse("0.9.0"), Some(Version::new(0, 9, 0)));
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "01.2.3",
            "1.2.3-beta.1",
            "1.2.3+7",
            " 1.2.3",
            "-1.2.3",
            "+1.2.3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?} must not parse");
        }
    }

    #[test]
    fn orders_numerically_not_as_text() {
        assert!(Version::new(0, 10, 0) > Version::new(0, 9, 9));
        assert!(Version::new(0, 9, 10) > Version::new(0, 9, 9));
        assert!(Version::new(1, 0, 0) > Version::new(0, 99, 99));
        assert_eq!(Version::new(0, 9, 0).to_string(), "0.9.0");
    }
}
