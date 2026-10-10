//! The write token (spec `docs/specs/2026-10-09-macos-finder-write-versions.md` §5):
//! the content version a reply names a File Provider write's bytes with.

// Only the macOS File Provider path mints and reads write tokens; other
// targets compile this module but never call it.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

/// `W` is 32 lowercase hex digits (spec §5.1).
pub const WRITE_ID_HEX_LEN: usize = 32;

/// `{b}:w{W}`: `b` is the server version these bytes derive from, `W` the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteToken {
    pub base: i64,
    pub write_id: String,
}

impl WriteToken {
    pub fn render(&self) -> String {
        format!("{}:w{}", self.base, self.write_id)
    }
}

/// A fresh write id: a random v4 UUID without hyphens. Never derived from a name or path.
pub fn mint_write_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Strict parse of `^\d+:w[0-9a-f]{32}$`. Anything else is not a token.
pub fn parse_token(text: &str) -> Option<WriteToken> {
    let (base, rest) = text.split_once(':')?;
    if base.is_empty() || !base.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let write_id = rest.strip_prefix('w')?;
    if write_id.len() != WRITE_ID_HEX_LEN || !write_id.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    Some(WriteToken {
        base: base.parse().ok()?,
        write_id: write_id.to_string(),
    })
}

/// The `files.held_*` columns of one row (spec §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldWrite {
    pub write_id: String,
    pub base: i64,
    /// The server version this write produced; `None` until it lands.
    pub version: Option<i64>,
    /// The object version this write produced; `None` until it lands.
    pub object_version_id: Option<String>,
}

impl HeldWrite {
    /// The token string; derived, never stored (spec §5.2).
    pub fn token(&self) -> String {
        WriteToken {
            base: self.base,
            write_id: self.write_id.clone(),
        }
        .render()
    }
}

/// The predicate of spec §5.3. `held_write_queued`: an op with `write_id = held.write_id`
/// exists, in any state. Both inputs must come from one read (`StateDb::item_presentation`).
pub fn held_token(
    held: Option<&HeldWrite>,
    held_write_queued: bool,
    current_version: i64,
    current_object_version_id: Option<&str>,
) -> Option<String> {
    let held = held?;
    if held_write_queued {
        return Some(held.token());
    }
    let landed_here = held.version == Some(current_version)
        && matches!(
            (held.object_version_id.as_deref(), current_object_version_id),
            (Some(ours), Some(row)) if ours == row
        );
    landed_here.then(|| held.token())
}

/// What the accept transaction knows before this write changes the row (spec §6.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseFacts {
    pub current_version: i64,
    /// Rule 6a: the version was filled by a snapshot and nothing has touched the row since.
    pub version_filled: bool,
    pub held: Option<HeldWrite>,
    /// An op carries `held.write_id`, in any state.
    pub held_write_queued: bool,
    /// A create of this file is queued, in any state, and the server has no version of
    /// it yet: the row is provisional (plan Spec issue 4).
    pub create_queued: bool,
    /// Resolved bases of this file's queued uploads minted by this code.
    pub minted_resolved_bases: Vec<i64>,
    /// What a build before the server-version-led format reported for the row now.
    pub legacy_identifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseDecision {
    /// Rules 1a, 2, 3: the successor of the queued write `write_id`; `b` is that write's `b`.
    After { write_id: String, b: i64 },
    /// Rules 1b, 4, 5, 6a: the server version this upload replaces.
    Resolved { base: i64 },
    /// Rule 6b: wait for a snapshot to learn the version.
    Pending,
    /// Rule 6a′: queued and parked at once, bytes kept.
    ParkUnknown,
}

impl BaseDecision {
    /// `b` of the new write's token (spec §5.1).
    pub fn token_base(&self) -> i64 {
        match self {
            BaseDecision::After { b, .. } => *b,
            BaseDecision::Resolved { base } => *base,
            BaseDecision::Pending | BaseDecision::ParkUnknown => 0,
        }
    }
}

/// `{v}` or `{v}:{hash}` with `v > 0`; never a token or a three-segment identifier (rule 3).
fn content_version_number(identifier: &str) -> Option<i64> {
    if parse_token(identifier).is_some() {
        return None;
    }
    let mut parts = identifier.split(':');
    let version = parts.next()?.parse::<i64>().ok().filter(|v| *v > 0)?;
    let _hash = parts.next();
    parts.next().is_none().then_some(version)
}

/// The base of a File Provider write (spec §6.1): the first matching rule wins. Rule 1c
/// is split off (§12): a token that is not the held one falls to rule 5.
pub fn decide_base(facts: &BaseFacts, incoming: Option<&str>) -> BaseDecision {
    // "The newest write of the file's chain": the held write, while it is queued.
    let newest = facts.held.as_ref().filter(|_| facts.held_write_queued);
    let after = |held: &HeldWrite| BaseDecision::After {
        write_id: held.write_id.clone(),
        b: held.base,
    };
    if let (Some(held), Some(incoming)) = (facts.held.as_ref(), incoming)
        && incoming == held.token()
    {
        if facts.held_write_queued {
            return after(held); // 1a
        }
        if let Some(version) = held.version {
            return BaseDecision::Resolved { base: version }; // 1b
        }
    }
    if facts.create_queued
        && let Some(held) = newest
    {
        return after(held); // 2
    }
    if let (Some(v), Some(held)) = (incoming.and_then(content_version_number), newest)
        && facts.minted_resolved_bases.contains(&v)
    {
        return after(held); // 3
    }
    if let Some(incoming) = incoming
        && facts.current_version > 0
        && facts.legacy_identifiers.iter().any(|legacy| legacy == incoming)
    {
        if let Some(held) = newest.filter(|_| facts.minted_resolved_bases.contains(&facts.current_version)) {
            return after(held); // 4, then 3
        }
        return BaseDecision::Resolved {
            base: facts.current_version,
        }; // 4
    }
    if let Some(v) = crate::engine_bridge::parse_base_version_number(incoming) {
        return BaseDecision::Resolved { base: v }; // 5
    }
    if facts.current_version > 0 {
        if facts.version_filled {
            BaseDecision::Resolved {
                base: facts.current_version,
            } // 6a
        } else {
            BaseDecision::ParkUnknown // 6a′
        }
    } else {
        BaseDecision::Pending // 6b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_format_and_older_parse() {
        let w = mint_write_id();
        assert_eq!(w.len(), WRITE_ID_HEX_LEN, "{w}");
        assert!(w.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')), "{w}");
        assert_ne!(w, mint_write_id(), "a write id is never reused");

        let token = WriteToken {
            base: 2,
            write_id: w.clone(),
        }
        .render();
        assert_eq!(token, format!("2:w{w}"));
        let widest = WriteToken {
            base: i64::from(i32::MAX),
            write_id: w.clone(),
        }
        .render();
        assert!(widest.len() <= 44 && widest.len() <= 128, "{widest}");
        assert_eq!(
            parse_token(&token),
            Some(WriteToken {
                base: 2,
                write_id: w.clone()
            })
        );

        // Older code reads the first segment as the base (EB:4718-4726).
        assert_eq!(crate::engine_bridge::parse_base_version_number(Some(&token)), Some(2));
        assert_eq!(
            crate::engine_bridge::parse_base_version_number(Some(&format!("0:w{w}"))),
            None
        );

        // Never confused with the other identifier forms.
        for other in [
            "2".to_string(),
            "2:abc123".to_string(),
            "2:1700000000:10".to_string(),
            format!("2:W{w}"),
            format!("2:w{}", w.to_uppercase()),
            format!("2:w{w}0"),
            format!("-2:w{w}"),
            format!(":w{w}"),
        ] {
            assert_eq!(parse_token(&other), None, "{other}");
        }
    }

    #[test]
    fn held_token_is_some_exactly_in_the_predicate_cases() {
        let queued = HeldWrite {
            write_id: "a".repeat(32),
            base: 1,
            version: None,
            object_version_id: None,
        };
        assert_eq!(
            held_token(Some(&queued), true, 1, Some("o1")),
            Some(queued.token()),
            "queued, any state"
        );
        assert_eq!(
            held_token(Some(&queued), false, 1, Some("o1")),
            None,
            "neither queued nor landed"
        );

        let landed = HeldWrite {
            version: Some(2),
            object_version_id: Some("o2".into()),
            ..queued.clone()
        };
        assert_eq!(
            held_token(Some(&landed), false, 2, Some("o2")),
            Some(landed.token()),
            "our own landing"
        );
        assert_eq!(held_token(Some(&landed), false, 3, Some("o3")), None, "a remote change");
        assert_eq!(
            held_token(Some(&landed), false, 3, Some("o2")),
            None,
            "a new version under the same object id (a legacy one-shot replace keeps the object id, EB:6169-6174)"
        );
        assert_eq!(
            held_token(Some(&landed), false, 2, Some("o9")),
            None,
            "same number, other object"
        );
        assert_eq!(
            held_token(Some(&landed), false, 2, None),
            None,
            "m-3: the row's NULL never matches"
        );
        let no_object = HeldWrite {
            object_version_id: None,
            ..landed.clone()
        };
        assert_eq!(
            held_token(Some(&no_object), false, 2, Some("o2")),
            None,
            "m-3: the held NULL never matches"
        );
        assert_eq!(held_token(None, true, 2, Some("o2")), None, "no held write");
    }
}
