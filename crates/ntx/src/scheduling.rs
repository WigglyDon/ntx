// SPDX-License-Identifier: MIT
//! Portable decisions for the first foreground execution slot. Host resources
//! and native instance limits are supplied by adapters, never session identity.
use crate::library::SavedSession;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct HostAvailability {
    pub foreground: bool,
    pub surface_ready: bool,
    pub player_visible: bool,
}

/// Background/unavailable hosts, unresolved native state and a different
/// selected session all pause and silence this slot. More slots can apply the
/// same decision to their own session without changing GameId or SaveProfile.
pub fn should_pause(
    runtime_game: &str,
    selected: Option<&SavedSession>,
    host: HostAvailability,
    restore_pending: bool,
    recovery_pending: bool,
) -> bool {
    restore_pending
        || recovery_pending
        || !host.foreground
        || !host.surface_ready
        || !host.player_visible
        || selected.is_none_or(|s| s.game_id.0 != runtime_game || s.paused || !s.boot_allowed())
}

/// Backend frame deadlines are portable; adapters provide their actual wait
/// primitive and validated console rate. This does not infer measured speed.
pub struct FramePacer {
    next: Instant,
}
impl FramePacer {
    pub fn new(now: Instant) -> Self {
        Self { next: now }
    }
    pub fn reset(&mut self, now: Instant) {
        self.next = now;
    }
    pub fn ready(&self, now: Instant) -> bool {
        now >= self.next
    }
    pub fn schedule(&mut self, now: Instant, fps: f64) -> Result<(), &'static str> {
        if !fps.is_finite() || fps <= 0. {
            return Err("Unsupported frame scheduling rate");
        }
        let period = Duration::try_from_secs_f64(1. / fps)
            .map_err(|_| "Unsupported frame scheduling period")?;
        if period.is_zero() {
            return Err("Frame scheduling period is below clock precision");
        }
        let next = self
            .next
            .checked_add(period)
            .filter(|next| *next > now)
            .or_else(|| now.checked_add(period))
            .ok_or("Frame deadline exceeds host clock range")?;
        // Preserve phase through ordinary wake-up jitter. Once an entire
        // deadline was missed, rebase instead of running a catch-up burst.
        self.next = next;
        Ok(())
    }
    pub fn lateness(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.next)
    }
    pub fn wait(&self, now: Instant, maximum_idle: Duration) -> Duration {
        self.next.saturating_duration_since(now).min(maximum_idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::{Backend, System},
        library::{GameId, SaveProfileId},
    };

    #[test]
    fn foreground_slot_follows_game_identity_across_systems_and_background() {
        let mut gc = SavedSession {
            game_id: GameId("a".repeat(32)),
            profile_id: SaveProfileId("b".repeat(32)),
            system: System::GameCube,
            backend: Backend::Dolphin,
            uri: "file:///owned/gc.rvz".into(),
            title: "GC".into(),
            media_relink_required: false,
            media_sha256: None,
            paused: false,
            last_error: None,
            restart_required: false,
            runtime_incomplete: true,
            stopped: false,
            checkpoint: None,
            shortcut_requested: false,
            shortcut_pinned: None,
        };
        let mut other_gc = gc.clone();
        other_gc.game_id = GameId("e".repeat(32));
        other_gc.profile_id = SaveProfileId("f".repeat(32));
        let mut n64 = gc.clone();
        n64.game_id = GameId("c".repeat(32));
        n64.profile_id = SaveProfileId("d".repeat(32));
        n64.system = System::Nintendo64;
        n64.backend = Backend::Mupen64PlusNext;
        gc.paused = false;
        n64.paused = false;
        let host = HostAvailability {
            foreground: true,
            surface_ready: true,
            player_visible: true,
        };
        assert!(!should_pause(&gc.game_id.0, Some(&gc), host, false, false));
        assert!(should_pause(
            &gc.game_id.0,
            Some(&other_gc),
            host,
            false,
            false
        ));
        assert!(should_pause(&gc.game_id.0, Some(&n64), host, false, false));
        assert!(!should_pause(
            &n64.game_id.0,
            Some(&n64),
            host,
            false,
            false
        ));
        assert!(should_pause(
            &n64.game_id.0,
            Some(&n64),
            HostAvailability {
                foreground: false,
                ..host
            },
            false,
            false
        ));
        assert!(should_pause(&n64.game_id.0, Some(&n64), host, true, false));
        assert!(should_pause(&n64.game_id.0, None, host, false, false));
        assert!(!should_pause(&gc.game_id.0, Some(&gc), host, false, false));
    }

    #[test]
    fn deadline_waits_for_fractional_frame_and_rejects_invalid_rate_without_mutation() {
        let now = Instant::now();
        let mut pacer = FramePacer::new(now);
        assert!(pacer.ready(now));
        pacer.schedule(now, 60.).unwrap();
        assert!(!pacer.ready(now + Duration::from_millis(16)));
        assert!(
            pacer.wait(now + Duration::from_millis(16), Duration::from_millis(8))
                < Duration::from_millis(1)
        );
        assert!(pacer.schedule(now, f64::NAN).is_err());
        assert!(!pacer.ready(now + Duration::from_millis(16)));
        pacer.reset(now);
        assert!(pacer.ready(now));
    }

    #[test]
    fn repeated_small_wakeup_delays_do_not_accumulate_into_frame_clock_drift() {
        let epoch = Instant::now();
        let period = Duration::from_secs_f64(1. / 60.);
        let jitter = Duration::from_micros(500);
        let mut pacer = FramePacer::new(epoch);
        pacer.schedule(epoch, 60.).unwrap();
        let mut now = epoch;
        for _ in 0..120 {
            now += pacer.wait(now, Duration::MAX) + jitter;
            assert!(pacer.ready(now));
            pacer.schedule(now, 60.).unwrap();
            assert!(!pacer.ready(now));
        }
        assert_eq!(pacer.wait(epoch + period * 121, period), Duration::ZERO);
    }

    #[test]
    fn missed_periods_rebase_without_a_catchup_burst_and_resume_resets_phase() {
        let epoch = Instant::now();
        let period = Duration::from_secs_f64(1. / 60.);
        let mut pacer = FramePacer::new(epoch);
        pacer.schedule(epoch, 60.).unwrap();
        let stalled = epoch + Duration::from_secs(2);
        pacer.schedule(stalled, 60.).unwrap();
        assert_eq!(pacer.wait(stalled, Duration::MAX), period);
        assert!(!pacer.ready(stalled));
        let resumed = stalled + Duration::from_secs(2);
        pacer.reset(resumed);
        assert!(pacer.ready(resumed));
        pacer.schedule(resumed, 50.).unwrap();
        assert_eq!(
            pacer.wait(resumed, Duration::MAX),
            Duration::from_millis(20)
        );
    }
}
