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
