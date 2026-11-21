//! The one table of refusals (ux-23): reason -> the sentence on screen, the button under it and the word the log
//! uses (ux-02's vocabulary, `media::refusal_log`). Video, audio, the Photos stage and the Office viewers all say a
//! refusal through here, so no reason has a generic catch-all and none can lack an honest text.
//!
//! A sentence names the FORMAT (never the file), why it cannot be shown, and what the person can do.

use beebeeb_media_protocol::{Codec, MEDIA_SYSTEM_DECODER_ON};

use crate::media::player::Refusal;

/// "Open in another app\u{2026}": the button of every refusal that is not about the PC's decoders.
pub const OPEN_ELSEWHERE: &str = "Open in another app\u{2026}";
/// The button where this PC has no system decoder for the format. The press is the same one.
pub const OPEN_DEFAULT: &str = "Open in default app";
/// The Microsoft Store page of "HEVC Video Extensions", offered only where Windows lacks it.
pub const HEVC_STORE_URL: &str = "https://apps.microsoft.com/search?query=HEVC%20Video%20Extensions";

/// Photos the app cannot draw (HEIC, RAW): said once at the top of Photos and on the viewer.
pub const UNSHOWABLE: &str = "HEIC and RAW photos can\u{2019}t be shown in Beebeeb yet. Open them in your default app.";

/// The Photos Original tab's words for a HEIC (ux-24: its own reason, not the combined one).
pub const HEIC_WORDS: &str = "HEIC photos can\u{2019}t be shown at full size on this PC yet. This is the preview.";
/// The same for a camera RAW.
pub const RAW_WORDS: &str = "RAW photos can\u{2019}t be shown at full size on this PC yet. This is the camera\u{2019}s preview.";

/// A refusal in full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    pub sentence: String,
    pub action: &'static str,
    /// A link to offer beside the button: only where the fix is something to install.
    pub link: Option<&'static str>,
    /// The log's word for the reason (`media::refusal_log`).
    pub log_word: &'static str,
}

fn codec_name(codec: Codec) -> &'static str {
    codec.name().unwrap_or("this")
}

/// The refusal as shipped.
pub fn text(why: &Refusal) -> Text {
    text_with(MEDIA_SYSTEM_DECODER_ON, why)
}

/// [`text`] for a given state of `MEDIA_SYSTEM_DECODER_ON`. While the switch is off the app does not use the PC's own
/// decoders, so a refusal from their absence never tells a person to install one: it says Beebeeb can't play the
/// format yet.
pub fn text_with(system_decoder_on: bool, why: &Refusal) -> Text {
    let (sentence, action, link, log_word): (String, &'static str, Option<&'static str>, &'static str) = match why {
        Refusal::NoDecoder(name) if name.is_empty() => (
            "Beebeeb doesn't support this kind of file yet. Open it in another app.".into(),
            OPEN_ELSEWHERE,
            None,
            "no decoder",
        ),
        Refusal::NoDecoder(name) => (
            format!("This file uses {name}, which Beebeeb doesn't support yet. Open it in another app."),
            OPEN_ELSEWHERE,
            None,
            "no decoder",
        ),
        Refusal::NoSystemDecoder(Codec::Hevc) if !system_decoder_on => (
            "This video uses HEVC (H.265), which Beebeeb can't play yet. Open it in another app.".into(),
            OPEN_ELSEWHERE,
            None,
            "no system decoder",
        ),
        Refusal::NoSystemDecoder(codec) if !system_decoder_on => (
            format!(
                "This file uses {}, which Beebeeb can't play yet. Open it in another app.",
                codec_name(*codec)
            ),
            OPEN_ELSEWHERE,
            None,
            "no system decoder",
        ),
        Refusal::NoSystemDecoder(Codec::Hevc) => (
            "This video uses HEVC (H.265) and Windows on this PC doesn't have the decoder for it. Install \u{201C}HEVC \
             Video Extensions\u{201D} from the Microsoft Store, or open the file in its default app."
                .into(),
            OPEN_DEFAULT,
            Some(HEVC_STORE_URL),
            "no system decoder",
        ),
        Refusal::NoSystemDecoder(codec) => (
            format!(
                "This file uses {}, and Windows on this PC has no decoder for it. Open the file in its default app.",
                codec_name(*codec)
            ),
            OPEN_DEFAULT,
            None,
            "no system decoder",
        ),
        Refusal::NoSystemMedia(codec) => (
            format!(
                "This file uses {}, and this edition of Windows has no media codecs. Open the file in its default \
                 app.",
                codec_name(*codec)
            ),
            if system_decoder_on { OPEN_DEFAULT } else { OPEN_ELSEWHERE },
            None,
            "no system media",
        ),
        Refusal::TenBit => (
            "This video is 10-bit H.264/HEVC; Windows\u{2019} decoder on this PC can\u{2019}t play it. Open it in its default app.".into(),
            OPEN_DEFAULT,
            None,
            "ten bit",
        ),
        Refusal::UnknownCodec => (
            "This video uses a codec Beebeeb doesn\u{2019}t recognise. Open it in another app.".into(),
            OPEN_ELSEWHERE,
            None,
            "unknown codec",
        ),
        Refusal::Heic => (HEIC_WORDS.into(), OPEN_DEFAULT, None, "heic"),
        Refusal::Raw => (RAW_WORDS.into(), OPEN_DEFAULT, None, "raw"),
        Refusal::CouldNotPlay => (
            "Beebeeb could not play this file: it may be damaged, or the player stopped while opening it. Open it in \
             another app."
                .into(),
            OPEN_ELSEWHERE,
            None,
            "failed",
        ),
        Refusal::NotSignedIn => ("Sign in to play this file.".into(), OPEN_ELSEWHERE, None, "not signed in"),
        Refusal::NeedsWindowsUpdate => (
            crate::windows_floor::NEEDS_WINDOWS_UPDATE.into(),
            OPEN_ELSEWHERE,
            None,
            "needs windows update",
        ),
        Refusal::NotInstalled => (
            "Beebeeb's player is missing from this install (the helper next to the app was not found). Reinstall \
             Beebeeb, or open the file in another app."
                .into(),
            OPEN_ELSEWHERE,
            None,
            "no helper",
        ),
        Refusal::NoSandbox => (
            "Beebeeb's player could not start safely on this PC (its protected sandbox failed to start). Open the \
             file in another app."
                .into(),
            OPEN_ELSEWHERE,
            None,
            "no sandbox",
        ),
    };
    Text { sentence, action, link, log_word }
}

/// Every reason, for the test that none has an empty or generic text.
#[cfg(test)]
pub fn every_reason() -> Vec<Refusal> {
    vec![
        Refusal::NoDecoder("ProRes".into()),
        Refusal::NoDecoder(String::new()),
        Refusal::NoSystemDecoder(Codec::Hevc),
        Refusal::NoSystemDecoder(Codec::H264),
        Refusal::NoSystemMedia(Codec::Aac),
        Refusal::TenBit,
        Refusal::UnknownCodec,
        Refusal::Heic,
        Refusal::Raw,
        Refusal::CouldNotPlay,
        Refusal::NotSignedIn,
        Refusal::NeedsWindowsUpdate,
        Refusal::NotInstalled,
        Refusal::NoSandbox,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal reason with no honest text is a failure: every one has a sentence of its own, an action, and a log
    /// word; the sentences are pairwise different.
    #[test]
    fn every_refusal_has_its_own_honest_text() {
        for on in [true, false] {
            let mut seen = std::collections::BTreeSet::new();
            for why in every_reason() {
                let t = text_with(on, &why);
                assert!(t.sentence.len() > 20, "{why:?}: {}", t.sentence);
                assert!(!t.sentence.contains("  "), "{why:?}: double space");
                assert!(!t.sentence.contains("can't play  "), "{why:?}");
                assert!(!t.action.is_empty() && !t.log_word.is_empty(), "{why:?}");
                assert!(seen.insert(t.sentence.clone()), "{why:?} repeats another reason's sentence");
            }
        }
    }

    #[test]
    fn hevc_names_the_codec_and_the_store_only_when_windows_lacks_it() {
        let on = text_with(true, &Refusal::NoSystemDecoder(Codec::Hevc));
        assert!(on.sentence.contains("HEVC (H.265)") && on.sentence.contains("HEVC Video Extensions"));
        assert_eq!(on.link, Some(HEVC_STORE_URL));
        let off = text_with(false, &Refusal::NoSystemDecoder(Codec::Hevc));
        assert!(off.sentence.contains("HEVC (H.265)") && !off.sentence.contains("Extensions"));
        assert_eq!(off.link, None);
        assert_eq!(text_with(true, &Refusal::NoSystemDecoder(Codec::Aac)).link, None);
    }

    /// Every variant is named in an exhaustive match (no wildcard arm), so a new reason cannot ship without a count here.
    #[test]
    fn every_variant_is_listed_in_every_reason() {
        let n = every_reason()
            .iter()
            .map(|why| match why {
                Refusal::NoDecoder(_) => 1,
                Refusal::NoSystemDecoder(_) => 1,
                Refusal::NoSystemMedia(_) => 1,
                Refusal::TenBit => 1,
                Refusal::UnknownCodec => 1,
                Refusal::Heic => 1,
                Refusal::Raw => 1,
                Refusal::CouldNotPlay => 1,
                Refusal::NotSignedIn => 1,
                Refusal::NotInstalled => 1,
                Refusal::NoSandbox => 1,
                Refusal::NeedsWindowsUpdate => 1,
            })
            .sum::<usize>();
        assert_eq!(n, 14);
    }

    #[test]
    fn the_new_reasons_say_their_own_cause() {
        assert!(text(&Refusal::TenBit).sentence.contains("10-bit"));
        assert!(text(&Refusal::UnknownCodec).sentence.contains("recognise"));
        assert!(text(&Refusal::Heic).sentence.contains("HEIC"));
        assert!(text(&Refusal::Raw).sentence.contains("RAW"));
    }

    #[test]
    fn the_failure_steps_are_named() {
        assert!(text(&Refusal::NotInstalled).sentence.contains("helper"));
        assert!(text(&Refusal::NoSandbox).sentence.contains("sandbox"));
        assert!(text(&Refusal::CouldNotPlay).sentence.contains("damaged"));
    }
}
