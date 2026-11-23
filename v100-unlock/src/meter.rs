//! Measuring the real memory clock, without CUDA.
//!
//! Each HBM PLL has a free-running 36-bit DRAM cycle counter. Two readings a
//! known time apart give the clock. `nvidia-smi` keeps reporting the stock
//! clock after a live change; this is the number to trust.

use std::time::Duration;

use crate::bar0::{Bar0, Regs};
use crate::clock::Clock;
use crate::error::Error;
use crate::regs::{dram_counter_hi, dram_counter_lo, HBM_PLL_COUNT};

/// Sampling window, as `bare_test.sh` uses.
pub const WINDOW: Duration = Duration::from_millis(500);
/// How far each PLL may read from the target and still count as on target.
pub const TOLERANCE_MHZ: f64 = 1.0;
const COUNTER_MOD: u64 = 1 << 36;
/// A counter read takes a few microseconds. One that took longer was
/// interrupted, so its timestamp is uncertain, and it is read again.
const MAX_READ_SPAN: Duration = Duration::from_micros(50);
const READ_TRIES: usize = 5;

pub type Counts = [u64; HBM_PLL_COUNT];
/// MHz per HBM PLL.
pub type Rates = [f64; HBM_PLL_COUNT];

/// One counter value, stamped with the midpoint of the read.
#[derive(Clone, Copy)]
struct Reading {
    count: u64,
    at: Duration,
    span: Duration,
}

type Sample = [Reading; HBM_PLL_COUNT];

/// Measure every card in one shared window, so N cards take as long as one.
/// Every counter is timed by its own readings, so a stalled read or the time
/// spent on other cards adds no error.
pub fn measure<B: Bar0>(cards: &[&Regs<B>], clock: &dyn Clock) -> Vec<Result<Rates, Error>> {
    if cards.is_empty() {
        return Vec::new();
    }
    let first: Vec<Result<Sample, Error>> = cards.iter().map(|regs| sample(regs, clock)).collect();
    clock.sleep(WINDOW);
    first
        .into_iter()
        .zip(cards)
        .map(|(before, regs)| {
            let (before, after) = (before?, sample(regs, clock)?);
            Ok(std::array::from_fn(|j| {
                let secs = after[j].at.saturating_sub(before[j].at).as_secs_f64();
                mhz(before[j].count, after[j].count, secs)
            }))
        })
        .collect()
}

/// MHz per PLL from two counter snapshots `secs` apart, across the 36-bit wrap.
pub fn rates(before: &Counts, after: &Counts, secs: f64) -> Rates {
    std::array::from_fn(|j| mhz(before[j], after[j], secs))
}

/// True when every PLL is within [`TOLERANCE_MHZ`] of `mhz`.
pub fn on_target(rates: &Rates, mhz: f64) -> bool {
    rates.iter().all(|r| (r - mhz).abs() <= TOLERANCE_MHZ)
}

pub fn mean(rates: &Rates) -> f64 {
    rates.iter().sum::<f64>() / rates.len() as f64
}

fn mhz(before: u64, after: u64, secs: f64) -> f64 {
    let cycles = after.wrapping_sub(before) % COUNTER_MOD;
    cycles as f64 / secs / 1e6
}

fn sample<B: Bar0>(regs: &Regs<B>, clock: &dyn Clock) -> Result<Sample, Error> {
    regs.canary()?;
    let mut readings = [Reading {
        count: 0,
        at: Duration::ZERO,
        span: Duration::ZERO,
    }; HBM_PLL_COUNT];
    for (j, reading) in readings.iter_mut().enumerate() {
        *reading = steady_reading(regs, j, clock)?;
    }
    regs.canary()?;
    Ok(readings)
}

/// A reading that was not interrupted, or the last of [`READ_TRIES`].
fn steady_reading<B: Bar0>(regs: &Regs<B>, j: usize, clock: &dyn Clock) -> Result<Reading, Error> {
    let mut reading = timed_reading(regs, j, clock)?;
    for _ in 1..READ_TRIES {
        if reading.span <= MAX_READ_SPAN {
            break;
        }
        reading = timed_reading(regs, j, clock)?;
    }
    Ok(reading)
}

fn timed_reading<B: Bar0>(regs: &Regs<B>, j: usize, clock: &dyn Clock) -> Result<Reading, Error> {
    let before = clock.now();
    let count = read_counter(regs, j)?;
    let span = clock.now().saturating_sub(before);
    Ok(Reading {
        count,
        at: before + span / 2,
        span,
    })
}

/// High, low, high: if the high bits moved, the low word wrapped; read it again.
fn read_counter<B: Bar0>(regs: &Regs<B>, j: usize) -> Result<u64, Error> {
    let hi_before = regs.read(dram_counter_hi(j))?;
    let lo = regs.read(dram_counter_lo(j))?;
    let hi = regs.read(dram_counter_hi(j))?;
    let lo = if hi == hi_before {
        lo
    } else {
        regs.read(dram_counter_lo(j))?
    };
    Ok((u64::from(hi & 0xF) << 32) | u64::from(lo))
}
