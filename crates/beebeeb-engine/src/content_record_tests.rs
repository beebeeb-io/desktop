//! `ContentRecord`: the engine's one way from a file's authenticated record to a reader and a name
//! seal. Each test names the mutant it fails on.

use super::*;
use beebeeb_core::chunk_stream::{ChunkEncryptor, ChunkFormat};
use beebeeb_core::encrypt::decrypt_file_metadata_with_key;
use beebeeb_core::kdf::{MasterKey, derive_file_key};

const ID: &str = "a0000000-0000-4000-8000-0000000000c1";
const CHUNK: u64 = 16;

fn key() -> FileKey {
    derive_file_key(&MasterKey::from_bytes([7; 32]), ID.as_bytes())
}

/// One v2 upload as the engine's writers make it: the frames, and the sealed name `complete`
/// publishes (the commitment follows `previous`).
fn v2_upload(plain: &[u8], name: &str, previous: Option<u64>) -> (Vec<Vec<u8>>, String) {
    let mut writer = ChunkEncryptor::from_reader_with_file_key(
        key(),
        ID,
        plain.len() as u64,
        CHUNK,
        std::io::Cursor::new(plain.to_vec()),
    )
    .unwrap();
    assert_eq!(writer.format(), ChunkFormat::V2, "this core writes v2 only");
    let mut frames = Vec::new();
    while let Some(chunk) = writer.next_chunk().unwrap() {
        frames.push(chunk.data);
    }
    let version = writer.version_commitment_after(previous).unwrap();
    writer.finish().unwrap();
    let sealed = ContentRecord::pin_only(ExpectedChunkFormat::recorded_for(ChunkFormat::V2))
        .seal_name_for_version(&key(), name, Some("text/plain"), version)
        .unwrap();
    (frames, sealed)
}

/// A file as a build on core 0.2.8 and earlier left it: v1 frames (AES-256-GCM under the file key,
/// no associated data) and a name envelope sealed under the file key that records no format.
fn v1_file(plain: &[u8], name: &str) -> (Vec<Vec<u8>>, String) {
    let frames = plain
        .chunks(CHUNK as usize)
        .map(|chunk| beebeeb_core::encrypt::encrypt_chunk_raw(&key(), chunk).unwrap())
        .collect();
    let envelope =
        beebeeb_core::encrypt::encrypt_metadata(&key(), &serde_json::json!({ "name": name }).to_string()).unwrap();
    (frames, serde_json::to_string(&envelope).unwrap())
}

fn record_of(sealed: &str) -> ContentRecord {
    ContentRecord::of(&decrypt_file_metadata_with_key(&key(), sealed).unwrap())
}

fn read_whole(record: ContentRecord, frames: &[Vec<u8>]) -> Result<Vec<u8>, beebeeb_core::CoreError> {
    let mut reader = record.decryptor(key(), ID);
    let mut out = Vec::new();
    for frame in frames {
        out.extend_from_slice(&reader.push_frame(frame)?.data);
    }
    reader.finish()?;
    Ok(out)
}

fn read_seeking(record: ContentRecord, frames: &[Vec<u8>]) -> Result<Vec<u8>, beebeeb_core::CoreError> {
    let mut out = Vec::new();
    // Back to front: a seeking read opens any chunk without the ones before it.
    for (index, frame) in frames.iter().enumerate().rev() {
        let mut chunk = record.open_chunk(&key(), ID, index as u32, index + 1 == frames.len(), frame)?;
        chunk.extend_from_slice(&out);
        out = chunk;
    }
    Ok(out)
}

const PLAIN: &[u8] = b"forty-one bytes of invented file content.";

/// v2 write, then read: the record read from the completion name opens the content, whole and
/// seeking. Mutant: a reader built from the pin alone (`for_push_with_file_key`) reads v1 only.
#[test]
fn a_v2_upload_reads_back_against_the_record_its_completion_name_carries() {
    let (frames, sealed) = v2_upload(PLAIN, "notes.txt", None);
    let record = record_of(&sealed);
    assert_eq!(record.format, ExpectedChunkFormat::V2);
    assert_eq!(record.version.map(|version| version.counter), Some(1));
    assert_eq!(frames.len(), 3);
    assert_eq!(read_whole(record, &frames).unwrap(), PLAIN);
    assert_eq!(read_seeking(record, &frames).unwrap(), PLAIN);
}

/// A file from before chunk format v2 still opens: its record names nothing, and v1 frames open.
/// Mutant: a reader that insists on a commitment refuses every existing file.
#[test]
fn a_v1_file_from_an_older_build_still_reads() {
    let (frames, sealed) = v1_file(PLAIN, "old.txt");
    let record = record_of(&sealed);
    assert_eq!(record, ContentRecord::UNRECORDED);
    assert_eq!(read_whole(record, &frames).unwrap(), PLAIN);
    assert_eq!(read_seeking(record, &frames).unwrap(), PLAIN);
}

/// Core's ruling of 2026-11-07: v2 content opens only against its record's commitment. A record
/// that says v2 and carries none opens nothing, and neither does a record that names nothing.
/// Mutant: a reader that takes the version id from the served frame 0.
#[test]
fn v2_content_without_a_commitment_in_the_record_is_refused() {
    let (frames, sealed) = v2_upload(PLAIN, "notes.txt", None);
    let committed = record_of(&sealed);
    for record in [
        ContentRecord::pin_only(ExpectedChunkFormat::V2),
        ContentRecord::UNRECORDED,
    ] {
        assert!(
            read_whole(record, &frames).is_err(),
            "{record:?} must not open v2 content"
        );
        assert!(
            read_seeking(record, &frames).is_err(),
            "{record:?} must not open v2 chunks"
        );
    }
    // The control: the same frames open with the commitment, so the refusals above are the rule's.
    assert_eq!(read_whole(committed, &frames).unwrap(), PLAIN);
}

/// A record opens its own version only: an older upload of the same file under the same key is
/// refused (a server cannot serve yesterday's content under today's name).
#[test]
fn a_record_does_not_open_another_version_of_the_same_file() {
    let (old_frames, old_sealed) = v2_upload(b"the first version of the file", "notes.txt", None);
    let previous = record_of(&old_sealed).version.map(|version| version.counter);
    let (new_frames, new_sealed) = v2_upload(PLAIN, "notes.txt", previous);
    let current = record_of(&new_sealed);
    assert_eq!(
        current.version.map(|version| version.counter),
        Some(2),
        "a replacement follows the counter"
    );
    assert!(read_whole(current, &old_frames).is_err());
    assert!(read_seeking(current, &old_frames).is_err());
    assert_eq!(read_whole(current, &new_frames).unwrap(), PLAIN);
}

/// A rename seals the new name for the SAME version: pin and commitment carried, so the content
/// still opens. Mutant: `seal_name` sealing from the pin alone drops the commitment, and the file
/// no longer opens after a rename.
#[test]
fn a_renamed_file_keeps_its_commitment_and_still_reads() {
    let (frames, sealed) = v2_upload(PLAIN, "notes.txt", None);
    let before = record_of(&sealed);
    let renamed = before.seal_name(&key(), "renamed.txt", Some("text/plain")).unwrap();
    let opened = decrypt_file_metadata_with_key(&key(), &renamed).unwrap();
    assert_eq!(opened.name, "renamed.txt");
    assert_eq!(
        ContentRecord::of(&opened),
        before,
        "the record is carried forward unchanged"
    );
    assert_eq!(read_whole(ContentRecord::of(&opened), &frames).unwrap(), PLAIN);
}

/// A rename of a v1 file records no commitment and no v2 pin: its content stays v1 and readable.
#[test]
fn a_renamed_v1_file_stays_a_v1_file() {
    let (frames, sealed) = v1_file(PLAIN, "old.txt");
    let renamed = record_of(&sealed).seal_name(&key(), "older.txt", None).unwrap();
    let record = record_of(&renamed);
    assert_eq!(record, ContentRecord::UNRECORDED);
    assert_eq!(read_whole(record, &frames).unwrap(), PLAIN);
}

/// A new version of a file with v1 history: v2 content under the kept unrecorded pin, with the
/// commitment (core never records `chunk_format: 2` on a file id with v1 history).
#[test]
fn a_new_version_of_a_v1_file_is_v2_under_the_unrecorded_pin() {
    let (_, old_sealed) = v1_file(b"old", "old.txt");
    let old = record_of(&old_sealed);
    let mut writer = ChunkEncryptor::from_reader_with_file_key(
        key(),
        ID,
        PLAIN.len() as u64,
        CHUNK,
        std::io::Cursor::new(PLAIN.to_vec()),
    )
    .unwrap()
    .with_recorded_pin(old.format)
    .unwrap();
    let mut frames = Vec::new();
    while let Some(chunk) = writer.next_chunk().unwrap() {
        frames.push(chunk.data);
    }
    let version = writer.version_commitment_after(None).unwrap();
    writer.finish().unwrap();
    let sealed = ContentRecord::pin_only(old.format)
        .seal_name_for_version(&key(), "old.txt", None, version)
        .unwrap();
    let record = record_of(&sealed);
    assert_eq!(record.format, ExpectedChunkFormat::Unrecorded);
    assert!(record.version.is_some());
    assert_eq!(read_whole(record, &frames).unwrap(), PLAIN);
}
