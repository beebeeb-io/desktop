//! Uploads: the system's file picker (DW-0415, DW-0416, DW-0480: Windows draws it), a drag from
//! File Explorer over the list (DW-0481), uploads in their folder and in the status line
//! (DW-0426), and every upload with its state (DW-0427).
//!
//! **The picker is Windows' own** (`IFileDialog`): this group draws no picker. It sets the
//! picker's words ([`PickerAsk`]: title, button, filter) and hands the request to the shell
//! ([`State::take_picker`]); what the person picked comes back through [`State::picked`], which
//! uploads it as a drop does. Cancel comes back as nothing, and nothing happens.
//!
//! What uploads is the source's ([`Uploads`], `FilesSource::uploads`): a source that knows
//! nothing has none, and the status line says what it said before.

use super::Intent;
use super::source::Action;
use crate::State;
use crate::theme::icons_files;
use crate::theme::scale;
use crate::theme::tokens::{Theme, type_};
use crate::ui::dialog::{Dialog, Foot, Head, Surface, dialog};
use crate::ui::file_row::{self, RowLook};
use crate::ui::file_tile::{self, TilePicture, TileSize};
use crate::ui::grid_tiles::Thumb;
use crate::ui::kit;
use crate::ui::menu::{self, Entry};
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::status_bar::{Dot, bar as status_bar};
use rune::{Color, El, Tx, col, el, row};

// ---- The system's picker ----

/// What is picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    /// Files, several at once (DW-0415; on Pictures it is DW-0416).
    Files,
    /// One folder, with everything in it (DW-0480).
    Folder,
}

/// A request for Windows' picker, with Beebeeb's words on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerAsk {
    pub kind: PickerKind,
    /// `IFileDialog::SetTitle`: "Upload to Files".
    pub title: String,
    /// `IFileDialog::SetOkButtonLabel`: "Upload".
    pub button: String,
    /// The file type filter's one entry; a folder picker has none.
    pub filter: Option<&'static str>,
    /// The folder the picked items go into (`None`: the root).
    pub into: Option<String>,
    /// The session it was asked under (`Actions::session_no`): an answer that comes back under
    /// another one is dropped.
    pub asked_in: u64,
}

/// The picker's words for the folder shown (`folder`: its name).
pub fn picker_words(kind: PickerKind, folder: &str, into: Option<String>) -> PickerAsk {
    PickerAsk {
        kind,
        title: match kind {
            PickerKind::Files => format!("Upload to {folder}"),
            PickerKind::Folder => format!("Upload a folder to {folder}"),
        },
        button: upload_button(None),
        filter: (kind == PickerKind::Files).then_some("All files"),
        into,
        asked_in: 0,
    }
}

/// The picker's button: "Upload", or "Upload 3" once photos are chosen (DW-0416).
pub fn upload_button(chosen: Option<usize>) -> String {
    match chosen {
        Some(count) if count > 0 => format!("Upload {count}"),
        _ => "Upload".to_string(),
    }
}

impl State {
    /// Upload files… and Upload a folder…: ask for Windows' picker (the shell opens it).
    pub fn ask_picker(&mut self, kind: PickerKind) {
        if !self.can_add() {
            return;
        }
        let into = self.listing.folder.id.clone();
        self.actions.picker = Some(PickerAsk {
            asked_in: self.actions.session_no,
            ..picker_words(kind, &self.listing.folder.name, into)
        });
    }

    /// The picker to open, once.
    pub fn take_picker(&mut self) -> Option<PickerAsk> {
        self.actions.picker.take()
    }

    /// What the picker came back with. `None` or nothing chosen (Cancel, Esc): nothing happens.
    /// Else the chosen files or folder upload into the folder the picker was asked for (DW-0426).
    ///
    /// An answer to a picker that was asked under an EARLIER session (the account signed out or
    /// changed while Windows' dialog was up) is dropped: its folder is another account's.
    pub fn picked(&mut self, ask: &PickerAsk, chosen: Option<Vec<std::path::PathBuf>>, tx: &mut Tx) -> bool {
        if ask.asked_in != self.actions.session_no {
            return false;
        }
        match chosen {
            Some(files) if !files.is_empty() => self.upload(ask.into.clone(), &files, tx),
            _ => false,
        }
    }
}

// ---- A drag from File Explorer over the list (DW-0481) ----

/// Files are being dragged over the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropHover {
    /// The folder row under them. `None`: the list's own ground (they go into the folder shown).
    pub row: Option<usize>,
    /// How many items are dragged, when the system says (the board draws "3").
    pub count: Option<usize>,
}

impl State {
    /// The drag came over a folder's row, or left it. A folder that takes nothing does not
    /// light up (DW-0481: Backups, a service folder, Trash).
    pub fn drop_over(&mut self, row: usize, over: bool) {
        let takes = self
            .listing
            .items
            .get(row)
            .is_some_and(|item| item.is_folder && self.actions.source.can_add(Some(&item.id)));
        if over && takes && self.list_error.is_none() {
            let count = self.actions.drop_hover.and_then(|d| d.count);
            self.actions.drop_hover = Some(DropHover { row: Some(row), count });
        } else if self.actions.drop_hover.is_some_and(|d| d.row == Some(row)) {
            self.actions.drop_hover = None;
        }
    }

    /// The row a drag is over.
    pub fn drop_row(&self) -> Option<usize> {
        self.actions.drop_hover.and_then(|d| d.row)
    }
}

/// `.mk-page` at the drag's size: 34 x 42, a heading and two lines.
fn drag_page(i: usize, t: &Theme) -> El {
    let line = |key: String, h: f32, w: f32, ink: Color| el().key(key).h(h).w(w).radius(2.0).fill(ink);
    col()
        .key(format!("drag-page-{i}"))
        .absolute(i as f32 * 5.0, i as f32 * 2.0)
        .size(34.0, 42.0)
        .radius(crate::theme::tokens::radius::SM)
        .fill(t.white)
        .box_shadow(kit::drop(2.0, 10.0, t.shadow_page))
        .py(scale::space::S6)
        .px(scale::space::S4)
        .gap(scale::space::S4)
        .rotate((i as f32 - 1.0) * 6.0_f32.to_radians())
        .child(line(format!("drag-page-{i}-h"), 6.0, 24.0 * 0.6, Color::BLACK.alpha(0.30)).mb(3.0))
        .child(line(format!("drag-page-{i}-a"), 3.0, 24.0, t.shadow_page))
        .child(line(format!("drag-page-{i}-b"), 3.0, 24.0, t.shadow_page))
}

/// What goes with a drag: the dragged pages, how many, and where a drop goes. Beside the row.
pub fn drop_layer(s: &State, t: &Theme) -> Option<El> {
    let hover = s.actions.drop_hover?;
    let at = hover.row?;
    let folder = s.listing.items.get(at)?;
    // `.mk-rows` starts 110 pt under the caption's top edge; a row is 30 pt and its first 4 pt
    // in; the board hangs the badge 18 pt under the row's top, 210 pt into the list.
    let top = 114.0 + 30.0 * at as f32 + 18.0;
    // The pages are turned, so the board paints them over the badge: the badge first.
    let mut pages = el().key("drag-pages").size(58.0, 46.0);
    if let Some(count) = hover.count {
        pages = pages.child(
            row()
                .key("drag-badge")
                .absolute(40.0, -1.0)
                .min_w(18.0)
                .h(18.0)
                .px(scale::space::S4)
                .center()
                .radius(scale::radius::TILE)
                .fill(crate::theme::tokens::red_fill())
                .child(kit::label("drag-count", count.to_string(), type_::BADGE, t.white)),
        );
    }
    for i in [2usize, 1, 0] {
        pages = pages.child(drag_page(i, t));
    }
    let words = format!("Upload to {}", folder.name);
    let tip = row()
        .key("tip")
        .role("status")
        .label(words.clone())
        .h(25.0)
        .px(scale::space::S8)
        .gap(scale::space::S6)
        .center_y()
        .radius(crate::theme::tokens::radius::MD)
        .fill(t.card)
        .box_shadow(kit::drop(4.0, 14.0, Color::BLACK.alpha(0.16)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(kit::glyph("tip-icon", &icons_files::UPLOAD, 13.0, t.text))
        .child(kit::label("tip-text", words, TIP, t.text));
    let layer = row()
        .key("drop-layer")
        .absolute(430.0, top)
        .center_y()
        .gap(scale::space::S8)
        .child(pages);
    Some(layer.child(tip))
}

/// `.mk-tip`: 11.5 px on the status line's 17 pt line.
const TIP: rune::TextStyle = crate::theme::tokens::type_::STATUS;

// ---- Uploads (DW-0426, DW-0427) ----

/// A sample picture standing in for a thumbnail until the engine hands over pixels: the
/// board's gradient (its angle in CSS degrees, three stops).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    pub angle: f32,
    pub stops: [(f32, &'static str); 3],
}

/// Where one upload stands.
#[derive(Clone, Debug, PartialEq)]
pub enum UploadState {
    /// It stopped and stays here until someone acts: why, in words.
    Failed(String),
    /// Moving: the panel's words ("2.1 of 3.6 MB"), the row's ("Uploading, 2.1 of 3.6 MB"),
    /// and how far (encrypted bytes sent).
    Uploading { words: String, row: String, done: f32 },
    /// Not started: "3.2 MB · To Lisbon trip".
    Waiting(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Upload {
    /// The item's id (its row in the folder it goes to has the same).
    pub id: String,
    pub name: String,
    pub state: UploadState,
    pub picture: Option<Picture>,
}

/// Everything that uploads, as the source has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Uploads {
    /// "Uploading 3 of 5 · 42 MB left": the status line.
    pub status: String,
    /// "3 of 5 · 42 MB left": the panel's figures.
    pub figures: String,
    pub items: Vec<Upload>,
    pub paused: bool,
}

impl Uploads {
    pub fn of(&self, id: &str) -> Option<&Upload> {
        self.items.iter().find(|u| u.id == id)
    }
}

impl State {
    /// The status line was pressed: Uploads while something uploads (DW-0427), else the b's
    /// flyout (DW-1201).
    pub fn status_pressed(&mut self) {
        self.actions.source.ask_uploads();
        if self.actions.source.uploads().is_some() {
            self.actions.uploads_open = !self.actions.uploads_open;
        } else {
            self.intent(Intent::Frame("DW-1201"));
        }
    }

    pub fn close_uploads(&mut self) {
        self.actions.uploads_open = false;
        self.actions.upload_menu = None;
    }

    /// Try again, on an upload that stopped: asked of the source once.
    pub fn retry_one_upload(&mut self, id: &str, tx: &mut Tx) {
        if self.needs_session(tx) {
            return;
        }
        if let Err(why) = self.actions.source.retry_upload(id) {
            // Its own label: the message has no Try again (the panel's button is still there),
            // and it is not mistaken for a drop that failed, whose Try again uploads again.
            self.failed(Action::RetryUpload, why, tx);
        }
    }

    /// Pause all, or Resume all: asked once. The b then shows the pause badge (DW-1204).
    pub fn pause_uploads(&mut self, tx: &mut Tx) {
        if self.needs_session(tx) {
            return;
        }
        let Some(uploads) = self.actions.source.uploads() else {
            return;
        };
        let pause = !uploads.paused;
        match self.actions.source.pause_uploads(pause) {
            Ok(()) if pause => self.intent(Intent::Frame("DW-1204")),
            Ok(()) => {}
            Err(why) => self.failed(Action::PauseUploads, why, tx),
        }
    }

    /// Cancel upload, from a row's menu: asked once. A source that answers later says what it did
    /// (`Change::Cancelled`); the row leaves the folder then, not before.
    pub fn cancel_one_upload(&mut self, id: &str, tx: &mut Tx) {
        self.actions.upload_menu = None;
        if self.needs_session(tx) {
            return;
        }
        match self.actions.source.cancel_upload(id) {
            Ok(()) if self.actions.source.answers_later() => {}
            Ok(()) => {
                // Its row in the folder goes with it. What was selected stays selected BY ITEM:
                // the rows under the one that went move up, and the selection moves with them.
                let id_of = |s: &State, row: usize| s.listing.items.get(row).map(|i| i.id.clone());
                let lead = self.selected.and_then(|i| id_of(self, i));
                let anchor = self.actions.anchor.and_then(|i| id_of(self, i));
                let selected: Vec<String> = self.actions.selection.iter().filter_map(|&i| id_of(self, i)).collect();
                self.listing.items.retain(|item| item.id != id);
                let row_of = |s: &State, id: &String| s.listing.items.iter().position(|i| &i.id == id);
                self.selected = lead.and_then(|id| row_of(self, &id));
                self.actions.anchor = anchor.and_then(|id| row_of(self, &id));
                self.actions.selection = selected.iter().filter_map(|id| row_of(self, id)).collect();
            }
            Err(why) => self.failed(Action::CancelUpload, why, tx),
        }
    }
}

/// The status line while something uploads: the source's words, a neutral dot (uploading is not
/// a fault and not "Up to date"). `None`: nothing uploads.
pub fn status(s: &State, details: &str, t: &Theme) -> Option<(El, String)> {
    let uploads = s.actions.source.uploads()?;
    let words = if uploads.paused {
        format!("Paused \u{00B7} {}", uploads.figures)
    } else {
        uploads.status.clone()
    };
    let line = status_bar(Some(Dot::Busy), Some(&words), details, t);
    Some((line, words))
}

/// `.mk-bar`: 4 pt, Amber for an upload (uploading is the encryption step).
pub fn bar(key: &str, done: f32, name: &str, t: &Theme) -> El {
    el().key(key.to_string())
        .role("progressbar")
        .label(name)
        .w_full()
        .h(4.0)
        .radius(2.0)
        .clip_rounded(2.0)
        .fill(t.text.alpha(0.09))
        .child(
            el().key(format!("{key}-fill"))
                .h(4.0)
                .w_frac(done.clamp(0.0, 1.0))
                .radius(2.0)
                .fill(t.accent),
        )
}

/// An upload's tile: the kind's icon from its name, or the stand-in picture until the engine hands over pixels.
fn picture_tile(key: &str, upload: &Upload, t: &Theme) -> El {
    let icon = crate::theme::file_icons::icon_for(&upload.name, false, None);
    let picture = match upload.picture {
        Some(_) => TilePicture::Thumb(&Thumb::Plain),
        None => TilePicture::Icon(icon),
    };
    let tile = file_tile::tile(
        &format!("{key}-tile"),
        &upload.name,
        picture,
        &upload.name,
        None,
        RowLook::default(),
        TileSize::Small,
        t,
    )
    .a11y_hidden();
    match upload.picture {
        Some(p) => {
            let stop = |hex: &str| rune::color::hex(hex).unwrap_or(Color::BLACK);
            tile.linear_gradient(
                p.angle.to_radians(),
                [
                    (p.stops[0].0, stop(p.stops[0].1)),
                    (p.stops[1].0, stop(p.stops[1].1)),
                    (p.stops[2].0, stop(p.stops[2].1)),
                ],
            )
        }
        None => tile,
    }
}

fn heading(key: &str, words: &str, top: f32, t: &Theme) -> El {
    el().key(format!("{key}-box"))
        .px(scale::space::S12)
        .pt(top)
        .child(kit::label(
            key.to_string(),
            words.to_uppercase(),
            type_::SECTION_LABEL,
            t.text_tertiary,
        ))
}

/// The one entry of an upload's menu.
const UPLOAD_MENU: [Entry; 1] = [Entry::item("cancel-upload", "Cancel upload")];

fn pick_upload_menu(s: &mut State, index: usize, tx: &mut Tx) {
    if UPLOAD_MENU.get(index).and_then(|e| e.id()) != Some("cancel-upload") {
        return;
    }
    if let Some(id) = s.actions.upload_menu.clone() {
        s.cancel_one_upload(&id, tx);
    }
}

fn close_upload_menu(s: &mut State, _: &mut Tx) {
    s.actions.upload_menu = None;
}

fn upload_row(i: usize, of: usize, upload: &Upload, menu_open: bool, t: &Theme) -> El {
    let key = format!("upload-{i}");
    let (sub, ink, done) = match &upload.state {
        UploadState::Failed(why) => (why.clone(), crate::theme::tokens::red_text(t.dark), None),
        UploadState::Uploading { words, done, .. } => (words.clone(), t.text_tertiary, Some(*done)),
        UploadState::Waiting(words) => (words.clone(), t.text_tertiary, None),
    };
    let mut cells = vec![
        kit::label(format!("{key}-sub"), sub.clone(), type_::CAPTION, ink)
            .truncate()
            .w(150.0),
    ];
    if let Some(done) = done {
        cells.push(bar(&format!("{key}-bar"), done, &format!("{}, {sub}", upload.name), t).w(80.0));
    }
    if matches!(upload.state, UploadState::Failed(_)) {
        let id = upload.id.clone();
        cells.push(
            controls::button(i, "Try again", None, Emphasis::Plain, t)
                .key(format!("{key}-retry"))
                .on_press_tx(move |s: &mut State, tx: &mut Tx| s.retry_one_upload(&id, tx)),
        );
    }
    let id = upload.id.clone();
    let mut r = file_row::row_with(
        &key,
        &format!("{}. {sub}", upload.name),
        picture_tile(&key, upload, t),
        &upload.name,
        Vec::new(),
        cells,
        RowLook {
            position: Some((i + 1, of)),
            ..RowLook::default()
        },
        t,
    )
    .on_secondary_press(move |s: &mut State, _: rune::PointerPress, _: &mut Tx| {
        s.actions.upload_menu = Some(id.clone());
    });
    if menu_open {
        r = r.child(
            menu::menu(
                "upload-menu",
                "Upload",
                130.0,
                &UPLOAD_MENU,
                None,
                t,
                pick_upload_menu,
                |_, _| {},
                close_upload_menu,
            )
            .absolute(52.0, 30.0),
        );
    }
    r
}

/// Uploads, over the status line (DW-0427): what needs attention, what moves, what waits.
pub fn layer(s: &State, t: &Theme) -> Option<El> {
    if !s.actions.uploads_open {
        return None;
    }
    let uploads = s.actions.source.uploads()?;
    let menu = s.actions.upload_menu.as_deref();
    // The figures are a machine's: JetBrains Mono inside the Inter line.
    let line = format!("{} \u{00B7} Continues in the background", uploads.figures);
    let head = col()
        .key("uploads-head")
        .px(scale::space::S12)
        .pt(12.0)
        .pb(8.0)
        .child(
            kit::label("uploads-title", "Uploads", type_::HEAD, t.text)
                .role("heading")
                .level(2),
        )
        .child(
            kit::label("uploads-figures", line, type_::CAPTION, t.text_tertiary)
                .spans(vec![rune::Span {
                    start: 0,
                    end: uploads.figures.len() as u32,
                    mono: true,
                    ..rune::Span::default()
                }])
                .mt(2.0),
        );
    let mut body: Vec<El> = Vec::new();
    let groups: [(&str, fn(&UploadState) -> bool, f32); 3] = [
        ("Needs attention", |s| matches!(s, UploadState::Failed(_)), 4.0),
        ("Uploading", |s| matches!(s, UploadState::Uploading { .. }), 8.0),
        ("Waiting", |s| matches!(s, UploadState::Waiting(_)), 8.0),
    ];
    for (g, (title, is, top)) in groups.into_iter().enumerate() {
        let rows: Vec<(usize, &Upload)> = uploads.items.iter().enumerate().filter(|(_, u)| is(&u.state)).collect();
        if rows.is_empty() {
            continue;
        }
        body.push(heading(&format!("uploads-group-{g}"), title, top, t));
        for (i, upload) in rows {
            body.push(upload_row(
                i,
                uploads.items.len(),
                upload,
                menu == Some(upload.id.as_str()) && s.actions.source.cancels_upload(&upload.id),
                t,
            ));
        }
    }
    let pause = if uploads.paused { "Resume all" } else { "Pause all" };
    let foot = row()
        .w_full()
        .justify_end()
        .px(scale::space::S12)
        .py(scale::space::S10)
        .mt(4.0)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(
            controls::button(8, pause, Some(&icons_files::PAUSE), Emphasis::Plain, t)
                .key("uploads-pause")
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.pause_uploads(tx)),
        );
    // 232 pt from the window's left edge, 34 pt over its bottom: above the status line.
    let card = dialog(
        Dialog {
            surface: Surface::Panel,
            head: Head::Raw(head),
            body,
            foot: Foot::Custom(foot),
            ..Dialog::new("uploads", "Uploads", 380.0)
        },
        t,
    )
    .pin(Some(232.0), None, None, Some(34.0));
    Some(
        el().key("uploads-layer")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(card)
            // A press outside and Esc close it; the list under it is not dimmed.
            .modal(true, |s: &mut State| s.close_uploads()),
    )
}

/// The board's sample: 3 of 5, one stopped, two moving, one waiting (DW-0426, DW-0427).
pub fn board_uploads() -> Uploads {
    let pic = |angle: f32, a: &'static str, mid: f32, b: &'static str, c: &'static str| {
        Some(Picture {
            angle,
            stops: [(0.0, a), (mid, b), (1.0, c)],
        })
    };
    let up = |id: &str, name: &str, state, picture| Upload {
        id: id.to_string(),
        name: name.to_string(),
        state,
        picture,
    };
    Uploads {
        status: "Uploading 3 of 5 \u{00B7} 42 MB left".to_string(),
        figures: "3 of 5 \u{00B7} 42 MB left".to_string(),
        paused: false,
        items: vec![
            up(
                "up-scan",
                "Scan 12 Oct.pdf",
                UploadState::Failed("Stopped after 5 tries. Not enough space.".to_string()),
                None,
            ),
            up(
                "up-miradouro",
                "Miradouro.heic",
                UploadState::Uploading {
                    words: "2.1 of 3.6 MB".to_string(),
                    row: "Uploading, 2.1 of 3.6 MB".to_string(),
                    done: 0.58,
                },
                pic(120.0, "#d9dfe4", 0.6, "#8ea4b4", "#4a5866"),
            ),
            up(
                "tram",
                "Tram 28.mov",
                UploadState::Uploading {
                    words: "Continuing, 34 of 82 MB".to_string(),
                    row: "Continuing, 34 of 82 MB".to_string(),
                    done: 0.41,
                },
                pic(150.0, "#33435c", 0.55, "#7a7f96", "#e3b58c"),
            ),
            up(
                "up-ponte",
                "Ponte.heic",
                UploadState::Waiting("3.2 MB \u{00B7} To Lisbon trip".to_string()),
                pic(170.0, "#9cc4dc", 0.6, "#e9e2cf", "#8d9a78"),
            ),
        ],
    }
}
