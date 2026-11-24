//! The sections the app grants, as the helper sees them: the document read-only, each page buffer read-write.
//!
//! A grant's handle belongs to the helper once it is on the wire, and the only way to close it is to map it and
//! drop the mapping (`beebeeb_office_sandbox::helper::map`). So [`crate::serve`] maps each grant exactly once and
//! drops the mapping as soon as its request is done, before the answer is written: on an answer, on an error and
//! on a skipped request alike.

use beebeeb_office_protocol::office::{ErrorCode, GrantRef};
use beebeeb_office_sandbox::{Access, helper};

/// A section mapped read-only. Unmapped, and its grant closed, when dropped.
pub trait Mapped {
    fn as_slice(&self) -> &[u8];
}

/// A section mapped read-write. Unmapped, and its grant closed, when dropped.
pub trait MappedMut {
    fn as_mut_slice(&mut self) -> &mut [u8];
}

/// Where grants are mapped: the sandbox in the helper ([`Granted`]), a fake in the tests.
pub trait Memory {
    /// Maps the grant `g` read-only (a document). A refusal is a code from the fixed list.
    fn read(&self, g: GrantRef) -> Result<Box<dyn Mapped>, ErrorCode>;
    /// Maps the grant `g` read-write (a page buffer). A refusal is a code from the fixed list.
    fn write(&self, g: GrantRef) -> Result<Box<dyn MappedMut>, ErrorCode>;
}

/// The grants the app handed this process through the sandbox.
pub struct Granted;

/// The sandbox's grant: the handle's value as this process sees it, and the section's length.
fn grant(g: GrantRef) -> beebeeb_office_sandbox::Granted {
    beebeeb_office_sandbox::Granted {
        wire: g.wire,
        len: g.len,
    }
}

struct ReadOnly(helper::Mapped);

impl Mapped for ReadOnly {
    fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }
}

struct ReadWrite(helper::Mapped);

impl MappedMut for ReadWrite {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        // Mapped read-write and checked in `Granted::write`, so this is the section; were it not, an empty buffer
        // fails the renderer's exact size check and the page is an error, never a write elsewhere.
        self.0.as_mut_slice().unwrap_or_default()
    }
}

impl Memory for Granted {
    fn read(&self, g: GrantRef) -> Result<Box<dyn Mapped>, ErrorCode> {
        // A refused map has closed the grant already (`helper::map`).
        let mapped = helper::map(grant(g), Access::Read).map_err(|_| ErrorCode::Internal)?;
        Ok(Box::new(ReadOnly(mapped)))
    }

    fn write(&self, g: GrantRef) -> Result<Box<dyn MappedMut>, ErrorCode> {
        let mut mapped = helper::map(grant(g), Access::ReadWrite).map_err(|_| ErrorCode::Internal)?;
        if mapped.as_mut_slice().is_none() {
            // Dropping `mapped` here closes the grant.
            return Err(ErrorCode::Internal);
        }
        Ok(Box::new(ReadWrite(mapped)))
    }
}
