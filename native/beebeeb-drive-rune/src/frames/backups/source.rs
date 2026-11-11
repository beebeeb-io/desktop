//! `BackupsSource`: what the Backups frames ask of the engine, and the board's fixture
//! (DW-0737, DW-1411, DW-0739, DW-0741: exactly the board's sample content).

/// What a person can try to do to a folder from the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootAction {
    Open,
    Rename,
    Move,
    Delete,
    Share,
}

/// The Backups root in Drive. It is protected exactly as on the web: it opens, and it cannot
/// be renamed, moved, deleted or shared. The engine sets `is_protected`; a frame draws every
/// such control absent or disabled from [`BackupsRoot::allows`] and never decides by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupsRoot {
    pub id: String,
    pub is_protected: bool,
    /// This PC's folder inside it ("Anna-Laptop": the PC's name).
    pub device_folder: String,
}

impl BackupsRoot {
    pub fn allows(&self, action: RootAction) -> bool {
        !self.is_protected || action == RootAction::Open
    }
}

/// The status card (DW-0737, DW-1411).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// "All backed up": the last backup's time and how much came from this PC.
    AllBackedUp { last: String, from_this_pc: String },
    /// "Backing up Documents": 612 of 1,820 files, about 20 min left.
    Running {
        folder: String,
        done: u32,
        total: u32,
        minutes_left: u32,
    },
}

/// One folder of this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub items: u32,
    pub size: String,
    /// One of Windows' six known folders (a switch), or one the person chose (Remove).
    pub chosen_by_you: bool,
    pub on: bool,
    /// "Today, 09:30"; `None` while it is off.
    pub last_backup: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Phone,
    Tablet,
}

/// Another device's backup, seen from this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub photos: u32,
    /// "38 GB" (DW-0739); the list (DW-0737) shows the count only.
    pub size: Option<String>,
    /// "Today", "19 Oct".
    pub last_backup: String,
    pub signed_in: bool,
}

/// An item Beebeeb stopped trying, or one it never backs up (DW-0741). Never a raw error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotBackedUp {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub why: String,
    /// By design (a file larger than 4 GB): listed apart, without Try again.
    pub excluded: bool,
}

pub trait BackupsSource {
    fn root(&self) -> BackupsRoot;
    fn status(&self) -> Status;
    fn folders(&self) -> Vec<Folder>;
    fn devices(&self) -> Vec<Device>;
    fn not_backed_up(&self) -> Vec<NotBackedUp>;
    fn mail_backup_on(&self) -> bool;
    fn back_up_now(&mut self);
    /// Start or stop backing up one of the six known folders.
    fn set_folder(&mut self, id: &str, on: bool);
    /// Add a chosen folder. `false`: Beebeeb's own folder, or one inside it, cannot be chosen.
    fn add_folder(&mut self, path: &str) -> bool;
    /// Stop backing up a chosen folder; its copy stays (read-only) or goes to Trash.
    fn remove_folder(&mut self, id: &str, move_copy_to_trash: bool);
    fn retry(&mut self, id: &str);
    fn retry_all(&mut self);
}

pub struct FixtureSource {
    status: Status,
    folders: Vec<Folder>,
    failures: Vec<NotBackedUp>,
}

impl FixtureSource {
    /// DW-0737.
    pub fn board() -> FixtureSource {
        let folder = |name: &str, items, size: &str, on: bool, chosen: bool, last: Option<&str>| Folder {
            id: name.to_lowercase(),
            name: name.into(),
            items,
            size: size.into(),
            chosen_by_you: chosen,
            on,
            last_backup: last.map(str::to_string),
        };
        let today = Some("Today, 09:30");
        let failure = |id: &str, name: &str, folder: &str, why: &str, excluded| NotBackedUp {
            id: id.into(),
            name: name.into(),
            folder: folder.into(),
            why: why.into(),
            excluded,
        };
        FixtureSource {
            status: Status::AllBackedUp {
                last: "09:30".into(),
                from_this_pc: "27.8 GB".into(),
            },
            folders: vec![
                folder("Desktop", 214, "3.1 GB", true, false, today),
                folder("Documents", 1_820, "5.9 GB", true, false, today),
                folder("Pictures", 3_412, "18.2 GB", true, false, today),
                folder("Music", 1_106, "7.4 GB", false, false, None),
                folder("Videos", 38, "21.6 GB", false, false, None),
                folder("Downloads", 642, "4.8 GB", false, false, None),
                folder("Projects", 46, "640 MB", true, true, Some("Today, 09:28")),
            ],
            failures: vec![
                failure(
                    "f-render",
                    "Render final.mov",
                    "Documents",
                    "the file is open in another app",
                    false,
                ),
                failure(
                    "f-invoice",
                    "Invoice scan.pdf",
                    "Desktop",
                    "this file is damaged",
                    false,
                ),
                failure(
                    "f-budget",
                    "Budget 2025.xlsx",
                    "Documents",
                    "it kept changing while it was backing up",
                    false,
                ),
                failure(
                    "x-site",
                    "Site walkthrough.mkv",
                    "Desktop",
                    "files larger than 4 GB are not backed up from this PC",
                    true,
                ),
            ],
        }
    }

    /// DW-1411: the first backup of Documents is running.
    pub fn first_backup() -> FixtureSource {
        FixtureSource {
            status: Status::Running {
                folder: "Documents".into(),
                done: 612,
                total: 1_820,
                minutes_left: 20,
            },
            ..Self::board()
        }
    }
}

impl BackupsSource for FixtureSource {
    fn root(&self) -> BackupsRoot {
        BackupsRoot {
            id: "backups".into(),
            is_protected: true,
            device_folder: "Anna-Laptop".into(),
        }
    }

    fn status(&self) -> Status {
        self.status.clone()
    }

    fn folders(&self) -> Vec<Folder> {
        self.folders.clone()
    }

    fn devices(&self) -> Vec<Device> {
        let device = |id: &str, name: &str, kind, photos, size: Option<&str>, last: &str, signed_in| Device {
            id: id.into(),
            name: name.into(),
            kind,
            photos,
            size: size.map(str::to_string),
            last_backup: last.into(),
            signed_in,
        };
        vec![
            device(
                "iphone",
                "Anna's iPhone",
                DeviceKind::Phone,
                12_418,
                None,
                "Today",
                true,
            ),
            device(
                "ipad",
                "Anna's iPad",
                DeviceKind::Tablet,
                4_120,
                Some("38 GB"),
                "19 Oct",
                true,
            ),
            device("old-iphone", "Old iPhone", DeviceKind::Phone, 8_204, None, "", false),
        ]
    }

    fn not_backed_up(&self) -> Vec<NotBackedUp> {
        self.failures.clone()
    }

    fn mail_backup_on(&self) -> bool {
        true
    }

    fn back_up_now(&mut self) {
        self.status = FixtureSource::first_backup().status;
    }

    fn set_folder(&mut self, id: &str, on: bool) {
        if let Some(folder) = self.folders.iter_mut().find(|f| f.id == id && !f.chosen_by_you) {
            folder.on = on;
        }
    }

    fn add_folder(&mut self, path: &str) -> bool {
        // Beebeeb's own folder in File Explorer already is Beebeeb (DW-0738's rule).
        !path.split(['\\', '/']).any(|part| part.eq_ignore_ascii_case("Beebeeb"))
    }

    fn remove_folder(&mut self, id: &str, _move_copy_to_trash: bool) {
        self.folders.retain(|f| !(f.id == id && f.chosen_by_you));
    }

    fn retry(&mut self, id: &str) {
        self.failures.retain(|f| f.id != id || f.excluded);
    }

    fn retry_all(&mut self) {
        self.failures.retain(|f| f.excluded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_protected_root_opens_and_refuses_rename_move_delete_and_share() {
        let root = FixtureSource::board().root();
        assert!(root.is_protected);
        assert!(root.allows(RootAction::Open));
        for action in [
            RootAction::Rename,
            RootAction::Move,
            RootAction::Delete,
            RootAction::Share,
        ] {
            assert!(!root.allows(action), "{action:?} on the protected Backups root");
        }
        // The rule follows the engine's flag, never the folder's name.
        let plain = BackupsRoot {
            is_protected: false,
            ..root
        };
        assert!(plain.allows(RootAction::Rename));
    }

    #[test]
    fn the_board_fixture_holds_7_folders_3_devices_3_failures_and_1_exclusion() {
        let source = FixtureSource::board();
        assert_eq!(source.folders().len(), 7);
        assert_eq!(source.folders().iter().filter(|f| f.on && !f.chosen_by_you).count(), 3);
        assert_eq!(source.devices().len(), 3);
        assert_eq!(source.not_backed_up().iter().filter(|f| !f.excluded).count(), 3);
        assert_eq!(source.not_backed_up().iter().filter(|f| f.excluded).count(), 1);
        assert!(matches!(
            FixtureSource::first_backup().status(),
            Status::Running {
                done: 612,
                total: 1_820,
                ..
            }
        ));
    }

    #[test]
    fn try_again_never_retries_an_exclusion_and_a_switch_never_removes_a_chosen_folder() {
        let mut source = FixtureSource::board();
        source.retry("x-site");
        assert_eq!(source.not_backed_up().len(), 4, "an exclusion is not a failure");
        source.retry_all();
        assert_eq!(source.not_backed_up().len(), 1);
        source.set_folder("music", true);
        assert!(source.folders().iter().any(|f| f.id == "music" && f.on));
        source.set_folder("projects", false);
        assert!(source.folders().iter().any(|f| f.id == "projects" && f.on));
        source.remove_folder("projects", false);
        assert_eq!(source.folders().len(), 6);
        assert!(!source.add_folder(r"C:\Users\Anna\Beebeeb\Work"));
        assert!(source.add_folder(r"C:\Users\Anna\Projects"));
    }
}
