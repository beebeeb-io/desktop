//! The parts of the sign-out keep step that only read the state database (spec 2026-10-10, task 1887). The flow,
//! its windows and the macOS keep step stay in the shell (`src-tauri/src/sign_out/`).

pub mod kept_set;
pub mod waiting;
