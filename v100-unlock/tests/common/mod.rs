//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use v100_unlock::bar0::{Access, Regs};
use v100_unlock::card::{model_for, Card};
use v100_unlock::outcome::Outcome;
use v100_unlock::protocol::Line;
use v100_unlock::run::{run, Goal, Unit};
use v100_unlock::sim::{Fault, SimCard, SimClock};

pub const V100: u16 = 0x1db1;
pub const V100S: u16 = 0x1df6;
pub const OK: Fault = Fault::Healthy;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fresh, empty directory under the system temp dir, unique to this test.
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "v100-unlock-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// The binary under test. `V100_UNLOCK_BIN` overrides the build path, so the
/// suite can run on another machine (the V100 host) from copied binaries.
pub fn bin() -> String {
    std::env::var("V100_UNLOCK_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_v100-unlock").into())
}

/// Simulated cards on PCI buses 04, 05, ... sharing one virtual clock.
pub struct Rig {
    pub clock: SimClock,
    pub sims: Vec<SimCard>,
}

impl Rig {
    /// One card per (device ID, starting NDIV, fault).
    pub fn new(specs: &[(u16, u8, Fault)]) -> Rig {
        let clock = SimClock::new();
        let sims = specs
            .iter()
            .map(|&(id, ndiv, fault)| SimCard::new(&clock, id, ndiv, fault))
            .collect();
        Rig { clock, sims }
    }

    /// `n` healthy V100-SXM2-16GBs at stock.
    pub fn v100s(n: usize) -> Rig {
        Rig::new(&vec![(V100, 65, OK); n])
    }

    pub fn units(&self) -> Vec<Unit<&SimCard>> {
        self.sims
            .iter()
            .enumerate()
            .map(|(i, sim)| Unit {
                card: Card {
                    bdf: format!("0000:{:02x}:00.0", i + 4).parse().unwrap(),
                    model: model_for(sim.device_id()).unwrap(),
                },
                regs: Regs::new(sim, Access::ReadWrite),
            })
            .collect()
    }

    pub fn go(&self, goal: Goal, cancelled: &dyn Fn() -> bool) -> (Outcome, Vec<Line>) {
        let mut lines = Vec::new();
        let outcome = run(&self.units(), goal, &self.clock, cancelled, &mut |line| {
            lines.push(line)
        });
        (outcome, lines)
    }

    pub fn ndivs(&self) -> Vec<u8> {
        self.sims.iter().map(SimCard::ndiv).collect()
    }

    pub fn write_count(&self) -> usize {
        self.sims.iter().map(|s| s.writes().len()).sum()
    }
}

pub fn never() -> bool {
    false
}

pub fn notes(lines: &[Line]) -> Vec<&str> {
    lines
        .iter()
        .filter_map(|line| match line {
            Line::Note(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

pub fn steps(lines: &[Line]) -> Vec<f32> {
    lines
        .iter()
        .filter_map(|line| match line {
            Line::Step { progress, .. } => Some(*progress),
            _ => None,
        })
        .collect()
}
