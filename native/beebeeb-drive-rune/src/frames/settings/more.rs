//! What the panes of lane settings-rest add to the group's shared state, kept in ONE field
//! each so the shared files change by one line: `MoreUi` in `SettingsUi` (what is open and
//! drafted inside these panes) and `BoardMore` in `BoardDrive` (their sample content).

use super::diagnostics::{Activity, Bundle, Diagnostics, Measure};
use super::keep::{Folder, Keep, KeepDraft};

/// What is open or drafted inside Diagnostics and Keep on this PC.
#[derive(Default)]
pub struct MoreUi {
    /// The choices of Keep on this PC that are not applied yet (DW-0784).
    pub keep: KeepDraft,
}

/// The board's sample content for Diagnostics (DW-0790) and Keep on this PC (DW-0784).
pub struct BoardMore {
    pub diagnostics: Diagnostics,
    pub keep: Keep,
}

fn folder(id: &str, name: &str, count: &str, mb: f32, kept: bool, expandable: bool) -> Folder {
    Folder {
        id: id.into(),
        name: name.into(),
        count: count.into(),
        mb,
        kept,
        expandable,
        open: false,
        children: Vec::new(),
    }
}

/// DW-0784 before any choice: Documents and Invoices 2026 are kept; the rest is online only.
/// (The board draws the dialog two choices later: Documents unchecked, Day 1 checked.)
pub fn board_folders() -> Vec<Folder> {
    let mut documents = folder("documents", "Documents", "48 items", 1400.0, true, true);
    documents.children = vec![
        folder("documents/taxes", "Taxes", "20 items", 600.0, true, false),
        folder("documents/letters", "Letters", "28 items", 800.0, true, false),
    ];
    let mut lisbon = folder("lisbon", "Lisbon trip", "212 items", 4200.0, false, true);
    lisbon.children = vec![
        folder("lisbon/day-1", "Day 1", "48 items", 1300.0, false, false),
        folder("lisbon/day-2", "Day 2", "61 items", 1600.0, false, false),
        folder("lisbon/day-3", "Day 3", "103 items", 1300.0, false, false),
    ];
    vec![
        folder("backups", "Backups", "4 devices", 18200.0, false, true),
        documents,
        folder("invoices-2026", "Invoices 2026", "11 items", 27.8, true, false),
        lisbon,
        folder("shoot-selects", "Shoot selects", "64 items", 2400.0, false, true),
    ]
}

/// The size of everything in the board's account, in megabytes (DW-0784's foot: 1.3 GB on
/// this PC and 27.9 GB online only after Apply).
pub const BOARD_TOTAL_MB: f32 = 29_227.8;

impl BoardMore {
    pub fn new() -> Self {
        let activity = |measure, now: &str, samples: &[f32]| Activity {
            measure,
            now: now.into(),
            samples: samples.to_vec(),
        };
        Self {
            diagnostics: Diagnostics {
                // The last ten samples of each measure, as the board draws them.
                activity: vec![
                    activity(
                        Measure::Cpu,
                        "3.1 %",
                        &[0.0, 1.0, 0.0, 2.0, 4.0, 1.0, 0.0, 1.0, 3.0, 1.0],
                    ),
                    activity(
                        Measure::Memory,
                        "212 MB",
                        &[0.0, 1.0, 1.0, 3.0, 5.0, 6.0, 6.0, 7.0, 7.0, 7.0],
                    ),
                    activity(
                        Measure::Upload,
                        "3.2 MB/s",
                        &[0.0, 3.5, 10.0, 15.0, 13.0, 9.0, 12.0, 16.0, 14.5, 14.0],
                    ),
                    activity(
                        Measure::Download,
                        "410 KB/s",
                        &[0.0, 6.0, 12.0, 16.0, 6.0, 0.0, 10.0, 16.0, 12.0, 12.0],
                    ),
                ],
                bundle: Bundle::None,
            },
            keep: Keep::Ready {
                folders: board_folders(),
                total_mb: BOARD_TOTAL_MB,
            },
        }
    }

    /// What an export ends in on the board (DW-0790b). The folder is named by its
    /// environment name, never by a person's own path.
    pub fn board_bundle() -> Bundle {
        Bundle::Saved {
            folder: "%LOCALAPPDATA%\\io.beebeeb.app\\diagnostics\\".into(),
            file: "beebeeb-diagnostics-1793956320.json".into(),
            draft_opened: true,
        }
    }

    /// The fixture's answer to a change: those folders, and nothing else.
    pub fn apply_keep(&mut self, keep: &[String], release: &[String]) {
        if let Keep::Ready { folders, .. } = &mut self.keep {
            let mut tree = super::keep::normalised(std::mem::take(folders));
            for id in release {
                super::keep::set_in(&mut tree, id, false);
            }
            for id in keep {
                super::keep::set_in(&mut tree, id, true);
            }
            *folders = tree;
        }
    }
}
