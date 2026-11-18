//! Diagnostics > App activity (DW-0790): this process's CPU and memory and the upload and download rates, sampled
//! every 2 seconds while the pane is shown and not otherwise. Nothing leaves the PC: the probe reads this process
//! only (sysinfo) and the engine's own latest bandwidth sample; no file or network is touched.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::frames::settings::diagnostics::{Activity, Measure};

/// How often the open pane takes a sample.
pub const EVERY: Duration = Duration::from_secs(2);
/// How many samples a sparkline keeps (one minute).
pub const KEPT: usize = 30;

/// One reading of this app's use of the PC.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Reading {
    /// Share of the whole PC's CPU, in percent.
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub up_bytes_per_s: u64,
    pub down_bytes_per_s: u64,
}

/// Where a reading comes from (the real process, or a fake in a test).
pub trait Probe: Send {
    fn read(&mut self) -> Reading;
}

/// This process, through sysinfo. Rates are the engine's, filled by the caller.
pub struct ProcessProbe {
    system: sysinfo::System,
    pid: Option<sysinfo::Pid>,
    cores: f32,
}

impl ProcessProbe {
    pub fn new() -> Self {
        Self {
            system: sysinfo::System::new(),
            pid: sysinfo::get_current_pid().ok(),
            cores: std::thread::available_parallelism().map_or(1.0, |n| n.get() as f32),
        }
    }
}

impl Default for ProcessProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe for ProcessProbe {
    fn read(&mut self) -> Reading {
        let Some(pid) = self.pid else {
            return Reading::default();
        };
        let kind = sysinfo::ProcessRefreshKind::nothing().with_cpu().with_memory();
        self.system
            .refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), true, kind);
        self.system.process(pid).map_or_else(Reading::default, |p| Reading {
            cpu_percent: (p.cpu_usage() / self.cores).clamp(0.0, 100.0),
            memory_bytes: p.memory(),
            ..Reading::default()
        })
    }
}

/// The four sparklines, filled only while the pane is shown.
pub struct AppActivity {
    probe: Box<dyn Probe>,
    every: Duration,
    shown: bool,
    last: Option<Instant>,
    now: Reading,
    cpu: VecDeque<f32>,
    memory: VecDeque<f32>,
    up: VecDeque<f32>,
    down: VecDeque<f32>,
}

impl AppActivity {
    pub fn new() -> Self {
        Self::with_probe(Box::new(ProcessProbe::new()), EVERY)
    }

    pub fn with_probe(probe: Box<dyn Probe>, every: Duration) -> Self {
        Self {
            probe,
            every,
            shown: false,
            last: None,
            now: Reading::default(),
            cpu: VecDeque::new(),
            memory: VecDeque::new(),
            up: VecDeque::new(),
            down: VecDeque::new(),
        }
    }

    /// Whether the sampler runs (the pane is shown).
    pub fn running(&self) -> bool {
        self.shown
    }

    /// The pane was shown: sampling starts from nothing (the old samples are of another visit).
    pub fn show(&mut self) {
        if !self.shown {
            *self = Self::with_probe(std::mem::replace(&mut self.probe, Box::new(Nothing)), self.every);
            self.shown = true;
        }
    }

    /// The pane was left: sampling stops and the samples go.
    pub fn hide(&mut self) {
        if self.shown {
            *self = Self::with_probe(std::mem::replace(&mut self.probe, Box::new(Nothing)), self.every);
        }
    }

    /// One tick at `at`: a sample when the pane is shown and `every` has passed. `rates` is the engine's latest
    /// (up, down) in bytes per second, asked only when a sample is due. `true` when a sample was added.
    pub fn tick(&mut self, at: Instant, rates: impl FnOnce() -> (u64, u64)) -> bool {
        if !self.shown || self.last.is_some_and(|last| at.saturating_duration_since(last) < self.every) {
            return false;
        }
        self.last = Some(at);
        let mut reading = self.probe.read();
        (reading.up_bytes_per_s, reading.down_bytes_per_s) = rates();
        self.now = reading;
        push(&mut self.cpu, reading.cpu_percent);
        push(&mut self.memory, reading.memory_bytes as f32 / 1_000_000.0);
        push(&mut self.up, reading.up_bytes_per_s as f32 / 1000.0);
        push(&mut self.down, reading.down_bytes_per_s as f32 / 1000.0);
        true
    }

    /// How many samples each line holds.
    pub fn samples(&self) -> usize {
        self.cpu.len()
    }

    /// The four rows as the page draws them; none before the first sample.
    pub fn rows(&self) -> Vec<Activity> {
        if self.cpu.is_empty() {
            return Vec::new();
        }
        let row = |measure, now: String, samples: &VecDeque<f32>| Activity {
            measure,
            now,
            samples: samples.iter().copied().collect(),
        };
        vec![
            row(Measure::Cpu, format!("{:.1} %", self.now.cpu_percent), &self.cpu),
            row(Measure::Memory, megabytes(self.now.memory_bytes), &self.memory),
            row(Measure::Upload, rate_words(self.now.up_bytes_per_s), &self.up),
            row(Measure::Download, rate_words(self.now.down_bytes_per_s), &self.down),
        ]
    }
}

impl Default for AppActivity {
    fn default() -> Self {
        Self::new()
    }
}

struct Nothing;
impl Probe for Nothing {
    fn read(&mut self) -> Reading {
        Reading::default()
    }
}

fn push(line: &mut VecDeque<f32>, value: f32) {
    if line.len() == KEPT {
        line.pop_front();
    }
    line.push_back(value);
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes / 1_000_000)
}

/// "410 KB/s", "3.2 MB/s".
pub fn rate_words(bytes_per_s: u64) -> String {
    if bytes_per_s >= 1_000_000 {
        format!("{:.1} MB/s", bytes_per_s as f64 / 1_000_000.0)
    } else {
        format!("{} KB/s", bytes_per_s / 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(u64);
    impl Probe for Fake {
        fn read(&mut self) -> Reading {
            self.0 += 1;
            Reading {
                cpu_percent: 3.14,
                memory_bytes: 212_000_000 + self.0,
                ..Reading::default()
            }
        }
    }

    fn sampler() -> AppActivity {
        AppActivity::with_probe(Box::new(Fake(0)), EVERY)
    }

    #[test]
    fn samples_grow_every_tick_while_shown() {
        let mut a = sampler();
        a.show();
        let t0 = Instant::now();
        for n in 0..5u32 {
            assert!(a.tick(t0 + EVERY * n, || (3_200_000, 410_000)));
            assert_eq!(a.samples(), n as usize + 1);
        }
        let rows = a.rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].now, "3.1 %");
        assert_eq!(rows[1].now, "212 MB");
        assert_eq!(rows[2].now, "3.2 MB/s");
        assert_eq!(rows[3].now, "410 KB/s");
        assert!(rows.iter().all(|r| r.samples.len() == 5));
    }

    #[test]
    fn a_tick_inside_the_interval_adds_nothing() {
        let mut a = sampler();
        a.show();
        let t0 = Instant::now();
        assert!(a.tick(t0, || (0, 0)));
        assert!(!a.tick(t0 + Duration::from_millis(500), || (0, 0)));
        assert_eq!(a.samples(), 1);
    }

    #[test]
    fn closed_the_sampler_stops_and_forgets() {
        let mut a = sampler();
        let t0 = Instant::now();
        assert!(!a.tick(t0, || panic!("never asked while closed")));
        a.show();
        assert!(a.tick(t0, || (0, 0)));
        a.hide();
        assert!(!a.running());
        assert!(!a.tick(t0 + EVERY * 10, || panic!("never asked while closed")));
        assert!(a.rows().is_empty());
        a.show();
        assert_eq!(a.samples(), 0, "a new visit starts empty");
    }

    #[test]
    fn a_sparkline_keeps_the_last_samples_only() {
        let mut a = sampler();
        a.show();
        let t0 = Instant::now();
        for n in 0..(KEPT as u32 + 7) {
            a.tick(t0 + EVERY * n, || (0, 0));
        }
        assert_eq!(a.samples(), KEPT);
    }
}
