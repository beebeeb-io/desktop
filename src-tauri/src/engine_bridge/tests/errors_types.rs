//! One unit test per new type of D-P2-2: `EngineError`, `LandRequest`,
//! `ProducedObject` and `Op`. Each proves the type's contract without a
//! database or network: the Display text a refusal keeps, the boundary
//! conversion, and the newtype's parse/round-trip.

use super::*;

/// `EngineError` keeps the refusal's text and converts to `anyhow::Error` only
/// at the boundary. The text is what the File Provider extension and the
/// person read, so it must be the same as the existing struct's Display.
#[test]
fn engine_error_keeps_the_refusals_text() {
    let error = EngineError::UnknownItem(UnknownItem {
        requested_id: "11111111-1111-1111-1111-111111111111".to_string(),
    });
    assert_eq!(error.to_string(), "this item is not known to Beebeeb");
    let converted: anyhow::Error = error.into();
    assert_eq!(converted.to_string(), "this item is not known to Beebeeb");
}
