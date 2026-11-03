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

#[derive(Default)]
pub struct FrameTimings {
    previous_start: Option<Instant>,
    pub intervals: DurationStats,
    pub lateness: DurationStats,
    /// Includes callbacks and any presentation blocking inside the core call.
    pub core: DurationStats,
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
