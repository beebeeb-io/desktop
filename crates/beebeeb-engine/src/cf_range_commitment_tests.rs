//! `cf_range::session_commitment`: which commitment a range session opens its frames against.

use super::*;

/// The commitment of a range session: the current record's for the current version; for a
/// continued older version the id recorded for THAT version, and none when it has none (a v1
/// version), never the current version's. Mutant: falling back to the current commitment.
#[test]
fn a_continued_older_version_is_never_opened_against_the_current_commitment() {
    use beebeeb_core::chunk_stream::VersionCommitment;
    let current = VersionCommitment::new([9; 12], 4).unwrap();
    assert_eq!(
        session_commitment(false, Some(current), Some([3; 12])).unwrap(),
        Some(current)
    );
    assert_eq!(session_commitment(false, None, None).unwrap(), None);
    // An older v2 version: its own id, under the current counter.
    assert_eq!(
        session_commitment(true, Some(current), Some([3; 12])).unwrap(),
        Some(VersionCommitment::new([3; 12], 4).unwrap())
    );
    // An older v1 version under a v2 current version: no commitment, so its frames open as v1.
    assert_eq!(session_commitment(true, Some(current), None).unwrap(), None);
    // An older v2 version under a current record without a commitment keeps its own id.
    assert_eq!(
        session_commitment(true, None, Some([3; 12])).unwrap(),
        Some(VersionCommitment::new([3; 12], 1).unwrap())
    );
}
