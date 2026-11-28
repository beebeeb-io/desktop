//! The ports' facts and config-write rules (ported from the tag's `macos_ports/tests/config.rs`) and the gate rules
//! (the tag's `tests/gate.rs`, on fakes behind the port traits: no test here makes an FFI call).
mod config;
mod gate;
mod wait;
