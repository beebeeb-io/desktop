//! What a file's first bytes say when it is a bare (elementary) audio stream: FLAC, MPEG audio, ADTS AAC, AIFF,
//! AC-3 / E-AC-3.
//!
//! FilmCraft's whole-file audio opener picks its decoder by the file name's extension, and the helper has no name (it
//! opens every file as ""). So a bare FLAC or MP3 stream matched no opener and answered `NoDecoder`. A sniffed
//! stream gets a made-up name ([`Bare::name`], e.g. "a.flac") that goes to FilmCraft's openers and nowhere else; its
//! codec's route is checked before FilmCraft reads a byte of it whole. AC-3 has no FilmCraft opener: its answer comes
//! from the route table alone.
//!
//! Nothing here decodes. The head is hostile: every read is bounds-checked, every length is checked arithmetic.

use beebeeb_media_protocol::Codec;

/// A bare audio stream, as its first bytes say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bare {
    /// `fLaC`.
    Flac,
    /// MPEG audio Layer I or II: two frame headers in a row.
    Mp2,
    /// MPEG audio Layer III: two frame headers in a row, or an ID3v2 tag before frames this head does not show.
    Mp3,
    /// ADTS AAC (ISO/IEC 13818-7: the 12-bit sync and layer `00`).
    Aac,
    /// `FORM` … `AIFF` / `AIFC`.
    Aiff,
    /// AC-3: the sync `0x0B77` and a bit stream id up to 10.
    Ac3,
    /// E-AC-3: the sync `0x0B77` and a bit stream id from 11 to 16.
    Eac3,
}

impl Bare {
    /// The codec whose route decides the stream.
    pub fn codec(self) -> Codec {
        match self {
            Bare::Flac => Codec::Flac,
            Bare::Mp2 => Codec::Mp2,
            Bare::Mp3 => Codec::Mp3,
            Bare::Aac => Codec::Aac,
            Bare::Aiff => Codec::Pcm,
            Bare::Ac3 => Codec::Ac3,
            Bare::Eac3 => Codec::Eac3,
        }
    }

    /// The name FilmCraft's whole-file audio opener knows the stream by (its extension is all it reads), or `None`
    /// where FilmCraft has no opener for it (AC-3, E-AC-3). Only ever handed to FilmCraft's openers.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Bare::Flac => Some("a.flac"),
            Bare::Mp2 => Some("a.mp2"),
            Bare::Mp3 => Some("a.mp3"),
            Bare::Aac => Some("a.aac"),
            Bare::Aiff => Some("a.aiff"),
            Bare::Ac3 | Bare::Eac3 => None,
        }
    }

    /// The stream's kind as a container name, for the operating system's reader when the stream goes there
    /// (`crate::mf::convert::container_hint` turns it into a name hint).
    pub fn container(self) -> &'static str {
        match self {
            Bare::Flac => "FLAC",
            Bare::Mp2 | Bare::Mp3 => "MP3",
            Bare::Aac => "ADTS",
            Bare::Aiff => "AIFF",
            Bare::Ac3 => "AC-3",
            Bare::Eac3 => "E-AC-3",
        }
    }
}

/// The bare stream `head` (the file's first bytes) starts, or `None` for anything else (a container, or bytes this
/// does not know).
pub fn sniff(head: &[u8]) -> Option<Bare> {
    if head.starts_with(b"ID3") {
        // An ID3v2 tag (ID3v2.4 §3.1): what follows it says the format. A tag that runs past the head, or one followed
        // by bytes this does not know, is MP3's (where ID3v2 is at home).
        let after = id3v2_len(head).and_then(|n| head.get(n..));
        return Some(after.and_then(untagged).unwrap_or(Bare::Mp3));
    }
    untagged(head)
}

fn untagged(head: &[u8]) -> Option<Bare> {
    if head.starts_with(b"fLaC") {
        return Some(Bare::Flac);
    }
    if head.starts_with(b"FORM") && matches!(head.get(8..12), Some(b"AIFF" | b"AIFC")) {
        return Some(Bare::Aiff);
    }
    if let Some(bare) = ac3(head) {
        return Some(bare);
    }
    if adts(head) {
        return Some(Bare::Aac);
    }
    mpeg_audio(head)
}

/// The length of the ID3v2 tag that starts `head`: its 10-byte header, its syncsafe size, and a 10-byte footer when
/// flagged. `None` for a header that is cut short or whose size bytes are not syncsafe.
fn id3v2_len(head: &[u8]) -> Option<usize> {
    let [b'I', b'D', b'3', _, _, flags, s0, s1, s2, s3, ..] = *head else {
        return None;
    };
    let size = [s0, s1, s2, s3]
        .into_iter()
        .try_fold(0usize, |size, b| (b < 0x80).then_some((size << 7) | usize::from(b)))?;
    let footer = if flags & 0x10 != 0 { 10 } else { 0 };
    size.checked_add(10 + footer)
}

/// AC-3 (ATSC A/52 §5.4.1) or E-AC-3 (Annex E): the sync word, then the bit stream id in the top five bits of the sixth
/// byte in both syntaxes. AC-3's sample rate code `3` is reserved.
fn ac3(head: &[u8]) -> Option<Bare> {
    let [0x0B, 0x77, _, _, b4, b5, ..] = *head else {
        return None;
    };
    match b5 >> 3 {
        0..=10 if b4 >> 6 != 3 => Some(Bare::Ac3),
        11..=16 => Some(Bare::Eac3),
        _ => None,
    }
}

/// An ADTS header at the start of `b`: the frame's length (header included), or `None`.
fn adts_frame_len(b: &[u8]) -> Option<usize> {
    let [0xFF, b1, b2, b3, b4, b5, ..] = *b else {
        return None;
    };
    // The 12-bit sync and layer `00`; sampling frequency indexes 13 to 15 are reserved or the escape.
    if b1 & 0xF6 != 0xF0 || (b2 >> 2) & 0x0F > 12 {
        return None;
    }
    let len = (usize::from(b3 & 0x03) << 11) | (usize::from(b4) << 3) | usize::from(b5 >> 5);
    let header = if b1 & 0x01 != 0 { 7 } else { 9 };
    (len > header).then_some(len)
}

/// ADTS: a whole first frame, and another header where it ends (unless the stream ends there).
fn adts(head: &[u8]) -> bool {
    let Some(len) = adts_frame_len(head) else {
        return false;
    };
    match head.get(len..) {
        // The head ends inside the first frame: nothing to confirm it by.
        None => false,
        Some(next) if next.len() >= 6 => adts_frame_len(next).is_some(),
        // The stream ends with its first frame.
        Some(_) => true,
    }
}

/// One MPEG audio frame header (ISO/IEC 11172-3 §2.4.2.3, 13818-3; MPEG 2.5 included).
#[derive(Clone, Copy, PartialEq, Eq)]
struct MpaHeader {
    /// 1, 2 or 3.
    layer: u8,
    /// The two version bits: `0` MPEG 2.5, `2` MPEG-2, `3` MPEG-1 (`1` is reserved and never parsed).
    version: u8,
    rate: u32,
    /// The frame's length in bytes, header included.
    len: usize,
}

/// Bit rates in kbit/s by index 1 to 14 (0 is the free format, 15 is forbidden: neither is parsed).
const MPEG1_L1: [u32; 14] = [32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448];
const MPEG1_L2: [u32; 14] = [32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
const MPEG1_L3: [u32; 14] = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const MPEG2_L1: [u32; 14] = [32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256];
const MPEG2_L23: [u32; 14] = [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

fn mpa_header(b: &[u8]) -> Option<MpaHeader> {
    let [0xFF, b1, b2, _, ..] = *b else {
        return None;
    };
    if b1 & 0xE0 != 0xE0 {
        return None;
    }
    let version = (b1 >> 3) & 0x03;
    let layer = match (b1 >> 1) & 0x03 {
        0b11 => 1,
        0b10 => 2,
        0b01 => 3,
        _ => return None,
    };
    if version == 1 {
        return None;
    }
    let bitrate_index = usize::from(b2 >> 4);
    let mpeg1 = version == 3;
    let table = match (mpeg1, layer) {
        (true, 1) => &MPEG1_L1,
        (true, 2) => &MPEG1_L2,
        (true, _) => &MPEG1_L3,
        (false, 1) => &MPEG2_L1,
        (false, _) => &MPEG2_L23,
    };
    let kbps = table.get(bitrate_index.checked_sub(1)?).copied()?;
    let rates: [u32; 3] = match version {
        3 => [44_100, 48_000, 32_000],
        2 => [22_050, 24_000, 16_000],
        _ => [11_025, 12_000, 8_000],
    };
    let rate = rates.get(usize::from((b2 >> 2) & 0x03)).copied()?;
    let padding = u32::from((b2 >> 1) & 0x01);
    let bits = kbps * 1000;
    // Samples per frame / 8 × bit rate / sample rate, plus the padding slot (Layer I: four-byte slots).
    let len = match layer {
        1 => (12 * bits / rate + padding) * 4,
        3 if !mpeg1 => 72 * bits / rate + padding,
        _ => 144 * bits / rate + padding,
    };
    Some(MpaHeader {
        layer,
        version,
        rate,
        len: usize::try_from(len).ok()?,
    })
}

/// MPEG audio: a frame header, and another of the same layer, version and rate where the first frame ends. One header
/// alone is not enough (two bytes of `0xFF` and a little luck make one).
fn mpeg_audio(head: &[u8]) -> Option<Bare> {
    let first = mpa_header(head)?;
    let second = mpa_header(head.get(first.len..)?)?;
    if (second.layer, second.version, second.rate) != (first.layer, first.version, first.rate) {
        return None;
    }
    Some(if first.layer == 3 { Bare::Mp3 } else { Bare::Mp2 })
}

/// The MPEG audio layer of an MPEG-TS or MPEG-PS file's audio, from its first bytes: FilmCraft names every layer
/// "MPEG Audio" there, which reads as MP1/MP2 (row 15 of MCP2). Every MPEG audio PES packet (stream id `0xC0` to
/// `0xDF`) that starts in `head` is read up to its first frame header. `Some(Bare::Mp3)` only when every one found is
/// Layer III, `Some(Bare::Mp2)` when every one is Layer I or II; `None` when none is found or they disagree (the label's
/// answer then stands). An ADTS AAC payload has layer `00` and is never counted.
pub fn program_stream_mpeg_audio(head: &[u8]) -> Option<Bare> {
    let mut layer3 = 0usize;
    let mut layer12 = 0usize;
    let mut at = 0usize;
    while let Some(found) = head
        .get(at..)?
        .windows(4)
        .position(|w| w[..3] == [0, 0, 1] && (0xC0..=0xDF).contains(&w[3]))
    {
        let start = at + found;
        at = start + 4;
        let Some(header) = pes_payload(head.get(start..)?).and_then(mpa_header) else {
            continue;
        };
        if header.layer == 3 {
            layer3 += 1;
        } else {
            layer12 += 1;
        }
    }
    match (layer3, layer12) {
        (0, 0) => None,
        (_, 0) => Some(Bare::Mp3),
        (0, _) => Some(Bare::Mp2),
        _ => None,
    }
}

/// The payload of the PES packet that starts `pes` (`00 00 01`, the stream id, the 16-bit length): after an MPEG-2
/// header (ISO/IEC 13818-1 §2.4.3.6: its marker bits `10`, then its header length) or an MPEG-1 one (11172-1
/// §2.4.3.3: stuffing, the optional buffer size, the time stamps). `None` for a header cut short or one that is
/// neither.
fn pes_payload(pes: &[u8]) -> Option<&[u8]> {
    let flags = *pes.get(6)?;
    if flags & 0xC0 == 0x80 {
        let len = usize::from(*pes.get(8)?);
        return pes.get(9 + len..);
    }
    let mut at = 6;
    // At most 16 stuffing bytes.
    while *pes.get(at)? == 0xFF && at < 6 + 16 {
        at += 1;
    }
    if *pes.get(at)? & 0xC0 == 0x40 {
        at += 2;
    }
    let skip = match *pes.get(at)? {
        b if b & 0xF0 == 0x20 => 5,
        b if b & 0xF0 == 0x30 => 10,
        0x0F => 1,
        _ => return None,
    };
    pes.get(at + skip..)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` frames of `len` bytes, each starting with `header` (the rest zeros).
    fn frames(header: [u8; 4], len: usize, n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..n {
            let mut frame = vec![0u8; len];
            frame[..4].copy_from_slice(&header);
            out.extend(frame);
        }
        out
    }

    /// An ADTS frame of `len` bytes (AAC-LC, 44.1 kHz, stereo, no CRC).
    fn adts_frame(len: usize) -> Vec<u8> {
        let mut frame = vec![0u8; len];
        frame[..7].copy_from_slice(&[
            0xFF,
            0xF1,
            0x50,
            0x80 | ((len >> 11) & 0x03) as u8,
            ((len >> 3) & 0xFF) as u8,
            (((len & 0x07) << 5) as u8) | 0x1F,
            0xFC,
        ]);
        frame
    }

    /// An ID3v2.4 tag of `size` body bytes (zeros).
    fn id3(size: usize) -> Vec<u8> {
        let mut tag = vec![b'I', b'D', b'3', 4, 0, 0];
        tag.extend([21, 14, 7, 0].map(|shift| ((size >> shift) & 0x7F) as u8));
        tag.extend(std::iter::repeat_n(0u8, size));
        tag
    }

    #[test]
    fn bare_streams_are_known_by_their_first_bytes() {
        // FLAC and AIFF by their magic.
        assert_eq!(sniff(b"fLaC\0\0\0\x22\x10\x00"), Some(Bare::Flac));
        assert_eq!(sniff(b"FORM\0\0\x10\0AIFFCOMM"), Some(Bare::Aiff));
        assert_eq!(sniff(b"FORM\0\0\x10\0AIFCFVER"), Some(Bare::Aiff));
        assert_eq!(sniff(b"FORM\0\0\x10\0ILBMBMHD"), None, "an IFF picture is not AIFF");

        // MPEG audio: two headers in a row. MPEG-1 Layer III at 128 kbit/s, 44.1 kHz: 417-byte frames.
        let mp3 = frames([0xFF, 0xFB, 0x90, 0x00], 417, 3);
        assert_eq!(sniff(&mp3), Some(Bare::Mp3));
        // MPEG-1 Layer II at 256 kbit/s, 48 kHz: 768 bytes. MPEG-1 Layer I at 32 kbit/s, 44.1 kHz: 32 bytes.
        assert_eq!(sniff(&frames([0xFF, 0xFD, 0xC4, 0x00], 768, 2)), Some(Bare::Mp2));
        assert_eq!(sniff(&frames([0xFF, 0xFF, 0x10, 0x00], 32, 2)), Some(Bare::Mp2));
        // MPEG-2 Layer III at 64 kbit/s, 22.05 kHz (576 samples a frame): 208 bytes.
        assert_eq!(sniff(&frames([0xFF, 0xF3, 0x80, 0x00], 208, 2)), Some(Bare::Mp3));
        // One header alone: not enough, whether the bytes after it are zeros or the head ends inside the frame.
        let mut one = frames([0xFF, 0xFB, 0x90, 0x00], 417, 1);
        assert_eq!(sniff(&one), None);
        one.extend([0u8; 600]);
        assert_eq!(sniff(&one), None);
        // A second header that disagrees (another layer) is not a stream either.
        let mut mixed = frames([0xFF, 0xFB, 0x90, 0x00], 417, 1);
        mixed.extend(frames([0xFF, 0xFD, 0xC4, 0x00], 768, 1));
        assert_eq!(sniff(&mixed), None);
        // The free format and the forbidden bit rate are never parsed.
        assert_eq!(sniff(&frames([0xFF, 0xFB, 0x00, 0x00], 417, 3)), None);
        assert_eq!(sniff(&frames([0xFF, 0xFB, 0xF0, 0x00], 417, 3)), None);

        // ID3v2 before the audio: what follows the tag says the format; a tag alone is MP3's.
        let mut tagged = id3(32);
        tagged.extend(&mp3);
        assert_eq!(sniff(&tagged), Some(Bare::Mp3));
        assert_eq!(sniff(&id3(32)), Some(Bare::Mp3));
        assert_eq!(sniff(b"ID3"), Some(Bare::Mp3), "a tag cut short is still MP3's");
        let mut tagged_flac = id3(16);
        tagged_flac.extend(b"fLaC\0\0\0\x22");
        assert_eq!(sniff(&tagged_flac), Some(Bare::Flac));

        // ADTS AAC: a header, and the next one where the frame ends (or the end of the stream).
        let mut aac = adts_frame(371);
        aac.extend(adts_frame(371));
        aac.extend(adts_frame(371));
        assert_eq!(sniff(&aac), Some(Bare::Aac));
        assert_eq!(sniff(&adts_frame(371)), Some(Bare::Aac), "a one-frame stream");
        let mut tagged_aac = id3(8);
        tagged_aac.extend(&aac);
        assert_eq!(
            sniff(&tagged_aac),
            Some(Bare::Aac),
            "the AAC route is checked behind a tag too"
        );
        let mut broken = adts_frame(371);
        broken.extend([0u8; 400]);
        assert_eq!(sniff(&broken), None, "a header with no second one where the frame ends");
        assert_eq!(
            sniff(&adts_frame(371)[..200]),
            None,
            "a head that ends inside the first frame"
        );

        // AC-3 and E-AC-3 by the bit stream id; above 16 it is neither.
        assert_eq!(sniff(&[0x0B, 0x77, 0, 0, 0x14, 8 << 3, 0, 0]), Some(Bare::Ac3));
        assert_eq!(sniff(&[0x0B, 0x77, 0, 0, 0x14, 10 << 3, 0, 0]), Some(Bare::Ac3));
        assert_eq!(sniff(&[0x0B, 0x77, 0, 0, 0x14, 11 << 3, 0, 0]), Some(Bare::Eac3));
        assert_eq!(sniff(&[0x0B, 0x77, 0, 0, 0x3F, 16 << 3, 0, 0]), Some(Bare::Eac3));
        assert_eq!(sniff(&[0x0B, 0x77, 0, 0, 0x14, 17 << 3, 0, 0]), None);
        assert_eq!(
            sniff(&[0x0B, 0x77, 0, 0, 0xC0, 8 << 3, 0, 0]),
            None,
            "AC-3's reserved rate code"
        );

        // Containers and nothing at all are no bare stream.
        assert_eq!(sniff(b"\0\0\0\x20ftypisom"), None);
        assert_eq!(sniff(&[0x1A, 0x45, 0xDF, 0xA3]), None);
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(sniff(b"OggS"), None);
        assert_eq!(sniff(&[]), None);
        assert_eq!(sniff(&[0xFF]), None);

        // Random bytes: none (64 heads of 4 KiB from a fixed xorshift).
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..64 {
            let head: Vec<u8> = (0..4096)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x >> 24) as u8
                })
                .collect();
            assert_eq!(sniff(&head), None, "random head starting {:02x?}", &head[..6]);
        }
    }

    /// One 188-byte transport stream packet that starts a PES packet (MPEG-2 header, a PTS) of `stream_id` whose
    /// payload begins with `frame`.
    fn ts_packet(stream_id: u8, frame: &[u8]) -> Vec<u8> {
        let mut packet = vec![0x47, 0x41, 0x01, 0x10];
        packet.extend([0, 0, 1, stream_id, 0, 0, 0x84, 0x80, 5, 0x21, 0, 1, 0, 1]);
        packet.extend(frame);
        packet.resize(188, 0xFF);
        packet
    }

    /// A program stream pack header (MPEG-1) and one PES packet (MPEG-1 header: two stuffing bytes, the buffer size,
    /// a PTS) of `stream_id` whose payload begins with `frame`.
    fn ps_pack(stream_id: u8, frame: &[u8]) -> Vec<u8> {
        let mut pack = vec![0, 0, 1, 0xBA, 0x21, 0, 1, 0, 1, 0x80, 0, 1];
        pack.extend([0, 0, 1, stream_id, 0, 0, 0xFF, 0xFF, 0x40, 0x20, 0x21, 0, 1, 0, 1]);
        pack.extend(frame);
        pack.resize(pack.len() + 400, 0);
        pack
    }

    const MP3_HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];
    const MP2_HEADER: [u8; 4] = [0xFF, 0xFD, 0xC4, 0x00];

    /// Row 15 (MCP2): MP3 inside MPEG-TS/PS is told from MP2 by the frame header after the PES header; a head with no
    /// MPEG audio packet, with ADTS in it, or with layers that disagree says nothing.
    #[test]
    fn the_layer_of_mpeg_audio_in_a_transport_or_program_stream_is_read() {
        let mut ts = ts_packet(0xE0, &[0, 0, 0, 1, 0x67]);
        ts.extend(ts_packet(0xC0, &MP3_HEADER));
        ts.extend(ts_packet(0xC0, &MP3_HEADER));
        assert_eq!(program_stream_mpeg_audio(&ts), Some(Bare::Mp3));
        assert_eq!(
            program_stream_mpeg_audio(&ts_packet(0xC1, &MP2_HEADER)),
            Some(Bare::Mp2)
        );
        assert_eq!(program_stream_mpeg_audio(&ps_pack(0xC0, &MP3_HEADER)), Some(Bare::Mp3));
        assert_eq!(program_stream_mpeg_audio(&ps_pack(0xC0, &MP2_HEADER)), Some(Bare::Mp2));

        let mut mixed = ts_packet(0xC0, &MP3_HEADER);
        mixed.extend(ts_packet(0xC1, &MP2_HEADER));
        assert_eq!(program_stream_mpeg_audio(&mixed), None, "two streams that disagree");
        // ADTS AAC in the same stream ids (ffmpeg puts it in 0xC0): layer 00, not MPEG audio.
        let adts = [0xFF, 0xF1, 0x50, 0x80, 0x2E, 0x7F, 0xFC];
        assert_eq!(program_stream_mpeg_audio(&ts_packet(0xC0, &adts)), None);
        // Video only, a PES header cut short, nothing at all.
        assert_eq!(program_stream_mpeg_audio(&ts_packet(0xE0, &MP3_HEADER)), None);
        assert_eq!(program_stream_mpeg_audio(&[0, 0, 1, 0xC0, 0, 0, 0x84]), None);
        assert_eq!(program_stream_mpeg_audio(&[]), None);
        // The packet's PES header is skipped by its own length: a header that does not end where it says is no frame.
        let mut wrong = ts_packet(0xC0, &MP3_HEADER);
        wrong[12] = 9;
        assert_eq!(program_stream_mpeg_audio(&wrong), None);
    }

    #[test]
    fn a_bare_stream_names_its_codec_and_only_filmcrafts_openers_get_a_name() {
        assert_eq!(Bare::Flac.codec(), Codec::Flac);
        assert_eq!(Bare::Mp2.codec(), Codec::Mp2);
        assert_eq!(Bare::Mp3.codec(), Codec::Mp3);
        assert_eq!(Bare::Aac.codec(), Codec::Aac);
        assert_eq!(Bare::Aiff.codec(), Codec::Pcm);
        assert_eq!(Bare::Ac3.codec(), Codec::Ac3);
        assert_eq!(Bare::Eac3.codec(), Codec::Eac3);
        // The names carry only the extension FilmCraft's whole-file opener matches; AC-3 has no opener there.
        assert_eq!(Bare::Flac.name(), Some("a.flac"));
        assert_eq!(Bare::Mp3.name(), Some("a.mp3"));
        assert_eq!(Bare::Aac.name(), Some("a.aac"));
        assert_eq!(Bare::Ac3.name(), None);
        assert_eq!(Bare::Eac3.name(), None);
    }
}
