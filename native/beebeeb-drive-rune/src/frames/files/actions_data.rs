//! What the action dialogs ask of `FilesSource`, and the board's sample answers.
//!
//! Share (DW-0407): [`ShareOptions`]. Version history (DW-0413): [`Version`]. Preview (DW-0408,
//! DW-0408b): [`PreviewOf`]. Two versions (DW-0429): [`Conflict`], [`Keep`]. The fixture's side
//! is [`More`]: what `FilesFixture` keeps for these beside its own fields. Every trait method
//! that hands these out has a default body for a source that knows nothing (no versions, no
//! preview, no conflict; every action refused), so a build that ships draws no sample data.

use crate::source::DriveItem;

/// What someone with the link may do (DW-0407: "Can view (menu: Can view, Can download)").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinkAccess {
    #[default]
    View,
    Download,
}

impl LinkAccess {
    pub const ALL: [LinkAccess; 2] = [LinkAccess::View, LinkAccess::Download];
    pub fn words(self) -> &'static str {
        match self {
            LinkAccess::View => "Can view",
            LinkAccess::Download => "Can download",
        }
    }
}

/// When the link stops working (DW-0407: "1 day, 7 days, 30 days, Never"; 30 days unless chosen).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinkExpiry {
    Day,
    Week,
    #[default]
    Month,
    Never,
}

impl LinkExpiry {
    pub const ALL: [LinkExpiry; 4] = [LinkExpiry::Day, LinkExpiry::Week, LinkExpiry::Month, LinkExpiry::Never];
    /// As the row says it.
    pub fn words(self) -> &'static str {
        match self {
            LinkExpiry::Day => "In 1 day",
            LinkExpiry::Week => "In 7 days",
            LinkExpiry::Month => "In 30 days",
            LinkExpiry::Never => "Never",
        }
    }
    /// As the menu says it.
    pub fn choice(self) -> &'static str {
        match self {
            LinkExpiry::Day => "1 day",
            LinkExpiry::Week => "7 days",
            LinkExpiry::Month => "30 days",
            LinkExpiry::Never => "Never",
        }
    }
}

/// The link a person asked for. The password is a secret: it is never printed (`Debug` says
/// only whether there is one) and it is wiped when this is dropped.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ShareOptions {
    pub access: LinkAccess,
    pub expires: LinkExpiry,
    /// `Some`: the link asks for this password.
    pub password: Option<String>,
}

impl std::fmt::Debug for ShareOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShareOptions")
            .field("access", &self.access)
            .field("expires", &self.expires)
            .field("password", &self.password.as_ref().map(|_| "(set)"))
            .finish()
    }
}

impl Drop for ShareOptions {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        if let Some(password) = self.password.as_mut() {
            password.zeroize();
        }
    }
}

/// One version of a file (DW-0413).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub id: String,
    /// "Yesterday, 16:42"
    pub when: String,
    /// "1.2 MB"
    pub size: String,
    pub current: bool,
    /// "Anna's iPhone": the device that made the change, where the source knows it.
    pub changed_on: Option<String>,
}

/// What Preview shows of a file (DW-0408). Decrypted content: it lives in the open panel and
/// nowhere else, and goes when the panel closes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewOf {
    /// "Page 1 of 6"
    pub place: String,
    pub page: Page,
}

/// A page of text, as the board draws the first page of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub byline: String,
    /// Heading, paragraph.
    pub sections: Vec<(String, String)>,
}

/// The kind of device a version was changed on: which mark stands beside its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Computer,
    Tablet,
    Phone,
}

/// One of the two versions of a file that changed in two places (DW-0429).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    /// "This PC", "Anna's iPad"
    pub device: String,
    pub kind: Device,
    /// "Today, 09:12"
    pub when: String,
    /// "1.2 MB"
    pub size: String,
}

/// A file that changed in two places at once. Both versions exist until a choice is made, and
/// after it: the one not kept stays in the file's version history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The file's id.
    pub file: String,
    pub name: String,
    /// This PC's version, then the other device's.
    pub here: Side,
    pub there: Side,
}

/// The answer to a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Both: the other device's version is saved next to the file, with the device's name added.
    Both,
    /// This PC's version stays the file.
    Here,
    /// The other device's version becomes the file.
    There,
}

/// "Contract draft (Anna's iPad).pdf": the name Keep both gives the other device's version.
pub fn kept_beside_name(name: &str, device: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({device}).{ext}"),
        _ => format!("{name} ({device})"),
    }
}

/// An item with nothing known about it but what the caller fills in.
pub fn blank_item() -> DriveItem {
    DriveItem {
        id: String::new(),
        parent_id: None,
        name: String::new(),
        is_folder: false,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

/// "3 versions", "1 version".
pub fn versions_words(count: usize) -> String {
    if count == 1 {
        "1 version".to_string()
    } else {
        format!("{count} versions")
    }
}

/// What the fixture keeps for the action dialogs.
#[derive(Default)]
pub struct More {
    /// Versions by file id, once something changed them (a restore, a conflict's answer).
    versions: std::collections::HashMap<String, Vec<Version>>,
    /// The files that wait for a choice.
    pub conflicts: Vec<Conflict>,
}

impl More {
    /// The board's two waiting files (DW-0429 draws the first; the second is a sample of ours:
    /// the board says only "1 of 2").
    pub fn board_conflicts() -> Vec<Conflict> {
        let side = |device: &str, kind, when: &str, size: &str| Side {
            device: device.to_string(),
            kind,
            when: when.to_string(),
            size: size.to_string(),
        };
        vec![
            Conflict {
                file: "contract".to_string(),
                name: "Contract draft.pdf".to_string(),
                here: side("This PC", Device::Computer, "Today, 09:12", "1.2 MB"),
                there: side("Anna's iPad", Device::Tablet, "Today, 08:50", "1.3 MB"),
            },
            Conflict {
                file: "budget".to_string(),
                name: "Budget 2026.xlsx".to_string(),
                here: side("This PC", Device::Computer, "Today, 09:12", "84 KB"),
                there: side("Anna's iPhone", Device::Phone, "Today, 09:05", "86 KB"),
            },
        ]
    }

    /// The file a conflict is about, as a row would show the other device's version of it (the
    /// sample's day is 5 Nov 2026).
    pub fn conflict_item(conflict: &Conflict) -> DriveItem {
        let (figure, unit) = conflict.there.size.split_once(' ').unwrap_or(("0", "KB"));
        let bytes = figure.parse::<f64>().unwrap_or(0.0) * if unit == "MB" { 1_048_576.0 } else { 1024.0 };
        let clock = conflict.there.when.rsplit(' ').next().unwrap_or("00:00");
        DriveItem {
            id: conflict.file.clone(),
            name: conflict.name.clone(),
            size: Some(bytes as i64),
            modified_at: format!("2026-11-05T{clock}:00Z"),
            mime: conflict.name.ends_with(".pdf").then(|| "application/pdf".to_string()),
            pc_status: Some("on_this_pc".to_string()),
            ..blank_item()
        }
    }

    /// The versions of a file: the board's three for Contract draft.pdf, one (the file as it is)
    /// for any other file; none for a folder.
    pub fn versions(&self, item: &DriveItem, row: &super::source::Details, copy: bool) -> Vec<Version> {
        if item.is_folder {
            return Vec::new();
        }
        if let Some(changed) = self.versions.get(&item.id) {
            return changed.clone();
        }
        let v = |id: &str, when: &str, size: &str, current: bool, on: &str| Version {
            id: id.to_string(),
            when: when.to_string(),
            size: size.to_string(),
            current,
            changed_on: Some(on.to_string()),
        };
        if item.id == "contract" && !copy {
            return vec![
                v("contract-v3", "Yesterday, 16:42", "1.2 MB", true, "Anna's iPhone"),
                v("contract-v2", "Yesterday, 11:05", "1.1 MB", false, "This PC"),
                v("contract-v1", "28 Oct, 14:20", "980 KB", false, "This PC"),
            ];
        }
        vec![Version {
            id: format!("{}-v1", item.id),
            when: row.changed.clone(),
            size: row.size.clone(),
            current: true,
            changed_on: row.changed_on.clone(),
        }]
    }

    /// How many versions a file has now, when something changed them in this session.
    pub fn version_count(&self, id: &str) -> Option<usize> {
        self.versions.get(id).map(Vec::len)
    }

    /// Go back to `version`: its content becomes a NEW current version; every version that was
    /// there stays (DW-0413: "Restoring never deletes a version"). `false`: no such version, or
    /// it is the current one.
    pub fn restore(&mut self, item: &DriveItem, mut all: Vec<Version>, version: &str) -> bool {
        let Some(from) = all.iter().find(|v| v.id == version && !v.current).cloned() else {
            return false;
        };
        for v in &mut all {
            v.current = false;
        }
        all.insert(
            0,
            Version {
                id: format!("{}-v{}", item.id, all.len() + 1),
                when: "Just now".to_string(),
                size: from.size,
                current: true,
                changed_on: Some("This PC".to_string()),
            },
        );
        self.versions.insert(item.id.clone(), all);
        true
    }

    /// A conflict was answered: the version that is not the file goes into its version history
    /// (DW-0429: "Either way, the other version stays in version history"). Keep both keeps
    /// the file as it is; its history is untouched, and the other version is a file beside it.
    pub fn answered(&mut self, conflict: &Conflict, keep: Keep, mut all: Vec<Version>) {
        self.conflicts.retain(|c| c.file != conflict.file);
        let (kept, other) = match keep {
            Keep::Both => return,
            Keep::Here => (&conflict.here, &conflict.there),
            Keep::There => (&conflict.there, &conflict.here),
        };
        let next = all.len() + 1;
        for v in &mut all {
            v.current = false;
        }
        let version = |n: usize, side: &Side, current: bool| Version {
            id: format!("{}-v{n}", conflict.file),
            when: side.when.clone(),
            size: side.size.clone(),
            current,
            changed_on: Some(side.device.clone()),
        };
        all.insert(0, version(next, other, false));
        all.insert(0, version(next + 1, kept, true));
        self.versions.insert(conflict.file.clone(), all);
    }

    /// The board's first page of Contract draft.pdf (DW-0408). Files of a kind Windows cannot
    /// preview have none (DW-0408b: design files, archives, unknown types).
    pub fn preview(item: &DriveItem) -> Option<PreviewOf> {
        // The board draws one file's page; the sample has no other file's content.
        if item.is_folder || item.id != "contract" {
            return None;
        }
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        Some(PreviewOf {
            place: "Page 1 of 6".to_string(),
            page: Page {
                title: "Service agreement".to_string(),
                byline: "Northwind Studio and Lisbon Harbour Hotel \u{00B7} Draft 3".to_string(),
                sections: vec![
                    s(
                        "1. Scope of work",
                        "Northwind Studio photographs the hotel's terrace, rooms and harbour view over two days and delivers 60 edited photos and a 90-second film.",
                    ),
                    s(
                        "2. Delivery",
                        "Edited photos within 14 days of the shoot, as full-size JPEG and web-size copies, shared through a Beebeeb link that expires after 30 days.",
                    ),
                    s(
                        "3. Fees",
                        "A fixed fee of \u{20AC}6,400, half on signing and half on delivery. Travel within Lisbon is included.",
                    ),
                ],
            },
        })
    }
}
