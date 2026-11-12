//! Thumbnails and previews (DW-0744): how photos preview, the size and format of the stored
//! previews, and what that costs. Over it: the confirm of a new size or format with its cost
//! (DW-0744b), and the compare view (DW-0745), a large dialog that hangs from the caption.
//!
//! Nothing changes until Apply: a segment (or Done in the compare view) only DRAFTS a choice
//! (`PreviewsUi::draft`); the pane keeps drawing the stored one until the source has it.
//!
//! The compare view is the one of first-run setup (DW-0209, `frames::signin::setup`), whose
//! drawing is private to that group: the photo, the options and the PNG decoder are repeated
//! here with this pane's keys. One shared component is the follow-up.

use crate::State;
use crate::frames::signin::setup::source::BUNDLED_SAMPLE_PNG;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::kit;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_dialog::{self as dialogs, Weight};
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::settings_shell::{self as shell, SUB};
use crate::ui::setup::{self, SegInk, SegSize};
use rune::{Color, El, Family, Pixels, Span, TextStyle, col, el, rich, row};

pub use crate::frames::signin::setup::source::{PreviewFormat, PreviewSize};
pub use crate::frames::signin::setup::{Compare, CompareMode};

use super::{Dialog, Intent, SettingsUi};

/// The compare view on the board: 700 x 560, hanging from the caption.
pub const SHEET: (f32, f32) = (700.0, 560.0);
/// The photo in the compare view.
pub const PHOTO: (f32, f32) = (560.0, 330.0);
/// One arrow key moves the wipe line this much of the photo's width.
pub const WIPE_STEP: f32 = 0.05;
/// What the pane says once new previews are being made.
pub const REBUILDING: &str = "Making new previews in the background";

/// 11px/500 mono: what a choice weighs, in the compare view (`.t-cap.font-mono`).
const CAP_MONO: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);

/// What Thumbnails and previews shows. Every figure is the engine's, as words.
#[derive(Clone, PartialEq, Debug)]
pub struct Previews {
    /// Photo previews are made at all.
    pub on: bool,
    /// The stored choice.
    pub size: PreviewSize,
    pub format: PreviewFormat,
    /// How many photos have previews: "2,431".
    pub photos: String,
    /// The sample's original: "30.9 MB".
    pub original: String,
    /// What a preview of the sample weighs at a size and format, where it has been measured.
    pub weights: Vec<(PreviewSize, PreviewFormat, String)>,
    /// How many of the account's photos the average was measured on.
    pub measured_on: u32,
    /// What it uses of the account's storage: all of it, then previews, fallback previews
    /// and grid thumbnails.
    pub total: String,
    pub uses: [String; 3],
    /// On this PC: what is kept, the limit, and whether every photo fits under it.
    pub on_pc: String,
    pub limit: String,
    pub all_fit: bool,
}

impl Previews {
    pub fn weight(&self, size: PreviewSize, format: PreviewFormat) -> Option<String> {
        self.weights
            .iter()
            .find(|(s, f, _)| *s == size && *f == format)
            .map(|(_, _, w)| w.clone())
    }
}

/// "4K HEIC".
pub fn name(size: PreviewSize, format: PreviewFormat) -> String {
    format!("{} {}", size.label(), format.label())
}

/// What the source answers for this pane.
#[derive(Clone, PartialEq, Debug)]
pub enum PhotoPreviews {
    /// Nothing answers yet (the default of `DriveSettings::photo_previews`): no value is drawn.
    Unavailable,
    Ready(Previews),
}

/// What is open or drafted inside Thumbnails and previews.
pub struct PreviewsUi {
    /// A size and format chosen and not applied yet (DW-0744b asks; DW-0745 chooses).
    pub draft: Option<(PreviewSize, PreviewFormat)>,
    /// How the open compare view shows the two pictures (DW-0745).
    pub compare: Compare,
    original: Pixels,
    preview: Pixels,
}

impl Default for PreviewsUi {
    fn default() -> Self {
        Self {
            draft: None,
            compare: fresh_compare(),
            // The sample that ships inside the app; the engine lane hands over the person's own.
            original: decode(BUNDLED_SAMPLE_PNG),
            preview: decode(BUNDLED_SAMPLE_PNG),
        }
    }
}

fn fresh_compare() -> Compare {
    Compare {
        mode: CompareMode::Wipe,
        wipe: 0.5,
        original_showing: false,
    }
}

/// Decode a PNG into pixels Rune draws. A file that does not decode draws nothing.
fn decode(png_bytes: &[u8]) -> Pixels {
    let pixels = Pixels::new();
    let Ok(mut reader) = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info() else {
        return pixels;
    };
    let Some(size) = reader.output_buffer_size() else {
        return pixels;
    };
    let mut buf = vec![0; size];
    let Ok(info) = reader.next_frame(&mut buf) else {
        return pixels;
    };
    let bytes = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => bytes.to_vec(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => {
            bytes.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
        }
        _ => return pixels,
    };
    pixels.set(info.width, info.height, rgba);
    pixels
}

impl SettingsUi {
    fn stored_previews(&self) -> Option<(PreviewSize, PreviewFormat)> {
        match self.drive.photo_previews() {
            PhotoPreviews::Ready(p) => Some((p.size, p.format)),
            PhotoPreviews::Unavailable => None,
        }
    }

    /// The choice in hand: the draft, or what is stored.
    pub fn previews_choice(&self) -> Option<(PreviewSize, PreviewFormat)> {
        self.previews.draft.or(self.stored_previews())
    }

    /// A segment on the pane: draft this size or format and ask with its cost (DW-0744b).
    /// The stored choice asks nothing.
    pub fn choose_previews(&mut self, size: Option<PreviewSize>, format: Option<PreviewFormat>) {
        let Some(stored) = self.stored_previews() else {
            return;
        };
        self.toast = None;
        let want = (size.unwrap_or(stored.0), format.unwrap_or(stored.1));
        if want == stored {
            self.previews.draft = None;
            return;
        }
        self.previews.draft = Some(want);
        self.dialog = Some(Dialog::ApplyPreviews);
    }

    /// "Compare" (DW-0745): the view opens on the stored choice, wiping.
    pub fn open_compare(&mut self) {
        if self.stored_previews().is_none() {
            return;
        }
        self.toast = None;
        self.previews.draft = None;
        self.previews.compare = fresh_compare();
        self.dialog = Some(Dialog::Compare);
    }

    /// A size or a format in the compare view: the choice in hand, nothing stored.
    pub fn pick_in_compare(&mut self, size: Option<PreviewSize>, format: Option<PreviewFormat>) {
        if let Some(now) = self.previews_choice() {
            self.previews.draft = Some((size.unwrap_or(now.0), format.unwrap_or(now.1)));
        }
    }

    /// Done or Esc in the compare view: back, with the choice set. A choice that differs
    /// from the stored one is confirmed with its cost first (DW-0744b).
    pub fn close_compare(&mut self) {
        if self.dialog != Some(Dialog::Compare) {
            return;
        }
        self.dialog = None;
        if self.previews.draft.is_some() && self.previews.draft != self.stored_previews() {
            self.dialog = Some(Dialog::ApplyPreviews);
        } else {
            self.previews.draft = None;
        }
    }

    /// "Apply to 2,431 photos": the source makes the new previews in the background.
    pub fn apply_previews(&mut self) {
        if let Some((size, format)) = self.previews.draft.take() {
            self.drive.apply_previews(size, format);
            self.toast = Some(REBUILDING);
        }
        self.dialog = None;
    }

    /// Cancel: the stored choice stays.
    pub fn cancel_previews(&mut self) {
        self.previews.draft = None;
        self.dialog = None;
    }

    pub fn set_compare_mode(&mut self, mode: CompareMode) {
        self.previews.compare.mode = mode;
        self.previews.compare.original_showing = false;
    }

    /// Move the wipe line by `by` of the photo's width.
    pub fn move_wipe(&mut self, by: f32) {
        let compare = &mut self.previews.compare;
        if compare.mode == CompareMode::Wipe {
            compare.wipe = (compare.wipe + by).clamp(0.0, 1.0);
        }
    }

    pub fn flip(&mut self) {
        let compare = &mut self.previews.compare;
        if compare.mode == CompareMode::Flip {
            compare.original_showing = !compare.original_showing;
        }
    }
}

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    const HEAD_SUB: &str = "Photos open instantly from a saved preview, instead of downloading the full original.";
    let mut children = vec![shell::heading("Thumbnails and previews", Some(HEAD_SUB), t)];
    let p = match ui.drive.photo_previews() {
        PhotoPreviews::Ready(p) => p,
        PhotoPreviews::Unavailable => {
            children.push(notices::problem(
                0,
                0,
                "Couldn't load your preview settings",
                "Nothing was changed.",
                None,
                t,
            ));
            return (shell::pane(children, t), None);
        }
    };

    let on = p.on;
    children.push(lists::list(
        0,
        None,
        vec![lists::item_with(
            0,
            0,
            &Item::new(Some(&ic::IMAGE), "Photo previews")
                .sub("Photos open instantly, without downloading the original"),
            vec![
                lists::switch("item-0-0", "Photo previews", on, t)
                    .on_press(move |s: &mut State| s.settings.drive.set_photo_previews(!on)),
            ],
            t,
        )],
        None,
        t,
    ));
    children.push(sample_card(ui, &p, t));

    let sizes: Vec<&str> = PreviewSize::ALL.iter().map(|s| s.label()).collect();
    let formats: Vec<&str> = PreviewFormat::ALL.iter().map(|f| f.label()).collect();
    let size_at = PreviewSize::ALL.iter().position(|s| *s == p.size).unwrap_or(0);
    let format_at = PreviewFormat::ALL.iter().position(|f| *f == p.format).unwrap_or(0);
    let seg_row = |key: &str, seg: El| el().key(key.to_string()).w_full().px(12.0).py(10.0).child(seg);
    children.push(lists::list(
        1,
        Some("Preview size"),
        vec![seg_row(
            "size-row",
            setup::segmented(
                "seg-size",
                "Preview size",
                &sizes,
                size_at,
                SegSize::Md,
                None,
                t,
                |j, tab| tab.on_press(move |s: &mut State| s.settings.choose_previews(Some(PreviewSize::ALL[j]), None)),
            ),
        )],
        None,
        t,
    ));
    children.push(lists::list(
        2,
        Some("Format"),
        vec![seg_row(
            "format-row",
            setup::segmented(
                "seg-format",
                "Format",
                &formats,
                format_at,
                SegSize::Md,
                None,
                t,
                |j, tab| {
                    tab.on_press(move |s: &mut State| s.settings.choose_previews(None, Some(PreviewFormat::ALL[j])))
                },
            ),
        )],
        Some("HEIC is about half the size at the same quality. JPEG opens anywhere."),
        t,
    ));

    let uses = ["Previews", "Fallback previews", "Grid thumbnails"]
        .iter()
        .zip(&p.uses)
        .enumerate()
        .map(|(i, (title, value))| {
            let mut it = Item::new(None, *title).trailing(Trailing::Mono(value.clone()));
            if i > 0 {
                it = it.ruled();
            }
            lists::item(3, i, &it, t)
        })
        .collect();
    let mut counts = format!(
        "This counts toward your Beebeeb storage. All {} photos at {}",
        p.photos,
        name(p.size, p.format)
    );
    match p.weight(p.size, p.format) {
        Some(average) => counts.push_str(&format!(
            "; average {average} per preview, measured on {} of your photos.",
            p.measured_on
        )),
        None => counts.push('.'),
    }
    children.push(lists::list(3, Some("What it uses"), uses, Some(&counts), t));

    children.push(lists::list(
        4,
        Some("On this PC"),
        vec![
            lists::item(
                4,
                0,
                &Item::new(None, format!("{} of {} limit", p.on_pc, p.limit))
                    .sub(if p.all_fit {
                        "All photos fit. Change the limit or clear these in Storage."
                    } else {
                        "Not every photo fits. Change the limit or clear these in Storage."
                    })
                    .opens(),
                t,
            )
            .on_press(|s: &mut State| super::go(s, "DW-0712")),
        ],
        None,
        t,
    ));
    children.push(lists::list(
        5,
        Some("Made automatically"),
        vec![
            lists::item(
                5,
                0,
                &Item::new(None, "Grid thumbnails").trailing(Trailing::Value("WebP, 384 and 768 px".into())),
                t,
            ),
            lists::item(
                5,
                1,
                &Item::new(None, "Fallback preview, with HEIC")
                    .trailing(Trailing::Value("WebP, 720p".into()))
                    .ruled(),
                t,
            ),
        ],
        Some(
            "Thumbnails are made for every photo and can't be changed. If you choose HEIC, a 720p WebP is also stored so your photos still preview on devices that can't open HEIC.",
        ),
        t,
    ));
    children.push(lists::list(
        6,
        Some("Rebuild"),
        vec![
            lists::item(6, 0, &Item::new(Some(&ic::REBUILD), "Rebuild previews"), t)
                .role("button")
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(|s: &mut State| {
                    s.settings.drive.rebuild_previews();
                    s.settings.toast = Some(REBUILDING);
                }),
        ],
        Some("Clearing the cache never rebuilds anything. Previews just download again when you open a photo."),
        t,
    ));

    let overlay = match ui.dialog {
        Some(Dialog::ApplyPreviews) => Some(apply_question(ui, &p, t)),
        Some(Dialog::Compare) => Some(compare_view(ui, &p, t)),
        _ => ui.toast.map(|words| notices::toast(0, words, t)),
    };
    (shell::pane(children, t), overlay)
}

/// The sample: the photo, what its preview weighs against the original, and Compare
/// (`.kit-card.p-3`). The board's line is one text of mixed runs; so is this (`card-0-sub`).
fn sample_card(ui: &SettingsUi, p: &Previews, t: &Theme) -> El {
    let strong = format!("{} \u{b7} ", name(p.size, p.format));
    let weight = p.weight(p.size, p.format).unwrap_or_default();
    let middle = ", original ";
    let words = format!("{strong}{weight}{middle}{}", p.original);
    let weight_at = strong.len()..strong.len() + weight.len();
    let original_at = weight_at.end + middle.len()..words.len();
    let mut spans = vec![Span::new(0, weight_at.start).weight(500.0).color(t.text)];
    if !weight.is_empty() {
        spans.push(
            Span::new(weight_at.start, weight_at.end)
                .weight(500.0)
                .color(t.text)
                .mono(),
        );
    }
    spans.push(Span::new(original_at.start, original_at.end).mono());
    kit::note_text("card-0-sub", "card-0-sub", &words, SUB, t.text_tertiary);
    row()
        .key("card-0")
        .role("group")
        .label(format!("Sample: {words}"))
        .w_full()
        .padding(12.0)
        .gap(12.0)
        .center_y()
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
        .child(
            el().key("card-0-thumb")
                .role("image")
                .label("Sample photo")
                .size(84.0, 64.0)
                .radius(6.0)
                .pixels(&ui.previews.original),
        )
        .child(
            rich(words, spans)
                .key("card-0-sub")
                .font(SUB)
                .color(t.text_tertiary)
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            controls::button(0, "Compare", None, Emphasis::Plain, t)
                .on_press(|s: &mut State| s.settings.open_compare()),
        )
}

/// DW-0744b: a change of size or format, confirmed with its cost.
fn apply_question(ui: &SettingsUi, p: &Previews, t: &Theme) -> El {
    let (size, format) = ui.previews.draft.unwrap_or((p.size, p.format));
    let stays = "The old ones stay until the new ones have uploaded.";
    let says = match ui.drive.previews_cost(size, format) {
        Some(cost) => format!(
            "Previews go from about {} to about {cost} of your storage. {stays}",
            p.total
        ),
        None => stays.to_string(),
    };
    let foot = vec![
        dialogs::button(1, &format!("Apply to {} photos", p.photos), Weight::Primary, t)
            .default_button()
            .on_press(|s: &mut State| s.settings.apply_previews()),
        dialogs::button(2, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.cancel_previews()),
    ];
    dialogs::alert(
        "Apply new previews",
        380.0,
        notices::tile(0, Tone::Neutral, &ic::IMAGE, t),
        &format!("Apply {} to {} photos?", name(size, format), p.photos),
        &says,
        foot,
        t,
    )
    .modal(false, |s: &mut State| s.settings.cancel_previews())
}

/// The compare view's ground: "color-mix(in oklch, black 88%, paper)".
fn stage(t: &Theme) -> Color {
    if t.dark {
        Color::BLACK
    } else {
        rune::color::hex("#060505").unwrap_or(Color::BLACK)
    }
}

fn tag(key: &str, words: &str) -> El {
    el().key(format!("{key}-box"))
        .h(18.0)
        .px(8.0)
        .center()
        .radius(4.0)
        .fill(Color::BLACK.alpha(0.5))
        .child(kit::label(key.to_string(), words, type_::CAPTION, Color::WHITE))
}

fn photo(ui: &SettingsUi, p: &Previews, t: &Theme) -> El {
    let (original, preview) = (&ui.previews.original, &ui.previews.preview);
    let compare = ui.previews.compare;
    let (w, h) = PHOTO;
    let (size, format) = ui.previews.draft.unwrap_or((p.size, p.format));
    let original_tag = format!("Original \u{b7} {}", p.original);
    let preview_tag = match p.weight(size, format) {
        Some(weight) => format!("{} \u{b7} {weight}", name(size, format)),
        None => name(size, format),
    };
    let frame = |key: &str| el().key(key.to_string()).size(w, h).radius(4.0);
    match compare.mode {
        CompareMode::Wipe => {
            let x = (compare.wipe * w).round();
            frame("cmp-photo")
                .role("slider")
                .label("Wipe between the original and the preview")
                .range_value(f64::from(compare.wipe * 100.0), 0.0, 100.0)
                .on_step(|s: &mut State, delta: i32, _: &mut rune::Tx| {
                    s.settings.move_wipe(delta as f32 * WIPE_STEP);
                })
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .pixels(original)
                // The preview from the line to the right edge, cut from the same picture box.
                .child(
                    el().key("cmp-preview-at")
                        .absolute(x, 0.0)
                        .size(w - x, h)
                        .clip_corners([0.0, 4.0, 4.0, 0.0])
                        .child(el().absolute(-x, 0.0).size(w, h).pixels(preview)),
                )
                .child(
                    el().key("cmp-wipe")
                        .absolute(x - 1.0, 0.0)
                        .size(2.0, h)
                        .fill(Color::WHITE),
                )
                .child(el().absolute(12.0, 12.0).child(tag("cmp-tag-0", &original_tag)))
                .child(
                    el().pin(None, Some(12.0), Some(12.0), None)
                        .child(tag("cmp-tag-1", &preview_tag)),
                )
        }
        CompareMode::Stacked => {
            // Two pictures of half the height, the original above the preview.
            let half = (h - 10.0) / 2.0;
            let one = |key: &str, pixels: &Pixels, words: &str, name: &str| {
                el().key(format!("{key}-pic"))
                    .role("image")
                    .label(name.to_string())
                    .size(half * w / h, half)
                    .radius(4.0)
                    .pixels(pixels)
                    .child(el().absolute(12.0, 12.0).child(tag(key, words)))
            };
            col()
                .key("cmp-photo")
                .size(w, h)
                .gap(10.0)
                .items_center()
                .child(one("cmp-tag-0", original, &original_tag, "The original"))
                .child(one("cmp-tag-1", preview, &preview_tag, "The preview"))
        }
        CompareMode::Flip => {
            let (pixels, key, words) = if compare.original_showing {
                (original, "cmp-tag-0", original_tag.as_str())
            } else {
                (preview, "cmp-tag-1", preview_tag.as_str())
            };
            frame("cmp-photo")
                .role("button")
                .label(if compare.original_showing {
                    "Showing the original. Press to show the preview."
                } else {
                    "Showing the preview. Press to show the original."
                })
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .cursor(rune::Cursor::Pointer)
                .pixels(pixels)
                .on_press(|s: &mut State| s.settings.flip())
                .child(el().absolute(12.0, 12.0).child(tag(key, words)))
        }
    }
}

/// One choice at the foot of the compare view: a size or a format, with what it weighs.
fn option(j: usize, label: &str, weight: Option<String>, on: bool, t: &Theme) -> El {
    let dim = Color::WHITE.alpha(0.7);
    let mut o = col()
        .key(format!("cmp-opt-{j}-tab"))
        .role("radio")
        .label(match &weight {
            Some(weight) => format!("{label}, {weight}"),
            None => label.to_string(),
        })
        .checked(on)
        // As wide as its wider line, to the fraction of a point: the row is centred.
        .w(24.0
            + rune::text::advance(label, type_::HEAD)
                .max(weight.as_deref().map_or(0.0, |w| rune::text::advance(w, CAP_MONO))))
        .h(43.0)
        .py(6.0)
        .items_center()
        .radius(8.0)
        .focusable(true)
        .focus_ring(2.0, 0.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(kit::label(format!("cmp-opt-{j}"), label, type_::HEAD, Color::WHITE).a11y_hidden());
    if on {
        o = o.fill(Color::WHITE.alpha(0.18));
    }
    if let Some(weight) = weight {
        o = o.child(kit::label(format!("cmp-opt-{j}-bytes"), weight, CAP_MONO, dim).a11y_hidden());
    }
    o
}

/// DW-0745: a preview against the original, before choosing a size.
fn compare_view(ui: &SettingsUi, p: &Previews, t: &Theme) -> El {
    let dim = Color::WHITE.alpha(0.7);
    let compare = ui.previews.compare;
    let (now_size, now_format) = ui.previews.draft.unwrap_or((p.size, p.format));
    let modes: Vec<&str> = CompareMode::ALL.iter().map(|m| m.label()).collect();
    let mode = CompareMode::ALL.iter().position(|m| *m == compare.mode).unwrap_or(1);
    let head = row()
        .key("cmp-head")
        .w_full()
        .h(48.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(
            controls::button(1, "Done", None, Emphasis::Plain, t)
                .default_button()
                .on_press(|s: &mut State| s.settings.close_compare()),
        )
        .child(setup::segmented(
            "cmp-seg",
            "How to compare",
            &modes,
            mode,
            SegSize::Sm,
            Some(SegInk {
                track: Color::WHITE.alpha(0.14),
                rest: Color::WHITE.alpha(0.75),
            }),
            t,
            |j, tab| tab.on_press(move |s: &mut State| s.settings.set_compare_mode(CompareMode::ALL[j])),
        ))
        .child(el().grow(1.0))
        .child(kit::label(
            "cmp-hint",
            match compare.mode {
                CompareMode::Wipe => "Pinch to zoom \u{b7} double-click for 1:1",
                CompareMode::Stacked => "The original above, the preview below",
                CompareMode::Flip => "Press the photo to flip",
            },
            type_::CAPTION,
            dim,
        ));
    let mut foot = row()
        .key("cmp-foot")
        .w_full()
        .pb(16.0)
        .gap(24.0)
        .center()
        .role("group")
        .label("Preview size and format");
    for (j, size) in PreviewSize::ALL.iter().enumerate() {
        let size = *size;
        foot = foot.child(
            option(j, size.label(), p.weight(size, now_format), size == now_size, t)
                .on_press(move |s: &mut State| s.settings.pick_in_compare(Some(size), None)),
        );
    }
    foot = foot.child(
        el().key("cmp-sep")
            .a11y_hidden()
            .size(kit::HAIR, 18.0)
            .mx(2.0)
            .fill(Color::WHITE.alpha(0.3)),
    );
    for (j, format) in PreviewFormat::ALL.iter().enumerate() {
        let format = *format;
        foot = foot.child(
            option(
                PreviewSize::ALL.len() + j,
                format.label(),
                p.weight(now_size, format),
                format == now_format,
                t,
            )
            .on_press(move |s: &mut State| s.settings.pick_in_compare(None, Some(format))),
        );
    }
    foot = foot.child(
        controls::button(2, "Choose photo", None, Emphasis::Plain, t)
            // As wide as its words to the fraction of a point (12 pt and the border each
            // side): the row is centred, so a rounded width would move every choice in it.
            .w(26.0 + rune::text::advance("Choose photo", type_::BUTTON_SM))
            .on_press(|s: &mut State| s.settings.ask(Intent::Open("choose-photo"))),
    );
    // The buttons are the light theme's paper on the dark ground in both themes, as the
    // board draws them; the sheet hangs from the caption, centred across the window.
    let sheet = setup::sheet(
        "Compare a preview with the original",
        SHEET.0,
        SHEET.1,
        col()
            .key("cmp")
            .w_full()
            .h_full()
            .corners([0.0, 0.0, 14.0, 14.0])
            .fill(stage(t))
            .child(head)
            .child(
                el().key("cmp-stage")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .center()
                    .child(photo(ui, p, t)),
            )
            .child(foot),
        t,
    );
    col()
        .key("scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .items_center()
        // The sheet hangs from the caption: its shadow and ring never reach over it.
        .clip_rounded(0.0)
        .fill(dialogs::scrim_color(t))
        .child(sheet)
        .modal(false, |s: &mut State| s.settings.close_compare())
}

/// The board's sample content for Thumbnails and previews (DW-0744).
pub struct BoardPreviews {
    pub previews: Previews,
}

/// The board's 2,431 photos.
const BOARD_PHOTOS: f32 = 2431.0;

/// What a preview of the board's sample weighs, in KB, where the board says.
fn board_kb(size: PreviewSize, format: PreviewFormat) -> Option<f32> {
    Some(match (size, format) {
        (PreviewSize::P720, PreviewFormat::Heic) => 213.0,
        (PreviewSize::P1080, PreviewFormat::Heic) => 420.0,
        (PreviewSize::K2, PreviewFormat::Heic) => 640.0,
        (PreviewSize::K4, PreviewFormat::Heic) => 1000.0,
        (PreviewSize::K4, PreviewFormat::Jpeg) => 2100.0,
        _ => return None,
    })
}

fn words_of(kb: f32) -> String {
    if kb >= 1_000_000.0 {
        format!("{:.1} GB", kb / 1_000_000.0)
    } else if kb >= 1000.0 {
        format!("{:.1} MB", kb / 1000.0)
    } else {
        format!("{kb:.0} KB")
    }
}

impl BoardPreviews {
    pub fn new() -> Self {
        let mut weights = Vec::new();
        for size in PreviewSize::ALL {
            for format in PreviewFormat::ALL {
                if let Some(kb) = board_kb(size, format) {
                    weights.push((size, format, words_of(kb)));
                }
            }
        }
        Self {
            previews: Previews {
                on: true,
                size: PreviewSize::K4,
                format: PreviewFormat::Heic,
                photos: "2,431".into(),
                original: "30.9 MB".into(),
                weights,
                measured_on: 50,
                total: "3.1 GB".into(),
                uses: ["2.4 GB".into(), "420 MB".into(), "300 MB".into()],
                on_pc: "3.1 GB".into(),
                limit: "20 GB".into(),
                all_fit: true,
            },
        }
    }

    /// What all previews would take at a size and format: the board's own figure where it
    /// gives one (DW-0744b: 2K HEIC, about 1.6 GB), otherwise the photos times the sample's
    /// weight, plus the fallback previews (with HEIC) and the grid thumbnails.
    pub fn cost(&self, size: PreviewSize, format: PreviewFormat) -> Option<String> {
        if (size, format) == (PreviewSize::K2, PreviewFormat::Heic) {
            return Some("1.6 GB".into());
        }
        let fallback = if format == PreviewFormat::Heic { 420_000.0 } else { 0.0 };
        board_kb(size, format).map(|kb| words_of(kb * BOARD_PHOTOS + fallback + 300_000.0))
    }

    pub fn apply(&mut self, size: PreviewSize, format: PreviewFormat) {
        if let Some(total) = self.cost(size, format) {
            self.previews.total = total.clone();
            self.previews.on_pc = total;
        }
        if let Some(kb) = board_kb(size, format) {
            self.previews.uses[0] = words_of(kb * BOARD_PHOTOS);
        }
        self.previews.uses[1] = if format == PreviewFormat::Heic {
            "420 MB".into()
        } else {
            "0 MB".into()
        };
        self.previews.size = size;
        self.previews.format = format;
    }
}
