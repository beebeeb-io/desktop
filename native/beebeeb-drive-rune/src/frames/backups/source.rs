//! `BackupsSource`: what the Backups frames ask of the engine, and the board's fixture
//! (DW-0737, DW-1411, DW-0739, DW-0741: exactly the board's sample content).

/// What a person can try to do to a folder from the app.
/// (Backups itself only opens the root; the lane that draws the root's row in Files reads the rest.)
#[cfg_attr(not(test), allow(dead_code))]
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
        /// The first backup of this folder ("the first backup takes longest").
        first: bool,
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
    /// What the device is called in a sentence ("iPad": "Photos still on the iPad …").
    pub model: String,
    pub kind: DeviceKind,
    pub photos: u32,
    /// "38 GB" (DW-0739); the list (DW-0737) shows the count only.
    pub size: Option<String>,
    /// "Today", "19 Oct"; `None` for a device that is not signed in (the board shows only that).
    pub last_backup: Option<String>,
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
    /// Which glyph its tile carries.
    pub kind: FileKind,
}

/// The kind of file, for its glyph (neutral ink, as the board draws file types here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Video,
    Document,
    Sheet,
    Image,
}

/// One part of the section a source may or may not answer for (an engine-backed source binds part of it).
#[cfg_attr(not(test), allow(dead_code))] // read by the view lane that draws the unbound parts (PR body)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Root,
    Folders,
    Status,
    Devices,
    NotBackedUp,
    MailBackup,
    BackUpNow,
    AddFolder,
    RemoveFolder,
    Retry,
    DeleteDevice,
}

/// What [`BackupsSource::pump`] found.
#[cfg_attr(not(test), allow(dead_code))] // read by the view lane's poll (PR body)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pumped {
    /// Something the section draws changed: draw again.
    pub changed: bool,
    /// An answer is still out: look again soon.
    pub busy: bool,
    /// A change the person asked for was refused: the window says so (a fixed sentence).
    pub refusal: Option<&'static str>,
}

#[cfg_attr(not(test), allow(dead_code))] // the defaulted methods are for the view lane's poll (PR body)
pub trait BackupsSource {
    /// Whether this source answers for `part` (`false`: the view draws that part's "not built yet" state, never a
    /// value). The fixture answers for everything.
    fn wired(&self, _part: Part) -> bool {
        true
    }
    /// Drain what a source that answers late has received. The window calls it on its poll.
    fn pump(&mut self) -> Pumped {
        Pumped::default()
    }
    /// The section opened: a source that reads late reads again when what it holds is old.
    fn opened(&mut self) {}
    /// Nothing is held yet and a read is out: the section's loading state.
    fn loading(&self) -> bool {
        false
    }
    /// Why the section shows nothing of this PC (another account's data on it, or it could not be read), in a
    /// fixed sentence the view draws. `None`: nothing to say.
    fn refusal(&self) -> Option<&'static str> {
        None
    }
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
    /// Move another device's backup to Trash. It is a risky action: the engine checks the
    /// second-factor `code` first. `false`: the code was not accepted, nothing changed.
    fn delete_device_backup(&mut self, id: &str, code: &str) -> bool;
}

/// What stands in for the engine in a build that ships before the engine lane wires Backups:
/// nothing to show (the view draws the shell's "not built yet" frame, never sample values).
pub fn first(dev_build: bool) -> Option<Box<dyn BackupsSource>> {
    dev_build.then(|| Box::new(FixtureSource::board()) as Box<dyn BackupsSource>)
}

pub struct FixtureSource {
    status: Status,
    folders: Vec<Folder>,
    devices: Vec<Device>,
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
        let failure = |id: &str, name: &str, folder: &str, why: &str, excluded, kind| NotBackedUp {
            kind,
            id: id.into(),
            name: name.into(),
            folder: folder.into(),
            why: why.into(),
            excluded,
        };
        let device = |id: &str, name: &str, kind, photos, size: Option<&str>, last: Option<&str>, signed_in| Device {
            id: id.into(),
            name: name.into(),
            model: if kind == DeviceKind::Tablet { "iPad" } else { "iPhone" }.into(),
            kind,
            photos,
            size: size.map(str::to_string),
            last_backup: last.map(str::to_string),
            signed_in,
        };
        let devices = vec![
            device(
                "iphone",
                "Anna's iPhone",
                DeviceKind::Phone,
                12_418,
                None,
                Some("Today"),
                true,
            ),
            device(
                "ipad",
                "Anna's iPad",
                DeviceKind::Tablet,
                4_120,
                Some("38 GB"),
                Some("19 Oct"),
                true,
            ),
            device("old-iphone", "Old iPhone", DeviceKind::Phone, 8_204, None, None, false),
        ];
        FixtureSource {
            devices,
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
                    FileKind::Video,
                ),
                failure(
                    "f-invoice",
                    "Invoice scan.pdf",
                    "Desktop",
                    "this file is damaged",
                    false,
                    FileKind::Document,
                ),
                failure(
                    "f-budget",
                    "Budget 2025.xlsx",
                    "Documents",
                    "it kept changing while it was backing up",
                    false,
                    FileKind::Sheet,
                ),
                failure(
                    "x-site",
                    "Site walkthrough.mkv",
                    "Desktop",
                    "files larger than 4 GB are not backed up from this PC",
                    true,
                    FileKind::Image,
                ),
            ],
        }
    }

    /// DW-1411: the first backup of Documents is running. The board draws only Documents'
    /// switch on here, nothing that could not back up, and the rows' words as in DW-0737.
    pub fn first_backup() -> FixtureSource {
        let mut source = FixtureSource {
            status: Status::Running {
                folder: "Documents".into(),
                done: 612,
                total: 1_820,
                minutes_left: 20,
                first: true,
            },
            failures: Vec::new(),
            ..Self::board()
        };
        for folder in &mut source.folders {
            if !folder.chosen_by_you {
                folder.on = folder.id == "documents";
            }
        }
        source
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
        self.devices.clone()
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

    fn delete_device_backup(&mut self, id: &str, code: &str) -> bool {
        // The fixture's authenticator: six digits pass, except six zeros (so a refusal can be seen).
        let accepted = code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()) && code != "000000";
        if accepted {
            self.devices.retain(|d| d.id != id);
        }
        accepted
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
        assert!(source.mail_backup_on());
        let mut running = FixtureSource::board();
        running.back_up_now();
        assert!(matches!(running.status(), Status::Running { .. }));
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
