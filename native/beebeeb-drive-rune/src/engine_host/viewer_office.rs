//! An open Office file in the viewer: one [`OfficeView`] per file, over the file's confined renderer
//! (`engine_host::office_host`). Child of `viewer.rs`.
//!
//! The route is `kinds::office_route` (behind `OFFICE_VIEWER_ON` and `OFFICE_SHIPS`); the view asks for a page, a
//! tile, a thumbnail or an anchor while it is drawn (`ViewerSource::office_picture`) and for a block of cells
//! (`ViewerSource::office_cells`), as a PDF's pages are asked (`viewer_pages.rs`).
//!
//! Must not: build the editor's body (`Body::Word` is the fixture's only), save anything, log a name or a byte.

#![allow(dead_code)] // V1-S: the signatures the scenes and the scroll harness build against; the bodies follow.

use std::path::Path;

use zeroize::Zeroizing;

use crate::engine_host::office_host::{CloseWhy, Wake};
use crate::frames::viewers::kinds::{OfficeKind, OfficePicture};
use crate::frames::viewers::source::Body;

/// One open Office file: its renderer, the pictures it rendered, the body the window draws.
pub(crate) struct OfficeView {
    _private: (),
}

impl OfficeView {
    /// Start the renderer for `bytes` (moved in, never copied elsewhere). Returns at once.
    pub(crate) fn open(_helper: &Path, _kind: OfficeKind, _bytes: Zeroizing<Vec<u8>>, _wake: Wake) -> OfficeView {
        unimplemented!("body follows V1-S")
    }

    /// A new body when something changed.
    pub(crate) fn poll(&mut self) -> Option<Body> {
        unimplemented!("body follows V1-S")
    }

    /// The picture held for `which`, else asked once (and the nearest held one of the same page meanwhile).
    pub(crate) fn picture(&self, _which: OfficePicture) -> Option<rune::Pixels> {
        unimplemented!("body follows V1-S")
    }

    /// The view shows these cells of `sheet`: asked unless held or already asked.
    pub(crate) fn wants_cells(&self, _sheet: u32, _first_row: u32, _first_col: u32, _rows: u32, _cols: u32) {
        unimplemented!("body follows V1-S")
    }

    /// Try again after the renderer stopped. `false`: not possible now.
    pub(crate) fn retry(&mut self) -> bool {
        unimplemented!("body follows V1-S")
    }

    /// End the renderer and drop every picture and the body's text.
    pub(crate) fn close(&mut self, _why: CloseWhy) {
        unimplemented!("body follows V1-S")
    }
}

impl Drop for OfficeView {
    fn drop(&mut self) {
        // The body follows V1-S: `close(CloseWhy::Closed)`.
    }
}
