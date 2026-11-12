//! Keep on this PC (DW-0784): choose which folders stay on this PC, many at once, and see
//! what each choice does before it happens. Over it: the question before downloaded copies
//! leave (DW-0784b). States: the folder list could not load (DW-0784c), no folders yet
//! (DW-0784d).
//!
//! Nothing changes until Apply. The choices are a draft (`KeepDraft`, a copy of the tree)
//! that the dialog edits; the source hears of them once, in `DriveSettings::apply_keep`.

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_dialog::{self as dialogs, Weight};
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::settings_shell::SUB;
use rune::tok::Family;
use rune::{Align, El, TextStyle, col, el, row};

use super::{Dialog, SettingsUi};

/// One folder of the tree.
#[derive(Clone, PartialEq, Debug)]
pub struct Folder {
    /// The server's id of the folder.
    pub id: String,
    pub name: String,
    /// What it holds, as the Size column says it: "48 items", "4 devices".
    pub count: String,
    /// Its size in megabytes (the sums of the foot are made from these).
    pub mb: f32,
    /// Kept on this PC: this folder and everything in it, also what is added later.
    pub kept: bool,
    /// It has folders inside (it gets a disclosure), loaded or not.
    pub expandable: bool,
    /// Shown open.
    pub open: bool,
    pub children: Vec<Folder>,
}

/// The folder list, as the source has it.
#[derive(Clone, PartialEq, Debug)]
pub enum Keep {
    /// It could not load: nothing is changed (DW-0784c).
    Failed,
    /// The account's folders (none: DW-0784d), and the size of everything in the account
    /// in megabytes (what is not kept is online only).
    Ready { folders: Vec<Folder>, total_mb: f32 },
}

/// A folder's box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Check {
    /// Kept on this PC, every folder inside too.
    On,
    /// Some folders inside are kept.
    Mixed,
    /// Online only.
    Off,
}

impl Folder {
    pub fn check(&self) -> Check {
        if self.kept {
            Check::On
        } else if self.children.iter().any(|c| c.check() != Check::Off) {
            Check::Mixed
        } else {
            Check::Off
        }
    }
    fn set_all(&mut self, kept: bool) {
        self.kept = kept;
        for c in &mut self.children {
            c.set_all(kept);
        }
    }
    /// Megabytes kept on this PC under this folder.
    fn kept_mb(&self) -> f32 {
        if self.kept {
            self.mb
        } else {
            self.children.iter().map(Folder::kept_mb).sum()
        }
    }
}

/// "1.4 GB", "27.8 MB": a size as the tree says it.
pub fn size(mb: f32) -> String {
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{mb:.1} MB")
    }
}

fn find<'a>(folders: &'a [Folder], id: &str) -> Option<&'a Folder> {
    folders
        .iter()
        .find_map(|f| if f.id == id { Some(f) } else { find(&f.children, id) })
}

/// Checks or unchecks folder `id` and everything in it; the folders it is in follow (all of
/// a folder's folders kept: kept; else not). `false`: no such folder.
fn toggle_in(folders: &mut [Folder], id: &str) -> bool {
    for f in folders.iter_mut() {
        if f.id == id {
            let keep = f.check() != Check::On;
            f.set_all(keep);
            return true;
        }
        if toggle_in(&mut f.children, id) {
            // Unchecking one folder inside a kept folder keeps the others, one by one.
            f.kept = !f.children.is_empty() && f.children.iter().all(|c| c.kept);
            return true;
        }
    }
    false
}

fn open_in(folders: &mut [Folder], id: &str) -> bool {
    folders.iter_mut().any(|f| {
        if f.id == id {
            f.open = !f.open;
            true
        } else {
            open_in(&mut f.children, id)
        }
    })
}

/// What a choice does once applied, on the folder's own row.
#[derive(Clone, PartialEq, Debug)]
pub enum Change {
    /// Its downloaded copies leave this PC.
    Leaves,
    /// It downloads.
    Downloads,
}

/// The choices not applied yet: a copy of the source's tree, edited by the dialog.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct KeepDraft {
    pub folders: Vec<Folder>,
}

impl KeepDraft {
    pub fn toggle(&mut self, id: &str) {
        toggle_in(&mut self.folders, id);
    }
    pub fn toggle_open(&mut self, id: &str) {
        open_in(&mut self.folders, id);
    }
    /// The folders whose choice differs from the stored one, top-most only: a folder inside
    /// one that changed the same way is part of that change.
    pub fn changes(&self, stored: &[Folder]) -> Vec<(Folder, Change)> {
        fn walk(folders: &[Folder], stored: &[Folder], out: &mut Vec<(Folder, Change)>) {
            for f in folders {
                let was = find(stored, &f.id).is_some_and(|s| s.kept);
                match (was, f.kept) {
                    (true, false) if f.check() == Check::Off => out.push((f.clone(), Change::Leaves)),
                    (false, true) => out.push((f.clone(), Change::Downloads)),
                    (true, true) => {}
                    _ => walk(&f.children, find(stored, &f.id).map_or(&[], |s| &s.children), out),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.folders, stored, &mut out);
        out
    }
    /// The ids to keep, top-most only (a kept folder keeps everything in it).
    pub fn kept_ids(&self) -> Vec<String> {
        fn walk(folders: &[Folder], out: &mut Vec<String>) {
            for f in folders {
                if f.kept {
                    out.push(f.id.clone());
                } else {
                    walk(&f.children, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.folders, &mut out);
        out
    }
    pub fn kept_mb(&self) -> f32 {
        self.folders.iter().map(Folder::kept_mb).sum()
    }
}

/// Open the dialog on the stored choices (a fresh draft).
pub fn open(ui: &mut SettingsUi) {
    ui.more.keep = match ui.drive.keep() {
        Keep::Ready { folders, .. } => KeepDraft { folders },
        Keep::Failed => KeepDraft::default(),
    };
    ui.dialog = Some(Dialog::Keep);
}

/// Apply was pressed in the dialog: asked first when copies leave this PC (DW-0784b);
/// checking folders alone applies without a question.
fn apply(s: &mut State) {
    let ui = &mut s.settings;
    let Keep::Ready { folders: stored, .. } = ui.drive.keep() else {
        return;
    };
    let changes = ui.more.keep.changes(&stored);
    if changes.is_empty() {
        return;
    }
    if changes.iter().any(|(_, c)| *c == Change::Leaves) {
        ui.dialog = Some(Dialog::KeepConfirm);
    } else {
        confirm(s);
    }
}

/// Both kinds of change go to the source; the dialog closes on Sync and File Explorer.
fn confirm(s: &mut State) {
    let ui = &mut s.settings;
    let ids = ui.more.keep.kept_ids();
    ui.drive.apply_keep(&ids);
    ui.more.keep = KeepDraft::default();
    ui.dialog = None;
}

/// 11px/600 on a 17px line: the tree's column heads (`.mk-trow.is-head`).
const TREE_HEAD: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0);
/// 13px/500 on a 17px line: a folder's name (`.mk-tname b`).
const NAME: TextStyle = TextStyle::new(13.0, 500.0).lh(17.0);
/// 11px/500 on a 17px line: what a choice does (`.mk-tnote`).
const NOTE: TextStyle = TextStyle::new(11.0, 500.0).lh(17.0);
/// 11.5px/400 on a 17px line: "Some folders kept" (`.mk-tname i`).
const MIXED: TextStyle = TextStyle::new(11.5, 400.0).lh(17.0);
/// 11.5px/400 mono: a folder's size (`.mk-tsize`).
const SIZE: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(15.0);
/// 11px/500 mono on a 14px line: the foot's sum (`.t-cap .font-mono`).
const SUM: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);

/// A folder's box (`.mk-ck`), 15 pt: a check when kept, a dash when some inside are kept.
fn check_mark(key: &str, state: Check, t: &Theme) -> El {
    let b = el().key(key.to_string()).size(15.0, 15.0).radius(4.0).center();
    match state {
        Check::On => b
            .fill(t.text)
            .child(kit::glyph(format!("{key}-mark"), &ic::CHECK, 11.0, t.paper)),
        Check::Mixed => b
            .fill(t.text)
            .child(el().key(format!("{key}-mark")).size(7.0, 1.6).radius(1.0).fill(t.paper)),
        Check::Off => b
            .fill(t.card)
            // `box-shadow: inset 0 0 0 1px` in the board's `--color-line-strong`.
            .box_shadow(rune::Shadow {
                spread: 1.0,
                ..kit::ring_inset(t.separator_strong)
            }),
    }
}

fn tree_row(f: &Folder, depth: usize, change: Option<&Change>, t: &Theme) -> El {
    let key = format!("tree-{}", f.id);
    let state = f.check();
    let (id_open, id_check) = (f.id.clone(), f.id.clone());
    let mut disclosure = el().key(format!("{key}-disc")).w(12.0).h(15.0).center_y();
    if f.expandable {
        disclosure = disclosure
            .role("button")
            .label(format!("{} {}", if f.open { "Close" } else { "Open" }, f.name))
            .expanded(f.open)
            .radius(3.0)
            .hover(t.hover)
            .focusable(true)
            .on_press(move |s: &mut State| s.settings.more.keep.toggle_open(&id_open))
            .child(kit::glyph(
                format!("{key}-disc-icon"),
                if f.open { &ic::CARET_DOWN } else { &ic::CHEV_RIGHT_TREE },
                11.0,
                t.text_tertiary,
            ));
    }
    let said = match state {
        Check::On => "kept on this PC",
        Check::Mixed => "mixed",
        Check::Off => "online only",
    };
    let check = el()
        .key(format!("{key}-check"))
        .role("checkbox")
        .label(format!("{}, {said}", f.name))
        .checked(state == Check::On)
        .radius(4.0)
        .focusable(true)
        .on_press(move |s: &mut State| s.settings.more.keep.toggle(&id_check))
        .child(check_mark(&format!("{key}-ck"), state, t));
    let mut name = row()
        .key(format!("{key}-name"))
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(7.0)
        .pl(22.0 * depth as f32)
        .child(disclosure)
        .child(check)
        .child(kit::glyph(format!("{key}-icon"), &ic::FOLDER, 15.0, t.text))
        .child(kit::label(format!("{key}-label"), f.name.clone(), NAME, t.text));
    match change {
        Some(change) => {
            let words = match change {
                Change::Leaves => format!("Leaves this PC · {}", size(f.mb)),
                Change::Downloads => format!("Downloads · {}", size(f.mb)),
            };
            name = name.child(
                el().key(format!("{key}-note"))
                    .py(1.0)
                    .px(6.0)
                    .radius(5.0)
                    .fill(t.text.alpha(0.07))
                    .child(kit::label(format!("{key}-note-text"), words, NOTE, t.text_secondary)),
            );
        }
        None if state == Check::Mixed => {
            name = name.child(kit::label(
                format!("{key}-mixed"),
                "Some folders kept",
                MIXED,
                t.text_tertiary,
            ));
        }
        None => {}
    }
    row()
        .key(key.clone())
        .role("treeitem")
        .label(f.name.clone())
        .w_full()
        .h(32.0)
        .px(12.0)
        .center_y()
        // The hairline takes room (a CSS border): the row's content is centred under it.
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(name)
        .child(
            kit::label(
                format!("{key}-size"),
                format!("{} · {}", f.count, size(f.mb)),
                SIZE,
                t.text_tertiary,
            )
            .w(150.0)
            .align(Align::End),
        )
}

fn tree_rows(folders: &[Folder], depth: usize, changes: &[(Folder, Change)], out: &mut Vec<El>, t: &Theme) {
    for f in folders {
        let change = changes.iter().find(|(c, _)| c.id == f.id).map(|(_, c)| c);
        out.push(tree_row(f, depth, change, t));
        if f.open {
            tree_rows(&f.children, depth + 1, changes, out, t);
        }
    }
}

/// The frame of the tree (`.mk-tree`): a card with an inset hairline.
fn tree_box() -> El {
    col().key("tree").w_full().radius(10.0)
}

fn legend(t: &Theme) -> El {
    let entry = |i: usize, state: Check, words: &str| {
        row()
            .key(format!("legend-{i}"))
            .center_y()
            .gap(6.0)
            .child(check_mark(&format!("legend-{i}-ck"), state, t))
            .child(kit::label(
                format!("legend-{i}-label"),
                words,
                type_::CAPTION,
                t.text_tertiary,
            ))
    };
    row()
        .key("legend")
        .a11y_hidden()
        .w_full()
        .center_y()
        .gap(16.0)
        .child(entry(0, Check::On, "Kept on this PC"))
        .child(entry(1, Check::Mixed, "Some folders inside kept"))
        .child(entry(2, Check::Off, "Online only"))
}

/// A state of the dialog without a tree: a tile, what is the case, and one thing to do.
fn stated(tile: El, head: &str, words: &str, max_w: Option<f32>, action: Option<El>, t: &Theme) -> El {
    let mut says = kit::label("tree-state-sub", words, SUB, t.text_tertiary)
        .wrap()
        .align(Align::Center);
    if let Some(w) = max_w {
        says = says.w(w);
    }
    let mut b = tree_box()
        .role("status")
        .label(format!("{head} {words}"))
        .items_center()
        .py(40.0)
        .px(24.0)
        .gap(8.0)
        .fill(t.card)
        .box_shadow(kit::ring_inset(t.separator))
        .child(tile)
        .child(kit::label("tree-state-head", head, type_::HEAD, t.text))
        .child(says);
    if let Some(action) = action {
        b = b.child(action);
    }
    b
}

/// The dialog (DW-0784, DW-0784c, DW-0784d). `first`: the number of its first button, after
/// the pane's own.
pub fn dialog(ui: &SettingsUi, first: usize, t: &Theme) -> El {
    let draft = &ui.more.keep;
    let mut n = first;
    let (body, sum, changed) = match ui.drive.keep() {
        Keep::Failed => {
            let again = controls::button(n, "Try again", None, Emphasis::Plain, t).on_press(|s: &mut State| {
                s.settings.drive.reload_keep();
                open(&mut s.settings);
            });
            n += 1;
            let b = stated(
                notices::tile(0, Tone::Red, &ic::WARN_CIRCLE, t),
                "Couldn't load your folders.",
                "Nothing was changed. The folders you keep stay on this PC.",
                None,
                Some(again),
                t,
            );
            (vec![b], None, false)
        }
        Keep::Ready { folders, .. } if folders.is_empty() => {
            let b = stated(
                notices::tile(0, Tone::Neutral, &ic::FOLDER, t),
                "No folders yet",
                "Once you add folders, you can choose which ones stay on this PC. Files you keep one by one are in Available offline.",
                Some(302.81),
                None,
                t,
            );
            (vec![b], None, false)
        }
        Keep::Ready {
            folders: stored,
            total_mb,
        } => {
            let changes = draft.changes(&stored);
            let head = row()
                .key("tree-head")
                .w_full()
                .h(26.0)
                .px(12.0)
                .center_y()
                .fill(t.text.alpha(0.03))
                .child(kit::label("tree-head-folder", "Folder", TREE_HEAD, t.text_tertiary).grow(1.0))
                .child(
                    kit::label("tree-head-size", "Size", TREE_HEAD, t.text_tertiary)
                        .w(150.0)
                        .align(Align::End),
                );
            let mut rows = vec![head];
            tree_rows(&draft.folders, 0, &changes, &mut rows, t);
            let mut clip = col()
                .key("tree-rows")
                .role("tree")
                .label("Folders")
                .w_full()
                .clip_rounded(10.0);
            for r in rows {
                clip = clip.child(r);
            }
            let tree = tree_box()
                .fill(t.card)
                .child(clip)
                // The hairline is inside the card's edge and over the rows' tint.
                .child(
                    el().key("tree-ring")
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .radius(10.0)
                        .box_shadow(kit::ring_inset(t.separator)),
                );
            let on_pc = draft.kept_mb();
            let sum = format!(
                "After Apply: {} on this PC · {} online only",
                size(on_pc),
                size((total_mb - on_pc).max(0.0))
            );
            let body = vec![
                tree,
                legend(t),
                dialogs::caption(
                    0,
                    "Unchecking a folder removes its downloaded copies from this PC. The files stay in Beebeeb and download again when you open them. Changes that haven't uploaded yet stay on this PC until they have.",
                    t,
                ),
            ];
            (body, Some(sum), !changes.is_empty())
        }
    };
    // Apply is dimmed until something changed.
    let mut apply_button = dialogs::button(n, "Apply", Weight::Primary, t);
    apply_button = if changed {
        apply_button.default_button().on_press(apply)
    } else {
        apply_button.opacity(0.45).disabled(true)
    };
    let cancel = dialogs::button(n + 1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None);
    let lead = match sum {
        Some(sum) => kit::label("dlg-sum", sum, SUM, t.text_tertiary).grow(1.0),
        None => el().key("dlg-sum").grow(1.0),
    };
    dialogs::dialog(
        "Keep on this PC",
        600.0,
        "Keep on this PC",
        Some("Folders you choose stay on this PC. Everything else downloads when you open it."),
        body,
        vec![lead, apply_button, cancel],
        t,
    )
    .modal(false, |s: &mut State| s.settings.dialog = None)
}

/// The question before downloaded copies leave this PC (DW-0784b): what leaves, that nothing
/// is deleted from Beebeeb, that changes not uploaded yet stay, and what downloads.
pub fn question(ui: &SettingsUi, first: usize, t: &Theme) -> El {
    let stored = match ui.drive.keep() {
        Keep::Ready { folders, .. } => folders,
        Keep::Failed => Vec::new(),
    };
    let changes = ui.more.keep.changes(&stored);
    let of = |kind: Change| -> Vec<&Folder> { changes.iter().filter(|(_, c)| *c == kind).map(|(f, _)| f).collect() };
    let (leaves, downloads) = (of(Change::Leaves), of(Change::Downloads));
    let (title, mut says) = match leaves.as_slice() {
        [one] => (
            format!("Stop keeping {} on this PC?", one.name),
            format!("Its {} ({}) become online only.", one.count, size(one.mb)),
        ),
        many => (
            format!("Stop keeping {} folders on this PC?", many.len()),
            format!(
                "{} ({}) become online only.",
                many.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "),
                size(many.iter().map(|f| f.mb).sum())
            ),
        ),
    };
    says.push_str(
        " They stay in Beebeeb and download again when you open them. Changes that haven't uploaded yet stay on this PC until they have.",
    );
    match downloads.as_slice() {
        [] => {}
        [one] => says.push_str(&format!(" {} ({}) downloads now.", one.name, size(one.mb))),
        many => says.push_str(&format!(
            " {} folders ({}) download now.",
            many.len(),
            size(many.iter().map(|f| f.mb).sum())
        )),
    }
    let foot = vec![
        dialogs::button(first, "Apply", Weight::Primary, t)
            .default_button()
            .on_press(confirm),
        // Back to the dialog, with the choices kept.
        dialogs::button(first + 1, "Cancel", Weight::Quiet, t)
            .on_press(|s: &mut State| s.settings.dialog = Some(Dialog::Keep)),
    ];
    alert(
        "Stop keeping folders on this PC",
        notices::tile(0, Tone::Neutral, &ic::OFFLINE_PIN, t),
        &title,
        &says,
        foot,
        t,
    )
    .modal(false, |s: &mut State| s.settings.dialog = Some(Dialog::Keep))
}

/// `settings_dialog::alert` for a question that does not fit one line: the title goes on to
/// a second line (`.wn-dlg-alert .t-title` wraps), everything else is the same 380 pt alert.
fn alert(name: &str, tile: El, title: &str, sub: &str, foot: Vec<El>, t: &Theme) -> El {
    const WIDTH: f32 = 380.0;
    let words = col()
        .key("dlg-alert-words")
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .child(
            kit::label("dlg-title", title, dialogs::DIALOG_TITLE, t.text)
                .role("heading")
                .level(2)
                .wrap()
                .w_full(),
        )
        .child(kit::label("dlg-sub", sub, SUB, t.text_tertiary).wrap());
    let head = row()
        .key("dlg-head")
        .pt(20.0)
        .px(24.0)
        .pb(16.0)
        .gap(14.0)
        .items_start()
        .child(tile)
        .child(words);
    let each = (WIDTH - 48.0 - 8.0 * (foot.len().max(1) - 1) as f32) / foot.len().max(1) as f32;
    let dlg = col()
        .key("dlg")
        .role("alertdialog")
        .label(name)
        .pixel_snap(false)
        .w(WIDTH)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, rune::Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key("dlg-clip")
                .w_full()
                .clip_rounded(8.0)
                .child(head)
                .child(dialogs::foot_row(foot, Some(each), t)),
        );
    col()
        .key("scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .fill(dialogs::scrim_color(t))
        .child(dlg)
}
