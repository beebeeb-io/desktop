//! The connection speed test (Settings > Bandwidth, DW-0786), one step per call, on Beebeeb's own endpoints: the same
//! parameters as the `bb` command line and the iPhone, so the figures compare. Latency: 5 pings of
//! `GET /api/v1/health`. Upload and download: 3 rounds of 10 MB each of `POST` and `GET /api/v1/speedtest`.
//!
//! Every call is for the session of the press (`Engine::account_call_for`): the token of a later session never
//! carries it, and a session that moves on ends the step. Each call BLOCKS on the handle's runtime: a worker's.
//! Nothing here holds a name, a path or a file: the upload is zeros and the download is discarded.

use std::time::Instant;

use super::settings_api::AccountCallError;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;

/// The pings of the latency step.
pub const PINGS: u32 = 5;
/// The rounds of an upload or a download step.
pub const ROUNDS: u32 = 3;
/// The bytes of one round.
pub const ROUND_BYTES: u64 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedStep {
    Latency,
    Upload,
    Download,
}

/// What one step measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpeedFigure {
    Latency {
        answered: u32,
        of: u32,
        min_ms: u32,
        median_ms: u32,
        max_ms: u32,
    },
    Throughput {
        rounds_ok: u32,
        of: u32,
        mbps: f64,
        bytes: u64,
    },
}

/// The latency figures of the pings that answered (sorted here); `None`: none answered.
pub fn latency_of(mut ms: Vec<u32>, of: u32) -> Option<SpeedFigure> {
    ms.sort_unstable();
    let (min_ms, max_ms) = (*ms.first()?, *ms.last()?);
    Some(SpeedFigure::Latency {
        answered: ms.len() as u32,
        of,
        min_ms,
        median_ms: ms[ms.len() / 2],
        max_ms,
    })
}

/// Megabits per second for `bytes` moved in `seconds`; `None` when no time passed.
pub fn mbps_of(bytes: u64, seconds: f64) -> Option<f64> {
    (seconds > 0.0).then(|| bytes as f64 * 8.0 / seconds / 1_000_000.0)
}

impl<C: CredentialStores> Engine<C> {
    /// One step of the speed test under `session`. `progress(done, of)` is called after each ping or round, on the
    /// calling thread's runtime. `Err`: nothing answered (or the session moved on), in the call's kind.
    pub fn speed_step_for(
        &self,
        session: SessionGeneration,
        step: SpeedStep,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<SpeedFigure, AccountCallError> {
        self.account_call_for(session, move |api| async move {
            let mut last_error = None;
            match step {
                SpeedStep::Latency => {
                    let mut ms = Vec::new();
                    for done in 1..=PINGS {
                        let at = Instant::now();
                        match api.ping_health().await {
                            Ok(()) => ms.push(at.elapsed().as_millis().min(u128::from(u32::MAX)) as u32),
                            Err(error) => last_error = Some(error),
                        }
                        progress(done, PINGS);
                    }
                    latency_of(ms, PINGS)
                        .ok_or_else(|| last_error.unwrap_or_else(|| anyhow::anyhow!("no ping answered")))
                }
                SpeedStep::Upload | SpeedStep::Download => {
                    let body = (step == SpeedStep::Upload).then(|| vec![0u8; ROUND_BYTES as usize]);
                    let (mut bytes, mut seconds, mut ok) = (0u64, 0f64, 0u32);
                    for done in 1..=ROUNDS {
                        let at = Instant::now();
                        let moved = match &body {
                            Some(data) => api.speedtest_upload(data).await.map(|()| ROUND_BYTES),
                            None => api.speedtest_download(ROUND_BYTES).await.map(|got| got.len() as u64),
                        };
                        match moved {
                            Ok(moved) => {
                                bytes += moved;
                                seconds += at.elapsed().as_secs_f64();
                                ok += 1;
                            }
                            Err(error) => last_error = Some(error),
                        }
                        progress(done, ROUNDS);
                    }
                    match mbps_of(bytes, seconds) {
                        Some(mbps) if ok > 0 => Ok(SpeedFigure::Throughput {
                            rounds_ok: ok,
                            of: ROUNDS,
                            mbps,
                            bytes,
                        }),
                        _ => Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no round finished"))),
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_is_the_sorted_figures_of_the_pings_that_answered() {
        let figure = latency_of(vec![41, 18, 24, 30], 5).unwrap();
        assert_eq!(
            figure,
            SpeedFigure::Latency {
                answered: 4,
                of: 5,
                min_ms: 18,
                median_ms: 30,
                max_ms: 41
            }
        );
        assert_eq!(latency_of(Vec::new(), 5), None, "no ping answered: no figure");
    }

    #[test]
    fn a_megabit_is_a_million_bits_and_no_time_is_no_figure() {
        assert_eq!(mbps_of(10_000_000, 1.0), Some(80.0));
        assert_eq!(mbps_of(10_000_000, 0.0), None);
    }
}
