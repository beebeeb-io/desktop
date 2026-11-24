//! The part of the macOS Finder setup that is pure policy: the failure record, the error
//! classifier and retry schedule (§6.2, §7), and the launch-location check. No Tauri, no
//! FileProvider.framework; compiled and tested on every platform.
pub mod error;
pub mod launch_location;
pub mod policy;
