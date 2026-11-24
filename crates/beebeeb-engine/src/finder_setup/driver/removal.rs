//! The two result types of a domain removal, as the driver's handle returns them. The tag's
//! `finder_removal` module (bridge decoding, the sentence shown) is dm-23's; the kept-folder data
//! type lives in `core`.

pub use super::super::core::KeptFolder;

/// What one domain removal left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainRemoval {
    pub kept: KeptFolder,
}

/// A removal that failed. macOS may report a kept folder together with the error; both reach the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalFailure {
    pub message: String,
    pub kept: KeptFolder,
}

impl From<String> for RemovalFailure {
    fn from(message: String) -> Self {
        Self { message, kept: KeptFolder::default() }
    }
}

impl std::fmt::Display for RemovalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
