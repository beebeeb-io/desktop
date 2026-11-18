//! An open Office file in the viewer: one [`OfficeView`] per file, over the file's confined renderer
//! (`engine_host::office_host`), and the source's Office arm ([`Office`]). Child of `viewer.rs`.
//!
//! - **The route** is `kinds::office_route` (behind `OFFICE_VIEWER_ON` and `OFFICE_SHIPS`), asked in `begin` BEFORE the
//!   viewer's own kinds. A type that does not ship, and `.ppt`, show DW-1556 and nothing is fetched. A file over the
//!   contract's limit (`check::MAX_INPUT_BYTES`) is refused before anything is read, and so is a PC without the helper.
//! - **The bytes:** the engine's decrypted bytes (`Zeroizing`), read on a worker as any viewer's are, are MOVED into the
//!   renderer's host ([`OfficeView::open`]) and never copied elsewhere.
//! - **The asks:** the view asks for a page, a tile, a thumbnail or an anchor while it is drawn
//!   ([`OfficeView::picture`], as a PDF's pages are asked: `viewer_pages.rs`) and for a block of cells
//!   ([`OfficeView::wants_cells`]). A picture is asked once; meanwhile the nearest held picture of the same page at
//!   another scale stays up. An ask that scrolled past is cancelled; at most [`TEXTURE_CAP_BYTES`] of pixels are held.
//! - **Plaintext:** a close, a lock and a sign-out drop every picture and wipe the body this view holds (sheet names,
//!   cells, formulas, outline). Pixels handed to the toolkit cannot be overwritten there, only dropped.
//!
//! Thread rule: the window's thread only. Every host call returns at once (`office_host`'s rule); the asks live in
//! `RefCell`s because the view asks while it is drawn (`&self`).
//!
//! Must not: build the editor's body (`Body::Word` is the fixture source's only; lock 1), save anything (lock 3: the
//! source does not override `save_document`, `resolve_document` or `commands`), log a name, a cell or a byte.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::office_scratch::ScratchStore;
use beebeeb_engine::app::viewer_api::{OpenFileError, OpenFileFacts};
use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{Align, Dirty};
use zeroize::{Zeroize, Zeroizing};

use super::{EngineViewerSource, OpenError, OpenFile, Opening, Recorded, ViewerEngine, body_of, file_of, log, read};
use crate::engine_host::office_host::{
    self, CannotOpen, CellData, CellRange, CloseWhy, DocKind, EditEvent, EditKind, EditOp, Edited, Event, HitMode,
    NoPreview as HostNoPreview, OfficeHost, OfficeState as HostState, OutlineEntry, PageSize, Purpose, Rect, RequestId,
    Template, Wake,
};
use crate::frames::office::docs::{DocBody, DocView};
use crate::frames::office::sheets::{self, SheetAnchor, SheetView};
use crate::frames::office::slides::{DeckBody, DeckView, SlideThumb};
use crate::frames::office::states::{self, OfficeState as ViewState, Reason, StateBody};
use crate::frames::viewers::kinds::{
    self, OFFICE_SHIPS, OFFICE_VIEWER_ON, OfficeKind, OfficePicture, OfficeRoute, OfficeShips, THUMB_W_PT, TILE_PX,
};
use crate::frames::viewers::source::{Body, EditAnswer, Stopped};

#[path = "viewer_office/cells.rs"]
mod cells;
#[path = "viewer_office/edit.rs"]
mod edit;
#[path = "viewer_office/glue.rs"]
mod glue;
#[path = "viewer_office/recovery.rs"]
mod recovery;
use cells::{Inflight, covers};
#[cfg_attr(not(test), allow(unused_imports))] // the tests set it
pub(crate) use glue::{EditShips, Office};

/// The pixels one open file holds at most (`w × h × 4` of every held picture). Past it the least recently wanted
/// pictures go first, never one wanted since the last poll.
pub(crate) const TEXTURE_CAP_BYTES: u64 = 256 * 1024 * 1024;
/// An ask that is not wanted again during this many polls is cancelled.
const UNWANTED_POLLS: u8 = 2;
/// A sheet's first block, asked as soon as its sheets are known.
const FIRST_ROWS: u32 = 64;
const FIRST_COLS: u32 = 26;
/// Blocks of cells are asked on this grid, so a scroll of a row or two asks nothing new.
const BLOCK_ROWS: u32 = 32;
const BLOCK_COLS: u32 = 8;
/// A column or row the file has not sized yet (Excel's defaults, in points).
const DEFAULT_COL_PT: f32 = 48.0;
const DEFAULT_ROW_PT: f32 = 15.0;
/// How many asked blocks of cells are remembered (the oldest are forgotten first).
const ASKED_BLOCKS_KEPT: usize = 4_096;

// ---------------------------------------------------------------------------------------------------------------
// The host, as this view uses it
// ---------------------------------------------------------------------------------------------------------------

/// The calls the view makes of its renderer: [`OfficeHost`] in the app ([`Live`]), a fake in the tests. Every call
/// returns at once. A request is named by this view's own number (the host's ids are its own).
#[cfg_attr(not(test), allow(dead_code))] // the editor calls reach it through the window once it is wired
pub(crate) trait Host: Send {
    fn state(&self) -> HostState;
    fn poll(&mut self) -> Vec<HostEvent>;
    fn request_page(&mut self, page: u32, scale: f32, region: Option<Rect>) -> Option<u64>;
    fn request_anchor(&mut self, sheet: u32, anchor: u32, scale: f32) -> Option<u64>;
    fn cancel(&mut self, id: u64);
    fn request_outline(&mut self) -> bool;
    fn request_stats(&mut self) -> bool;
    fn request_cells(&mut self, sheet: u32, range: CellRange) -> bool;
    fn retry(&mut self) -> bool;
    fn close(&mut self, why: CloseWhy);
    // An editing session's calls. The defaults are a viewer's: nothing is taken. `Some(n)`: sent, answered by a
    // [`HostEdit`] carrying `n` (this view's own number, as for a picture).
    fn edit(&mut self, _op: EditOp) -> Option<u64> {
        None
    }
    fn hit(&mut self, _page: u32, _x_pt: f32, _y_pt: f32, _mode: HitMode) -> Option<u64> {
        None
    }
    fn copy(&mut self) -> Option<u64> {
        None
    }
    fn layouts(&mut self) -> Option<u64> {
        None
    }
    fn insert_image(&mut self, _bytes: Zeroizing<Vec<u8>>, _png: bool) -> Option<u64> {
        None
    }
    fn save_bytes(&mut self, _purpose: Purpose) -> Option<u64> {
        None
    }
    /// The editor's answers since the last call. Never blocks, and never loses what `poll` hands over.
    fn poll_edit(&mut self) -> Vec<HostEdit> {
        Vec::new()
    }
    /// Where this editing session's unsaved work is sealed when a lock comes (the host's worker calls it).
    fn set_flush_sink(&mut self, _sink: crate::engine_host::office_host::FlushSink) {}
}

/// What an editing session answered. Every payload passed the protocol's check in the host.
pub(crate) enum HostEdit {
    Edited(Box<Edited>),
    Clip {
        text: String,
        html: Option<String>,
    },
    Layouts(Vec<String>),
    /// The document serialised for request `id`, and the revision it is.
    Saved {
        id: u64,
        #[cfg_attr(not(test), allow(dead_code))] // a snapshot's (w04) is told apart by it
        purpose: Purpose,
        revision: u64,
        bytes: Zeroizing<Vec<u8>>,
    },
    /// The helper refused request `id` and the session goes on. `too_large`: it said the answer is too large (a copy
    /// of that much: "Too much to copy at once").
    Failed {
        id: u64,
        too_large: bool,
    },
    /// The helper restarted from a snapshot; `at` is its time of day (UTC, "09:41").
    Restored { at: String },
    /// A second crash within a minute: the document is read-only now (the editor is no longer live).
    ReadOnly,
}

/// What the host handed over since the last poll. Every payload passed the protocol's check in the host.
pub(crate) enum HostEvent {
    /// The state changed: read [`Host::state`].
    State,
    /// A page, a tile, a thumbnail or an anchor that was asked under `id`: `w` × `h` RGBA8.
    Picture {
        id: u64,
        w: u32,
        h: u32,
        rgba: Vec<u8>,
        missing_content: bool,
    },
    Outline(Vec<OutlineEntry>),
    Stats {
        words: u32,
    },
    Cells {
        sheet: u32,
        range: CellRange,
        data: CellData,
    },
}

/// The app's host, with this view's numbers for its requests.
struct Live {
    host: OfficeHost,
    ids: HashMap<u64, RequestId>,
    back: HashMap<RequestId, u64>,
    next: u64,
    /// What `poll_edit` moved out of the host's poll on its way to the next [`Host::poll`] (never lost).
    parked: Vec<HostEvent>,
}

impl Live {
    fn new(host: OfficeHost) -> Live {
        Live {
            host,
            ids: HashMap::new(),
            back: HashMap::new(),
            next: 0,
            parked: Vec::new(),
        }
    }

    fn number(&mut self, id: Option<RequestId>) -> Option<u64> {
        let id = id?;
        let ours = self.next;
        self.next = self.next.wrapping_add(1);
        self.ids.insert(ours, id);
        self.back.insert(id, ours);
        Some(ours)
    }

    /// The view's number of an answered request, which is not asked about again.
    fn answered(&mut self, id: RequestId) -> u64 {
        let ours = self.back.remove(&id).unwrap_or(u64::MAX);
        self.ids.remove(&ours);
        ours
    }

    /// What the host's poll handed over, as this view's events, after `out`.
    fn convert(&mut self, out: &mut Vec<HostEvent>) {
        let events = self.host.poll();
        for event in events {
            match event {
                Event::State => out.push(HostEvent::State),
                Event::Page {
                    id,
                    w,
                    h,
                    rgba,
                    missing_content,
                    ..
                }
                | Event::Anchor {
                    id,
                    w,
                    h,
                    rgba,
                    missing_content,
                    ..
                } => match self.back.remove(&id) {
                    Some(ours) => {
                        self.ids.remove(&ours);
                        out.push(HostEvent::Picture {
                            id: ours,
                            w,
                            h,
                            rgba,
                            missing_content,
                        });
                    }
                    // Cancelled here meanwhile: nobody waits for it.
                    None => wipe_rgba(rgba),
                },
                Event::Outline(entries) => out.push(HostEvent::Outline(entries)),
                Event::Stats { words } => out.push(HostEvent::Stats { words }),
                Event::Cells { sheet, range, data } => out.push(HostEvent::Cells { sheet, range, data }),
            }
        }
    }

}

impl Host for Live {
    fn state(&self) -> HostState {
        self.host.state()
    }

    fn poll(&mut self) -> Vec<HostEvent> {
        let mut out = std::mem::take(&mut self.parked);
        self.convert(&mut out);
        out
    }

    fn request_page(&mut self, page: u32, scale: f32, region: Option<Rect>) -> Option<u64> {
        let id = self.host.request_page(page, scale, region);
        self.number(id)
    }

    fn request_anchor(&mut self, sheet: u32, anchor: u32, scale: f32) -> Option<u64> {
        let id = self.host.request_anchor(sheet, anchor, scale);
        self.number(id)
    }

    fn cancel(&mut self, id: u64) {
        if let Some(theirs) = self.ids.remove(&id) {
            self.back.remove(&theirs);
            self.host.cancel(theirs);
        }
    }

    fn request_outline(&mut self) -> bool {
        self.host.request_outline()
    }

    fn request_stats(&mut self) -> bool {
        self.host.request_stats()
    }

    fn request_cells(&mut self, sheet: u32, range: CellRange) -> bool {
        self.host.request_cells(sheet, range)
    }

    fn retry(&mut self) -> bool {
        // Only a retry the host took starts afresh: a refused one leaves the asks on their way as they are.
        let taken = self.host.retry();
        if taken {
            self.ids.clear();
            self.back.clear();
        }
        taken
    }

    fn close(&mut self, why: CloseWhy) {
        self.ids.clear();
        self.back.clear();
        self.parked.clear();
        self.host.close(why);
    }

    fn edit(&mut self, op: EditOp) -> Option<u64> {
        let id = self.host.edit(op).ok();
        self.number(id)
    }

    fn hit(&mut self, page: u32, x_pt: f32, y_pt: f32, mode: HitMode) -> Option<u64> {
        let id = self.host.hit(page, x_pt, y_pt, mode).ok();
        self.number(id)
    }

    fn copy(&mut self) -> Option<u64> {
        let id = self.host.copy().ok();
        self.number(id)
    }

    fn layouts(&mut self) -> Option<u64> {
        let id = self.host.layouts().ok();
        self.number(id)
    }

    fn insert_image(&mut self, bytes: Zeroizing<Vec<u8>>, png: bool) -> Option<u64> {
        let id = self.host.insert_image(bytes, png).ok();
        self.number(id)
    }

    fn save_bytes(&mut self, purpose: Purpose) -> Option<u64> {
        let id = self.host.save_bytes(purpose).ok();
        self.number(id)
    }

    fn set_flush_sink(&mut self, sink: crate::engine_host::office_host::FlushSink) {
        self.host.set_flush_sink(sink);
    }

    fn poll_edit(&mut self) -> Vec<HostEdit> {
        // The host moves the editor's answers out of its queue in `poll`: what else came with them is kept for the
        // next `poll`, never dropped.
        let mut parked = std::mem::take(&mut self.parked);
        self.convert(&mut parked);
        self.parked = parked;
        let mut out = Vec::new();
        for event in self.host.poll_edit() {
            out.push(match event {
                EditEvent::Edited { id, edited } => {
                    self.answered(id);
                    HostEdit::Edited(edited)
                }
                EditEvent::Clip { id, text, html } => {
                    self.answered(id);
                    HostEdit::Clip { text, html }
                }
                EditEvent::Layouts { id, names } => {
                    self.answered(id);
                    HostEdit::Layouts(names)
                }
                EditEvent::Saved {
                    id,
                    purpose,
                    revision,
                    bytes,
                } => HostEdit::Saved {
                    id: self.answered(id),
                    purpose,
                    revision,
                    bytes,
                },
                EditEvent::Failed { id, why } => HostEdit::Failed {
                    id: self.answered(id),
                    too_large: why == crate::engine_host::office_host::EditFailure::TooLarge,
                },
                EditEvent::Restored { at, .. } => HostEdit::Restored { at: clock_words(at) },
                EditEvent::ReadOnly { .. } => HostEdit::ReadOnly,
            });
        }
        out
    }
}

/// A time of day as "09:41" (UTC; the words only say roughly when).
fn clock_words(at: std::time::SystemTime) -> String {
    let secs = at.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) % 86_400;
    format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

fn wipe_rgba(mut rgba: Vec<u8>) {
    rgba.zeroize();
}

/// The protocol's kind of an Office type. `None`: `.ppt`, which has no renderer (2482 Q5).
fn doc_kind(kind: OfficeKind) -> Option<DocKind> {
    Some(match kind {
        OfficeKind::Docx => DocKind::Docx,
        OfficeKind::Doc => DocKind::Doc,
        OfficeKind::Xlsx => DocKind::Xlsx,
        OfficeKind::Xls => DocKind::Xls,
        OfficeKind::Pptx => DocKind::Pptx,
        OfficeKind::Ppt => return None,
    })
}

/// The scale a page is asked at: [`check::render_scale`] once, the same `f32` to the host (the helper refuses a raster
/// one pixel off).
pub(crate) fn scale_of(zoom_pct: u16, dpi: u16) -> f32 {
    check::render_scale(f32::from(zoom_pct) / 100.0, f32::from(dpi))
}

/// The scale a filmstrip slide `w_pt` wide is asked at: [`THUMB_W_PT`] wide at `dpi`.
pub(crate) fn thumb_scale(dpi: u16, w_pt: f32) -> f32 {
    THUMB_W_PT * f32::from(dpi) / 96.0 / w_pt
}

/// Tile (`col`, `row`) of a page of `size` at `scale`: a [`TILE_PX`] square, the last column and row clipped to the
/// page's raster. `None`: past the page.
pub(crate) fn tile_region(size: &PageSize, scale: f32, col: u32, row: u32) -> Option<Rect> {
    let (w, h) = check::page_pixels(size, scale)?;
    let (x, y) = (col.checked_mul(TILE_PX)?, row.checked_mul(TILE_PX)?);
    if x >= w || y >= h {
        return None;
    }
    Some(Rect {
        x,
        y,
        w: TILE_PX.min(w - x),
        h: TILE_PX.min(h - y),
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------------------------

/// Where the host is, as far as the body goes: a change rebuilds the body; the same again does not.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Pages(u32),
    Sheets(u32),
    State(ViewState),
    Closed,
}

/// The body under construction and what the asks need of the document.
struct Model {
    phase: Phase,
    body: Body,
    /// A new body to hand at the next poll.
    dirty: bool,
    /// Handed once instead of the body (a lock or a sign-out closed the renderer).
    closing: Option<Body>,
    /// The pages' sizes, while the host is in `Pages`.
    sizes: Vec<PageSize>,
    /// Rows and columns of each named sheet, and how many sheets follow without a name.
    sheets: Vec<(u32, u32)>,
    more: u32,
    /// Where each held cell is in the sheet's `cells`.
    index: HashMap<(u32, u32), usize>,
    /// The blocks asked of the active sheet (asked once, whether the host took them or not).
    asked: Vec<CellRange>,
    /// The blocks the host took and has not answered yet: an edit sent meanwhile makes the answer stale.
    inflight: Vec<Inflight>,
}

/// One held picture.
struct Held {
    pixels: rune::Pixels,
    bytes: u64,
    /// The poll turn it was last wanted in.
    wanted: u64,
    /// The newest revision the editor had reported when it was held: nothing held is older than the revision the
    /// view claims (an `Edited` drops what it names).
    revision: u64,
}

/// One ask on its way.
struct Out {
    key: OfficePicture,
    /// Polls ended without it being wanted.
    misses: u8,
    wanted: u64,
}

/// The pictures of one open file and the asks for them.
#[derive(Default)]
struct Pictures {
    held: HashMap<OfficePicture, Held>,
    out: HashMap<u64, Out>,
    asking: HashMap<OfficePicture, u64>,
    /// Asked and refused by the host: never asked again (until the document opens again).
    refused: HashSet<OfficePicture>,
    /// Wanted since the last poll.
    wanted: HashSet<OfficePicture>,
    /// The poll turn: one per poll.
    turn: u64,
    /// The newest revision an `Edited` reported (0: none yet, a viewer's).
    revision: u64,
}

impl Pictures {
    /// Drop every held picture (and the refusals) `gone` names. An ask on its way is not touched: an edit sent after
    /// the ask cancelled it already (`OfficeView::sent`), so what comes back was drawn after the last edit sent.
    fn drop_where(&mut self, gone: impl Fn(&OfficePicture) -> bool) {
        self.held.retain(|key, _| !gone(key));
        self.refused.retain(|key| !gone(key));
    }

    fn forget(&mut self) {
        self.held.clear();
        self.out.clear();
        self.asking.clear();
        self.refused.clear();
        self.wanted.clear();
    }

    fn held_bytes(&self) -> u64 {
        self.held.values().map(|held| held.bytes).sum()
    }

    /// The nearest held picture of the same page (slide, anchor) at another scale. Tiles have none: a tile of another
    /// scale covers another part of the page.
    fn nearest(&mut self, which: OfficePicture) -> Option<rune::Pixels> {
        let key = *self
            .held
            .keys()
            .filter(|held| same_picture(held, &which))
            .min_by_key(|held| density(held).abs_diff(density(&which)))?;
        let turn = self.turn;
        let held = self.held.get_mut(&key)?;
        held.wanted = turn;
        Some(held.pixels.clone())
    }

    /// Drop the least recently wanted pictures until at most `cap` bytes are held; never one wanted this turn.
    fn hold_to(&mut self, cap: u64) {
        let mut total = self.held_bytes();
        while total > cap {
            let turn = self.turn;
            let Some(oldest) = self
                .held
                .iter()
                .filter(|(_, held)| held.wanted < turn)
                .min_by_key(|(_, held)| held.wanted)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(gone) = self.held.remove(&oldest) {
                total -= gone.bytes;
            }
        }
    }
}

fn same_picture(a: &OfficePicture, b: &OfficePicture) -> bool {
    match (a, b) {
        (OfficePicture::Page { page: x, .. }, OfficePicture::Page { page: y, .. }) => x == y,
        (OfficePicture::Thumb { slide: x, .. }, OfficePicture::Thumb { slide: y, .. }) => x == y,
        (
            OfficePicture::Anchor {
                sheet: s1, anchor: a1, ..
            },
            OfficePicture::Anchor {
                sheet: s2, anchor: a2, ..
            },
        ) => s1 == s2 && a1 == a2,
        _ => false,
    }
}

/// How dense a picture is (zoom × dpi), to find the nearest scale.
fn density(which: &OfficePicture) -> u32 {
    match *which {
        OfficePicture::Page { zoom_pct, dpi, .. }
        | OfficePicture::Tile { zoom_pct, dpi, .. }
        | OfficePicture::Anchor { zoom_pct, dpi, .. } => u32::from(zoom_pct) * u32::from(dpi),
        OfficePicture::Thumb { dpi, .. } => u32::from(dpi) * 100,
    }
}

/// What an ask came to.
enum Asked {
    Id(u64),
    /// The host refused it, or it is outside the document: never asked again.
    Refused,
    /// Not open yet (or no more): asked again when it is.
    NotNow,
}

/// One open Office file: its renderer, the pictures it rendered, the body the window draws.
pub(crate) struct OfficeView {
    host: RefCell<Option<Box<dyn Host>>>,
    kind: OfficeKind,
    kept: bool,
    model: RefCell<Model>,
    pictures: RefCell<Pictures>,
    cap: u64,
    /// An editing session (`open_edit`, `new_doc`): what it edits.
    editing: Option<EditKind>,
    /// The editor's answers not taken yet (`poll_edit`), in order.
    edit_out: VecDeque<EditAnswer>,
    /// Serialised documents the editor handed over, not taken yet.
    saved: Vec<SavedBytes>,
    /// Requests the helper refused, not taken yet.
    failed: Vec<u64>,
    /// The copies asked and not answered yet, oldest first: a refusal of one is said ("Too much to copy at once").
    copies: Vec<u64>,
    /// Where the snapshots the helper hands over are kept (an editing session of a file; not a new document).
    tap: Option<recovery::Tap>,
}

/// The document as the helper serialised it, for request `id`.
pub(crate) struct SavedBytes {
    pub(crate) id: u64,
    pub(crate) revision: u64,
    pub(crate) bytes: Zeroizing<Vec<u8>>,
}

/// Where a save the view asked for stands.
pub(crate) enum SaveWait {
    Waiting,
    /// The document's bytes and the revision they are.
    Done(Zeroizing<Vec<u8>>, u64),
    /// The helper refused it, or stopped: nothing will come.
    Failed,
}

/// The protocol's kind of an editing session, as the viewer's.
fn view_kind(kind: EditKind) -> OfficeKind {
    match kind {
        EditKind::Docx => OfficeKind::Docx,
        EditKind::Xlsx => OfficeKind::Xlsx,
        EditKind::Pptx => OfficeKind::Pptx,
    }
}

impl OfficeView {
    /// Start the renderer for `bytes` (moved in, never copied elsewhere). Returns at once. A `.ppt` starts nothing: its
    /// bytes are dropped (wiped) unread, and the view says there is no preview for this type.
    pub(crate) fn open(helper: &Path, kind: OfficeKind, bytes: Zeroizing<Vec<u8>>, wake: Wake) -> OfficeView {
        let host = doc_kind(kind).map(|doc| {
            Box::new(Live::new(OfficeHost::open(helper, doc, bytes, wake))) as Box<dyn Host>
        });
        OfficeView::on(host, kind)
    }

    /// Start an editing session on `bytes` (moved in, never copied elsewhere). Returns at once.
    /// `scratch`: the session's sealed scratch, where the host keeps large working copies.
    pub(crate) fn open_edit(
        helper: &Path,
        kind: EditKind,
        bytes: Zeroizing<Vec<u8>>,
        wake: Wake,
        scratch: Option<ScratchStore>,
    ) -> OfficeView {
        let host = Box::new(Live::new(OfficeHost::open_edit_scratch(helper, kind, bytes, wake, scratch))) as Box<dyn Host>;
        OfficeView::editing_on(host, kind)
    }

    /// Start a new document of `kind` from `template`. Returns at once.
    pub(crate) fn new_doc(
        helper: &Path,
        kind: EditKind,
        template: Template,
        wake: Wake,
        scratch: Option<ScratchStore>,
    ) -> OfficeView {
        let host = Box::new(Live::new(OfficeHost::new_doc_scratch(helper, kind, template, wake, scratch))) as Box<dyn Host>;
        OfficeView::editing_on(host, kind)
    }

    /// An editing view over `host` (the app's through [`OfficeView::open_edit`]; the tests' fake).
    pub(crate) fn editing_on(host: Box<dyn Host>, kind: EditKind) -> OfficeView {
        let mut view = OfficeView::on(Some(host), view_kind(kind));
        view.editing = Some(kind);
        view
    }

    /// A view over `host` (the tests' fake; the app's through [`OfficeView::open`]). Its first body is the host's
    /// state now.
    pub(crate) fn on(host: Option<Box<dyn Host>>, kind: OfficeKind) -> OfficeView {
        let host = if doc_kind(kind).is_some() { host } else { None };
        let mut view = OfficeView {
            host: RefCell::new(host),
            kind,
            kept: false,
            model: RefCell::new(Model {
                phase: Phase::State(ViewState::Opening),
                body: Body::OfficeState(StateBody::new(ViewState::Opening, false)),
                dirty: true,
                closing: None,
                sizes: Vec::new(),
                sheets: Vec::new(),
                more: 0,
                index: HashMap::new(),
                asked: Vec::new(),
                inflight: Vec::new(),
            }),
            pictures: RefCell::new(Pictures::default()),
            cap: TEXTURE_CAP_BYTES,
            editing: None,
            edit_out: VecDeque::new(),
            saved: Vec::new(),
            failed: Vec::new(),
            copies: Vec::new(),
            tap: None,
        };
        let state = match view.host.get_mut() {
            Some(host) => host.state(),
            None => HostState::NoPreview(HostNoPreview::ThisType),
        };
        view.take_state(state);
        view
    }

    /// Keep the snapshots this editing session hands over as `file_id`'s recovery blob (edited from version `base`).
    pub(crate) fn set_recovery(&mut self, recovery: recovery::Recovery, file_id: &str, base: i64) {
        // A lock seals the unsaved work through the same door, on the host's thread, before the session ends.
        if let Some(host) = self.host.get_mut() {
            host.set_flush_sink(recovery.sink(file_id, base));
        }
        self.tap = Some(recovery::Tap::new(recovery, file_id, base));
    }

    /// Queue an answer for the editor (the source's restore).
    pub(crate) fn push_answer(&mut self, answer: EditAnswer) {
        self.edit_out.push_back(answer);
    }

    /// The file is kept on this PC: the states offer "Open in another app…" (decision 2482 Q8).
    pub(crate) fn set_kept(&mut self, kept: bool) {
        self.kept = kept;
        if let Body::OfficeState(state) = &mut self.model.get_mut().body {
            state.kept = kept;
        }
    }

    /// The pixel cap ([`TEXTURE_CAP_BYTES`] unless a test sets a smaller one).
    #[cfg(test)]
    pub(crate) fn set_cap(&mut self, cap: u64) {
        self.cap = cap;
    }

    /// A new body when something changed (the state, the outline, the counts, cells, a picture that came).
    pub(crate) fn poll(&mut self) -> Option<Body> {
        let events = match self.host.get_mut() {
            Some(host) => host.poll(),
            None => Vec::new(),
        };
        let mut redraw = false;
        for event in events {
            match event {
                HostEvent::State => {
                    if let Some(state) = self.host.get_mut().as_ref().map(|host| host.state()) {
                        self.take_state(state);
                    }
                }
                HostEvent::Picture {
                    id,
                    w,
                    h,
                    rgba,
                    missing_content,
                } => redraw |= self.take_picture(id, w, h, rgba, missing_content),
                HostEvent::Outline(entries) => self.take_outline(entries),
                HostEvent::Stats { words } => {
                    let model = self.model.get_mut();
                    if let Body::Doc(doc) = &mut model.body {
                        doc.view.words = words;
                        model.dirty = true;
                    }
                }
                HostEvent::Cells { sheet, range, data } => self.take_cells(sheet, range, data),
            }
        }
        self.pump_edit();
        self.end_turn();
        let model = self.model.get_mut();
        if let Some(closing) = model.closing.take() {
            return Some(closing);
        }
        (std::mem::take(&mut model.dirty) || redraw).then(|| model.body.clone())
    }

    /// The picture held for `which`, else asked once (and, meanwhile, the nearest held one of the same page).
    pub(crate) fn picture(&self, which: OfficePicture) -> Option<rune::Pixels> {
        let mut pictures = self.pictures.borrow_mut();
        let turn = pictures.turn;
        pictures.wanted.insert(which);
        if let Some(held) = pictures.held.get_mut(&which) {
            held.wanted = turn;
            return Some(held.pixels.clone());
        }
        if let Some(id) = pictures.asking.get(&which).copied() {
            if let Some(out) = pictures.out.get_mut(&id) {
                out.wanted = turn;
            }
        } else if !pictures.refused.contains(&which) {
            match self.ask(which) {
                Asked::Id(id) => {
                    pictures.out.insert(
                        id,
                        Out {
                            key: which,
                            misses: 0,
                            wanted: turn,
                        },
                    );
                    pictures.asking.insert(which, id);
                }
                Asked::Refused => {
                    pictures.refused.insert(which);
                }
                Asked::NotNow => {}
            }
        }
        pictures.nearest(which)
    }

    /// Ask the host for `which`.
    fn ask(&self, which: OfficePicture) -> Asked {
        let model = self.model.borrow();
        let mut host = self.host.borrow_mut();
        let Some(host) = host.as_mut() else {
            return Asked::NotNow;
        };
        let pages = matches!(model.phase, Phase::Pages(_));
        let id = match which {
            _ if !pages && !matches!(which, OfficePicture::Anchor { .. }) => return Asked::NotNow,
            OfficePicture::Page { page, zoom_pct, dpi } => host.request_page(page, scale_of(zoom_pct, dpi), None),
            OfficePicture::Tile {
                page,
                zoom_pct,
                dpi,
                col,
                row,
            } => {
                let scale = scale_of(zoom_pct, dpi);
                match model
                    .sizes
                    .get(page as usize)
                    .and_then(|size| tile_region(size, scale, col, row))
                {
                    Some(region) => host.request_page(page, scale, Some(region)),
                    None => None,
                }
            }
            OfficePicture::Thumb { slide, dpi } => match model.sizes.get(slide as usize) {
                Some(size) if size.w_pt > 0.0 => host.request_page(slide, thumb_scale(dpi, size.w_pt), None),
                _ => None,
            },
            OfficePicture::Anchor { .. } if !matches!(model.phase, Phase::Sheets(_)) => return Asked::NotNow,
            OfficePicture::Anchor {
                sheet,
                anchor,
                zoom_pct,
                dpi,
            } => host.request_anchor(sheet, anchor, scale_of(zoom_pct, dpi)),
        };
        id.map_or(Asked::Refused, Asked::Id)
    }

    /// The view shows these cells of `sheet`: asked (on the block grid, cut to `office_host::MAX_ASKED_CELLS`) unless
    /// already asked. Another sheet than the one held: its cells replace the held ones.
    pub(crate) fn wants_cells(&self, sheet: u32, first_row: u32, first_col: u32, rows: u32, cols: u32) {
        let mut model = self.model.borrow_mut();
        if !matches!(model.phase, Phase::Sheets(_)) {
            return;
        }
        let held = match &model.body {
            Body::Sheet(view) => view.sheet,
            _ => return,
        };
        if held != sheet && !model.switch_sheet(sheet) {
            return;
        }
        let Some(range) = model.block(first_row, first_col, rows, cols) else {
            return;
        };
        if model.asked.iter().any(|asked| covers(asked, &range)) {
            return;
        }
        if let Some(host) = self.host.borrow_mut().as_mut() {
            if host.request_cells(sheet, range) {
                model.expect(sheet, range);
            }
        }
        model.remember(range);
    }

    /// Try again after the renderer stopped (once per open: the host's rule). `false`: not possible now.
    pub(crate) fn retry(&mut self) -> bool {
        let Some(host) = self.host.get_mut() else {
            return false;
        };
        if !host.retry() {
            return false;
        }
        self.forget();
        let model = self.model.get_mut();
        model.phase = Phase::State(ViewState::Opening);
        let mut body = StateBody::new(ViewState::Opening, self.kept);
        body.tries = 1;
        model.body = Body::OfficeState(body);
        model.dirty = true;
        true
    }

    /// End the renderer and drop every picture and the text the body holds.
    pub(crate) fn close(&mut self, why: CloseWhy) {
        if let Some(host) = self.host.get_mut() {
            host.close(why);
        }
        self.forget();
        let model = self.model.get_mut();
        model.phase = Phase::Closed;
        model.dirty = false;
    }

    // -----------------------------------------------------------------------------------------------------------
    // The editor (an editing session only)
    // -----------------------------------------------------------------------------------------------------------
    // The window reaches these through `ViewerSource` (`edit.rs`) once the editor's UI is wired.

    /// An editing session whose document is open (the renderer answered with its pages or sheets).
    pub(crate) fn can_edit(&self) -> bool {
        self.editing.is_some() && matches!(self.model.borrow().phase, Phase::Pages(_) | Phase::Sheets(_))
    }

    /// The renderer ended or failed (not opening, not open): nothing more comes of it.
    pub(crate) fn ended(&self) -> bool {
        !matches!(self.model.borrow().phase, Phase::Pages(_) | Phase::Sheets(_) | Phase::State(ViewState::Opening))
    }

    /// The body as it is now (a save's answer carries it: the document stays as it is on screen).
    pub(crate) fn body_now(&self) -> Body {
        self.model.borrow().body.clone()
    }

    /// An edit, a click or an image is on its way: every picture asked before it is older than it, so those asks end
    /// (their answers are discarded), and so are the blocks of cells on their way (`Model::stale_inflight`).
    fn sent(&mut self) {
        let asked: Vec<u64> = {
            let pictures = self.pictures.get_mut();
            pictures.asking.clear();
            pictures.out.drain().map(|(id, _)| id).collect()
        };
        if let Some(host) = self.host.get_mut() {
            for id in asked {
                host.cancel(id);
            }
        }
        self.model.get_mut().stale_inflight();
    }

    fn send(&mut self, call: impl FnOnce(&mut dyn Host) -> Option<u64>) -> Option<u64> {
        if !self.can_edit() {
            return None;
        }
        let id = call(self.host.get_mut().as_mut()?.as_mut())?;
        self.sent();
        Some(id)
    }

    pub(crate) fn edit(&mut self, op: EditOp) -> bool {
        self.send(|host| host.edit(op)).is_some()
    }

    pub(crate) fn hit(&mut self, page: u32, x_pt: f32, y_pt: f32, mode: HitMode) -> bool {
        self.send(|host| host.hit(page, x_pt, y_pt, mode)).is_some()
    }

    pub(crate) fn insert_image(&mut self, bytes: Zeroizing<Vec<u8>>, png: bool) -> bool {
        self.send(|host| host.insert_image(bytes, png)).is_some()
    }

    pub(crate) fn copy(&mut self) -> bool {
        if !self.can_edit() {
            return false;
        }
        let Some(id) = self.host.get_mut().as_mut().and_then(|host| host.copy()) else {
            return false;
        };
        self.copies.push(id);
        true
    }

    pub(crate) fn layouts(&mut self) -> bool {
        self.can_edit() && self.host.get_mut().as_mut().and_then(|host| host.layouts()).is_some()
    }

    /// Ask for the document's bytes. The request's number: [`OfficeView::save_wait`] says how it stands.
    pub(crate) fn save(&mut self, purpose: Purpose) -> Option<u64> {
        if !self.can_edit() {
            return None;
        }
        self.host.get_mut().as_mut()?.save_bytes(purpose)
    }

    /// How the save asked as `request` stands. Bytes handed over for another request are dropped (wiped).
    pub(crate) fn save_wait(&mut self, request: u64) -> SaveWait {
        self.pump_edit();
        if let Some(at) = self.saved.iter().position(|saved| saved.id == request) {
            let saved = self.saved.remove(at);
            return SaveWait::Done(saved.bytes, saved.revision);
        }
        if let Some(at) = self.failed.iter().position(|id| *id == request) {
            self.failed.remove(at);
            return SaveWait::Failed;
        }
        match self.model.get_mut().phase {
            Phase::Pages(_) | Phase::Sheets(_) => SaveWait::Waiting,
            _ => SaveWait::Failed,
        }
    }

    /// The editor's next answer, if any arrived.
    pub(crate) fn next_answer(&mut self) -> Option<EditAnswer> {
        self.pump_edit();
        self.edit_out.pop_front()
    }

    /// Move what the editing session answered into the view: an `Edited` first changes what the view holds, then it
    /// is handed on.
    fn pump_edit(&mut self) {
        let events = match self.host.get_mut() {
            Some(host) if self.editing.is_some() => host.poll_edit(),
            _ => return,
        };
        for event in events {
            match event {
                HostEdit::Edited(edited) => {
                    self.apply_edited(&edited);
                    self.edit_out.push_back(EditAnswer::Edited(*edited));
                }
                HostEdit::Clip { text, html } => {
                    // Answers come in the order asked: this one is the oldest copy's.
                    if !self.copies.is_empty() {
                        self.copies.remove(0);
                    }
                    self.edit_out.push_back(EditAnswer::Clip { text, html });
                }
                HostEdit::Layouts(names) => self.edit_out.push_back(EditAnswer::Layouts(names)),
                // A snapshot (the host's own timer) is kept as the file's recovery blob and never reaches a save.
                HostEdit::Saved {
                    purpose: Purpose::Snapshot,
                    bytes,
                    ..
                } => {
                    if let Some(tap) = &self.tap {
                        tap.keep(&bytes);
                    }
                }
                HostEdit::Saved { id, revision, bytes, .. } => self.saved.push(SavedBytes { id, revision, bytes }),
                // A copy the helper could not answer at all is said; any other refusal waits for who asked it.
                HostEdit::Failed { id, too_large } => match self.copies.iter().position(|copy| *copy == id) {
                    Some(at) => {
                        self.copies.remove(at);
                        if too_large {
                            self.edit_out.push_back(EditAnswer::CopyTooLarge);
                        }
                    }
                    None => self.failed.push(id),
                },
                HostEdit::Restored { at } => self.edit_out.push_back(EditAnswer::Restored { at }),
                HostEdit::ReadOnly => self.edit_out.push_back(EditAnswer::Stopped),
            }
        }
    }

    /// What an `Edited` names is dropped: the pictures of the pages it redraws (all of them when it says everything,
    /// or the pages or slides changed), the blocks of cells it changed. What is asked next is of this revision or a
    /// later one, and a re-asked block replaces the old block whole.
    fn apply_edited(&mut self, edited: &Edited) {
        let kind = self.kind;
        let everything = matches!(edited.dirty, Dirty::All) || edited.pages.is_some() || edited.slides.is_some();
        let pictures = self.pictures.get_mut();
        pictures.revision = pictures.revision.max(edited.revision);
        match kind {
            OfficeKind::Xlsx | OfficeKind::Xls => {
                let sheets: HashSet<u32> = edited.cells.iter().map(|(sheet, _)| *sheet).collect();
                if matches!(edited.dirty, Dirty::All) || !sheets.is_empty() {
                    let all = matches!(edited.dirty, Dirty::All);
                    pictures.drop_where(|key| {
                        matches!(key, OfficePicture::Anchor { sheet, .. } if all || sheets.contains(sheet))
                    });
                }
                self.model.get_mut().edited_cells(&edited.cells);
            }
            _ if everything => pictures.drop_where(|_| true),
            _ => {
                let pages: HashSet<u32> = match &edited.dirty {
                    Dirty::Rects(rects) => rects.iter().map(|rect| rect.page).collect(),
                    Dirty::All => HashSet::new(),
                };
                pictures.drop_where(|key| match key {
                    OfficePicture::Page { page, .. } | OfficePicture::Tile { page, .. } => pages.contains(page),
                    OfficePicture::Thumb { slide, .. } => pages.contains(slide),
                    OfficePicture::Anchor { .. } => false,
                });
            }
        }
        if let Some(opened) = &edited.pages {
            self.resize(opened.pages, opened.sizes.clone());
        } else if let Some(count) = edited.slides {
            self.recount_deck(count);
        }
        self.model.get_mut().dirty = true;
        // The words and the outline of a document are read again (what is drawn is of the revision it claims).
        if kind == OfficeKind::Docx {
            let regrown = edited.pages.is_some() || matches!(edited.dirty, Dirty::All);
            if let Some(host) = self.host.get_mut() {
                host.request_stats();
                if regrown {
                    host.request_outline();
                }
            }
        }
    }

    /// The host's pages changed under an editing session (the count or a size): the held body is updated in place
    /// (its page, zoom and outline stay), never rebuilt. `None`: handled. `Some`: not that case.
    fn resize_in_place(&mut self, state: HostState) -> Option<HostState> {
        let paged = matches!(self.kind, OfficeKind::Docx | OfficeKind::Pptx);
        match state {
            HostState::Pages { pages, sizes } if self.editing.is_some() && paged => {
                let model = self.model.get_mut();
                if !matches!(model.phase, Phase::Pages(_)) {
                    return Some(HostState::Pages { pages, sizes });
                }
                if model.sizes != sizes || model.phase != Phase::Pages(pages) {
                    self.resize(pages, sizes);
                }
                None
            }
            state => Some(state),
        }
    }

    /// `pages` pages of `sizes`: the body's counts and sizes, and every picture of the old geometry gone.
    fn resize(&mut self, pages: u32, sizes: Vec<PageSize>) {
        let asked: Vec<u64> = {
            let pictures = self.pictures.get_mut();
            pictures.held.clear();
            pictures.refused.clear();
            pictures.asking.clear();
            pictures.out.drain().map(|(id, _)| id).collect()
        };
        if let Some(host) = self.host.get_mut() {
            for id in asked {
                host.cancel(id);
            }
        }
        let model = self.model.get_mut();
        model.phase = Phase::Pages(pages);
        match &mut model.body {
            Body::Doc(doc) => {
                doc.view.pages = pages;
                doc.view.page_sizes_pt = sizes.iter().map(|size| (size.w_pt, size.h_pt)).collect();
                doc.page = doc.page.clamp(1, pages.max(1));
            }
            Body::Deck(deck) => set_deck_count(deck, pages),
            _ => {}
        }
        model.sizes = sizes;
        model.dirty = true;
    }

    fn recount_deck(&mut self, count: u32) {
        let model = self.model.get_mut();
        if let Body::Deck(deck) = &mut model.body {
            set_deck_count(deck, count);
            model.dirty = true;
        }
    }

    /// The body as this view holds it (a test reads it after a close: the text is overwritten).
    #[cfg(test)]
    pub(crate) fn held_body(&self) -> Body {
        self.model.borrow().body.clone()
    }

    /// How many pictures are held and their bytes.
    #[cfg(test)]
    pub(crate) fn held_pictures(&self) -> (usize, u64) {
        let pictures = self.pictures.borrow();
        (pictures.held.len(), pictures.held_bytes())
    }

    /// The revision `which` was held under.
    #[cfg(test)]
    pub(crate) fn held_at(&self, which: OfficePicture) -> Option<u64> {
        self.pictures.borrow().held.get(&which).map(|held| held.revision)
    }

    /// Whether `which` is held.
    #[cfg(test)]
    pub(crate) fn holds(&self, which: OfficePicture) -> bool {
        self.pictures.borrow().held.contains_key(&which)
    }

    /// Every picture and ask goes, and the body's text is overwritten.
    fn forget(&mut self) {
        self.pictures.get_mut().forget();
        let model = self.model.get_mut();
        model.body.wipe();
        model.sizes.clear();
        model.sheets.clear();
        model.more = 0;
        model.index.clear();
        model.asked.clear();
        model.inflight.clear();
        for answer in self.edit_out.drain(..) {
            wipe_answer(answer);
        }
        self.saved.clear();
        self.failed.clear();
    }

    /// The host's state, as the window draws it.
    fn take_state(&mut self, state: HostState) {
        let state = match self.resize_in_place(state) {
            Some(state) => state,
            None => return,
        };
        let kind = self.kind;
        let failure = |state| Phase::State(state);
        let phase = match &state {
            HostState::Opening => failure(ViewState::Opening),
            HostState::Pages { pages, .. } if matches!(kind, OfficeKind::Docx | OfficeKind::Doc | OfficeKind::Pptx) => {
                Phase::Pages(*pages)
            }
            HostState::Sheets { sheets, more } if matches!(kind, OfficeKind::Xlsx | OfficeKind::Xls) => {
                Phase::Sheets(u32::try_from(sheets.len()).unwrap_or(u32::MAX).saturating_add(*more))
            }
            // The renderer answered with another kind of document than the file's: not shown.
            HostState::Pages { .. } | HostState::Sheets { .. } => failure(ViewState::Stopped { retried: true }),
            HostState::NoPreview(HostNoPreview::ThisType | HostNoPreview::Password) => {
                failure(ViewState::NoPreview(Reason::UnsupportedType))
            }
            HostState::NoPreview(HostNoPreview::NoSandbox) => failure(ViewState::NoPreview(Reason::NotOnThisComputer)),
            HostState::NoPreview(HostNoPreview::NeedsWindowsUpdate) => {
                failure(ViewState::NoPreview(Reason::NeedsWindowsUpdate))
            }
            HostState::NoPreview(HostNoPreview::NotInstalled) => failure(ViewState::Stopped { retried: true }),
            HostState::CannotOpen(CannotOpen::Damaged) => failure(ViewState::NoPreview(Reason::DamagedOrOverLimit)),
            HostState::CannotOpen(CannotOpen::TooLarge) => failure(ViewState::NoPreview(Reason::TooLarge)),
            HostState::Stopped { can_retry, .. } => failure(ViewState::Stopped { retried: !can_retry }),
            HostState::Closed(_) => Phase::Closed,
        };
        if phase == self.model.get_mut().phase {
            wipe_state(state);
            return;
        }
        // Whatever was open before goes, whatever this one is.
        self.forget();
        // An editing session that ends is told once: the document on screen is no longer live.
        if self.editing.is_some() && matches!(phase, Phase::State(shown) if shown != ViewState::Opening) {
            self.edit_out.push_back(EditAnswer::Stopped);
        }
        let kept = self.kept;
        let model = self.model.get_mut();
        model.phase = phase;
        model.dirty = true;
        match (phase, state) {
            (Phase::Pages(pages), HostState::Pages { sizes, .. }) => {
                if kind == OfficeKind::Pptx {
                    model.body = Body::Deck(DeckBody::new(DeckView::of(pages)));
                } else {
                    let view = DocView {
                        pages,
                        page_sizes_pt: sizes.iter().map(|size| (size.w_pt, size.h_pt)).collect(),
                        outline: Vec::new(),
                        words: 0,
                        zoom: 100,
                        older_format: kind == OfficeKind::Doc,
                    };
                    // No language is known from the renderer: none is said.
                    model.body = Body::Doc(DocBody::new(view, ""));
                    if let Some(host) = self.host.get_mut() {
                        host.request_outline();
                        host.request_stats();
                    }
                }
                model.sizes = sizes;
            }
            (Phase::Sheets(_), HostState::Sheets { sheets: named, more }) => {
                model.sheets = named.iter().map(|sheet| (sheet.rows, sheet.cols)).collect();
                model.more = more;
                let extent = model.sheets.first().copied().unwrap_or((0, 0));
                let mut tabs = Vec::with_capacity(named.len());
                for mut sheet in named {
                    tabs.push(std::mem::take(&mut sheet.name));
                }
                model.body = Body::Sheet(SheetView {
                    tabs,
                    active: 0,
                    col_widths: Vec::new(),
                    row_heights: Vec::new(),
                    cells: Vec::new(),
                    selection: None,
                    current: sheets::Cell { row: 0, col: 0 },
                    formula: String::new(),
                    frozen: (0, 0),
                    merges: Vec::new(),
                    sheet: 0,
                    extent,
                    top_left: sheets::Cell { row: 0, col: 0 },
                    anchors: Vec::new(),
                    missing_content: false,
                    more: sheets::More {
                        unnamed: more,
                        open: false,
                    },
                });
                if let Some(range) = model.block(0, 0, FIRST_ROWS, FIRST_COLS) {
                    if let Some(host) = self.host.get_mut() {
                        if host.request_cells(0, range) {
                            model.expect(0, range);
                        }
                    }
                    model.remember(range);
                }
            }
            (Phase::State(shown), state) => {
                wipe_state(state);
                model.body = Body::OfficeState(StateBody::new(shown, kept));
            }
            (Phase::Closed, HostState::Closed(why)) => {
                // The body was wiped by `forget`, and every picture dropped. A lock or a sign-out says so once.
                model.dirty = false;
                if why != CloseWhy::Closed {
                    model.closing = Some(Body::Stopped(Stopped::SessionChanged));
                }
            }
            (_, state) => wipe_state(state),
        }
    }

    /// A picture came: held under the key it was asked for. `true`: the window draws it.
    fn take_picture(&mut self, id: u64, w: u32, h: u32, rgba: Vec<u8>, missing_content: bool) -> bool {
        let pictures = self.pictures.get_mut();
        let Some(out) = pictures.out.remove(&id) else {
            wipe_rgba(rgba);
            return false;
        };
        pictures.asking.remove(&out.key);
        let bytes = u64::from(w) * u64::from(h) * 4;
        if w == 0 || h == 0 || rgba.len() as u64 != bytes {
            wipe_rgba(rgba);
            pictures.refused.insert(out.key);
            return false;
        }
        let pixels = rune::Pixels::new();
        pixels.set(w, h, rgba);
        pictures.held.insert(
            out.key,
            Held {
                pixels,
                bytes,
                wanted: out.wanted,
                revision: pictures.revision,
            },
        );
        if missing_content {
            let model = self.model.get_mut();
            match &mut model.body {
                Body::Doc(doc) if doc.quiet.is_none() => {
                    doc.quiet = Some(states::QUIET_LINE);
                    model.dirty = true;
                }
                Body::Sheet(view) if !view.missing_content => {
                    view.missing_content = true;
                    model.dirty = true;
                }
                _ => {}
            }
        }
        true
    }

    fn take_outline(&mut self, mut entries: Vec<OutlineEntry>) {
        let model = self.model.get_mut();
        let Body::Doc(doc) = &mut model.body else {
            for entry in &mut entries {
                entry.text.zeroize();
            }
            return;
        };
        for (_, text, _) in &mut doc.view.outline {
            text.zeroize();
        }
        // The protocol's pages are from 0; the outline's from 1.
        doc.view.outline = entries
            .into_iter()
            .map(|entry| (entry.level, entry.text, entry.page.saturating_add(1)))
            .collect();
        model.dirty = true;
    }

    fn take_cells(&mut self, sheet: u32, range: CellRange, mut data: CellData) {
        let model = self.model.get_mut();
        // An answer to a block asked before an edit was sent is older than the edit: dropped, and the block is asked
        // again at the next draw (its fresh answer replaces the block whole).
        if !model.answered(sheet, &range) {
            wipe_cells(&mut data);
            model.forget_asked(&range);
            model.dirty = true;
            return;
        }
        match &model.body {
            Body::Sheet(view) if view.sheet == sheet => {}
            _ => {
                wipe_cells(&mut data);
                return;
            }
        }
        model.merge(range, data);
        model.dirty = true;
    }

    /// The end of a poll: the cap, then what scrolled past is cancelled; the wanted set starts again.
    fn end_turn(&mut self) {
        let pictures = self.pictures.get_mut();
        pictures.hold_to(self.cap);
        let Pictures {
            out, asking, wanted, ..
        } = pictures;
        let mut gone = Vec::new();
        for (id, ask) in out.iter_mut() {
            if wanted.contains(&ask.key) {
                ask.misses = 0;
            } else {
                ask.misses = ask.misses.saturating_add(1);
                if ask.misses >= UNWANTED_POLLS {
                    gone.push(*id);
                }
            }
        }
        for id in gone {
            if let Some(ask) = out.remove(&id) {
                asking.remove(&ask.key);
            }
            if let Some(host) = self.host.get_mut() {
                host.cancel(id);
            }
        }
        wanted.clear();
        pictures.turn += 1;
    }
}

impl Drop for OfficeView {
    fn drop(&mut self) {
        self.close(CloseWhy::Closed);
    }
}

/// Overwrite the names a state carries that nobody took.
fn wipe_state(state: HostState) {
    if let HostState::Sheets { mut sheets, .. } = state {
        for sheet in &mut sheets {
            sheet.name.zeroize();
        }
    }
}

fn wipe_cells(data: &mut CellData) {
    for cell in &mut data.cells {
        cell.text.zeroize();
        if let Some(formula) = cell.formula.as_mut() {
            formula.zeroize();
        }
        cell.value = None;
    }
}

/// An answer nobody took: its text is overwritten.
fn wipe_answer(answer: EditAnswer) {
    if let EditAnswer::Clip { mut text, html } = answer {
        text.zeroize();
        if let Some(mut html) = html {
            html.zeroize();
        }
    }
}

/// A deck of `count` slides: the filmstrip's slides again, the slide on the stage kept when it is still there.
fn set_deck_count(deck: &mut DeckBody, count: u32) {
    deck.view.slides = (1..=count).map(|n| SlideThumb { n }).collect();
    deck.view.count = count;
    deck.view.current = deck.view.current.clamp(1, count.max(1));
}
