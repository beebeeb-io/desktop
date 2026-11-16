//! The pure part of the Windows version resource (`build.rs` embeds it; `src/main.rs` includes this file under
//! `cfg(test)` so the choices are unit-tested on every host).
//!
//! The installer's release mode reads `ProductName` and `ProductVersion` out of the built exe and refuses anything
//! but `Beebeeb` and the release version (`packaging/windows/build-installer.ps1`), so what is chosen here is a
//! contract with it.
#![allow(dead_code)]

pub const PRODUCT_NAME: &str = "Beebeeb";
pub const COMPANY_NAME: &str = "Initlabs B.V.";
pub const LEGAL_COPYRIGHT: &str = "\u{a9} Initlabs B.V.";

/// The version the resource states: the release version when `BEEBEEB_RELEASE_VERSION` is set and not empty
/// (verbatim, it is what the installer compares), otherwise the crate's own version (a test build).
pub fn version_string(release: Option<&str>, crate_version: &str) -> String {
    match release.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => v.to_string(),
        None => crate_version.to_string(),
    }
}

/// The four numbers of `FILEVERSION` / `PRODUCTVERSION`: the leading dot-separated numeric parts (a pre-release or
/// build suffix such as `-alpha.1` is cut off), missing parts 0, more than four ignored.
pub fn version_numbers(version: &str) -> [u16; 4] {
    let core = version.split(['-', '+']).next().unwrap_or("");
    let mut out = [0u16; 4];
    for (slot, part) in out.iter_mut().zip(core.split('.')) {
        *slot = part.trim().parse().unwrap_or(0);
    }
    out
}

fn rc_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\"\"")
}

/// The `.rc` source. `icon` is the absolute path of the `.ico`; `binary` the exe's file name.
pub fn render_rc(version: &str, binary: &str, icon: &str) -> String {
    let n = version_numbers(version);
    let nums = format!("{},{},{},{}", n[0], n[1], n[2], n[3]);
    let v = rc_string(version);
    format!(
        "#pragma code_page(65001)\n\
1 ICON \"{icon}\"\n\
1 VERSIONINFO\n\
FILEVERSION {nums}\n\
PRODUCTVERSION {nums}\n\
FILEOS 0x40004\n\
FILETYPE 0x1\n\
BEGIN\n\
  BLOCK \"StringFileInfo\"\n\
  BEGIN\n\
    BLOCK \"040904B0\"\n\
    BEGIN\n\
      VALUE \"CompanyName\", \"{company}\"\n\
      VALUE \"FileDescription\", \"{product}\"\n\
      VALUE \"FileVersion\", \"{v}\"\n\
      VALUE \"LegalCopyright\", \"{copyright}\"\n\
      VALUE \"OriginalFilename\", \"{binary}\"\n\
      VALUE \"ProductName\", \"{product}\"\n\
      VALUE \"ProductVersion\", \"{v}\"\n\
    END\n\
  END\n\
  BLOCK \"VarFileInfo\"\n\
  BEGIN\n\
    VALUE \"Translation\", 0x409, 1200\n\
  END\n\
END\n",
        icon = rc_string(&icon.replace('\\', "/")),
        company = rc_string(COMPANY_NAME),
        product = rc_string(PRODUCT_NAME),
        copyright = rc_string(LEGAL_COPYRIGHT),
        binary = rc_string(binary),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_version_is_stated_verbatim() {
        assert_eq!(version_string(Some("0.9.0"), "0.0.1"), "0.9.0");
    }

    #[test]
    fn an_unset_or_empty_release_version_falls_back_to_the_crate_version() {
        assert_eq!(version_string(None, "0.0.1"), "0.0.1");
        assert_eq!(version_string(Some(""), "0.0.1"), "0.0.1");
        assert_eq!(version_string(Some("  "), "0.0.1"), "0.0.1");
    }

    #[test]
    fn version_numbers_cover_three_four_and_suffixed_versions() {
        assert_eq!(version_numbers("0.9.0"), [0, 9, 0, 0]);
        assert_eq!(version_numbers("0.9.0.3"), [0, 9, 0, 3]);
        assert_eq!(version_numbers("1.2.3.4.5"), [1, 2, 3, 4]);
        assert_eq!(version_numbers("0.9.0-alpha.1"), [0, 9, 0, 0]);
    }

    #[test]
    fn the_rc_carries_every_field_the_installer_and_explorer_read() {
        let rc = render_rc("0.9.0", "beebeeb-drive.exe", "C:\\w\\icon.ico");
        for needle in [
            "FILEVERSION 0,9,0,0",
            "PRODUCTVERSION 0,9,0,0",
            "VALUE \"ProductName\", \"Beebeeb\"",
            "VALUE \"ProductVersion\", \"0.9.0\"",
            "VALUE \"FileVersion\", \"0.9.0\"",
            "VALUE \"FileDescription\", \"Beebeeb\"",
            "VALUE \"CompanyName\", \"Initlabs B.V.\"",
            "VALUE \"LegalCopyright\", \"\u{a9} Initlabs B.V.\"",
            "VALUE \"OriginalFilename\", \"beebeeb-drive.exe\"",
            "1 ICON \"C:/w/icon.ico\"",
        ] {
            assert!(rc.contains(needle), "missing {needle:?} in:\n{rc}");
        }
    }
}
