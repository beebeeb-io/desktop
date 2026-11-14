//! The sync-folder rule, on Windows' paths and on Unix's, from any OS (the rule reads the path as text).

use super::*;

fn windows() -> Facts {
    Facts {
        windows: true,
        home: Some(r"C:\Users\Sam".into()),
        system: vec![r"D:\WinNT".into()],
        own: vec![r"C:\Users\Sam\AppData\Local\Beebeeb".into(), r"E:\state\beebeeb".into()],
        other_builds: vec![r"C:\Users\Sam\Beebeeb Test".into()],
        remote_drive: false,
        on_disk: OnDisk::Missing,
        registration: Registration::Nobody,
        own_provider: "Beebeeb",
    }
}

fn unix() -> Facts {
    Facts {
        windows: false,
        home: Some("/home/sam".into()),
        system: Vec::new(),
        own: vec!["/data/beebeeb-state".into()],
        other_builds: vec!["/home/sam/Beebeeb Test".into()],
        remote_drive: false,
        on_disk: OnDisk::Missing,
        registration: Registration::Nobody,
        own_provider: "Beebeeb",
    }
}

fn every(cases: &[&str], facts: &Facts, expected: Option<SyncFolderRefused>) {
    for case in cases {
        assert_eq!(refusal(case, facts), expected, "{case}");
    }
}

#[test]
fn a_folder_of_the_persons_own_may_be_the_sync_folder() {
    every(
        &[
            r"C:\Users\Sam\Beebeeb",
            r"C:\Users\Sam\Documents\Cloud\Beebeeb",
            r"c:/users/sam/beebeeb",
            r"\\?\C:\Users\Sam\Beebeeb",
            r"D:\Beebeeb",
            r"D:\Work\Sync\",
            // Not the apps' data folder: only a name that starts like it.
            r"C:\Users\Sam\AppDataBackup",
            // Not Windows' folder: only a name that starts like it.
            r"C:\Windows Old\Beebeeb",
        ],
        &windows(),
        None,
    );
    every(
        &["/home/sam/Beebeeb", "/mnt/disk/sync", "/home/sam/Documents/b/"],
        &unix(),
        None,
    );
}

#[test]
fn a_path_that_is_not_a_full_one_is_refused() {
    every(
        &[
            "",
            "Beebeeb",
            r"Users\Sam\Beebeeb",
            r"C:Beebeeb",
            r"\Users\Sam",
            r"C:\Users\Sam\..\Other",
            r"C:\Users\Sam\.\Beebeeb",
            r"\\.\PhysicalDrive0",
            r"1:\folder",
            // A name of dots and spaces only is no name on Windows.
            r"C:\Users\Sam\ . ",
        ],
        &windows(),
        Some(SyncFolderRefused::NotAFullPath),
    );
    every(
        &["", "Beebeeb", "home/sam", "/home/sam/../other", "/home/./sam/b"],
        &unix(),
        Some(SyncFolderRefused::NotAFullPath),
    );
}

#[test]
fn a_network_folder_is_refused_by_its_text_or_by_its_drive() {
    every(
        &[
            r"\\server\share",
            r"\\server\share\Beebeeb",
            r"//server/share/Beebeeb",
            r"\\?\UNC\server\share\Beebeeb",
            r"\\?\unc\server\share",
        ],
        &windows(),
        Some(SyncFolderRefused::Network),
    );
    // A mapped drive reads like any drive: only the OS knows it is a share.
    let mapped = Facts {
        remote_drive: true,
        ..windows()
    };
    every(&[r"Z:\Beebeeb", r"Z:\"], &mapped, Some(SyncFolderRefused::Network));
    assert_eq!(
        refusal(r"Z:\Beebeeb", &windows()),
        None,
        "the same text on a local drive"
    );
}

#[test]
fn a_drives_root_and_the_systems_folders_are_refused() {
    every(
        &[
            r"C:\",
            r"D:\",
            r"d:",
            r"C:\Windows",
            r"C:\WINDOWS\System32",
            r"c:\windows.\Temp",
            r"C:\Program Files\Beebeeb",
            r"C:\Program Files (x86)",
            r"C:\ProgramData\Beebeeb",
            r"E:\$Recycle.Bin\x",
            r"C:\System Volume Information",
            r"C:\Users",
            // What this computer names itself, and what holds it.
            r"D:\WinNT",
            r"D:\WinNT\Fonts",
        ],
        &windows(),
        Some(SyncFolderRefused::System),
    );
    every(
        &[
            "/",
            "/usr/local/sync",
            "/etc",
            "/home",
            "/Users",
            "/Library/x",
            "/proc/1",
        ],
        &unix(),
        Some(SyncFolderRefused::System),
    );
}

#[test]
fn the_home_folder_what_holds_it_and_the_apps_data_are_refused() {
    every(
        &[
            r"C:\Users\Sam",
            r"c:\users\SAM\",
            r"C:\Users\Sam\AppData",
            r"C:\Users\Sam\AppData\Roaming\Sync",
        ],
        &windows(),
        Some(SyncFolderRefused::Home),
    );
    every(
        &["/home/sam", "/home/sam/Library/Sync"],
        &unix(),
        Some(SyncFolderRefused::Home),
    );
    // Unix names keep their case: this is another folder.
    assert_eq!(refusal("/home/Sam", &unix()), None);
    // Another person's home is not this rule's (Windows refuses the access itself).
    assert_eq!(refusal(r"C:\Users\Kim\Beebeeb", &windows()), None);
}

#[test]
fn beebeebs_own_data_and_what_holds_it_are_refused() {
    every(
        &[r"E:\state\beebeeb", r"E:\STATE\Beebeeb\cache", r"E:\state"],
        &windows(),
        Some(SyncFolderRefused::OwnData),
    );
    every(
        &["/data/beebeeb-state", "/data/beebeeb-state/x", "/data"],
        &unix(),
        Some(SyncFolderRefused::OwnData),
    );
    // The own data inside the apps' data folder is the home rule's first.
    assert_eq!(
        refusal(r"C:\Users\Sam\AppData\Local\Beebeeb", &windows()),
        Some(SyncFolderRefused::Home)
    );
}

#[test]
fn what_is_not_a_folder_or_cannot_be_read_is_refused() {
    let with = |on_disk| Facts { on_disk, ..windows() };
    let folder = r"C:\Users\Sam\Beebeeb";
    assert_eq!(refusal(folder, &with(OnDisk::Folder)), None);
    assert_eq!(refusal(folder, &with(OnDisk::Missing)), None);
    assert_eq!(
        refusal(folder, &with(OnDisk::NotAFolder)),
        Some(SyncFolderRefused::NotAFolder)
    );
    assert_eq!(
        refusal(folder, &with(OnDisk::Unreadable)),
        Some(SyncFolderRefused::Unreadable)
    );
}

#[test]
fn another_apps_sync_folder_is_refused_and_this_apps_own_is_not() {
    let with = |registration| Facts {
        registration,
        ..windows()
    };
    let folder = r"C:\Users\Sam\Cloud";
    assert_eq!(refusal(folder, &with(Registration::Nobody)), None);
    assert_eq!(refusal(folder, &with(Registration::Provider("beebeeb".into()))), None);
    assert_eq!(
        refusal(folder, &with(Registration::Provider("Beebeeb Test".into()))),
        Some(SyncFolderRefused::AnotherApps)
    );
    assert_eq!(
        refusal(folder, &with(Registration::Provider(String::new()))),
        Some(SyncFolderRefused::AnotherApps)
    );
    assert_eq!(
        refusal(folder, &with(Registration::Unreadable)),
        Some(SyncFolderRefused::Unreadable)
    );
}

#[test]
fn the_other_builds_sync_folder_is_refused_registered_or_not() {
    every(
        &[r"C:\Users\Sam\Beebeeb Test", r"C:\Users\Sam\beebeeb test\Inside"],
        &windows(),
        Some(SyncFolderRefused::AnotherApps),
    );
    every(
        &["/home/sam/Beebeeb Test", "/home/sam/Beebeeb Test/inside"],
        &unix(),
        Some(SyncFolderRefused::AnotherApps),
    );
    // A folder beside it is free.
    assert_eq!(refusal(r"C:\Users\Sam\Beebeeb Test 2", &windows()), None);
}

#[test]
fn the_first_reason_in_the_rules_order_is_the_one_said() {
    // A share that is also "not a folder" and "another app's": the network is said.
    let all = Facts {
        remote_drive: true,
        on_disk: OnDisk::NotAFolder,
        registration: Registration::Provider("Other".into()),
        ..windows()
    };
    assert_eq!(refusal(r"C:\Windows", &all), Some(SyncFolderRefused::Network));
    let local = Facts {
        remote_drive: false,
        ..all
    };
    assert_eq!(refusal(r"C:\Windows", &local), Some(SyncFolderRefused::System));
    assert_eq!(
        refusal(r"C:\Users\Sam\Cloud", &local),
        Some(SyncFolderRefused::NotAFolder)
    );
}

#[test]
fn a_refusal_names_its_kind_and_never_a_path() {
    let kinds: Vec<&str> = [
        SyncFolderRefused::NotAFullPath,
        SyncFolderRefused::Network,
        SyncFolderRefused::System,
        SyncFolderRefused::Home,
        SyncFolderRefused::OwnData,
        SyncFolderRefused::NotAFolder,
        SyncFolderRefused::Unreadable,
        SyncFolderRefused::AnotherApps,
    ]
    .iter()
    .map(|refused| refused.kind())
    .collect();
    let mut unique = kinds.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 8, "{kinds:?}");
}
