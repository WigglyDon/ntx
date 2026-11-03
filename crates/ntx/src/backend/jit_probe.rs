// SPDX-License-Identifier: MIT
//! Bounded observations of the selected native compiler, never guest code/data.
use crate::frame_timing::DurationStats;
use std::{
    fmt,
    time::{Duration, Instant},
};

pub const PHASES: [&str; 11] = [
    "decode",
    "dependencies",
    "allocation",
    "branch",
    "preallocation",
    "dirty",
    "width",
    "assembly",
    "link",
    "expiration",
    "cache",
];
/// Actual duplication of the same bounded scope policy across the selected
/// compiler's ten passes and nested instruction-cache maintenance. Cache time
/// overlaps link/expiration; never sum it with its enclosing pass.
#[derive(Default)]
pub struct CompilerPhases {
    phases: [CompileProbe; 11],
    unknown_events: u32,
    cache_features: Option<(u32, u32)>,
}
impl CompilerPhases {
    pub fn collecting(&self) -> bool {
        self.phases.iter().any(CompileProbe::collecting)
    }
    pub fn begin_frame(&mut self) {
        for phase in &mut self.phases {
            phase.begin_frame();
        }
    }
    pub fn end_frame(&mut self, elapsed: Duration) {
        for phase in &mut self.phases {
            phase.end_frame(elapsed);
        }
    }
    pub fn event(&mut self, tag: u32) {
        let event = tag & 255;
        if !(16..38).contains(&event) || (event != 37 && tag >> 8 != 0) || tag >> 10 != 0 {
            self.unknown_events = self.unknown_events.saturating_add(1);
            return;
        }
        let index = ((event - 16) / 2) as usize;
        if event == 37 && self.phases[index].active {
            let flags = (tag >> 8) & 3;
            self.cache_features = Some(
                self.cache_features
                    .map_or((flags, flags), |(all, any)| (all & flags, any | flags)),
            );
        }
        self.phases[index].event(event & 1);
    }
    pub fn summaries(&self) -> Vec<(&'static str, String)> {
        PHASES
            .iter()
            .copied()
            .zip(self.phases.iter().map(ToString::to_string))
            .collect()
    }
    pub fn coverage(&self) -> String {
        format!(
            "unknown_events={} cache_ctr_features={:?} (intersection,union of IDC/DIC; no policy inferred)",
            self.unknown_events, self.cache_features
        )
    }
}

#[derive(Default)]
pub struct CompileProbe {
    frames: u32,
    events: u32,
    scopes: u32,
    invalid_frames: u32,
    compile_frames: u32,
    slow_core: u32,
    slow_with_compile: u32,
    active: bool,
    clipped: bool,
    invalid: bool,
    start: Option<Instant>,
    frame_time: Duration,
    frame_scopes: u32,
    frame_compile: DurationStats,
    first_scopes: DurationStats,
    slow_compile: DurationStats,
    slow_residual: DurationStats,
}
impl CompileProbe {
    const EVENT_LIMIT: u32 = 131_072;
    pub fn collecting(&self) -> bool {
        self.frames < DurationStats::LIMIT && self.events < Self::EVENT_LIMIT
    }
    pub fn begin_frame(&mut self) {
        self.active = self.collecting();
        self.clipped = false;
        self.invalid = false;
        self.start = None;
        self.frame_time = Duration::ZERO;
        self.frame_scopes = 0;
        if self.active {
            self.frames += 1;
        }
    }
    /// The native wrapper reports only begin=1/end=0; no guest address crosses
    /// the ABI. No Android/JNI or host frame manipulation runs on its stack.
    pub fn event(&mut self, entering: u32) {
        if !self.active {
            return;
        }
        if self.events == Self::EVENT_LIMIT {
            self.clipped = true;
            return;
        }
        self.event_at(entering, Instant::now());
    }
    fn event_at(&mut self, entering: u32, now: Instant) {
        if !self.active {
            return;
        }
        if self.events == Self::EVENT_LIMIT {
            self.clipped = true;
            return;
        }
        self.events += 1;
        match entering {
            1 if self.start.is_none() => self.start = Some(now),
            0 if self.start.is_some() => {
                let elapsed = now.checked_duration_since(self.start.take().unwrap());
                if let Some(elapsed) = elapsed {
                    self.frame_time = self.frame_time.saturating_add(elapsed);
                    self.frame_scopes += 1;
                    self.scopes += 1;
                    self.first_scopes.record(elapsed);
                } else {
                    self.invalid = true;
                }
            }
            // Nesting, unbalanced ends or a changed event ABI are unqualified;
            // exclude that frame rather than double-counting or forcing a match.
            _ => self.invalid = true,
        }
    }
    pub fn end_frame(&mut self, core_elapsed: Duration) {
        if !self.active {
            return;
        }
        self.active = false;
        if core_elapsed > Duration::from_millis(40) {
            self.slow_core += 1;
        }
        if self.invalid || self.clipped || self.start.is_some() || self.frame_time > core_elapsed {
            self.invalid_frames += 1;
            return;
        }
        self.frame_compile.record(self.frame_time);
        self.compile_frames += u32::from(self.frame_scopes != 0);
        if core_elapsed > Duration::from_millis(40) {
            self.slow_with_compile += u32::from(self.frame_scopes != 0);
            self.slow_compile.record(self.frame_time);
            self.slow_residual.record(core_elapsed - self.frame_time);
        }
    }
}
impl fmt::Display for CompileProbe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "frames={} events={} scopes={} invalid_frames={} event_limit={} compile_frames={} slow_core={} slow_with_compile={} frame_compile[{}] slow_compile[{}] slow_residual[{}] first4096scopes[{}]",
            self.frames,
            self.events,
            self.scopes,
            self.invalid_frames,
            Self::EVENT_LIMIT,
            self.compile_frames,
            self.slow_core,
            self.slow_with_compile,
            self.frame_compile,
            self.slow_compile,
            self.slow_residual,
            self.first_scopes
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiler_phases_route_nested_cache_separately_and_bound_feature_metadata() {
        let mut p = CompilerPhases::default();
        p.begin_frame();
        p.event(33); // Link begins.
        p.event(37 | (3 << 8));
        p.event(36); // Cache ends, link remains active.
        p.event(37 | (1 << 8));
        p.event(36);
        p.event(32); // Link ends.
        p.end_frame(Duration::MAX);
        assert_eq!(p.phases[8].scopes, 1);
        assert_eq!(p.phases[10].scopes, 2);
        assert_eq!(p.cache_features, Some((1, 3)));
        assert!(p.phases.iter().all(|phase| phase.invalid_frames == 0));
        assert_eq!(p.summaries().len(), 11);
        p.event(17 | (1 << 8)); // Flags belong only to cache begin.
        p.event(255);
        assert_eq!(p.unknown_events, 2);
        for _ in 1..DurationStats::LIMIT {
            p.begin_frame();
            p.end_frame(Duration::ZERO);
        }
        assert!(!p.collecting());
        p.event(37 | (2 << 8));
        p.event(36);
        assert_eq!(p.cache_features, Some((1, 3)));
        assert_eq!(p.phases[10].scopes, 2);
    }
    #[test]
    fn exact_scopes_are_correlated_only_with_their_own_core_call() {
        let now = Instant::now();
        let ms = Duration::from_millis;
        let mut p = CompileProbe::default();
        p.event_at(1, now); // Outside a core observation is not measured.
        p.begin_frame();
        for (start, end) in [(0, 30), (31, 81)] {
            p.event_at(1, now + ms(start));
            p.event_at(0, now + ms(end));
        }
        p.end_frame(ms(100));
        p.begin_frame();
        p.end_frame(ms(70));
        assert_eq!(
            (p.frames, p.events, p.scopes, p.invalid_frames),
            (2, 4, 2, 0)
        );
        assert_eq!((p.slow_core, p.slow_with_compile), (2, 1));
        assert!(
            p.slow_compile
                .to_string()
                .contains("mean_us=40000 max_us=80000")
        );
        assert!(
            p.slow_residual
                .to_string()
                .contains("mean_us=45000 max_us=70000")
        );
        assert!(
            p.first_scopes
                .to_string()
                .contains("n=2 mean_us=40000 max_us=50000")
        );
        for _ in p.frames..DurationStats::LIMIT {
            p.begin_frame();
            p.end_frame(Duration::ZERO);
        }
        assert!(!p.collecting());
        let frozen = p.to_string();
        p.begin_frame();
        p.event_at(1, now);
        p.event_at(0, now + ms(1000));
        p.end_frame(ms(2000));
        assert_eq!(p.to_string(), frozen);
    }
    #[test]
    fn unbalanced_nested_backward_and_clipped_events_exclude_the_frame() {
        let now = Instant::now();
        let ms = Duration::from_millis;
        for events in [vec![0], vec![1], vec![1, 1, 0, 0], vec![2]] {
            let mut p = CompileProbe::default();
            p.begin_frame();
            for event in events {
                p.event_at(event, now);
            }
            p.end_frame(ms(50));
            assert_eq!(p.invalid_frames, 1);
            assert_eq!(p.slow_residual.samples(), 0);
        }
        let mut p = CompileProbe::default();
        p.begin_frame();
        p.event_at(1, now + ms(1));
        p.event_at(0, now);
        p.end_frame(ms(50));
        assert_eq!(p.invalid_frames, 1);
        let mut p = CompileProbe::default();
        p.begin_frame();
        p.events = CompileProbe::EVENT_LIMIT - 1;
        p.event_at(1, now);
        p.event_at(0, now + ms(1));
        p.end_frame(ms(50));
        assert_eq!(p.invalid_frames, 1);
        assert!(!p.collecting());
        assert_eq!(p.slow_residual.samples(), 0);
    }
}
