//! Thumbnails made on this PC (095) are sent once: the owner's own photo, its bytes here, its thumbnail made from
//! them, sealed with core's thumbnail recipe (`encrypt_thumbnail_for`, the one the upload path and the phone use) and
//! put on the server like a preview is, so web and phone show it and no device makes it again.
//!
//! - [`not_now`]: the account's preview setting must be on, and no metered-network or battery-saver hold stands
//!   ([`crate::power`]); otherwise nothing is made or sent.
//! - [`seal_made`]: pixels to a sealed thumbnail (WebP, core's medium ladder), owner-class ids only. NOT the preview
//!   seal ([`super::seal`]): web and phone open a thumbnail with `decrypt_thumbnail`.
//! - [`send_all`]: one send per photo version, ever ([`Sent`]); a stop reason ends the run.
//!
//! Must not: write plaintext anywhere; log a name, a path, an id, a key or a byte value.

use std::collections::HashSet;

use beebeeb_core::chunk_stream::ExpectedChunkFormat;
use beebeeb_core::item_id::{ItemClass, item_class};
use beebeeb_core::kdf::{MasterKey, derive_file_key};
use beebeeb_core::thumbnail::{ThumbnailConfig, generate_thumbnail};
use zeroize::Zeroizing;

use super::PreviewSetting;
use super::runner::Stopped;
use crate::power::PausedFor;

/// The server stores a medium thumbnail up to this many sealed bytes.
pub const MAX_SEALED: usize = 512 * 1024;

/// A thumbnail made on this PC: the pixels (wiped when dropped) of one version of one of the owner's photos.
pub struct MadeThumb {
    pub file_id: String,
    pub version: i64,
    pub width: u32,
    pub height: u32,
    pub rgba: Zeroizing<Vec<u8>>,
    /// The chunk format the file's metadata records (what the thumbnail seal is bound to).
    pub pin: ExpectedChunkFormat,
}

/// Why nothing is made or sent now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotNow {
    SettingOff,
    Hold(PausedFor),
}

/// Why a thumbnail was not sealed. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealRefused {
    NotOwned,
    NotMade,
    TooLarge,
    Failed,
}

/// What the send of one thumbnail came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendResult {
    Sent,
    /// Refused for this file alone: skipped.
    Refused,
    /// Ends the run (storage full, offline).
    Stop(Stopped),
}

/// Which photo versions were sent: each goes once.
#[derive(Debug, Default)]
pub struct Sent(HashSet<(String, i64)>);

impl Sent {
    pub fn has(&self, file_id: &str, version: i64) -> bool {
        self.0.contains(&(file_id.to_owned(), version))
    }
}

/// What a run came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SendEnd {
    pub sent: u32,
    pub skipped: u32,
    pub stopped: Option<Stopped>,
}

pub fn not_now(setting: &PreviewSetting, hold: Option<PausedFor>) -> Option<NotNow> {
    if !setting.enabled {
        Some(NotNow::SettingOff)
    } else {
        hold.map(NotNow::Hold)
    }
}

/// Seal `thumb` for its file: WebP from core's medium ladder, sealed with `encrypt_thumbnail_for` under the file's
/// derived key. Plaintext is wiped when dropped.
pub fn seal_made(master: &[u8; 32], thumb: &MadeThumb) -> Result<Vec<u8>, SealRefused> {
    match item_class(&thumb.file_id) {
        Ok(ItemClass::Owner) => {}
        _ => return Err(SealRefused::NotOwned),
    }
    let made = generate_thumbnail(&thumb.rgba, thumb.width, thumb.height, &ThumbnailConfig::medium())
        .map_err(|_| SealRefused::NotMade)?;
    let webp = Zeroizing::new(made.data);
    let key = derive_file_key(&MasterKey::from_bytes(*master), thumb.file_id.as_bytes());
    #[allow(deprecated)]
    let sealed =
        beebeeb_core::encrypt::encrypt_thumbnail_for(&key, &webp, thumb.pin).map_err(|_| SealRefused::Failed)?;
    if sealed.len() > MAX_SEALED {
        return Err(SealRefused::TooLarge);
    }
    Ok(sealed)
}

/// Send each thumbnail not yet sent, one at a time. `Err`: the setting is off or a hold stands, nothing was made.
/// A photo that is sent is marked; one refused or not sealed is skipped; a stop ends the run (that photo is not
/// marked, a later run sends it).
pub fn send_all(
    master: &[u8; 32],
    setting: &PreviewSetting,
    hold: Option<PausedFor>,
    thumbs: &[MadeThumb],
    sent: &mut Sent,
    send: &mut dyn FnMut(&str, &[u8]) -> SendResult,
) -> Result<SendEnd, NotNow> {
    if let Some(why) = not_now(setting, hold) {
        return Err(why);
    }
    let mut end = SendEnd::default();
    for thumb in thumbs {
        if sent.has(&thumb.file_id, thumb.version) {
            end.skipped += 1;
            continue;
        }
        let Ok(sealed) = seal_made(master, thumb) else {
            end.skipped += 1;
            continue;
        };
        match send(&thumb.file_id, &sealed) {
            SendResult::Sent => {
                sent.0.insert((thumb.file_id.clone(), thumb.version));
                end.sent += 1;
            }
            SendResult::Refused => end.skipped += 1,
            SendResult::Stop(why) => {
                end.stopped = Some(why);
                break;
            }
        }
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use beebeeb_core::encrypt::decrypt_thumbnail;

    use super::*;
    use crate::power::{FakePower, decide};

    const OWNED: &str = "c0000000-0000-4000-8000-000000000001";
    const OWNED2: &str = "c0000000-0000-4000-8000-000000000002";
    const FOREIGN: &str = "c0000000-0000-8000-8000-000000000003";
    const MASTER: [u8; 32] = [7; 32];

    fn on() -> PreviewSetting {
        PreviewSetting {
            enabled: true,
            ..PreviewSetting::default()
        }
    }

    fn thumb(id: &str, version: i64) -> MadeThumb {
        let (w, h) = (64u32, 48u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[(x * 4) as u8, (y * 5) as u8, 90, 255]);
            }
        }
        MadeThumb {
            file_id: id.into(),
            version,
            width: w,
            height: h,
            rgba: Zeroizing::new(rgba),
            pin: ExpectedChunkFormat::V2,
        }
    }

    #[test]
    fn setting_off_or_a_hold_means_nothing_now() {
        assert_eq!(not_now(&PreviewSetting::default(), None), Some(NotNow::SettingOff));
        assert_eq!(not_now(&on(), None), None);
        let saver = decide(
            None,
            &FakePower {
                metered: false,
                saver: true,
                connection: None,
            },
        );
        assert_eq!(not_now(&on(), saver), Some(NotNow::Hold(PausedFor::BatterySaver)));
        let metered = decide(
            Some(true),
            &FakePower {
                metered: true,
                saver: false,
                connection: None,
            },
        );
        assert_eq!(not_now(&on(), metered), Some(NotNow::Hold(PausedFor::Metered)));
    }

    #[test]
    fn sealed_thumbnail_opens_with_the_thumbnail_recipe_of_the_files_key() {
        let sealed = seal_made(&MASTER, &thumb(OWNED, 1)).expect("sealed");
        let key = derive_file_key(&MasterKey::from_bytes(MASTER), OWNED.as_bytes());
        let webp = decrypt_thumbnail(&key, &sealed).expect("opens as a thumbnail");
        assert_eq!(&webp[..4], b"RIFF");
        // Not another file's key.
        let other = derive_file_key(&MasterKey::from_bytes(MASTER), OWNED2.as_bytes());
        assert!(decrypt_thumbnail(&other, &sealed).is_err());
    }

    #[test]
    fn a_foreign_id_is_not_sealed() {
        assert_eq!(seal_made(&MASTER, &thumb(FOREIGN, 1)), Err(SealRefused::NotOwned));
    }

    #[test]
    fn each_version_goes_once_and_a_new_version_goes_again() {
        let mut sent = Sent::default();
        let mut calls = Vec::new();
        let mut send = |id: &str, bytes: &[u8]| {
            assert!(!bytes.is_empty());
            calls.push(id.to_owned());
            SendResult::Sent
        };
        let thumbs = [thumb(OWNED, 1), thumb(OWNED2, 1)];
        let end = send_all(&MASTER, &on(), None, &thumbs, &mut sent, &mut send).unwrap();
        assert_eq!((end.sent, end.skipped), (2, 0));
        let end = send_all(&MASTER, &on(), None, &thumbs, &mut sent, &mut send).unwrap();
        assert_eq!((end.sent, end.skipped), (0, 2));
        let end = send_all(&MASTER, &on(), None, &[thumb(OWNED, 2)], &mut sent, &mut send).unwrap();
        assert_eq!(end.sent, 1);
        assert_eq!(calls.len(), 3);
    }

    #[test]
    fn nothing_is_sent_while_off_or_held() {
        let mut sent = Sent::default();
        let mut send = |_: &str, _: &[u8]| -> SendResult { panic!("sent") };
        let thumbs = [thumb(OWNED, 1)];
        assert_eq!(
            send_all(&MASTER, &PreviewSetting::default(), None, &thumbs, &mut sent, &mut send),
            Err(NotNow::SettingOff)
        );
        assert_eq!(
            send_all(&MASTER, &on(), Some(PausedFor::Metered), &thumbs, &mut sent, &mut send),
            Err(NotNow::Hold(PausedFor::Metered))
        );
    }

    #[test]
    fn a_refusal_skips_the_file_and_a_stop_ends_the_run_without_marking_it_sent() {
        let mut sent = Sent::default();
        let thumbs = [thumb(OWNED, 1), thumb(OWNED2, 1)];
        let mut n = 0;
        let mut send = |_: &str, _: &[u8]| {
            n += 1;
            if n == 1 {
                SendResult::Refused
            } else {
                SendResult::Stop(Stopped::StorageFull)
            }
        };
        let end = send_all(&MASTER, &on(), None, &thumbs, &mut sent, &mut send).unwrap();
        assert_eq!((end.sent, end.skipped, end.stopped), (0, 1, Some(Stopped::StorageFull)));
        assert!(!sent.has(OWNED2, 1));
    }
}
