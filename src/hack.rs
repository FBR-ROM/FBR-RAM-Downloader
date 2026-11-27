//! Pluggable "hack" interface.
//!
//! The TUI drives everything through this trait. Two implementations are wired
//! in via `make_hack()` in `src/main.rs`: [`crate::v100::V100Hack`] (the real
//! Tesla V100 HBM clock unlock) and [`StubHack`] (the `--demo` simulated
//! download). To wire in a custom C-backed hack, see `ARCHITECTURE.md` §10.

/// Snapshot of hack progress, polled once per frame by the TUI.
#[derive(Debug, Clone)]
pub struct HackState {
    /// 0.0 ..= 1.0
    pub progress: f32,
    /// Human-readable status line (e.g. "fetching 0x7F000000").
    pub status: String,
    /// True when the hack has finished (success).
    pub done: bool,
    /// Set on failure; the TUI shows it and stops.
    pub error: Option<String>,
    /// The completion message, shown where the bar was and left blinking
    /// after the blow-away. `None` keeps the default "RAM UNLOCKED • DONE".
    pub done_message: Option<String>,
}

/// The contract the TUI needs from the hack. Object-safe so it can be
/// held as `&mut dyn Hack`.
pub trait Hack {
    /// Begin the operation. Called once before the loop.
    fn start(&mut self) -> Result<(), String>;
    /// Poll current progress. Must be non-blocking (return the latest state).
    fn poll(&mut self) -> HackState;
    /// Clean up / tear down. Called on exit (success, abort, or error).
    fn finish(&mut self) -> Result<(), String>;
}

/// Simulated hack: fills the bar over 12 seconds (at ~30 polls/sec) with fake
/// status text. Frame-based so it's deterministic in both `--demo` and
/// headless `--dump`.
pub struct StubHack {
    polls: u32,
    total_polls: u32,
    progress: f32,
    done: bool,
}

impl StubHack {
    pub fn new() -> Self {
        // 12 seconds at 30 polls/sec.
        Self {
            polls: 0,
            total_polls: 360,
            progress: 0.0,
            done: false,
        }
    }
}

impl Hack for StubHack {
    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> HackState {
        if !self.done {
            self.polls = self.polls.saturating_add(1);
            self.progress = (self.polls as f32 / self.total_polls as f32).min(1.0);
            if self.progress >= 1.0 {
                self.done = true;
            }
        }
        let status = match self.progress {
            p if p < 0.12 => "enumerating VRAM banks".into(),
            p if p < 0.38 => "fetching 0x7F000000".into(),
            p if p < 0.66 => "mapping pages".into(),
            p if p < 0.92 => "verifying checksums".into(),
            _ => "finalizing".into(),
        };
        HackState {
            progress: self.progress,
            status,
            done: self.done,
            error: None,
            done_message: None,
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn stub_hack_advances_monotonically_through_all_phases_and_finishes() {
        let mut h = StubHack::new();
        assert_eq!(h.start(), Ok(()));
        let mut prev = 0.0f32;
        let mut statuses = HashSet::new();
        for step in 1..=360 {
            let s = h.poll();
            assert!(s.progress >= prev && s.progress <= 1.0);
            assert!(s.error.is_none());
            statuses.insert(s.status.clone());
            if step < 360 {
                assert!(!s.done);
            } else {
                assert!(s.done);
                assert_eq!(s.progress, 1.0);
            }
            prev = s.progress;
        }
        assert_eq!(statuses.len(), 5);
        // Subsequent polls stay done and clamped.
        let after = h.poll();
        assert!(after.done && after.progress == 1.0);
        assert_eq!(h.finish(), Ok(()));
    }
}
