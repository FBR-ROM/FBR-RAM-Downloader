//! Time, behind a trait so the simulator can run a 500 ms lock timeout instantly.

use std::time::{Duration, Instant};

pub trait Clock {
    /// Time since an arbitrary fixed start.
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

/// The host's clock: `CLOCK_MONOTONIC_RAW`, which NTP and chrony never slew,
/// so a 500 ms measurement window is 500 ms of hardware time. (A slewed clock
/// can run off by up to several percent and fail a good card.) Falls back to
/// `Instant` if the raw clock is unavailable.
pub struct SystemClock {
    start: Instant,
    raw_start: Option<Duration>,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            raw_start: raw_now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        match (self.raw_start, raw_now()) {
            (Some(start), Some(now)) => now.saturating_sub(start),
            _ => self.start.elapsed(),
        }
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn raw_now() -> Option<Duration> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes one timespec into the struct it is given.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts) };
    let secs = u64::try_from(ts.tv_sec).ok()?;
    let nanos = u32::try_from(ts.tv_nsec).ok()?;
    (rc == 0).then(|| Duration::new(secs, nanos))
}
