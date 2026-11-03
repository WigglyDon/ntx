// SPDX-License-Identifier: MIT
//! Bounded host observations, separate from emulated frames or display timing.
use std::{
    fmt,
    time::{Duration, Instant},
};

/// Keep only aggregates for the first4096 observations of a runtime. No sample
/// arrays, game identifiers, media paths or per-frame logging are retained.
#[derive(Default)]
pub struct DurationStats {
    samples: u32,
    total_us: u128,
    max_us: u128,
    over_20ms: u32,
    over_40ms: u32,
}
impl DurationStats {
    pub const LIMIT: u32 = 4096;
    pub fn record(&mut self, duration: Duration) {
        if self.samples == Self::LIMIT {
            return;
        }
        let us = duration.as_micros();
        self.samples += 1;
        self.total_us += us;
        self.max_us = self.max_us.max(us);
        self.over_20ms += u32::from(duration > Duration::from_millis(20));
        self.over_40ms += u32::from(duration > Duration::from_millis(40));
    }
    pub fn samples(&self) -> u32 {
        self.samples
    }
}
impl fmt::Display for DurationStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "n={} mean_us={} max_us={} gt20ms={} gt40ms={}",
            self.samples,
            self.total_us / u128::from(self.samples.max(1)),
            self.max_us,
            self.over_20ms,
            self.over_40ms
        )
    }
}

/// Compare clocks from the same core call, never unrelated stage maxima. CPU
/// time belongs to the calling thread; the remainder includes blocking and
/// descheduling, not a claim about a GPU or other worker's execution.
#[derive(Default)]
pub struct CoreWorkTimings {
    frames: u32,
    unavailable: u32,
    invalid: u32,
    slow_calls: u32,
    cpu_majority: u32,
    wall: DurationStats,
    cpu: DurationStats,
    off_thread: DurationStats,
}
impl CoreWorkTimings {
    pub fn collecting(&self) -> bool {
        self.frames < DurationStats::LIMIT
    }
    pub fn record(&mut self, wall: Duration, cpu: Option<Duration>) {
        if !self.collecting() {
            return;
        }
        self.frames += 1;
        let slow = wall > Duration::from_millis(40);
        self.slow_calls += u32::from(slow);
        let Some(cpu) = cpu else {
            self.unavailable += 1;
            return;
        };
        let Some(off_thread) = wall.checked_sub(cpu) else {
            self.invalid += 1;
            return;
        };
        if slow {
            self.cpu_majority += u32::from(cpu.saturating_mul(2) >= wall);
            self.wall.record(wall);
            self.cpu.record(cpu);
            self.off_thread.record(off_thread);
        }
    }
}
impl fmt::Display for CoreWorkTimings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "frames={} unavailable={} invalid={} slow_calls={} slow_cpu_majority={} matched_wall[{}] thread_cpu[{}] off_thread[{}]",
            self.frames,
            self.unavailable,
            self.invalid,
            self.slow_calls,
            self.cpu_majority,
            self.wall,
            self.cpu,
            self.off_thread
        )
    }
}

#[derive(Default)]
pub struct FrameTimings {
    previous_start: Option<Instant>,
    pub intervals: DurationStats,
    pub lateness: DurationStats,
    /// Includes callbacks and any presentation blocking inside the core call.
    pub core: DurationStats,
    pub work: CoreWorkTimings,
    /// Host-stack audio delivery after returning from the core.
    pub audio: DurationStats,
}
impl FrameTimings {
    pub fn start(&mut self, now: Instant, lateness: Duration) {
        if self.lateness.samples() == DurationStats::LIMIT {
            return;
        }
        if let Some(previous) = self.previous_start {
            self.intervals
                .record(now.saturating_duration_since(previous));
        }
        self.previous_start = Some(now);
        self.lateness.record(lateness);
    }
    /// Menus, background, capture and restore gaps are not active-frame cadence.
    pub fn suspend(&mut self) {
        self.previous_start = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn core_cpu_comparison_uses_matched_slow_calls_and_reports_missing_clocks() {
        let mut work = CoreWorkTimings::default();
        let ms = Duration::from_millis;
        for (wall, cpu) in [
            (40, Some(40)),
            (100, Some(90)),
            (80, Some(10)),
            (60, None),
            (50, Some(51)),
        ] {
            work.record(ms(wall), cpu.map(ms));
        }
        assert_eq!((work.frames, work.slow_calls, work.cpu_majority), (5, 4, 1));
        assert_eq!((work.unavailable, work.invalid), (1, 1));
        assert_eq!(
            (work.wall.samples, work.cpu.samples, work.off_thread.samples),
            (2, 2, 2)
        );
        assert_eq!(
            (
                work.wall.total_us,
                work.cpu.total_us,
                work.off_thread.total_us
            ),
            (180_000, 100_000, 80_000)
        );
        assert_eq!(work.off_thread.max_us, 70_000);
        for _ in work.frames..DurationStats::LIMIT {
            work.record(Duration::ZERO, Some(Duration::ZERO));
        }
        let frozen = work.to_string();
        assert!(!work.collecting());
        work.record(Duration::MAX, None);
        assert_eq!(work.to_string(), frozen);
    }
    #[test]
    fn thresholds_are_strict_and_collection_stops_at_its_fixed_bound() {
        let mut stats = DurationStats::default();
        assert!(stats.to_string().contains("mean_us=0"));
        stats.record(Duration::from_millis(20));
        stats.record(Duration::from_millis(40));
        stats.record(Duration::from_millis(41));
        assert_eq!((stats.over_20ms, stats.over_40ms), (2, 1));
        for _ in 3..DurationStats::LIMIT {
            stats.record(Duration::ZERO);
        }
        let frozen = stats.to_string();
        stats.record(Duration::MAX);
        assert_eq!(stats.to_string(), frozen);
        assert_eq!(stats.samples(), DurationStats::LIMIT);
    }
    #[test]
    fn pause_gaps_do_not_enter_active_cadence_or_modify_phase_durations() {
        let now = Instant::now();
        let mut timings = FrameTimings::default();
        timings.start(now, Duration::ZERO);
        timings.start(now + Duration::from_millis(17), Duration::from_micros(300));
        timings.core.record(Duration::from_millis(5));
        timings.suspend();
        timings.start(now + Duration::from_secs(30), Duration::ZERO);
        assert_eq!(timings.intervals.samples(), 1);
        assert_eq!(timings.intervals.max_us, 17_000);
        assert_eq!(timings.lateness.samples(), 3);
        assert_eq!(timings.core.samples(), 1);
    }
}
