//! What Drive's settings do besides reading and storing: the speed test's steps, the support bundle's hand-off to the
//! OS (show the file, open a draft) and the words each says. Split from `drive_settings` (its module doc has the
//! rules: session of the press, account and generation on every answer, nothing drawn before the engine answered).

use std::path::Path;

use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::speed_test_api::{PINGS, ROUNDS, SpeedFigure, SpeedStep};

use crate::frames::settings::bandwidth_data::{SpeedTest, Step};

/// The speed test is built and answers, and the test build hides its button (owner Q5, 2026-11-14): the whole
/// "Speed test" list and the "Test connection speed" link in Diagnostics are not drawn while this is `false`.
pub const SPEED_TEST_ON: bool = false;

/// The three steps of the test, in the order they run.
pub const STEPS: [SpeedStep; 3] = [SpeedStep::Latency, SpeedStep::Upload, SpeedStep::Download];

/// What the person is told when a step did not finish. Fixed sentences: never a server's words.
pub fn step_failed(step: SpeedStep, why: AccountCallError) -> String {
    let named = match step {
        SpeedStep::Latency => format!("Beebeeb didn't answer any of the {PINGS} pings."),
        SpeedStep::Upload => format!("Beebeeb didn't take any of the {ROUNDS} uploads."),
        SpeedStep::Download => format!("Beebeeb didn't send any of the {ROUNDS} downloads."),
    };
    match why {
        AccountCallError::SessionEnded => format!("{named} Sign in again and try again."),
        AccountCallError::ServerError | AccountCallError::Unreadable => {
            format!("{named} Beebeeb answered with an error. Try again later.")
        }
        AccountCallError::NotSignedIn | AccountCallError::Offline => {
            format!("{named} Check your internet connection and try again.")
        }
    }
}

/// A finished step as the pane draws it: "24 ms, 18 to 41 ms" or "35.2 Mbps, 3 of 3 . 30 MB".
pub fn done(figure: SpeedFigure) -> Step {
    match figure {
        SpeedFigure::Latency {
            median_ms,
            min_ms,
            max_ms,
            ..
        } => Step::Done(median_ms.to_string(), "ms".into(), format!("{min_ms} to {max_ms} ms")),
        SpeedFigure::Throughput {
            rounds_ok,
            of,
            mbps,
            bytes,
        } => Step::Done(
            format!("{mbps:.1}"),
            "Mbps".into(),
            format!("{rounds_ok} of {of} \u{b7} {} MB", bytes / 1_000_000),
        ),
    }
}

/// The three steps while step `at` of [`STEPS`] has finished `done` of its `of` pings or rounds, `before` it finished.
pub fn running(before: &[Step], at: usize, done: u32, of: u32) -> SpeedTest {
    let waits = ["Waits for the pings", "Waits for the uploads"];
    let steps: [Step; 3] = std::array::from_fn(|i| {
        if i < at {
            before[i].clone()
        } else if i == at {
            Step::Measuring(
                format!("{} of {of}", (done + 1).min(of)),
                done as f32 / of.max(1) as f32,
            )
        } else {
            Step::Waiting(waits[i - 1].into())
        }
    });
    SpeedTest::Running(steps)
}

/// The pings or rounds of a step.
pub fn parts(step: SpeedStep) -> u32 {
    if step == SpeedStep::Latency { PINGS } else { ROUNDS }
}

/// The state of a test just pressed: the first ping not answered yet.
pub fn started() -> SpeedTest {
    running(&[], 0, 0, PINGS)
}

/// The result and the line under it.
pub fn finished(steps: [Step; 3], moved_bytes: u64) -> SpeedTest {
    SpeedTest::Done(
        steps,
        format!(
            "Measured just now, from this PC to Beebeeb's servers in Europe. The test moved {} MB.",
            moved_bytes / 1_000_000
        ),
    )
}

/// The OS's two hand-offs of an exported bundle: show the file, open a draft to support. Replaced in a test.
#[derive(Clone, Copy)]
pub struct Os {
    /// Show the file in File Explorer. `false`: it could not be shown.
    pub reveal: fn(&Path) -> bool,
    /// Open a `mailto:` draft in the person's email app. `false`: no email app opened.
    pub open_draft: fn(&str) -> bool,
}

impl Os {
    pub fn real() -> Self {
        Os {
            reveal: |path| super::super::reveal_in_file_explorer(path).is_ok(),
            open_draft: |url| super::super::open_path(Path::new(url)).is_ok(),
        }
    }
}

/// The address of Beebeeb's support.
pub const SUPPORT: &str = "support@beebeeb.io";

/// A draft to support that names the file and nothing else of the person's (never its folder).
pub fn draft_for(file_name: &str) -> String {
    let subject = "Support bundle";
    let body = format!("I attached the support bundle file {file_name}. What happened:\r\n");
    format!("mailto:{SUPPORT}?subject={}&body={}", percent(subject), percent(&body))
}

/// Percent-encoding for a `mailto:` field: everything but the unreserved characters.
pub fn percent(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
