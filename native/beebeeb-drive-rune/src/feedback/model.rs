//! Notes on disk: the `note.json` / `tree.json` / `status.json` shapes, redaction, and the atomic
//! write. Nothing here knows about the window; `capture.rs` hands it plain data.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Schema id of `note.json`. Also the marker `scripts/check-feedback-absent.sh` greps a binary for.
pub const NOTE_SCHEMA: &str = "beebeeb-feedback-note/1";
pub const TREE_SCHEMA: &str = "beebeeb-feedback-tree/1";
/// What a secure element's text is replaced with, everywhere a note is written.
pub const REDACTED: &str = "[redacted: secure field]";
/// The colour secure boxes are painted in both PNGs (opaque mid grey, sRGB bytes).
pub const BLANK_RGBA: [u8; 4] = [128, 128, 128, 255];

pub const TAGS: [&str; 4] = ["Looks wrong", "Does not work", "Text", "Idea"];

/// One element as a note records it. Boxes are window points `[x, y, w, h]`.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementSnap {
    pub path: String,
    pub key: Option<String>,
    pub role: String,
    pub label: Option<String>,
    /// `None` for a secure element, whatever it held.
    pub text: Option<String>,
    pub rect: [f32; 4],
    pub visible: [f32; 4],
    pub fill: [f32; 4],
    pub text_color: Option<[f32; 4]>,
    pub radius: f32,
    pub opacity: f32,
    pub secure: bool,
    pub focused: bool,
    /// A spring still moves it (a flight, a layout change), or it is playing its exit.
    pub moving: bool,
    pub exiting: bool,
    pub parent: Option<usize>,
}

impl ElementSnap {
    pub fn to_json(&self) -> Value {
        let r = |v: f32| (f64::from(v) * 100.0).round() / 100.0;
        let rect = |b: [f32; 4]| json!({"x": r(b[0]), "y": r(b[1]), "w": r(b[2]), "h": r(b[3])});
        let colour = |c: [f32; 4]| json!([r(c[0]), r(c[1]), r(c[2]), r(c[3])]);
        json!({
            "key_path": self.path,
            "key": self.key,
            "role": self.role,
            // A label describes a field; Rune's contract is that it never holds the secret. A
            // secure element still loses it here: a careless label must not leak into a note.
            "label": if self.secure { None } else { self.label.clone() },
            "text": if self.secure { Some(REDACTED.to_string()) } else { self.text.clone() },
            "secure": self.secure,
            "box": rect(self.rect),
            "visible_box": rect(self.visible),
            "fill": colour(self.fill),
            "text_colour": self.text_color.map(colour),
            "radius": r(self.radius),
            "opacity": r(self.opacity),
            "focused": self.focused,
        })
    }
}

/// Index of the element with `path`.
pub fn index_of(tree: &[ElementSnap], path: &str) -> Option<usize> {
    tree.iter().position(|e| e.path == path)
}

/// `i` and its ancestors, nearest first.
pub fn ancestors(tree: &[ElementSnap], i: usize) -> Vec<usize> {
    std::iter::successors(Some(i), |&p| tree.get(p).and_then(|e| e.parent)).collect()
}

fn in_subtree(tree: &[ElementSnap], root: usize, i: usize) -> bool {
    ancestors(tree, i).contains(&root)
}

/// `tree.json`: the picked element, its ancestors (nearest first) and its whole subtree.
pub fn tree_json(tree: &[ElementSnap], picked: usize) -> Value {
    let up: Vec<Value> = ancestors(tree, picked)
        .into_iter()
        .skip(1)
        .map(|i| tree[i].to_json())
        .collect();
    let sub: Vec<Value> = (0..tree.len())
        .filter(|&i| i != picked && in_subtree(tree, picked, i))
        .map(|i| {
            let mut v = tree[i].to_json();
            v["depth"] = json!(ancestors(tree, i).iter().position(|&a| a == picked).unwrap_or(0));
            v
        })
        .collect();
    json!({
        "schema": TREE_SCHEMA,
        "element": tree[picked].to_json(),
        "ancestors": up,
        "subtree": sub,
        // Rune's element dump has no font per element (see README, "Rune limitations").
        "fonts": {"sans": "Inter", "mono": "JetBrains Mono"},
    })
}

/// Tightly packed sRGB RGBA8.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Rgba {
    /// The pixel box of a window-point rect at `scale`, clamped to the image.
    pub fn px_box(&self, rect: [f32; 4], scale: f32) -> [u32; 4] {
        let x0 = ((rect[0] * scale).round().max(0.0) as u32).min(self.w);
        let y0 = ((rect[1] * scale).round().max(0.0) as u32).min(self.h);
        let w = ((rect[2] * scale).round().max(0.0) as u32).min(self.w - x0);
        let h = ((rect[3] * scale).round().max(0.0) as u32).min(self.h - y0);
        [x0, y0, w, h]
    }

    pub fn fill(&mut self, b: [u32; 4], c: [u8; 4]) {
        for y in b[1]..b[1] + b[3] {
            let row = ((y * self.w + b[0]) * 4) as usize;
            for p in self.px[row..row + (b[2] * 4) as usize].chunks_exact_mut(4) {
                p.copy_from_slice(&c);
            }
        }
    }

    pub fn is_filled(&self, b: [u32; 4], c: [u8; 4]) -> bool {
        (b[1]..b[1] + b[3]).all(|y| {
            let row = ((y * self.w + b[0]) * 4) as usize;
            self.px[row..row + (b[2] * 4) as usize].chunks_exact(4).all(|p| p == c)
        })
    }

    pub fn crop(&self, b: [u32; 4]) -> Rgba {
        let mut px = Vec::with_capacity((b[2] * b[3] * 4) as usize);
        for y in b[1]..b[1] + b[3] {
            let row = ((y * self.w + b[0]) * 4) as usize;
            px.extend_from_slice(&self.px[row..row + (b[2] * 4) as usize]);
        }
        Rgba { w: b[2], h: b[3], px }
    }

    pub fn write_png(&self, path: &Path) -> std::io::Result<()> {
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(file, self.w, self.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(std::io::Error::other)?;
        w.write_image_data(&self.px).map_err(std::io::Error::other)?;
        w.finish().map_err(std::io::Error::other)
    }

    /// Decodes a PNG of at most `MAX_PNG_BYTES` and `MAX_PNG_SIDE` pixels a side: the inbox is a
    /// folder other programs can write to, so a picture in it is checked before it is decoded.
    pub fn read_png(path: &Path) -> std::io::Result<Rgba> {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file() || meta.len() > MAX_PNG_BYTES {
            return Err(std::io::Error::other("not a plain file, or too large"));
        }
        let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
        dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
        let mut reader = dec.read_info().map_err(std::io::Error::other)?;
        let (w, h) = (reader.info().width, reader.info().height);
        if w == 0 || h == 0 || w > MAX_PNG_SIDE || h > MAX_PNG_SIDE {
            return Err(std::io::Error::other("png dimensions out of range"));
        }
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| std::io::Error::other("png too large"))?;
        let mut px = vec![0; size];
        let info = reader.next_frame(&mut px).map_err(std::io::Error::other)?;
        if info.color_type != png::ColorType::Rgba {
            return Err(std::io::Error::other("png is not RGBA after normalising"));
        }
        px.truncate(info.buffer_size());
        Ok(Rgba {
            w: info.width,
            h: info.height,
            px,
        })
    }

    /// At most `max` pixels on the longer side, by dropping pixels (a thumbnail, not a resample).
    pub fn shrink(&self, max: u32) -> Rgba {
        let k = self.w.max(self.h).div_ceil(max.max(1)).max(1);
        let (w, h) = ((self.w / k).max(1), (self.h / k).max(1));
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * k * self.w + x * k) * 4) as usize;
                px.extend_from_slice(&self.px[i..i + 4]);
            }
        }
        Rgba { w, h, px }
    }
}

pub const MAX_PNG_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PNG_SIDE: u32 = 8192;

/// A capture is refused while anything on screen is still moving or leaving: a secure field in
/// flight has no settled box to blank, and a half-faded element is not what the note is about.
pub fn still(tree: &[ElementSnap]) -> Result<(), String> {
    match tree.iter().find(|e| e.moving || e.exiting) {
        Some(e) => Err(format!(
            "the screen is still moving ({}); press Enter again",
            e.key.as_deref().unwrap_or("an element")
        )),
        None => Ok(()),
    }
}

/// What an element and everything inside it says, to tell whether the element a note was opened
/// on is still the one on screen when the note is saved (rows are keyed by position).
pub fn fingerprint(tree: &[ElementSnap], i: usize) -> String {
    (0..tree.len())
        .filter(|&j| in_subtree(tree, i, j))
        .map(|j| {
            let e = &tree[j];
            format!(
                "{}|{}|{}|{}",
                e.role,
                e.key.as_deref().unwrap_or(""),
                e.label.as_deref().unwrap_or(""),
                e.text.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Paints every secure element's visible box in `BLANK_RGBA`, then checks each one: the
/// redaction is proven for this frame, or the note is not written. A secure box Rune draws scaled
/// or turned has no axis-aligned box to trust, so `rigid == false` refuses.
pub fn redact_pixels(img: &mut Rgba, tree: &[ElementSnap], scale: f32, rigid: bool) -> Result<usize, String> {
    let secure: Vec<&ElementSnap> = tree.iter().filter(|e| e.secure).collect();
    if secure.is_empty() {
        return Ok(0);
    }
    if !rigid {
        return Err(
            "a secure field is on screen inside a scaled or turned element: its box cannot be blanked reliably".into(),
        );
    }
    for e in &secure {
        // One point of margin: a caret, focus ring or glyph overhang stays inside the blank.
        let v = e.visible;
        let b = img.px_box([v[0] - 1.0, v[1] - 1.0, v[2] + 2.0, v[3] + 2.0], scale);
        img.fill(b, BLANK_RGBA);
    }
    for e in &secure {
        if !img.is_filled(img.px_box(e.visible, scale), BLANK_RGBA) {
            return Err(format!("could not prove the secure field at {} is blanked", e.path));
        }
    }
    Ok(secure.len())
}

/// Everything a note records besides the pixels and the tree.
#[derive(Clone, Debug)]
pub struct NoteMeta {
    pub comment: String,
    pub tags: Vec<String>,
    pub scale: f32,
    pub dark: bool,
    pub window_size: [f32; 2],
    pub frame: Value,
}

pub fn app_json() -> Value {
    json!({
        "name": "Beebeeb Drive",
        "version": env!("CARGO_PKG_VERSION"),
        "commit": option_env!("BEEBEEB_BUILD_COMMIT").unwrap_or("unknown"),
        "build_kind": option_env!("BEEBEEB_BUILD_KIND").unwrap_or("dev"),
    })
}

fn short_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    h.write_u32(std::process::id());
    format!("{:06x}", h.finish() & 0xff_ffff)
}

/// Writes one note folder into `inbox`, atomically: everything goes into a dot-prefixed temp
/// folder beside it, which is renamed into place once all four files are complete. Returns the
/// note's id (its folder name).
pub fn write_note(
    inbox: &Path,
    meta: &NoteMeta,
    tree: &[ElementSnap],
    picked: usize,
    window: &Rgba,
) -> Result<String, String> {
    let now = chrono::Utc::now();
    let id = format!("{}-{}", now.format("%Y%m%dT%H%M%SZ"), short_id());
    let el = &tree[picked];
    let crop = window.px_box(el.visible, meta.scale);
    if crop[2] == 0 || crop[3] == 0 {
        return Err("the element has no visible box to capture".into());
    }
    let note = json!({
        "schema": NOTE_SCHEMA,
        "id": id,
        "created_at": now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "app": app_json(),
        "platform": {
            "os": std::env::consts::OS,
            "scale": meta.scale,
            "theme": if meta.dark { "dark" } else { "light" },
            "window_size": {"w": meta.window_size[0], "h": meta.window_size[1]},
        },
        "frame": meta.frame,
        "element": el.to_json(),
        "comment": meta.comment,
        "tags": meta.tags,
        "files": {"window_png": "window.png", "element_png": "element.png", "tree_json": "tree.json"},
    });
    let io = |e: std::io::Error| format!("could not write the note: {e}");
    std::fs::create_dir_all(inbox).map_err(io)?;
    let tmp = inbox.join(format!(".tmp-{id}"));
    std::fs::create_dir(&tmp).map_err(io)?;
    let write = || -> std::io::Result<()> {
        window.write_png(&tmp.join("window.png"))?;
        window.crop(crop).write_png(&tmp.join("element.png"))?;
        std::fs::write(
            tmp.join("tree.json"),
            serde_json::to_vec_pretty(&tree_json(tree, picked))?,
        )?;
        std::fs::write(tmp.join("note.json"), serde_json::to_vec_pretty(&note)?)?;
        std::fs::rename(&tmp, inbox.join(&id))
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(io(e));
    }
    Ok(id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusState {
    New,
    Seen,
    Fixing,
    Fixed,
    WontFix,
}

pub const MAX_STATUS_BYTES: u64 = 16 * 1024;
pub const MAX_NOTE_JSON_BYTES: u64 = 256 * 1024;
pub const MAX_REPLY_CHARS: usize = 2000;
pub const MAX_BY_CHARS: usize = 40;
pub const MAX_COMMENT_CHARS: usize = 4000;
/// The inbox holds at most this many notes and bytes; see [`make_room`].
pub const MAX_NOTES: usize = 200;
pub const MAX_INBOX_BYTES: u64 = 1024 * 1024 * 1024;

/// Text from a file as the panel may show it: no control characters (a newline stays), no bidi
/// overrides or isolates, at most `max` characters.
pub fn clean_text(s: &str, max: usize) -> String {
    s.chars()
        .filter(|&c| c == '\n' || !(c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{feff}')))
        .take(max)
        .collect()
}

fn is_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A note's id is its folder name; only these may become element keys and paths.
pub fn valid_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `status.json`, as the lead's `feedback.sh set` writes it next to a note. It comes from outside
/// the app: anything that is not exactly the expected shape is dropped, never trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: StatusState,
    pub reply: Option<String>,
    pub fixed_in_commit: Option<String>,
    pub updated_at: Option<String>,
    pub by: Option<String>,
}

impl Status {
    pub const NEW: Status = Status {
        state: StatusState::New,
        reply: None,
        fixed_in_commit: None,
        updated_at: None,
        by: None,
    };

    pub fn parse(src: &str) -> Option<Status> {
        if src.len() as u64 > MAX_STATUS_BYTES {
            return None;
        }
        let v: Value = serde_json::from_str(src).ok()?;
        let v = v.as_object()?;
        let text = |k: &str, max: usize| {
            v.get(k)
                .and_then(Value::as_str)
                .map(|s| clean_text(s, max))
                .filter(|s| !s.trim().is_empty())
        };
        let state = match v.get("state")?.as_str()? {
            "new" => StatusState::New,
            "seen" => StatusState::Seen,
            "fixing" => StatusState::Fixing,
            "fixed" => StatusState::Fixed,
            "wontfix" => StatusState::WontFix,
            _ => return None,
        };
        Some(Status {
            state,
            reply: text("reply", MAX_REPLY_CHARS),
            fixed_in_commit: v
                .get("fixed_in_commit")
                .and_then(Value::as_str)
                .filter(|s| is_sha(s))
                .map(str::to_ascii_lowercase),
            updated_at: text("updated_at", 40),
            by: text("by", MAX_BY_CHARS),
        })
    }

    /// The chip's words.
    pub fn chip(&self) -> String {
        match self.state {
            StatusState::New => "New".into(),
            StatusState::Seen => "Seen".into(),
            StatusState::Fixing => "Being fixed".into(),
            StatusState::Fixed => match &self.fixed_in_commit {
                Some(c) => format!("Fixed in {}", c.chars().take(9).collect::<String>()),
                None => "Fixed".into(),
            },
            StatusState::WontFix => "Won't fix".into(),
        }
    }
}

type Stamp = (Option<std::time::SystemTime>, Option<std::time::SystemTime>);

/// A note as the panel lists it. `id` is the folder's name.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteEntry {
    pub id: String,
    pub dir: PathBuf,
    pub created_at: String,
    pub comment: String,
    pub tags: Vec<String>,
    pub element_path: String,
    pub status: Status,
    /// Bytes of the note's own files.
    pub bytes: u64,
    stamp: Stamp,
}

/// A plain file of at most `max` bytes (never a symlink), or nothing.
fn read_capped(path: &Path, max: u64) -> Option<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    (meta.is_file() && meta.len() <= max)
        .then(|| std::fs::read(path).ok())
        .flatten()
}

fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::symlink_metadata(path).ok()?.modified().ok()
}

const NOTE_FILES: [&str; 5] = ["note.json", "status.json", "window.png", "element.png", "tree.json"];

fn read_entry(dir: PathBuf, id: String, stamp: Stamp) -> Option<NoteEntry> {
    let v: Value = serde_json::from_slice(&read_capped(&dir.join("note.json"), MAX_NOTE_JSON_BYTES)?).ok()?;
    // The folder's name is the id: a copied folder cannot bring a second note with the same id.
    if v["schema"].as_str() != Some(NOTE_SCHEMA) || v["id"].as_str() != Some(id.as_str()) {
        return None;
    }
    let status = read_capped(&dir.join("status.json"), MAX_STATUS_BYTES)
        .and_then(|b| Status::parse(std::str::from_utf8(&b).ok()?))
        .unwrap_or(Status::NEW);
    let bytes = NOTE_FILES
        .iter()
        .filter_map(|f| std::fs::symlink_metadata(dir.join(f)).ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum();
    Some(NoteEntry {
        created_at: clean_text(v["created_at"].as_str().unwrap_or_default(), 40),
        comment: clean_text(v["comment"].as_str().unwrap_or_default(), MAX_COMMENT_CHARS),
        tags: v["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|t| TAGS.contains(t))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        element_path: clean_text(v["element"]["key_path"].as_str().unwrap_or_default(), 400),
        status,
        bytes,
        stamp,
        id,
        dir,
    })
}

/// Every complete note in `inbox`, newest first. A note whose `note.json` and `status.json` have
/// the modification times `previous` saw is taken from there without reading a byte. Skipped:
/// half-written (dot-prefixed) folders, symlinks, names that are not ids, and folders without a
/// readable `note.json` naming that folder. An unreadable `status.json` counts as "new".
pub fn scan(inbox: &Path, previous: &[NoteEntry]) -> Vec<NoteEntry> {
    let Ok(dir) = std::fs::read_dir(inbox) else {
        return Vec::new();
    };
    let mut out: Vec<NoteEntry> = dir
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().into_string().ok().filter(|n| valid_id(n))?;
            if !e.file_type().ok()?.is_dir() {
                return None;
            }
            let dir = e.path();
            let stamp: Stamp = (mtime(&dir.join("note.json")), mtime(&dir.join("status.json")));
            stamp.0?;
            match previous.iter().find(|p| p.id == id && p.stamp == stamp) {
                Some(p) => Some(p.clone()),
                None => read_entry(dir, id, stamp),
            }
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

/// Removes temp folders a crash left behind (older than ten minutes: never one being written).
pub fn sweep_temp(inbox: &Path) {
    let Ok(dir) = std::fs::read_dir(inbox) else { return };
    for e in dir.flatten() {
        let old = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|d| d.as_secs() > 600);
        if old && e.file_name().to_string_lossy().starts_with(".tmp-") && e.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Makes room for one more note under the caps: the oldest notes the lead marked "fixed" are
/// removed first, and only those. Nothing else is ever deleted; with no fixed note left, the new
/// note is refused and the message says why.
pub fn make_room(notes: &[NoteEntry], max_notes: usize, max_bytes: u64) -> Result<Vec<String>, String> {
    let mut count = notes.len();
    let mut bytes: u64 = notes.iter().map(|n| n.bytes).sum();
    let full = format!(
        "the inbox is full ({count} notes, {} MB) and too few are marked fixed: ask the lead to answer the oldest",
        bytes / (1024 * 1024)
    );
    // Plan first: when the fixed notes cannot make enough room, nothing is deleted.
    let mut plan = Vec::new();
    let mut fixed = notes.iter().rev().filter(|n| {
        n.status.state == StatusState::Fixed
            && valid_id(&n.id)
            && n.dir.file_name().and_then(|f| f.to_str()) == Some(n.id.as_str())
    });
    while count >= max_notes || bytes > max_bytes {
        let n = fixed.next().ok_or_else(|| full.clone())?;
        plan.push(n);
        count -= 1;
        bytes = bytes.saturating_sub(n.bytes);
    }
    for n in &plan {
        std::fs::remove_dir_all(&n.dir).map_err(|e| format!("could not remove the fixed note {}: {e}", n.id))?;
    }
    Ok(plan.into_iter().map(|n| n.id.clone()).collect())
}
