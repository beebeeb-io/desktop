//! The file's title and artist (decision D-MB2-1 (a)): read by symphonia's metadata readers inside this sandboxed
//! process, over the same app ranges as the decoders ([`crate::read::PipeReader`]). No artwork in 0.9.0: `Tags::art`
//! is always `None`, and the app shows the file name and the generic art (the board's own fallback). symphonia 0.5.5's
//! readers do not enforce `MetadataOptions`' limits (only `symphonia-core` defines them), so an embedded picture is
//! read and dropped; the limits are set anyway, for a version that honours them.
//!
//! Only an audio file's tags are read (the audio viewer shows them); a video shows its file name. The pass is bounded
//! by what it reads: at most [`TAG_READ_MAX`] bytes, never a byte past the file's end. A file whose tags cannot be read
//! within that (a picture larger than the limit among them) plays as before, without `Tags`.
//!
//! Never logs a word of a tag.

use std::io::{self, Read, Seek, SeekFrom};

use beebeeb_media_protocol::{Tags, cut_text};
use filmcraft_media::SharedReader;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::{Limit, Metadata, MetadataOptions, MetadataRevision, StandardTagKey, Value};
use symphonia::core::probe::Hint;

/// The most bytes one tag pass reads from the app.
pub const TAG_READ_MAX: u64 = 16 << 20;
/// The most bytes one tag may take once read, for a symphonia that enforces it (0.5.5 does not: see the module doc).
pub const TAG_BYTES_MAX: usize = 64 << 10;

/// The title and artist of the file behind `reader`, or `None` when it has neither (or they cannot be read).
pub fn read(reader: SharedReader) -> Option<Tags> {
    let stream = MediaSourceStream::new(Box::new(Ranges::new(reader)), MediaSourceStreamOptions::default());
    let options = MetadataOptions {
        limit_metadata_bytes: Limit::Maximum(TAG_BYTES_MAX),
        limit_visual_bytes: Limit::Maximum(0),
    };
    let mut probed = symphonia::default::get_probe()
        .format(&Hint::new(), stream, &FormatOptions::default(), &options)
        .ok()?;
    let mut found = Found::default();
    // The container's own tags first (Vorbis comments, MP4 `ilst`, RIFF `INFO`), then the tags before the container
    // (ID3v2; a file may carry more than one).
    found.take_all(probed.format.metadata());
    if let Some(outside) = probed.metadata.get() {
        found.take_all(outside);
    }
    found.into_tags()
}

#[derive(Default)]
struct Found {
    title: Option<String>,
    artist: Option<String>,
}

impl Found {
    /// The first title and artist seen are kept.
    fn take(&mut self, revision: &MetadataRevision) {
        for tag in revision.tags() {
            let slot = match tag.std_key {
                Some(StandardTagKey::TrackTitle) => &mut self.title,
                Some(StandardTagKey::Artist) => &mut self.artist,
                _ => continue,
            };
            if slot.is_none() {
                *slot = text(&tag.value);
            }
        }
    }

    /// Every revision of `log`, oldest first.
    fn take_all(&mut self, mut log: Metadata<'_>) {
        if let Some(revision) = log.current() {
            self.take(revision);
        }
        while log.pop().is_some() {
            if let Some(revision) = log.current() {
                self.take(revision);
            }
        }
    }

    fn into_tags(self) -> Option<Tags> {
        if self.title.is_none() && self.artist.is_none() {
            return None;
        }
        Some(Tags {
            title: self.title.unwrap_or_default(),
            artist: self.artist.unwrap_or_default(),
            art: None,
        })
    }
}

/// A tag's text as the wire takes it: control characters dropped, trimmed, at most `MAX_TEXT` characters (a longer
/// one would not encode, and the session would end). `None` for an empty or non-text value.
pub fn text(value: &Value) -> Option<String> {
    let Value::String(raw) = value else {
        return None;
    };
    let clean: String = raw.chars().filter(|c| !c.is_control()).collect();
    let cut = cut_text(clean.trim());
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_owned())
}

/// The file as symphonia reads it: a cursor over the app's ranges that never asks past the end and stops after
/// [`TAG_READ_MAX`] bytes.
struct Ranges {
    reader: SharedReader,
    len: u64,
    pos: u64,
    pulled: u64,
}

impl Ranges {
    fn new(reader: SharedReader) -> Ranges {
        Ranges {
            len: reader.len(),
            reader,
            pos: 0,
            pulled: 0,
        }
    }
}

impl Read for Ranges {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        if self.pulled >= TAG_READ_MAX {
            return Err(io::Error::other("the tag pass read its limit"));
        }
        let n = (buf.len() as u64)
            .min(self.len - self.pos)
            .min(TAG_READ_MAX - self.pulled);
        // `n` is at most `buf.len()`.
        let n = n as usize;
        self.reader.read_at(self.pos, &mut buf[..n])?;
        self.pos += n as u64;
        self.pulled += n as u64;
        Ok(n)
    }
}

impl Seek for Ranges {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let at = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
        };
        // Past the end is allowed (a read there gives 0 bytes); before the start is not.
        self.pos = at.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.pos)
    }
}

impl MediaSource for Ranges {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use beebeeb_media_protocol::MAX_TEXT;
    use filmcraft_media::ByteReader;

    use super::*;

    /// Bytes in memory that count what was read and refuse (and count) any read past the end.
    struct Counted {
        bytes: Vec<u8>,
        read: AtomicU64,
        past_end: AtomicUsize,
    }

    impl Counted {
        fn new(bytes: Vec<u8>) -> Arc<Counted> {
            Arc::new(Counted {
                bytes,
                read: AtomicU64::new(0),
                past_end: AtomicUsize::new(0),
            })
        }
    }

    impl ByteReader for Counted {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }

        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            let end = offset
                .checked_add(buf.len() as u64)
                .ok_or(io::ErrorKind::InvalidInput)?;
            if end > self.len() {
                self.past_end.fetch_add(1, Ordering::Relaxed);
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            buf.copy_from_slice(&self.bytes[offset as usize..end as usize]);
            self.read.fetch_add(buf.len() as u64, Ordering::Relaxed);
            Ok(())
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|_| panic!("fixture {name} is missing"))
    }

    /// `mp3.mp3` without the ID3v2 tag ffmpeg wrote (an encoder line only): the audio frames.
    fn mp3_frames() -> Vec<u8> {
        let file = fixture("mp3.mp3");
        assert_eq!(&file[..3], b"ID3", "the fixture starts with ffmpeg's tag");
        let size = file[6..10].iter().fold(0usize, |size, b| (size << 7) | usize::from(*b));
        file[10 + size..].to_vec()
    }

    /// One ID3v2.3 text frame (`id`, ISO-8859-1 text).
    fn id3_frame(id: &[u8; 4], text: &[u8]) -> Vec<u8> {
        let size = u32::try_from(text.len() + 1).expect("a short frame");
        let mut frame = id.to_vec();
        frame.extend(size.to_be_bytes());
        frame.extend([0, 0, 0]);
        frame.extend(text);
        frame
    }

    /// An ID3v2.3 tag holding `frames`, with a picture frame of `picture` bytes after them.
    fn id3_tag(frames: &[Vec<u8>], picture: usize) -> Vec<u8> {
        let mut body: Vec<u8> = frames.concat();
        if picture > 0 {
            // APIC: encoding, "image/png\0", picture type, empty description, the bytes.
            let mut apic = vec![0u8];
            apic.extend(b"image/png\0");
            apic.extend([3, 0]);
            apic.extend(std::iter::repeat_n(0xAB, picture));
            body.extend(b"APIC");
            body.extend(u32::try_from(apic.len()).expect("a short frame").to_be_bytes());
            body.extend([0, 0]);
            body.extend(apic);
        }
        let size = body.len();
        let mut tag = vec![b'I', b'D', b'3', 3, 0, 0];
        tag.extend([21, 14, 7, 0].map(|shift| ((size >> shift) & 0x7F) as u8));
        tag.extend(body);
        tag
    }

    /// D-MB2-1 (a): an MP3 file's ID3v2 title and artist are read, no picture is ever sent (the APIC frame is read and
    /// dropped), and the pass never reads past the end of the file.
    #[test]
    fn an_audio_files_title_and_artist_are_read_and_no_picture_is_sent() {
        let mut file = id3_tag(
            &[
                id3_frame(b"TIT2", b"Harbour sessions, take 3"),
                id3_frame(b"TPE1", b"Ana Alvares"),
            ],
            4096,
        );
        file.extend(mp3_frames());
        let reader = Counted::new(file);
        let tags = read(reader.clone()).expect("the tag is read");
        assert_eq!(tags.title, "Harbour sessions, take 3");
        assert_eq!(tags.artist, "Ana Alvares");
        assert_eq!(tags.art, None, "no artwork in 0.9.0");
        assert_eq!(reader.past_end.load(Ordering::Relaxed), 0, "never a read past the end");
        // Before another tag (ffmpeg's, an encoder line only): both are read, the title is still found.
        let mut file = id3_tag(&[id3_frame(b"TIT2", b"Harbour sessions, take 3")], 0);
        file.extend(fixture("mp3.mp3"));
        let tags = read(Counted::new(file)).expect("the tag is read");
        assert_eq!(tags.title, "Harbour sessions, take 3");
    }

    /// FLAC's own Vorbis comment (the container's tags, not an ID3v2 tag before it): title and artist are read.
    #[test]
    fn a_flac_files_vorbis_comment_gives_its_title_and_artist() {
        let file = fixture("flac.flac");
        assert_eq!(&file[..4], b"fLaC");
        assert_eq!(file[4], 0, "STREAMINFO first, more blocks after it");
        assert_eq!(&file[5..8], &[0, 0, 34], "a 34-byte STREAMINFO");
        let mut body = Vec::new();
        body.extend(4u32.to_le_bytes());
        body.extend(b"test");
        let comments: [&[u8]; 2] = [b"TITLE=Harbour sessions", b"ARTIST=Ana Alvares"];
        body.extend(2u32.to_le_bytes());
        for comment in comments {
            body.extend(u32::try_from(comment.len()).expect("short").to_le_bytes());
            body.extend(comment);
        }
        // A VORBIS_COMMENT block (type 4, not the last) right after STREAMINFO.
        let mut tagged = file[..42].to_vec();
        tagged.push(4);
        tagged.extend(&u32::try_from(body.len()).expect("short").to_be_bytes()[1..]);
        tagged.extend(body);
        tagged.extend(&file[42..]);
        let reader = Counted::new(tagged);
        let tags = read(reader.clone()).expect("the Vorbis comment is read");
        assert_eq!(tags.title, "Harbour sessions");
        assert_eq!(tags.artist, "Ana Alvares");
        assert_eq!(reader.past_end.load(Ordering::Relaxed), 0);
    }

    /// A file without tags (the fixture as ffmpeg writes it) has none: no empty `Tags` is sent.
    #[test]
    fn a_file_without_a_title_or_artist_has_no_tags() {
        let reader = Counted::new(fixture("pcm.wav"));
        let tags = read(reader.clone());
        assert!(tags.is_none(), "{tags:?}");
        assert_eq!(reader.past_end.load(Ordering::Relaxed), 0);
        // Bytes that are no audio file at all: none, and no panic.
        assert!(read(Counted::new(vec![0x5A; 10_000])).is_none());
        assert!(read(Counted::new(Vec::new())).is_none());
    }

    /// A text the wire would refuse (over `MAX_TEXT` characters) is cut, control characters are dropped, and an
    /// empty or non-text value is no text.
    #[test]
    fn tag_text_is_cut_to_what_the_wire_takes() {
        let long = "\u{1F3B5}".repeat(MAX_TEXT + 10);
        let cut = text(&Value::String(long)).expect("a text");
        assert_eq!(cut.chars().count(), MAX_TEXT);
        assert_eq!(
            text(&Value::String("  A\u{7}B\nC\u{1b}  ".into())).as_deref(),
            Some("ABC")
        );
        assert_eq!(text(&Value::String(" \t ".into())), None);
        assert_eq!(text(&Value::UnsignedInt(3)), None);
        // The ID3v2 tag's title at the wire's limit goes through whole.
        let title = "t".repeat(MAX_TEXT);
        let mut file = id3_tag(&[id3_frame(b"TIT2", title.as_bytes())], 0);
        file.extend(mp3_frames());
        let tags = read(Counted::new(file)).expect("the tag is read");
        assert_eq!(tags.title, title);
        assert_eq!(tags.artist, "");
    }

    /// Every committed fixture (audio and video, as the parser must survive any bytes): no panic, never a read past
    /// the end, and no title or artist where ffmpeg wrote none (it writes only its encoder line).
    #[test]
    fn the_tag_pass_survives_every_fixture() {
        let manifest = String::from_utf8(fixture("manifest.tsv")).expect("the manifest is text");
        let names: Vec<&str> = manifest
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split('\t').next())
            .collect();
        assert_eq!(names.len(), 26);
        for name in names {
            let reader = Counted::new(fixture(name));
            let tags = read(reader.clone());
            assert_eq!(reader.past_end.load(Ordering::Relaxed), 0, "{name}");
            assert!(reader.read.load(Ordering::Relaxed) <= TAG_READ_MAX, "{name}");
            assert!(
                tags.as_ref().is_none_or(|t| t.title.is_empty() && t.artist.is_empty()),
                "{name}: {tags:?}"
            );
        }
    }

    /// The pass stops after `TAG_READ_MAX` bytes, however long the file says it is.
    #[test]
    fn the_tag_pass_reads_at_most_its_limit() {
        let mut ranges = Ranges::new(Counted::new(vec![0u8; (TAG_READ_MAX + (1 << 20)) as usize]));
        let mut buf = vec![0u8; 1 << 20];
        let mut total = 0u64;
        let err = loop {
            match ranges.read(&mut buf) {
                Ok(0) => panic!("the end came before the limit"),
                Ok(n) => total += n as u64,
                Err(e) => break e,
            }
        };
        assert_eq!(total, TAG_READ_MAX);
        assert_eq!(err.kind(), io::ErrorKind::Other);
        // Past the end a read gives nothing and asks for nothing; before the start is refused.
        let reader = Counted::new(vec![1u8; 100]);
        let mut ranges = Ranges::new(reader.clone());
        assert_eq!(ranges.seek(SeekFrom::End(10)).expect("a seek"), 110);
        assert_eq!(ranges.read(&mut buf).expect("a read"), 0);
        assert_eq!(ranges.seek(SeekFrom::End(-10)).expect("a seek"), 90);
        assert_eq!(ranges.read(&mut buf).expect("a read"), 10);
        assert!(ranges.seek(SeekFrom::Current(-1000)).is_err());
        assert_eq!(reader.past_end.load(Ordering::Relaxed), 0);
    }
}
