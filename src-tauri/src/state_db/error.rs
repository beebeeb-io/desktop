//! The state database's own error type, for the seams that are no longer strings.

/// A refused id, a refused statement, or a caller's callback that failed.
///
/// The public methods still return `rusqlite::Result` (their callers live in other
/// modules and read it as before); this type is what the typed seams produce and
/// what converts back into the landing's `rusqlite::Error` at the transaction.
#[derive(Debug)]
pub enum StateDbError {
    /// The database refused the statement; rusqlite's own error, unchanged.
    Sql(rusqlite::Error),
    /// An id from outside was blank; the message names which kind.
    InvalidId(&'static str),
    /// A callback inside a transaction failed; the callback's error, boxed.
    Callback(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for StateDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => error.fmt(f),
            Self::InvalidId(kind) => write!(f, "a {kind} is empty"),
            Self::Callback(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StateDbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sql(error) => Some(error),
            Self::InvalidId(_) => None,
            Self::Callback(error) => Some(error.as_ref()),
        }
    }
}

impl From<rusqlite::Error> for StateDbError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

/// Kept while the engine bridge's rekey callback still yields `anyhow::Result`: its
/// lane (D-P2-2) maps that to `EngineError`, and this conversion goes with it.
impl From<anyhow::Error> for StateDbError {
    fn from(error: anyhow::Error) -> Self {
        Self::Callback(error.into_boxed())
    }
}

/// What a landing's `rekey` callback may fail with. Two implementations, both concrete:
/// the state database's own error, and the `anyhow::Error` the one external callback
/// still returns. The landing only needs the box, for the `rusqlite::Error` it reports.
pub trait RekeyFailure {
    fn into_boxed(self) -> Box<dyn std::error::Error + Send + Sync>;
}

impl RekeyFailure for StateDbError {
    fn into_boxed(self) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(self)
    }
}

impl RekeyFailure for anyhow::Error {
    fn into_boxed(self) -> Box<dyn std::error::Error + Send + Sync> {
        Box::<dyn std::error::Error + Send + Sync>::from(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refused id reads as the sentence its kind names, and is a real `Error`.
    #[test]
    fn an_invalid_id_names_the_kind_it_refused() {
        let error = StateDbError::InvalidId("claim id");
        assert_eq!(error.to_string(), "a claim id is empty");
        assert!(std::error::Error::source(&error).is_none());
        assert!(matches!(
            StateDbError::from(rusqlite::Error::InvalidQuery),
            StateDbError::Sql(_)
        ));
    }
}
