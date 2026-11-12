//! Diagnostics (DW-0790): what Beebeeb is doing on this PC, and a support bundle a person can
//! read before sending it. State: the bundle is saved (DW-0790b), with the two other ways the
//! export can end (no email app opened; the file could not be saved).
//!
//! A page of General (General › Help › Diagnostics). Nothing is sent by Beebeeb: the person
//! attaches the file to the draft.

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item};
use crate::ui::settings_notice::NOTE;
use crate::ui::settings_shell as shell;
use rune::tok::Family;
use rune::{El, Shape, Span, canvas, el, rich, row};

use super::{Intent, SettingsUi};

/// What one row of App activity measures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Measure {
    Cpu,
    Memory,
    Upload,
    Download,
}

impl Measure {
    pub fn title(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Upload => "Upload",
            Self::Download => "Download",
        }
    }
    fn icon(self) -> &'static rune::Icon {
        match self {
            Self::Cpu => &ic::CPU,
            Self::Memory => &ic::MEMORY,
            Self::Upload => &ic::ARROW_UP,
            Self::Download => &ic::ARROW_DOWN,
        }
    }
}

/// One measure of Beebeeb's own use of this PC: its value now, in the words the row shows
/// ("3.1 %"), and the last samples, oldest first (the sparkline).
#[derive(Clone, PartialEq, Debug)]
pub struct Activity {
    pub measure: Measure,
    pub now: String,
    pub samples: Vec<f32>,
}

/// Where the last export of a support bundle ended (DW-0790b and its rules).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Bundle {
    /// Nothing was exported since the page opened.
    #[default]
    None,
    /// Saved: the folder as the banner names it (an environment name, never a person's own
    /// path), the file's name, and whether a draft opened in the person's email app.
    Saved {
        folder: String,
        file: String,
        draft_opened: bool,
    },
    /// Not saved: the reason, in the engine's words.
    Failed(String),
}

/// The Diagnostics page.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Diagnostics {
    /// Empty: nothing is measured (the list is not drawn).
    pub activity: Vec<Activity>,
    pub bundle: Bundle,
}

/// The sentences under Export… (the 0.8 desktop app's, `src/diagnosticsCopy.ts`): pinned to
/// what the export really removes and keeps.
pub const BUNDLE_SAYS: &str = "Saves a support bundle you can read before you send it: queue counts, an error code and the last error message. Paths, file and folder names and sign-in tokens are removed. Standard error words, numbers, opaque ids and the server address are kept, so a name the app does not recognise can stay only if it is itself a standard word or a number.";

/// 12px/400 mono on a 15px line: a measure's value (`.font-mono.t-sub`).
const VALUE: rune::TextStyle = rune::TextStyle::new(12.0, 400.0).family(Family::Mono).lh(15.0);

/// A sparkline (`.kit-spark.tone-ink`), 72 x 20: the samples left to right, the lowest 2 pt
/// above the foot and the highest 2 pt under the top, a faint wash under the line.
pub fn spark(key: &str, samples: &[f32], t: &Theme) -> El {
    const W: f32 = 72.0;
    const H: f32 = 20.0;
    let mut shape = Shape::new().view_box(W, H);
    if samples.len() >= 2 {
        let lo = samples.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = samples.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let span = if hi > lo { hi - lo } else { 1.0 };
        let step = W / (samples.len() - 1) as f32;
        let line: Vec<[f32; 2]> = samples
            .iter()
            .enumerate()
            .map(|(i, v)| [i as f32 * step, H - 2.0 - (v - lo) / span * (H - 4.0)])
            .collect();
        let mut area = line.clone();
        area.push([W, H]);
        area.push([0.0, H]);
        shape = shape.fill(area, t.text.alpha(0.09)).stroke(line, 1.6, t.text);
    }
    canvas(shape).key(key.to_string()).a11y_hidden().size(W, H)
}

/// The banner over the page once an export ended (`.mk-banner`): where the file is and what
/// happens next, or why there is no file. Keys: `note-0`, `note-0-icon`, `note-0-text`, `btn-0`.
fn banner(bundle: &Bundle, t: &Theme) -> Option<El> {
    let (icon, strong, rest, mono): (&'static rune::Icon, &str, String, Vec<String>) = match bundle {
        Bundle::None => return None,
        Bundle::Saved {
            folder,
            file,
            draft_opened,
        } => (
            &ic::CHECK,
            "Support bundle saved.",
            if *draft_opened {
                "A draft to support is opening in your email app; attach the file yourself.".into()
            } else {
                "Your email app didn't open; send the file to support@beebeeb.io yourself.".into()
            },
            vec![folder.clone(), file.clone()],
        ),
        Bundle::Failed(reason) => (
            &ic::WARN_CIRCLE,
            "Couldn't save the support bundle.",
            reason.clone(),
            Vec::new(),
        ),
    };
    // One text, as the board's: the strong sentence, "Saved to", the folder and the file each
    // on a line of their own in mono, then what happens next.
    let mut words = strong.to_string();
    let mut spans = vec![Span::new(0, strong.len()).weight(600.0).color(t.text)];
    if !mono.is_empty() {
        words.push_str(" Saved to");
        for line in &mono {
            words.push('\n');
            let from = words.len();
            words.push_str(line);
            spans.push(Span::new(from, words.len()).mono());
        }
        words.push('\n');
    } else {
        words.push(' ');
    }
    words.push_str(&rest);
    kit::note_text("note-0-text", "note-0-text", &words, NOTE, t.text_secondary);
    let mut line = row()
        .key("note-0")
        .role("status")
        .label(words.replace('\n', " "))
        .w_full()
        .center_y()
        .gap(10.0)
        .py(9.0)
        .pl(12.0)
        .pr(10.0)
        .radius(10.0)
        .fill(t.text.alpha(0.06))
        .child(kit::glyph("note-0-icon", icon, 16.0, t.text_secondary))
        .child(
            rich(words, spans)
                .key("note-0-text")
                .font(NOTE)
                .color(t.text_secondary)
                .grow(1.0)
                .min_w(0.0),
        );
    if matches!(bundle, Bundle::Saved { .. }) {
        line = line.child(
            controls::button(0, "Show in File Explorer", None, Emphasis::Plain, t)
                .on_press(|s: &mut State| s.settings.ask(Intent::Open("support-bundle"))),
        );
    }
    Some(el().key("note-0-margin").w_full().pt(10.0).px(14.0).child(line))
}

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let d = ui.drive.diagnostics();
    let mut children = vec![
        shell::crumb("General", t, |s: &mut State| super::go(s, "DW-0701")),
        shell::heading(
            "Diagnostics",
            Some("What Beebeeb is doing on this PC, and a file you can read before you send it to support."),
            t,
        ),
    ];
    let note = banner(&d.bundle, t);
    // Buttons are numbered in the order the page draws them: the banner's comes first.
    let export = usize::from(matches!(d.bundle, Bundle::Saved { .. }));
    children.extend(note);

    if !d.activity.is_empty() {
        let rows = d
            .activity
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let mut it = Item::new(Some(a.measure.icon()), a.measure.title());
                if i > 0 {
                    it = it.ruled();
                }
                let figure = row()
                    .key(format!("spark-{i}-box"))
                    .center_y()
                    .gap(12.0)
                    .child(spark(&format!("spark-{i}"), &a.samples, t))
                    .child(kit::label(
                        format!("spark-{i}-value"),
                        a.now.clone(),
                        VALUE,
                        t.text_tertiary,
                    ));
                lists::item_with(0, i, &it, vec![figure], t).label(format!("{}, {}", a.measure.title(), a.now))
            })
            .collect();
        children.push(lists::list(
            0,
            Some("App activity"),
            rows,
            Some("Beebeeb's own use of this PC, sampled every 2 seconds while this page is open."),
            t,
        ));
    }

    children.push(lists::list(
        1,
        Some("Support bundle"),
        vec![lists::item_with(
            1,
            0,
            &Item::new(Some(&ic::BOX), "Export a support bundle").sub("A file on this PC, then a draft to support"),
            vec![
                controls::button(export, "Export…", None, Emphasis::Plain, t).on_press(|s: &mut State| {
                    s.settings.ask(Intent::SaveBundleAndWriteEmail);
                    super::go(s, "DW-0790b");
                }),
            ],
            t,
        )],
        Some(BUNDLE_SAYS),
        t,
    ));

    children.push(lists::list(
        2,
        Some("Help"),
        vec![
            lists::item(
                2,
                0,
                &Item::new(Some(&ic::GAUGE), "Test connection speed")
                    .sub("In Bandwidth")
                    .opens(),
                t,
            )
            .on_press(|s: &mut State| super::go(s, "DW-0785")),
            lists::item(
                2,
                1,
                &Item::new(Some(&ic::CHAT), "Get help")
                    .sub("Opens our support page in your browser.")
                    .trailing(lists::Trailing::External)
                    .role(lists::Role::Link),
                t,
            )
            .on_press(|s: &mut State| s.settings.ask(Intent::Web("support"))),
            lists::item(2, 2, &Item::new(Some(&ic::INFO), "Report a problem").opens(), t)
                .on_press(|s: &mut State| super::go(s, "DW-0782")),
        ],
        None,
        t,
    ));
    (shell::pane(children, t), None)
}
