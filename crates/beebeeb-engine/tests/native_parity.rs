//! The native-parity corpus (1612) as an engine integration test: an in-memory two-account server
//! (`fixture`) and the engine's real API client, sync bridge and state db run against it (`bridge`).
//! The corpus itself stays in `fixtures/native-parity/` and is read in place.
#[path = "native_parity/bridge.rs"]
mod bridge;
#[path = "native_parity/fixture.rs"]
mod fixture;
