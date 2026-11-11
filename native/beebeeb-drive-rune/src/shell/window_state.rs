//! Where a window was, and where to put it back: pure geometry plus one small JSON file.
//!
//! Saved per window kind in `<state dir>/native-window-state.json`
//! (`%LOCALAPPDATA%\io.beebeeb.app` on Windows: the folder the 0.8.x app keeps its state in).
//! The Tauri app's own `.window-state.json` is left alone: the engine boundary spec does not
//! ask the native app to keep it, its format belongs to the plugin, and 0.8.x can still read
//! it if someone goes back.
//!
//! All rectangles are physical desktop pixels, the space Rune's `WindowOptions::position` and
//! `physical_size` take; the saved scale turns the size back into points when the display's
//! scale has changed since.

use std::collections::BTreeMap;
use std::path::Path;

pub const FILE_NAME: &str = "native-window-state.json";
const VERSION: u64 = 1;
/// The file is a few hundred bytes; anything much larger is not ours.
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// How much of the window's top edge must stay on a display for it to be grabbed, in points.
const GRAB_W: f64 = 120.0;
const GRAB_H: f64 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
    pub primary: bool,
}

/// A window's frame position and content size, with the scale they were measured at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SavedWindow {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
    pub maximized: bool,
}

impl SavedWindow {
    fn is_sane(&self) -> bool {
        self.w > 0 && self.h > 0 && self.scale.is_finite() && self.scale >= 0.5 && self.scale <= 8.0
    }
}

/// What to open a window with. `None` leaves the choice to the OS and the window's defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Restored {
    pub position: Option<(i32, i32)>,
    pub physical_size: Option<(u32, u32)>,
    pub maximized: bool,
}

fn overlap(a0: i64, a1: i64, b0: i64, b1: i64) -> i64 {
    (a1.min(b1) - a0.max(b0)).max(0)
}

/// Whether enough of the window's top strip is on `m` to grab it with the pointer.
fn grabbable_on(s: &SavedWindow, m: &Monitor) -> bool {
    let (x, y, w) = (i64::from(s.x), i64::from(s.y), i64::from(s.w));
    let (mx, my, mw, mh) = (i64::from(m.x), i64::from(m.y), i64::from(m.w), i64::from(m.h));
    let strip_w = ((GRAB_W * m.scale) as i64).min(w);
    let strip_h = (GRAB_H * m.scale) as i64;
    overlap(x, x + w, mx, mx + mw) >= strip_w && y >= my && y + strip_h <= my + mh
}

fn area_on(s: &SavedWindow, m: &Monitor) -> i64 {
    let (x, y) = (i64::from(s.x), i64::from(s.y));
    let (mx, my) = (i64::from(m.x), i64::from(m.y));
    overlap(x, x + i64::from(s.w), mx, mx + i64::from(m.w)) * overlap(y, y + i64::from(s.h), my, my + i64::from(m.h))
}

/// The saved size in points, laid out for `m`: its scale, never larger than the display, never
/// smaller than the window's minimum (unless the display itself is smaller).
fn size_for(s: &SavedWindow, m: &Monitor, min_points: (f32, f32)) -> (u32, u32) {
    let fit = |saved: u32, min: f32, room: u32| {
        let points = f64::from(saved) / s.scale;
        let px = (points.max(f64::from(min)) * m.scale).round() as u32;
        px.min(room).max(1)
    };
    (fit(s.w, min_points.0, m.w), fit(s.h, min_points.1, m.h))
}

/// Where to open a window that was last at `saved`, on the displays that exist now.
///
/// - Nothing saved, or nothing usable: the defaults.
/// - Its display is still there: the same place, with the size re-laid for the display's
///   current scale and kept inside it.
/// - Its display is gone (or the window would be out of reach): centred on the primary display
///   at the same size in points.
pub fn restore(saved: Option<&SavedWindow>, monitors: &[Monitor], min_points: (f32, f32)) -> Restored {
    let Some(saved) = saved.filter(|s| s.is_sane()) else {
        return Restored::default();
    };
    let usable = |m: &&Monitor| m.w > 0 && m.h > 0 && m.scale.is_finite() && m.scale > 0.0;
    let home = monitors
        .iter()
        .filter(usable)
        .filter(|m| grabbable_on(saved, m))
        .max_by_key(|m| area_on(saved, m));
    if let Some(m) = home {
        let (w, h) = size_for(saved, m, min_points);
        // Keep the top-left where it was; only pull it in as far as the grab strip needs.
        let strip = ((GRAB_W * m.scale) as i64).min(i64::from(w));
        let (mx, my) = (i64::from(m.x), i64::from(m.y));
        let x = i64::from(saved.x).clamp(mx - i64::from(w) + strip, mx + i64::from(m.w) - strip);
        let y = i64::from(saved.y).clamp(my, (my + i64::from(m.h) - (GRAB_H * m.scale) as i64).max(my));
        return Restored {
            position: Some((x as i32, y as i32)),
            physical_size: Some((w, h)),
            maximized: saved.maximized,
        };
    }
    let primary = monitors
        .iter()
        .filter(usable)
        .find(|m| m.primary)
        .or_else(|| monitors.iter().find(usable));
    let Some(m) = primary else {
        return Restored {
            maximized: saved.maximized,
            ..Restored::default()
        };
    };
    let (w, h) = size_for(saved, m, min_points);
    let x = i64::from(m.x) + (i64::from(m.w) - i64::from(w)) / 2;
    let y = i64::from(m.y) + (i64::from(m.h) - i64::from(h)) / 2;
    Restored {
        position: Some((x as i32, y as i32)),
        physical_size: Some((w, h)),
        maximized: saved.maximized,
    }
}

/// A window's state as the OS reports it at close.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observed {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
    pub maximized: bool,
    pub minimized: bool,
    pub full_screen: bool,
}

/// What to save, given what was saved before. A maximized or full-screen window keeps its
/// earlier normal rectangle (so un-maximizing after a restart lands where it used to be); a
/// minimized one reports no usable rectangle at all, so nothing changes.
pub fn merge_for_save(previous: Option<SavedWindow>, now: Observed) -> Option<SavedWindow> {
    if now.minimized {
        return previous;
    }
    if now.maximized || now.full_screen {
        let base = previous.unwrap_or(SavedWindow {
            x: now.x,
            y: now.y,
            w: now.w,
            h: now.h,
            scale: now.scale,
            maximized: true,
        });
        return Some(SavedWindow {
            maximized: now.maximized,
            ..base
        });
    }
    let saved = SavedWindow {
        x: now.x,
        y: now.y,
        w: now.w,
        h: now.h,
        scale: now.scale,
        maximized: false,
    };
    if saved.is_sane() { Some(saved) } else { previous }
}

pub type SavedWindows = BTreeMap<String, SavedWindow>;

pub fn to_json(windows: &SavedWindows) -> String {
    let entries: serde_json::Map<String, serde_json::Value> = windows
        .iter()
        .map(|(key, s)| {
            let value = serde_json::json!({ "x": s.x, "y": s.y, "w": s.w, "h": s.h, "scale": s.scale, "maximized": s.maximized });
            (key.clone(), value)
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "version": VERSION, "windows": entries })).unwrap_or_default()
}

/// Reads what [`to_json`] wrote. Anything unreadable is dropped, entry by entry: a damaged
/// file costs a window its place, never the start.
pub fn from_json(text: &str) -> SavedWindows {
    let mut out = SavedWindows::new();
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return out;
    };
    if root["version"].as_u64() != Some(VERSION) {
        return out;
    }
    let Some(windows) = root["windows"].as_object() else {
        return out;
    };
    for (key, v) in windows {
        let int = |name: &str| v[name].as_i64().and_then(|n| i32::try_from(n).ok());
        let size = |name: &str| v[name].as_u64().and_then(|n| u32::try_from(n).ok());
        let (Some(x), Some(y), Some(w), Some(h), Some(scale)) =
            (int("x"), int("y"), size("w"), size("h"), v["scale"].as_f64())
        else {
            continue;
        };
        let saved = SavedWindow {
            x,
            y,
            w,
            h,
            scale,
            maximized: v["maximized"].as_bool().unwrap_or(false),
        };
        if saved.is_sane() {
            out.insert(key.clone(), saved);
        }
    }
    out
}

pub fn load(path: &Path) -> SavedWindows {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() <= MAX_FILE_BYTES => {}
        _ => return SavedWindows::new(),
    }
    std::fs::read_to_string(path)
        .map(|text| from_json(&text))
        .unwrap_or_default()
}

/// Writes beside the file and renames over it, so a crash mid-write leaves the old one.
pub fn save(path: &Path, windows: &SavedWindows) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, to_json(windows))?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
#[path = "window_state_tests.rs"]
mod tests;
