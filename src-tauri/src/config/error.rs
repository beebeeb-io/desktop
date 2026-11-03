//! `ConfigError`: the one error type of `crate::config`, and the exact line each
//! variant has always returned.
//!
//! Hand-written (`impl Display`, `impl Error`): desktop has no `thiserror`.
//! A Tauri command still gets a `String`, so [`From<ConfigError> for String`]
//! keeps every caller compiling with its old `?`.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// What stopped a config read, write or load. Messages match the strings the
/// module returned before it had a type, because logs and command errors show
/// them verbatim.
#[derive(Debug)]
pub enum ConfigError {
    ConfigDirectoryUnknown,
    CreateConfigDirectory(io::Error),
    ReadFile {
        path: PathBuf,
        source: io::Error,
    },
    ParseFile {
        path: PathBuf,
        source: toml::de::Error,
    },
    Serialize(toml::ser::Error),
    WriteFile {
        path: PathBuf,
        source: io::Error,
    },
    SetPermissions {
        path: PathBuf,
        source: io::Error,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        source: io::Error,
    },
    Flush {
        path: PathBuf,
        source: io::Error,
    },
    MissingParentDirectory,
    NotADirectory(PathBuf),
    CreateDirectory {
        path: PathBuf,
        source: io::Error,
    },
    UnknownSyncMode(String),
    /// Test builds resolve the config dir in a per-process sandbox; its failure keeps its own line.
    Sandbox(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigDirectoryUnknown => write!(f, "could not determine user config directory"),
            Self::CreateConfigDirectory(source) => write!(f, "create config dir: {source}"),
            Self::ReadFile { path, source } => write!(f, "read {}: {source}", path.display()),
            Self::ParseFile { path, source } => write!(f, "parse {}: {source}", path.display()),
            Self::Serialize(source) => write!(f, "serialize: {source}"),
            Self::WriteFile { path, source } => write!(f, "write {}: {source}", path.display()),
            Self::SetPermissions { path, source } => write!(f, "chmod {}: {source}", path.display()),
            Self::Rename { from, to, source } => {
                write!(f, "rename {} → {}: {source}", from.display(), to.display())
            }
            Self::Flush { path, source } => write!(f, "flush {}: {source}", path.display()),
            Self::MissingParentDirectory => write!(f, "the config file has no directory"),
            Self::NotADirectory(path) => write!(f, "{} exists but is not a directory", path.display()),
            Self::CreateDirectory { path, source } => {
                write!(f, "create directory {}: {source}", path.display())
            }
            Self::UnknownSyncMode(value) => write!(
                f,
                "sync_mode {value:?} is not one of everything, smart, custom or online_only"
            ),
            Self::Sandbox(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateConfigDirectory(source)
            | Self::ReadFile { source, .. }
            | Self::WriteFile { source, .. }
            | Self::SetPermissions { source, .. }
            | Self::Rename { source, .. }
            | Self::Flush { source, .. }
            | Self::CreateDirectory { source, .. } => Some(source),
            Self::ParseFile { source, .. } => Some(source),
            Self::Serialize(source) => Some(source),
            Self::ConfigDirectoryUnknown
            | Self::MissingParentDirectory
            | Self::NotADirectory(_)
            | Self::UnknownSyncMode(_)
            | Self::Sandbox(_) => None,
        }
    }
}

/// A Tauri command reports `Result<_, String>`; this keeps its `?` on a config
/// call working and shows the same line the old string error had.
impl From<ConfigError> for String {
    fn from(error: ConfigError) -> Self {
        error.to_string()
    }
}
