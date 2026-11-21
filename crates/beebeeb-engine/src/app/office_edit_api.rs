//! Save an Office document the editor made, for the native Office window: the helper's bytes become the file's next
//! version, a copy next to it (Keep both), or a new document in a folder. Each is for the session of the press
//! (nothing changes unless the session in memory is still that one).
//!
//! - [`Engine::save_office_for`]: the bytes as the file's next version, bound to the version they were edited from.
//!   It is [`Engine::save_text_for`]'s sibling and shares its body (`save_bytes_for`): the same opening epoch, the same
//!   [`SaveEnd::Conflict`] (a version past `base` is the person's choice, never overwritten), the same chain rule
//!   behind an unsent write, the same one write path every local edit takes, the same land wait.
//! - [`Engine::save_office_copy_for`]: Keep both, under the Keep-both name every conflict copy takes.
//! - [`Engine::create_office_for`]: a new document in a folder; a name that exists is refused.
//!
//! Before anything is queued the package is read again ([`check_package`]): the zip's central directory parses,
//! `[Content_Types].xml` and a main part are in it, and it is at most [`MAX_PACKAGE_BYTES`]. A bad package is refused
//! ([`EditRefused::Failed`]) and nothing is written: the bytes are the user's next version, so a damaged one never
//! becomes it. Nothing is decompressed here.
//!
//! BLOCKS: as [`Engine::save_text_for`] (the state database, and up to the land wait). Must not: log a name, a path or
//! a byte of the document.

use zeroize::Zeroizing;

use super::viewer_edit_api::{EditRefused, NewCopy, SaveEnd, anonymous_file};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::engine_bridge::{EngineBridge, FinderWriteItemKind, FinderWriteOutcome, FinderWriteTarget};
use crate::state_db::ItemKind;
use std::sync::Arc;

/// The most a saved package may hold. Kept equal to `beebeeb_office_protocol::MAX_SAVE_BYTES` (the engine does not
/// link the protocol crate; the host that does can assert the two agree).
pub const MAX_PACKAGE_BYTES: u64 = 2 * 100 * 1024 * 1024;

/// The longest name of a new document, in bytes (a file-name limit).
const MAX_NAME_BYTES: usize = 255;

/// The parts a package names its document by: a Word, Excel or PowerPoint package has one of them.
const MAIN_PARTS: [&[u8]; 3] = [b"word/document.xml", b"xl/workbook.xml", b"ppt/presentation.xml"];
const CONTENT_TYPES: &[u8] = b"[Content_Types].xml";

/// Why a package was not accepted. Never carries a name or a byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackageRefused {
    /// Larger than [`MAX_PACKAGE_BYTES`].
    TooLarge,
    /// Not a zip whose central directory parses (or one this check does not read: zip64).
    NotAZip,
    /// A zip without `[Content_Types].xml` or without a main part.
    MissingPart,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// Read the package's directory again: the end record, every central entry (inside the directory, pointing at a
/// local header), then the parts a document needs.
pub(crate) fn check_package(bytes: &[u8]) -> Result<(), PackageRefused> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(PackageRefused::TooLarge);
    }
    const EOCD: usize = 22;
    if bytes.len() < EOCD {
        return Err(PackageRefused::NotAZip);
    }
    // The end record is the last 22 bytes plus the comment it names: the one whose comment ends at the file's end.
    let lowest = bytes.len().saturating_sub(EOCD + usize::from(u16::MAX));
    let eocd = (lowest..=bytes.len() - EOCD)
        .rev()
        .find(|&at| {
            u32_at(bytes, at) == Some(0x0605_4b50)
                && u16_at(bytes, at + 20).is_some_and(|comment| at + EOCD + usize::from(comment) == bytes.len())
        })
        .ok_or(PackageRefused::NotAZip)?;
    let (this_disk, dir_disk) = (u16_at(bytes, eocd + 4), u16_at(bytes, eocd + 6));
    let entries = u16_at(bytes, eocd + 10).ok_or(PackageRefused::NotAZip)?;
    let dir_len = u32_at(bytes, eocd + 12).ok_or(PackageRefused::NotAZip)? as usize;
    let dir_at = u32_at(bytes, eocd + 16).ok_or(PackageRefused::NotAZip)? as usize;
    if this_disk != Some(0) || dir_disk != Some(0) || entries == u16::MAX || dir_at == u32::MAX as usize {
        return Err(PackageRefused::NotAZip);
    }
    if dir_at.checked_add(dir_len).is_none_or(|end| end > eocd) {
        return Err(PackageRefused::NotAZip);
    }
    let (mut at, mut has_types, mut has_main) = (dir_at, false, false);
    for _ in 0..entries {
        if u32_at(bytes, at) != Some(0x0201_4b50) {
            return Err(PackageRefused::NotAZip);
        }
        let name_len = usize::from(u16_at(bytes, at + 28).ok_or(PackageRefused::NotAZip)?);
        let extra_len = usize::from(u16_at(bytes, at + 30).ok_or(PackageRefused::NotAZip)?);
        let comment_len = usize::from(u16_at(bytes, at + 32).ok_or(PackageRefused::NotAZip)?);
        let local = u32_at(bytes, at + 42).ok_or(PackageRefused::NotAZip)? as usize;
        let name_at = at + 46;
        let next = name_at + name_len + extra_len + comment_len;
        if next > dir_at + dir_len {
            return Err(PackageRefused::NotAZip);
        }
        // Its data starts at a local header, before the directory.
        if local >= dir_at || u32_at(bytes, local) != Some(0x0403_4b50) {
            return Err(PackageRefused::NotAZip);
        }
        let name = &bytes[name_at..name_at + name_len];
        has_types |= name == CONTENT_TYPES;
        has_main |= MAIN_PARTS.contains(&name);
        at = next;
    }
    if has_types && has_main {
        Ok(())
    } else {
        Err(PackageRefused::MissingPart)
    }
}

/// The content type a new document's file carries, by its extension.
fn content_type_of(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => return None,
    })
}

/// A name a new file may have: one path part, not empty, not dot-only, no control characters.
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

impl<C: CredentialStores> Engine<C> {
    /// See the module's doc. `base`: the version the document was edited from; `opening`: the epoch of the Office
    /// window's opening that saves ([`Self::viewer_opening`]).
    pub fn save_office_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        bytes: &Zeroizing<Vec<u8>>,
        base: i64,
        opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        check_package(bytes).map_err(|_| EditRefused::Failed)?;
        self.stage_office_bytes(session, file_id, bytes, base, opening)
    }

    /// See the module's doc: Keep both. `device` and `hhmm` name the copy as every conflict copy is named.
    pub fn save_office_copy_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        bytes: &Zeroizing<Vec<u8>>,
        device: &str,
        hhmm: &str,
    ) -> Result<NewCopy, EditRefused> {
        check_package(bytes).map_err(|_| EditRefused::Failed)?;
        self.save_bytes_copy_for(session, file_id, bytes, device, hhmm)
    }

    /// See the module's doc: a new document called `name` in the folder `folder` (`None`: the top). Its id.
    pub fn create_office_for(
        &self,
        session: SessionGeneration,
        folder: Option<&str>,
        name: &str,
        bytes: &Zeroizing<Vec<u8>>,
    ) -> Result<String, EditRefused> {
        check_package(bytes).map_err(|_| EditRefused::Failed)?;
        if !valid_name(name) {
            return Err(EditRefused::Failed);
        }
        let (token, key) = self.edit_credentials(session)?;
        let state = self.local_state_for_changes().map_err(|_| EditRefused::Unavailable)?;
        if state.session() != session {
            return Err(EditRefused::NotSignedIn);
        }
        let read = state
            .with(|db| {
                let parent = match folder {
                    Some(id) => {
                        let contract = db.get_file_contract_state(id).ok().flatten()?;
                        let row = db.get_file(id).ok().flatten()?;
                        Some((Some(contract), row.path.trim_matches('/').to_string()))
                    }
                    None => Some((None, String::new())),
                }?;
                let taken = db.child_names_lowercase(folder).ok()?;
                Some((parent, taken))
            })
            .map_err(|_| EditRefused::NotSignedIn)?;
        let Some(((parent, parent_path), taken)) = read else {
            return Err(EditRefused::Unavailable);
        };
        if let Some(contract) = &parent {
            if contract.item_kind != ItemKind::Folder {
                return Err(EditRefused::Unavailable);
            }
            if !contract.can_write() {
                return Err(EditRefused::ReadOnly);
            }
        }
        // A name that exists is never replaced: the person picks another.
        if taken.contains(&name.to_lowercase()) {
            return Err(EditRefused::Failed);
        }
        let target = FinderWriteTarget {
            file_id: None,
            parent_id: folder.map(str::to_string),
            filename: name.to_string(),
            rel_path: Some(if parent_path.is_empty() {
                name.to_string()
            } else {
                format!("{parent_path}/{name}")
            }),
            kind: FinderWriteItemKind::File,
            contents_path: Some("viewer-edit".to_string()),
            content_type: content_type_of(name).map(str::to_string),
            base_version_identifier: None,
        };
        let held = anonymous_file(bytes).ok_or(EditRefused::Failed)?;
        let outcome = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                EngineBridge::new(self.identity, db, api)
                    .queue_file_provider_create_from(target, Some(&held))
                    .map(|write| write.outcome)
            })
            .map_err(|_| EditRefused::NotSignedIn)?
            .map_err(|_| EditRefused::Failed)?;
        match outcome {
            FinderWriteOutcome::Queued { file_id: Some(id), .. } => {
                // w29: sent now, not at the runner's next tick (the Windows test machine run 11: a New document took 11-26 s to open).
                self.nudge_runner();
                Ok(id)
            }
            _ => Err(EditRefused::Failed),
        }
    }

    /// The one place the saved bytes are staged for the upload: sealed with core's account-bound recovery-snapshot
    /// seal (never the session scratch key: a staged save must survive a restart), so the staging folder holds no byte
    /// of the document. The upload of the op opens it as it reads. Same checks and the same one write path as
    /// [`Self::save_bytes_for`].
    fn stage_office_bytes(
        &self,
        session: SessionGeneration,
        file_id: &str,
        bytes: &[u8],
        base: i64,
        opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        let kind = package_kind(bytes).ok_or(EditRefused::Failed)?;
        self.save_bytes_sealed_for(session, file_id, bytes, base, opening, Some(kind))
    }
}

/// The kind core records for a package (1 docx, 2 xlsx, 3 pptx), by the part names in its directory (the end).
pub(crate) fn package_kind(package: &[u8]) -> Option<u8> {
    let tail = &package[package.len().saturating_sub(8 * 1024 * 1024)..];
    let has = |needle: &[u8]| tail.windows(needle.len()).any(|w| w == needle);
    if has(b"word/document.xml") {
        Some(1)
    } else if has(b"xl/workbook.xml") {
        Some(2)
    } else if has(b"ppt/presentation.xml") {
        Some(3)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "office_edit_api_tests.rs"]
mod tests;
