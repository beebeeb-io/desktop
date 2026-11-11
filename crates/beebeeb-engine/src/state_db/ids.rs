//! The four values the state database identifies a row, a version and a claim by.
//!
//! The facade still takes `&str`: its callers live in modules this change may not
//! touch, so the types are built at the seam (`new` for a value this database
//! wrote itself, `parse` for one that arrives from outside) and `as_str` hands the
//! raw key to SQL. Only `parse` checks: a blank id can never address a row.

use super::StateDbError;

macro_rules! id_type {
    ($name:ident, $kind:literal, $what:literal) => {
        #[doc = concat!($what, " (`files`, `operation_queue`).")]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name<'a>(&'a str);

        impl<'a> $name<'a> {
            /// The id exactly as the caller holds it: no check, for a key this database wrote.
            pub fn new(raw: &'a str) -> Self {
                Self(raw)
            }

            /// The id from outside this database: blank is refused.
            pub fn parse(raw: &'a str) -> Result<Self, StateDbError> {
                if raw.trim().is_empty() {
                    return Err(StateDbError::InvalidId($kind));
                }
                Ok(Self(raw))
            }

            /// The stored key, for a SQL parameter.
            pub fn as_str(self) -> &'a str {
                self.0
            }
        }

        impl std::fmt::Display for $name<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
    };
}

id_type!(
    LocalFileId,
    "local file id",
    "A File Provider/local id for a file row, before its server id is known."
);
id_type!(
    ServerFileId,
    "server file id",
    "The server's UUID for a file: the `files` key once the row is landed."
);
id_type!(
    VersionId,
    "version id",
    "A server object version id: which bytes a row or an op was made from."
);
id_type!(
    ClaimId,
    "claim id",
    "One runner claim on a queued op: the token a later report must carry."
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Each id's door from outside: a blank key names its kind and is refused, a real one survives.
    #[test]
    fn a_local_file_id_refuses_a_blank_id() {
        assert_eq!(
            LocalFileId::parse("  ").unwrap_err().to_string(),
            "a local file id is empty"
        );
        assert_eq!(LocalFileId::parse("local-7").unwrap().as_str(), "local-7");
    }

    #[test]
    fn a_server_file_id_refuses_a_blank_id() {
        assert_eq!(
            ServerFileId::parse("").unwrap_err().to_string(),
            "a server file id is empty"
        );
        assert_eq!(ServerFileId::parse("8f14e45f").unwrap().as_str(), "8f14e45f");
    }

    #[test]
    fn a_version_id_refuses_a_blank_id() {
        assert_eq!(VersionId::parse(" ").unwrap_err().to_string(), "a version id is empty");
        assert_eq!(VersionId::parse("v-9").unwrap().as_str(), "v-9");
    }

    #[test]
    fn a_claim_id_refuses_a_blank_id() {
        assert_eq!(ClaimId::parse("").unwrap_err().to_string(), "a claim id is empty");
        assert_eq!(ClaimId::parse("c1a1m").unwrap().as_str(), "c1a1m");
    }
}
