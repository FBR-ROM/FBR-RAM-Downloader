//! A simulated GV100, for the tests and for `--simulate`.
//!
//! It has the registers the tool uses: PMC_BOOT_0, the 16 partition PLLs (which
//! lock 100 µs after a coefficient write), the write-only multicast coefficient,
//! and DRAM cycle counters that run at the programmed clock. Faults reproduce
//! the failures the real cards have shown.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::bar0::Bar0;
use crate::clock::Clock;
use crate::regs::{
    dram_counter_hi, dram_counter_lo, fbpa_cfg, fbpa_coeff, Coeff, BOOT0_GV100, FBPA_COUNT,
    HBMPLL_COEFF_MC, HBM_PLL_COUNT, NO_READ, PMC_BOOT_0,
};

const STOCK: Coeff = Coeff(0x0001_4102);
const ODD_DIVIDERS: Coeff = Coeff(0x0001_4103);
const CFG_UNLOCKED: u32 = 0x09;
const CFG_LOCKED: u32 = 0x29;
const LOCK_DELAY: Duration = Duration::from_micros(100);
const PRI_ERROR: u32 = 0xBADF_5040;
const OTHER_CHIP: u32 = 0x1700_00A1;
/// Counters start just below the 36-bit wrap, so every measurement crosses it.
const COUNTER_START: u64 = (1 << 36) - 3_000_000;
const COUNTER_MASK: u64 = (1 << 36) - 1;
const MIXED_PARTITION: usize = 3;
const DEAD_PARTITION: usize = 5;
const SLOW_METER: f64 = 0.95;
/// A preempted card stalls every 7th counter read for 2 ms.
const STALL_EVERY: u64 = 7;
const STALL: Duration = Duration::from_millis(2);

/// The simulator's time. [`SimClock::new`] is virtual: `sleep` advances it
/// instantly, for tests. [`SimClock::realtime`] follows the wall clock, so a
/// `--simulate` run takes as long as a real one.
#[derive(Clone, Default)]
pub struct SimClock {
    now: Rc<Cell<Duration>>,
    real: Option<Instant>,
}

impl SimClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn realtime() -> Self {
        Self {
            now: Rc::default(),
            real: Some(Instant::now()),
        }
    }
}

impl Clock for SimClock {
    fn now(&self) -> Duration {
        match self.real {
            Some(start) => start.elapsed(),
            None => self.now.get(),
        }
    }

    fn sleep(&self, duration: Duration) {
        match self.real {
            Some(_) => std::thread::sleep(duration),
            None => self.now.set(self.now.get() + duration),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Healthy,
    /// The PLLs never lock above this NDIV.
    NoLockAbove(u8),
    /// The register bus hangs on the first coefficient write (reads all-ones).
    HangOnWrite,
    /// One partition holds a different coefficient.
    Mixed,
    /// One partition answers with a PRI error.
    DeadPartition,
    /// The DRAM counters run 5% slow: the clock is not where the coefficient says.
    SlowMeter,
    /// PMC_BOOT_0 reads another chip.
    NotGv100,
    /// Some counter reads stall, as when the host preempts the reader.
    Preempted,
    /// Coefficient writes are accepted but change nothing.
    IgnoresWrites,
    /// The PLLs lock after the first coefficient write, never after a later one.
    LocksOnce,
    /// The register bus hangs when the meter reads a counter after a change.
    HangOnMeter,
    /// The PLL runs with MDIV 3 instead of the stock 2.
    OddDividers,
}

pub struct SimCard {
    device_id: u16,
    fault: Fault,
    clock: SimClock,
    state: RefCell<State>,
    counter_reads: Cell<u64>,
    hung: Cell<bool>,
    writes_while_hung: Cell<usize>,
}

struct State {
    coeff: Coeff,
    lock_at: Duration,
    /// Coefficient writes that took effect.
    programs: u32,
    counter: Counter,
    writes: Vec<(u32, u32)>,
}

#[derive(Clone, Copy)]
struct Counter {
    since: Duration,
    base: u64,
    mhz: f64,
}

impl Counter {
    fn at(&self, t: Duration) -> u64 {
        let elapsed = t.saturating_sub(self.since).as_secs_f64();
        (self.base + (elapsed * self.mhz * 1e6) as u64) & COUNTER_MASK
    }
}

impl SimCard {
    pub fn new(clock: &SimClock, device_id: u16, ndiv: u8, fault: Fault) -> Self {
        let dividers = if fault == Fault::OddDividers {
            ODD_DIVIDERS
        } else {
            STOCK
        };
        let coeff = dividers.with_ndiv(ndiv);
        let counter = Counter {
            since: clock.now(),
            base: COUNTER_START,
            mhz: rate(coeff, fault),
        };
        let state = State {
            coeff,
            lock_at: Duration::ZERO,
            programs: 0,
            counter,
            writes: Vec::new(),
        };
        Self {
            device_id,
            fault,
            clock: clock.clone(),
            state: RefCell::new(state),
            counter_reads: Cell::new(0),
            hung: Cell::new(false),
            writes_while_hung: Cell::new(0),
        }
    }

    /// A healthy V100-SXM2-16GB at stock.
    pub fn v100(clock: &SimClock) -> Self {
        Self::new(clock, 0x1db1, STOCK.ndiv(), Fault::Healthy)
    }

    pub fn device_id(&self) -> u16 {
        self.device_id
    }

    /// Every write so far, as (offset, value).
    pub fn writes(&self) -> Vec<(u32, u32)> {
        self.state.borrow().writes.clone()
    }

    /// Whether the register bus has hung.
    pub fn hung(&self) -> bool {
        self.hung.get()
    }

    /// Writes that arrived after the bus hung. The tool must never make one.
    pub fn writes_while_hung(&self) -> usize {
        self.writes_while_hung.get()
    }

    /// The NDIV the partitions hold.
    pub fn ndiv(&self) -> u8 {
        self.state.borrow().coeff.ndiv()
    }

    fn partition(&self, state: &State, i: usize, off: u32) -> Option<u32> {
        let dead = self.fault == Fault::DeadPartition && i == DEAD_PARTITION;
        let value = if off == fbpa_cfg(i) {
            self.cfg(state)
        } else if off == fbpa_coeff(i) {
            self.coeff(state, i)
        } else {
            return None;
        };
        Some(if dead { PRI_ERROR } else { value })
    }

    fn cfg(&self, state: &State) -> u32 {
        let refuses = match self.fault {
            Fault::NoLockAbove(max) => state.coeff.ndiv() > max,
            Fault::LocksOnce => state.programs > 1,
            _ => false,
        };
        if self.clock.now() >= state.lock_at && !refuses {
            CFG_LOCKED
        } else {
            CFG_UNLOCKED
        }
    }

    fn coeff(&self, state: &State, i: usize) -> u32 {
        if self.fault == Fault::Mixed && i == MIXED_PARTITION {
            state.coeff.with_ndiv(state.coeff.ndiv().wrapping_add(1)).0
        } else {
            state.coeff.0
        }
    }

    fn counter(&self, state: &State, j: usize, off: u32) -> Option<u32> {
        if off != dram_counter_lo(j) && off != dram_counter_hi(j) {
            return None;
        }
        self.maybe_stall();
        let cycles = (state.counter.at(self.clock.now()) + j as u64 * 1_000) & COUNTER_MASK;
        Some(if off == dram_counter_lo(j) {
            cycles as u32
        } else {
            (cycles >> 32) as u32
        })
    }

    fn maybe_stall(&self) {
        if self.fault != Fault::Preempted {
            return;
        }
        let reads = self.counter_reads.get() + 1;
        self.counter_reads.set(reads);
        if reads.is_multiple_of(STALL_EVERY) {
            self.clock.sleep(STALL);
        }
    }

    fn program(&self, state: &mut State, coeff: Coeff) {
        let now = self.clock.now();
        let base = state.counter.at(now);
        state.counter = Counter {
            since: now,
            base,
            mhz: rate(coeff, self.fault),
        };
        state.coeff = coeff;
        state.lock_at = now + LOCK_DELAY;
        state.programs += 1;
        if self.fault == Fault::HangOnWrite {
            self.hung.set(true);
        }
    }

    fn hangs_now(&self, state: &State, off: u32) -> bool {
        let counter =
            (0..HBM_PLL_COUNT).any(|j| off == dram_counter_lo(j) || off == dram_counter_hi(j));
        self.fault == Fault::HangOnMeter && state.programs > 0 && counter
    }
}

fn rate(coeff: Coeff, fault: Fault) -> f64 {
    let factor = if fault == Fault::SlowMeter {
        SLOW_METER
    } else {
        1.0
    };
    coeff.mhz() * factor
}

impl Bar0 for SimCard {
    fn read32(&self, off: u32) -> u32 {
        assert!(
            !NO_READ.contains(&off),
            "read of the write-only multicast window at 0x{off:06X}"
        );
        let state = self.state.borrow();
        if self.hangs_now(&state, off) {
            self.hung.set(true);
        }
        if self.hung.get() {
            return u32::MAX;
        }
        if off == PMC_BOOT_0 {
            return if self.fault == Fault::NotGv100 {
                OTHER_CHIP
            } else {
                BOOT0_GV100
            };
        }
        (0..FBPA_COUNT)
            .find_map(|i| self.partition(&state, i, off))
            .or_else(|| (0..HBM_PLL_COUNT).find_map(|j| self.counter(&state, j, off)))
            .unwrap_or(PRI_ERROR)
    }

    fn write32(&self, off: u32, val: u32) {
        let mut state = self.state.borrow_mut();
        state.writes.push((off, val));
        if self.hung.get() {
            self.writes_while_hung.set(self.writes_while_hung.get() + 1);
        }
        if off == HBMPLL_COEFF_MC && !self.hung.get() && self.fault != Fault::IgnoresWrites {
            self.program(&mut state, Coeff(val));
        }
    }
}
