//! Typed identifiers for the IPC socket's request path.
//!
//! The wire structs in [`super::protocol`] keep their `String` fields: the JSON
//! shape is the Swift extension's contract and the serialized form does not
//! change. The connection parses each id field into one of these newtypes at
//! the boundary, so no function below it can mix a file id with a container id
//! or with an idempotency key. They are opaque wrappers: the engine, not this
//! module, decides what an id means.

/// A File Provider item identifier as it arrives on the wire: a server file id,
/// a provisional (local) id, or one of the fixed container constants. Opaque
/// here — the engine decides what it names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ItemId(String);

impl ItemId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// A File Provider container identifier: the root, the system trash, a
/// `namespace:` constant, or a folder's item id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ContainerId(String);

impl ContainerId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The wire text back out; read by this module's tests.
    #[cfg(test)]
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// The parent a write is placed under, already normalized (the root and the
/// retired `namespace:` containers are `None`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ParentId(String);

impl ParentId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// The extension's idempotency key for a write (task 1684): one logical
/// `createItem`/`modifyItem`, retried by the system under the same key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RequestId(String);

impl RequestId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The wire text back out; read by this module's tests.
    #[cfg(test)]
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_id_keeps_the_wire_text_verbatim() {
        let id = ItemId::new("file-42");
        assert_eq!(id.as_str(), "file-42");
        assert_eq!(id.clone().into_string(), "file-42");
        // A provisional id and a server id are the same wire type here: the
        // engine resolves which one a row carries.
        assert_eq!(ItemId::new("prov:file-42").as_str(), "prov:file-42");
    }

    #[test]
    fn container_id_keeps_the_wire_text_verbatim() {
        let id = ContainerId::new("namespace:shared_with_me");
        assert_eq!(id.as_str(), "namespace:shared_with_me");
        assert_eq!(id.into_string(), "namespace:shared_with_me");
    }

    #[test]
    fn parent_id_keeps_the_wire_text_verbatim() {
        let id = ParentId::new("folder-7");
        assert_eq!(id.as_str(), "folder-7");
        assert_eq!(id.into_string(), "folder-7");
    }

    #[test]
    fn request_id_keeps_the_wire_text_verbatim() {
        let id = RequestId::new("req-9f2");
        assert_eq!(id.as_str(), "req-9f2");
        assert_eq!(id.into_string(), "req-9f2");
    }
}
