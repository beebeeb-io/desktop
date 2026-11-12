//! The parts of the sign-out keep step that only read the state database (spec 2026-10-10, task 1887). The flow,
//! its windows and the macOS keep step stay in the shell (`src-tauri/src/sign_out/`).

pub mod flow_window;
pub mod kept_names;
pub mod kept_set;
#[cfg(target_os = "macos")]
pub mod kept_writer;
pub mod send_phase;
pub mod snapshot;
pub mod waiting;
