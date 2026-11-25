//! Pluggable "hack" interface.
//!
//! The TUI drives everything through this trait. Today it's backed by
//! [`StubHack`], which simulates a RAM download so the UI can be developed
//! and demoed standalone. To wire in the real C hack, implement [`Hack`]
//! around your `extern "C"` bindings (worked example in the README) and swap
//! it in in `main.rs`.

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

/// Simulated hack: fills the bar over `total` seconds (at ~30 polls/sec)
/// with fake status text. Frame-based so it's deterministic in both the
/// interactive loop and headless `--dump`. Replace with the real C-backed
/// implementation.
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
