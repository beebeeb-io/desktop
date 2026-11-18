//! New request's four menus (DW-0504): where the files go, when it expires, how many files and how much in all.
//! The choices are what core's request options take (`RequestOptions`): a count, a size in bytes, an expiry in
//! days (here in hours), or no limit. The state is on `Fields` (`request`) and is emptied with them.

use crate::frames::shared::source::RequestForm;

/// The four settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Folder,
    Expires,
    Files,
    Size,
}

const GB: u64 = 1 << 30;

/// Where the files go: the account's top level (the one place a request can name without the folder tree).
pub const FOLDERS: [&str; 1] = ["All files"];
/// Expiry: its words, and hours (`None`: never). A new request starts at 30 days.
pub const EXPIRES: [(&str, Option<u32>); 4] = [
    ("In 1 day", Some(24)),
    ("In 7 days", Some(24 * 7)),
    ("In 30 days", Some(24 * 30)),
    ("Never", None),
];
pub const FILES: [(&str, Option<u32>); 6] = [
    ("No limit", None),
    ("1 file", Some(1)),
    ("5 files", Some(5)),
    ("10 files", Some(10)),
    ("25 files", Some(25)),
    ("100 files", Some(100)),
];
pub const SIZES: [(&str, Option<u64>); 5] = [
    ("No limit", None),
    ("100 MB", Some(100 << 20)),
    ("1 GB", Some(GB)),
    ("5 GB", Some(5 * GB)),
    ("10 GB", Some(10 * GB)),
];

/// What the four menus hold: the index chosen in each, and the menu that is open.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub expires: usize,
    pub files: usize,
    pub size: usize,
    pub open: Option<Setting>,
}

impl Default for Pick {
    fn default() -> Self {
        Pick {
            expires: 2,
            files: 0,
            size: 0,
            open: None,
        }
    }
}

impl Pick {
    /// The words of each choice of `setting`, in order.
    pub fn words(setting: Setting) -> Vec<&'static str> {
        match setting {
            Setting::Folder => FOLDERS.to_vec(),
            Setting::Expires => EXPIRES.iter().map(|c| c.0).collect(),
            Setting::Files => FILES.iter().map(|c| c.0).collect(),
            Setting::Size => SIZES.iter().map(|c| c.0).collect(),
        }
    }

    /// The index chosen in `setting`.
    pub fn chosen(&self, setting: Setting) -> usize {
        match setting {
            Setting::Folder => 0,
            Setting::Expires => self.expires,
            Setting::Files => self.files,
            Setting::Size => self.size,
        }
    }

    pub fn toggle(&mut self, setting: Setting) {
        self.open = (self.open != Some(setting)).then_some(setting);
    }

    /// Choose `index` in `setting` (one that is not a choice changes nothing); the menu closes.
    pub fn choose(&mut self, setting: Setting, index: usize) {
        if index < Self::words(setting).len() {
            match setting {
                Setting::Folder => {}
                Setting::Expires => self.expires = index,
                Setting::Files => self.files = index,
                Setting::Size => self.size = index,
            }
        }
        self.open = None;
    }

    /// The form these choices and `title` and `note` make.
    pub fn form(&self, title: &str, note: &str) -> RequestForm {
        RequestForm {
            title: title.trim().to_string(),
            note: note.trim().to_string(),
            target_folder: None,
            max_files: FILES[self.files].1,
            max_total_bytes: SIZES[self.size].1,
            expires_in_hours: EXPIRES[self.expires].1,
        }
    }
}
