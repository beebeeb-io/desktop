//! The native half of the frame gate: every keyed element of one rendered frame, as JSON,
//! in the shape `scripts/capture-golden.mjs` exports the board in. `scripts/gate.py`
//! compares the two (geometry, pixels, text, icons); `scripts/compare.sh` runs all of it.
//!
//! Box, string and text colour come from Rune's element tree after layout. Typography and
//! glyph identity come from the notes `ui::kit::label` / `glyph` keep while this records.

use std::collections::BTreeMap;

use rune::text::TextSystem;
use rune::{Color, Cx, El, Env, Icon, Runtime};
use serde_json::{Map, Value, json};

use super::kit;

fn rgba(c: Color) -> Value {
    let hex = c.hex();
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    json!([byte(1), byte(3), byte(5), byte(7)])
}

fn round2(v: f32) -> f64 {
    (f64::from(v) * 100.0).round() / 100.0
}

/// The shapes of an icon body in the board export's notation: a path is its `d`, any other
/// shape is `tag:attr=value,…` without paint attributes.
pub fn icon_shapes(ic: &Icon) -> Vec<String> {
    let mut out = Vec::new();
    for tag in ic.body.split('<').skip(1) {
        let Some(end) = tag.find('>') else { continue };
        let tag = tag[..end].trim_end_matches('/').trim();
        let (name, rest) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        if name.starts_with('/') || name == "g" {
            continue;
        }
        let mut attrs = Vec::new();
        let mut rest = rest.trim_start();
        while let Some(eq) = rest.find("=\"") {
            let key = rest[..eq].trim();
            let Some(close) = rest[eq + 2..].find('"') else { break };
            let value = &rest[eq + 2..eq + 2 + close];
            attrs.push((key, value));
            rest = rest[eq + 2 + close + 1..].trim_start();
        }
        if name == "path" {
            if let Some((_, d)) = attrs.iter().find(|(k, _)| *k == "d") {
                out.push((*d).to_string());
            }
        } else {
            let kept: Vec<String> = attrs
                .iter()
                .filter(|(k, _)| !["fill", "stroke", "class", "style"].iter().any(|p| k.starts_with(p)))
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            out.push(format!("{name}:{}", kept.join(",")));
        }
    }
    out
}

/// Render `state` once, headless, and describe every keyed element.
pub fn export<S: 'static>(
    frame: &str,
    state: S,
    view: fn(&S, &mut Cx<S>) -> El,
    size: (f32, f32),
    dark: bool,
    fonts: &'static [&'static [u8]],
    families: (&'static str, &'static str),
) -> Value {
    kit::record();
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::with_text(state, view, size, env, TextSystem::new(fonts, families));
    rt.frame(0.0);
    let notes = kit::take();

    let mut out: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    for info in rt.elements() {
        let Some(key) = info.key.clone() else { continue };
        if info.exiting || out.contains_key(&key) {
            continue;
        }
        let mut e = Map::new();
        for (name, v) in ["x", "y", "w", "h"].iter().zip(info.rect) {
            e.insert((*name).into(), json!(round2(v)));
        }
        if let Some(text) = &info.text {
            e.insert("text".into(), json!(text));
        }
        if let Some(c) = info.text_color {
            e.insert("color".into(), rgba(c));
        }
        if info.fill.a > 0.0 {
            e.insert("fill".into(), rgba(info.fill));
        }
        out.insert(key, e);
    }
    for (key, note) in &notes {
        let mut e = match &note.of {
            Some(of) => {
                let Some(src) = out.get(of) else { continue };
                let mut e = Map::new();
                for f in ["x", "y", "w", "h"] {
                    e.insert(f.into(), src[f].clone());
                }
                e
            }
            None => match out.remove(key) {
                Some(e) => e,
                None => continue, // noted but not in the tree (a virtual row out of view)
            },
        };
        if let Some(style) = &note.style {
            e.insert("family".into(), json!(kit::family_name(style)));
            e.insert("size".into(), json!(round2(style.size)));
            e.insert("weight".into(), json!(style.weight.round() as i64));
            e.insert("tracking".into(), json!(round2(style.tracking)));
        }
        if let Some(text) = &note.text {
            e.insert("text".into(), json!(text));
        }
        if let Some(c) = note.color {
            e.entry("color").or_insert_with(|| rgba(c));
        }
        if let Some(ic) = note.icon {
            e.insert("iconName".into(), json!(ic.name));
            e.insert("icon".into(), json!(icon_shapes(ic)));
            e.insert("filled".into(), json!(note.filled));
        }
        if let Some(state) = note.state {
            e.insert("state".into(), json!(state));
        }
        out.insert(key.clone(), e);
    }
    json!({
        "frame": frame,
        "theme": if dark { "dark" } else { "light" },
        "width": size.0,
        "height": size.1,
        "elements": out,
    })
}

/// `--gate-export <out.json> --scene <frame-id> [--theme light|dark] [--size WxH]`: write the
/// export and return the process's exit code. `None` when the arguments ask for something else.
pub fn run_from_args<S: 'static>(
    scenes: &[(&str, fn(&mut S))],
    init: fn() -> S,
    view: fn(&S, &mut Cx<S>) -> El,
    fonts: &'static [&'static [u8]],
    families: (&'static str, &'static str),
) -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let out = value("--gate-export")?;
    let Some(frame) = value("--scene") else {
        eprintln!("gate-export: --scene <frame-id> is required");
        return Some(64);
    };
    let Some((_, apply)) = scenes.iter().find(|(name, _)| *name == frame) else {
        let known: Vec<&str> = scenes.iter().map(|(n, _)| *n).collect();
        eprintln!("gate-export: no scene named {frame}; scenes: {}", known.join(", "));
        return Some(2);
    };
    let size = value("--size")
        .and_then(|s| {
            s.split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        })
        .unwrap_or((1278.0, 798.0));
    let mut state = init();
    apply(&mut state);
    let doc = export(
        &frame,
        state,
        view,
        size,
        value("--theme").as_deref() == Some("dark"),
        fonts,
        families,
    );
    let count = doc["elements"].as_object().map_or(0, Map::len);
    let text = match serde_json::to_string_pretty(&doc) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("gate-export: {e}");
            return Some(1);
        }
    };
    if let Err(e) = std::fs::write(&out, text + "\n") {
        eprintln!("gate-export: cannot write {out}: {e}");
        return Some(1);
    }
    eprintln!("gate-export: {frame}: {count} keyed elements -> {out}");
    Some(if count == 0 { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shapes_reads_paths_and_other_shapes_as_the_board_export_does() {
        static IC: Icon = Icon {
            name: "t",
            view: 10.0,
            body: r#"<g fill="none" stroke="currentColor"><path d="M0 5.5h10"></path><rect x=".5" y=".5" width="9" height="9" rx="1.5"></rect></g>"#,
        };
        assert_eq!(
            icon_shapes(&IC),
            vec![
                "M0 5.5h10".to_string(),
                "rect:x=.5,y=.5,width=9,height=9,rx=1.5".to_string()
            ]
        );
    }
}
