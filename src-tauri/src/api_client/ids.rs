//! The ids the API names, one newtype each, so a folder id cannot be passed
//! where a file id belongs.
//!
//! Owns: the borrowed form and the wire spelling (raw; the URL builder encodes
//! it). Must not: create an id — only the server does — or validate one.
//! Tests: `api_client/tests/ids.rs`.

/// Declares one borrowed id. `Copy` because an id is a `&str` and the URL
/// builders take it by value; `Hash`/`Eq` so it can key a map without a
/// `to_string`.
macro_rules! api_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name<'a>(&'a str);

        impl<'a> $name<'a> {
            /// The id as the server spelled it.
            pub fn new(raw: &'a str) -> Self {
                Self(raw)
            }

            /// The raw id, for a URL path or a body field.
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

api_id! {
    /// A file row (`POST /api/v1/files`, chunk and version routes).
    FileId
}
api_id! {
    /// A folder row. Folders are file rows with a kind, but the routes that
    /// take a folder never take a file, so they get their own id.
    FolderId
}
api_id! {
    /// One stored version of a file.
    VersionId
}
api_id! {
    /// An upload session (init returns it, the chunk and complete routes take
    /// it).
    UploadSessionId
}
api_id! {
    /// A shared folder root.
    SharedFolderId
}
api_id! {
    /// An invite to share a folder.
    InviteId
}
