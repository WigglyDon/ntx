// SPDX-License-Identifier: MIT
//! Bounded observations of the selected native compiler, never guest code/data.
use crate::frame_timing::DurationStats;
use std::{
    fmt,
    time::{Duration, Instant},
};

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
