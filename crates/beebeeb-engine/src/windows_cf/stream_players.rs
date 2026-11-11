//! LD-0105 D2, ruled "Yes, per app" (Guus, 2026-11-05 22:55 CET): which open may stream.
//!
//! Cloud Files has no "stream or whole file" handshake. An ordinary open makes ONE fetch for the
//! whole file and waits for it (P1 spike (a)); only when the provider declines that open-time fetch
//! does the open return at once and the reader's reads become range fetches (spike (x2)). Declining
//! breaks one-call readers (`ReadAllBytes`, P1b), so it is done only for a known player opening a
//! media file. Everyone else (copy tools, editors, backup, unknown apps, our own "Always keep on this
//! device") gets the whole file. The decline is a non-success status with 0 bytes: no session, no
//! identity, no poison, no request (spec §6.4); a wrong decision costs availability, never integrity.
//!
//! **This module is the ONE place the player list and the media types live** (spec §6.4). It is pure
//! (no Windows call) and compiled on every platform, so the rules are tested everywhere; the Windows
//! fetch callback feeds it the requester's `CF_PROCESS_INFO`.
//!
//! **Switched on** ([`STREAMING_ENABLED`]) with the streaming callback (plan W2): a declined player's
//! reads become range demands on the file's one range session (`windows_cf::streaming`).

/// OFF for 0.8.22 (lead ruling 2026-11-06, desktop #225: D2-2 failed natively, VLC's open was served
/// whole and never declined; per Amendment 5 the per-app decline ships switched off). The code stays.
pub const STREAMING_ENABLED: bool = false;

/// How a player is recognised. Never by image name alone: a renamed `vlc.exe` in a temp folder must
/// get the whole file (D2-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerMatch {
    /// A Store app, by package family name (`Name_PublisherId`), from the requester's package name.
    PackageFamily(&'static str),
    /// A Win32 player, by its full image path relative to `%ProgramFiles%` or `%ProgramFiles(x86)%`
    /// (folders only an administrator can write).
    UnderProgramFiles(&'static str),
}

/// One allowlisted player: the name is for the record only.
#[derive(Debug, Clone, Copy)]
pub struct Player {
    pub name: &'static str,
    pub matches: PlayerMatch,
}

/// The allowlist. Each entry was checked against an installed copy on the Legion (Windows 11,
/// 2026-11-05, read-only `Test-Path` / `Get-AppxPackage`); spec §6.4's unverified candidates (Films &
/// TV, MPC-HC, MPC-BE, mpv, PotPlayer) stay out until someone verifies one on a real install.
pub const PLAYERS: &[Player] = &[
    // Media Player (Windows 11): `Microsoft.ZuneMusic_11.2609.11.0_x64__8wekyb3d8bbwe`; the P0 and P1b requester.
    Player {
        name: "Media Player",
        matches: PlayerMatch::PackageFamily("Microsoft.ZuneMusic_8wekyb3d8bbwe"),
    },
    Player {
        name: "Windows Media Player (legacy)",
        matches: PlayerMatch::UnderProgramFiles(r"Windows Media Player\wmplayer.exe"),
    },
    Player {
        name: "VLC",
        matches: PlayerMatch::UnderProgramFiles(r"VideoLAN\VLC\vlc.exe"),
    },
];

/// Media types by extension (spec §6.4): video, then audio.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mpg", "mpeg", "ts", //
    "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "wma",
];

/// Is a file with this decrypted `name` (or this MIME type from its metadata) a media file?
pub fn is_media(name: &str, mime: Option<&str>) -> bool {
    let by_name = name
        .rsplit_once('.')
        .map(|(stem, ext)| !stem.is_empty() && MEDIA_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    let by_mime = mime.is_some_and(|m| {
        let m = m.trim().to_ascii_lowercase();
        m.starts_with("video/") || m.starts_with("audio/")
    });
    by_name || by_mime
}

/// The package family name (`Name_PublisherId`) of a package full name
/// (`Name_Version_Architecture_ResourceId_PublisherId`); a family name is returned as it is.
pub fn package_family(package: &str) -> Option<String> {
    let parts: Vec<&str> = package.split('_').collect();
    match parts.as_slice() {
        [name, publisher] if !name.is_empty() && !publisher.is_empty() => Some(format!("{name}_{publisher}")),
        [name, _version, _arch, _resource, publisher] if !name.is_empty() && !publisher.is_empty() => {
            Some(format!("{name}_{publisher}"))
        }
        _ => None,
    }
}

/// A requester's image path as a lower-case DOS path (`c:\program files\…`), or `None` when it cannot
/// be read as one. Accepts `C:\…`, `\\?\C:\…`, and the NT form `\Device\HarddiskVolumeN\…` through
/// `devices` (`(\Device\HarddiskVolumeN, C:)` pairs from `QueryDosDevice`). A path with `..`, `.` or an
/// empty component is refused rather than resolved.
pub fn dos_image_path(image: &str, devices: &[(String, String)]) -> Option<String> {
    let image = image.replace('/', "\\");
    let rest = image
        .strip_prefix(r"\\?\")
        .or_else(|| image.strip_prefix(r"\??\"))
        .unwrap_or(&image);
    let dos = if rest.len() >= 3
        && rest.as_bytes()[1] == b':'
        && rest.as_bytes()[2] == b'\\'
        && rest.as_bytes()[0].is_ascii_alphabetic()
    {
        rest.to_string()
    } else {
        let lower = rest.to_ascii_lowercase();
        let (device, drive) = devices.iter().find(|(device, _)| {
            let device = device.to_ascii_lowercase();
            lower.starts_with(&device) && lower.as_bytes().get(device.len()) == Some(&b'\\')
        })?;
        format!("{}{}", drive.trim_end_matches('\\'), &rest[device.len()..])
    };
    let components: Vec<&str> = dos[3..].split('\\').collect();
    if components.iter().any(|c| c.is_empty() || *c == "." || *c == "..") {
        return None;
    }
    Some(dos.to_ascii_lowercase())
}

/// Who asked for the bytes, as the fetch callback read it from `CF_CALLBACK_INFO`.
#[derive(Debug, Clone, Default)]
pub struct Requester<'a> {
    /// This process (our own fetch: "Always keep on this device", a stamp, an upload's read).
    pub ours: bool,
    /// `CF_CALLBACK_FETCH_DATA_FLAG_EXPLICIT_HYDRATION` (0x2): a `CfHydratePlaceholder`.
    pub explicit_hydration: bool,
    /// `CF_PROCESS_INFO.PackageName`, when the requester is a packaged app.
    pub package: Option<&'a str>,
    /// The image path as [`dos_image_path`] returned it.
    pub image: Option<&'a str>,
}

/// Is `requester` an allowlisted player? `program_files` are the lower-case DOS `%ProgramFiles%` and
/// `%ProgramFiles(x86)%` folders.
pub fn is_listed_player(requester: &Requester<'_>, program_files: &[String]) -> bool {
    let family = requester.package.and_then(package_family);
    PLAYERS.iter().any(|player| match player.matches {
        PlayerMatch::PackageFamily(want) => family.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(want)),
        PlayerMatch::UnderProgramFiles(rel) => requester.image.is_some_and(|image| {
            program_files.iter().any(|root| {
                let full = format!("{}\\{}", root.trim_end_matches('\\'), rel).to_ascii_lowercase();
                image == full
            })
        }),
    })
}

/// The file being fetched.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub name: &'a str,
    pub mime: Option<&'a str>,
    /// A file-request upload or shared-with-me content: always the whole file (the lead's
    /// conservative default, spec §6.4; Guus may lift it).
    pub excluded_origin: bool,
}

/// The whole rule (spec §6.4): decline this fetch? Only when streaming is `enabled`, the fetch is the
/// first whole-remainder fetch after this requester's open (`first_after_open`, from [`OpenFetches`]),
/// the requester is a listed player and not us nor an explicit hydration, the file is a media file of
/// no excluded origin, and (desktop #225 review P1) the file can actually stream: `can_stream`,
/// asked last and only then ([`crate::cf_range::file_streams`]: a recorded version to pin and the
/// direct range route). A legacy or API-route file is never declined: declined, each of its later
/// reads would download the whole file again.
pub fn decline_open_fetch(
    enabled: bool,
    requester: &Requester<'_>,
    program_files: &[String],
    target: Target<'_>,
    first_after_open: bool,
    can_stream: impl FnOnce() -> bool,
) -> bool {
    enabled
        && first_after_open
        && !requester.ours
        && !requester.explicit_hydration
        && !target.excluded_origin
        && is_media(target.name, target.mime)
        && is_listed_player(requester, program_files)
        && can_stream()
}

/// Is this fetch the open-time whole-file kind: it runs to the end of the file and offers no
/// optional range (P1b: `required = [first missing byte, EoF)`, `optional = 0`)?
pub fn is_whole_remainder(required_offset: i64, required_length: i64, optional_length: i64, file_size: i64) -> bool {
    optional_length == 0
        && required_offset >= 0
        && required_length > 0
        && required_offset.saturating_add(required_length) >= file_size
}

/// Spec §6.4 rule 3: only the FIRST whole-remainder fetch after a requester's open completion may be
/// declined; a later whole-size read from the same open (`ReadAllBytes`' single `ReadFile`) is served.
#[derive(Debug, Default)]
pub struct OpenFetches {
    /// `(requester pid, file id)` opened and not yet asked for its whole remainder.
    armed: std::collections::HashSet<(u32, String)>,
}

impl OpenFetches {
    /// `NOTIFY_FILE_OPEN_COMPLETION` from `pid` on `file_id`.
    pub fn opened(&mut self, pid: u32, file_id: &str) {
        // Bounded (desktop #224 follow-up): forgetting an armed open only means that open gets the
        // whole file, never a wrong decline.
        if self.armed.len() >= Self::MAX_ARMED {
            self.armed.clear();
        }
        self.armed.insert((pid, file_id.to_string()));
    }

    const MAX_ARMED: usize = 4096;

    /// A whole-remainder fetch from `pid` on `file_id`: `true` only for the first one after its open.
    pub fn first_after_open(&mut self, pid: u32, file_id: &str) -> bool {
        self.armed.remove(&(pid, file_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Vec<String> {
        vec![r"c:\program files".into(), r"c:\program files (x86)".into()]
    }

    const MEDIA_PLAYER_PKG: &str = "Microsoft.ZuneMusic_11.2609.11.0_x64__8wekyb3d8bbwe";

    fn player() -> Requester<'static> {
        Requester {
            package: Some(MEDIA_PLAYER_PKG),
            image: Some(
                r"c:\program files\windowsapps\microsoft.zunemusic_11.2609.11.0_x64__8wekyb3d8bbwe\microsoft.media.player.exe",
            ),
            ..Requester::default()
        }
    }

    fn movie() -> Target<'static> {
        Target {
            name: "holiday.mp4",
            mime: Some("video/mp4"),
            excluded_origin: false,
        }
    }

    /// Lead ruling 2026-11-06 (desktop #225): W3 ships off for 0.8.22. Red on the mutant "the switch
    /// is on".
    #[test]
    fn the_switch_is_off_for_0822() {
        assert!(!STREAMING_ENABLED, "W3 ships switched off");
        assert!(
            !decline_open_fetch(STREAMING_ENABLED, &player(), &roots(), movie(), true, || true),
            "the shipped switch declines nothing, not even a listed player's open of a media file"
        );
    }

    #[test]
    fn d2_2_a_listed_player_opening_a_media_file_is_declined_when_streaming_is_on() {
        assert!(decline_open_fetch(true, &player(), &roots(), movie(), true, || true));
        let vlc = Requester {
            image: Some(r"c:\program files\videolan\vlc\vlc.exe"),
            ..Requester::default()
        };
        assert!(decline_open_fetch(true, &vlc, &roots(), movie(), true, || true));
        let wmp = Requester {
            image: Some(r"c:\program files (x86)\windows media player\wmplayer.exe"),
            ..Requester::default()
        };
        assert!(decline_open_fetch(true, &wmp, &roots(), movie(), true, || true));
    }

    #[test]
    fn d2_1_and_d2_3_non_players_and_unknown_apps_get_the_whole_file() {
        for image in [
            r"c:\windows\system32\windowspowershell\v1.0\powershell.exe", // Get-FileHash, ReadAllBytes
            r"c:\windows\system32\robocopy.exe",
            r"c:\windows\system32\cmd.exe",
            r"c:\windows\system32\sihost.exe", // P1b: the second whole-file fetch after Media Player's open
            r"c:\program files\some vendor\unknown.exe",
        ] {
            let other = Requester {
                image: Some(image),
                ..Requester::default()
            };
            assert!(
                !decline_open_fetch(true, &other, &roots(), movie(), true, || true),
                "{image}"
            );
        }
        assert!(
            !decline_open_fetch(true, &Requester::default(), &roots(), movie(), true, || true),
            "no process info at all"
        );
    }

    #[test]
    fn d2_4_a_renamed_player_outside_program_files_gets_the_whole_file() {
        for image in [
            r"c:\users\guus\appdata\local\temp\vlc.exe",
            r"c:\users\guus\scoop\apps\vlc\current\vlc.exe",
            r"c:\program files\videolan\vlc\..\..\..\users\guus\vlc.exe",
            r"c:\program files\videolan\vlc\vlc.exe.bak",
        ] {
            let normalized = dos_image_path(image, &[]);
            let renamed = Requester {
                image: normalized.as_deref(),
                ..Requester::default()
            };
            assert!(
                !decline_open_fetch(true, &renamed, &roots(), movie(), true, || true),
                "{image}"
            );
        }
        // A package that only looks like Media Player's (another publisher id).
        let lookalike = Requester {
            package: Some("Microsoft.ZuneMusic_11.2609.11.0_x64__evilpublisher00"),
            ..Requester::default()
        };
        assert!(!decline_open_fetch(true, &lookalike, &roots(), movie(), true, || true));
    }

    #[test]
    fn d2_5_our_own_process_and_an_explicit_hydration_are_never_declined() {
        let ours = Requester { ours: true, ..player() };
        assert!(!decline_open_fetch(true, &ours, &roots(), movie(), true, || true));
        let pin = Requester {
            explicit_hydration: true,
            ..player()
        };
        assert!(
            !decline_open_fetch(true, &pin, &roots(), movie(), true, || true),
            "Always keep on this device hydrates"
        );
    }

    #[test]
    fn d2_6_a_player_opening_a_non_media_file_gets_the_whole_file() {
        for (name, mime) in [
            ("report.pdf", Some("application/pdf")),
            ("notes.txt", None),
            ("mp4", None),
            (".mp4", None),
        ] {
            let target = Target {
                name,
                mime,
                excluded_origin: false,
            };
            assert!(
                !decline_open_fetch(true, &player(), &roots(), target, true, || true),
                "{name}"
            );
        }
        assert!(is_media("CLIP.MKV", None), "extensions are case-blind");
        assert!(
            is_media("recording", Some("audio/ogg")),
            "the metadata's MIME type counts"
        );
    }

    #[test]
    fn d2_7_only_the_first_whole_remainder_fetch_after_an_open_is_declined() {
        let mut opens = OpenFetches::default();
        opens.opened(42, "f");
        assert!(opens.first_after_open(42, "f"), "the open-time fetch");
        assert!(
            !opens.first_after_open(42, "f"),
            "a later whole-size read of the same open is served"
        );
        assert!(!opens.first_after_open(7, "f"), "another process never opened it");
        opens.opened(42, "f");
        assert!(opens.first_after_open(42, "f"), "a new open arms it again");
        assert!(is_whole_remainder(0, 64_016_202, 0, 64_016_202), "P1b, cloud-only");
        assert!(
            is_whole_remainder(8_388_608, 55_627_594, 0, 64_016_202),
            "P1b, first 8 MiB on disk"
        );
        assert!(
            !is_whole_remainder(0, 65_536, 64_016_202 - 65_536, 64_016_202),
            "a range read with read-ahead"
        );
    }

    #[test]
    fn d2_8_file_request_and_shared_content_get_the_whole_file_even_in_a_player() {
        let shared = Target {
            excluded_origin: true,
            ..movie()
        };
        assert!(!decline_open_fetch(true, &player(), &roots(), shared, true, || true));
    }

    /// Desktop #225 review P1: a listed player opening a legacy file (no recorded version to pin) or an
    /// API-route file (transport forced to the API, or the server offers no direct download) is NOT
    /// declined, so its open-time fetch is served whole (one fetch, S2) instead of each read
    /// downloading the whole file again. Red on the mutants "the decline ignores `can_stream`" and
    /// "`file_streams` ignores the recorded version" / "... the route".
    #[test]
    fn ld0105_p1_a_listed_player_opening_a_legacy_or_api_route_file_is_not_declined() {
        use crate::cf_range::file_streams;
        let legacy = || file_streams(false, false, true);
        let forced_api = || file_streams(true, true, true);
        let not_offered = || file_streams(true, false, false);
        for (what, can_stream) in [
            ("legacy", legacy as fn() -> bool),
            ("forced api", forced_api),
            ("no direct route", not_offered),
        ] {
            assert!(!can_stream(), "{what}");
            assert!(
                !decline_open_fetch(true, &player(), &roots(), movie(), true, can_stream),
                "{what}: never declined"
            );
        }
        assert!(
            file_streams(true, false, true),
            "a pinned version on the direct route streams"
        );
        assert!(decline_open_fetch(true, &player(), &roots(), movie(), true, || {
            file_streams(true, false, true)
        }));
        let mut asked = false;
        assert!(!decline_open_fetch(
            true,
            &Requester::default(),
            &roots(),
            movie(),
            true,
            || {
                asked = true;
                true
            }
        ));
        assert!(
            !asked,
            "asked only when everything else says decline (it may make a discovery call)"
        );
    }

    #[test]
    fn image_paths_are_read_as_dos_paths_and_package_names_as_families() {
        let devices = vec![(r"\Device\HarddiskVolume3".to_string(), "C:".to_string())];
        assert_eq!(
            dos_image_path(r"\Device\HarddiskVolume3\Program Files\VideoLAN\VLC\vlc.exe", &devices).as_deref(),
            Some(r"c:\program files\videolan\vlc\vlc.exe")
        );
        assert_eq!(
            dos_image_path(r"\\?\C:\Program Files\VideoLAN\VLC\vlc.exe", &[]).as_deref(),
            Some(r"c:\program files\videolan\vlc\vlc.exe")
        );
        assert_eq!(
            dos_image_path(r"\Device\HarddiskVolume33\Program Files\VideoLAN\VLC\vlc.exe", &devices),
            None,
            "volume 33 is not volume 3"
        );
        assert_eq!(
            dos_image_path(r"\Device\HarddiskVolume9\x.exe", &devices),
            None,
            "an unknown volume"
        );
        assert_eq!(dos_image_path(r"C:\Program Files\.\VideoLAN\VLC\vlc.exe", &[]), None);
        assert_eq!(
            package_family(MEDIA_PLAYER_PKG).as_deref(),
            Some("Microsoft.ZuneMusic_8wekyb3d8bbwe")
        );
        assert_eq!(
            package_family("Microsoft.ZuneMusic_8wekyb3d8bbwe").as_deref(),
            Some("Microsoft.ZuneMusic_8wekyb3d8bbwe")
        );
        assert_eq!(package_family("nonsense"), None);
    }
}
