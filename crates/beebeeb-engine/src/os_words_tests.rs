use super::*;

const SENTENCE: &str = concat!(
    "Open ",
    file_manager!(),
    " on ",
    device_word!(),
    " (",
    primary_key!(),
    "C)"
);

#[cfg(target_os = "windows")]
mod os {
    pub const W: [&str; 5] = ["this PC", "This PC", "File Explorer", "Windows Hello", "Ctrl+"];
    pub const SC: [&str; 2] = ["Ctrl+C", "Ctrl+Shift+N"];
}
#[cfg(target_os = "macos")]
mod os {
    pub const W: [&str; 5] = ["this Mac", "This Mac", "Finder", "Touch ID", "⌘"];
    pub const SC: [&str; 2] = ["⌘C", "⇧⌘N"];
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod os {
    pub const W: [&str; 5] = ["this computer", "This computer", "Files", "", "Ctrl+"];
    pub const SC: [&str; 2] = ["Ctrl+C", "Ctrl+Shift+N"];
}

#[test]
fn words_and_macros_match_this_os() {
    let got = [
        device_word(),
        device_word_cap(),
        file_manager(),
        unlock_name(),
        primary_key(),
    ];
    assert_eq!(got, os::W);
    let mac = [
        device_word!(),
        device_word_cap!(),
        file_manager!(),
        unlock_name!(),
        primary_key!(),
    ];
    assert_eq!(mac, os::W);
}

#[test]
fn concat_const_compiles() {
    assert_eq!(SENTENCE, format!("Open {} on {} ({}C)", os::W[2], os::W[0], os::W[4]));
}

#[test]
fn shortcut_per_os() {
    assert_eq!(shortcut("C"), os::SC[0]);
    assert_eq!(shortcut("Shift+N"), os::SC[1]);
}

#[test]
fn for_this_os_maps_words() {
    assert_eq!(for_this_os("this PC"), os::W[0]);
    assert_eq!(for_this_os("This PC"), os::W[1]);
    assert_eq!(for_this_os("File Explorer"), os::W[2]);
    assert_eq!(for_this_os("Windows Hello"), os::W[3]);
    assert_eq!(for_this_os("Ctrl+C"), os::SC[0]);
    assert_eq!(for_this_os("Ctrl+Shift+N"), os::SC[1]);
    assert_eq!(for_this_os("Nothing to map, Ctrlx."), "Nothing to map, Ctrlx.");
}
