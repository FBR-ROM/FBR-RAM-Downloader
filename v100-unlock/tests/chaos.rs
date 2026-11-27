//! Thousands of random runs: 1 to 16 cards of any V100 model, at any
//! starting clock, with random faults and random cancellation. Whatever
//! happens, the hardware must end up where the outcome says it is.
//!
//! Every scenario comes from its seed, so a failure names the seed that
//! reproduces it.

mod common;

use std::time::Duration;

use common::Rig;
use v100_unlock::card::model_for;
use v100_unlock::clock::Clock;
use v100_unlock::outcome::{Kind, Outcome};
use v100_unlock::protocol::Line;
use v100_unlock::regs::HBMPLL_COEFF_MC;
use v100_unlock::run::{Goal, DAILY_NDIV, RESTORE_FROM_MAX_NDIV, SPRINT_NDIV};
use v100_unlock::sim::Fault;

/// Scenarios per test run; `CHAOS_RUNS` raises it for a long soak.
const RUNS: u64 = 3_000;
const MAX_CARDS: u64 = 16;
/// SXM2 16 GB, SXM2 32 GB, PCIe 16 GB, FHHL 16 GB (stock 810 MHz), and V100S (stock 1107 MHz).
const DEVICES: [u16; 5] = [0x1db1, 0x1db5, 0x1db4, 0x1db3, 0x1df6];

/// xorshift64*: small, deterministic, good enough to spread scenarios.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cancel {
    Never,
    /// Once this many coefficient writes have happened across all cards.
    AfterWrites(usize),
    /// Once simulated time passes this point.
    AtTime(Duration),
}

#[derive(Debug)]
struct Scenario {
    specs: Vec<(u16, u8, Fault)>,
    goal: Goal,
    cancel: Cancel,
}

fn scenario(rng: &mut Rng) -> Scenario {
    let cards = 1 + rng.below(MAX_CARDS) as usize;
    // A quarter of scenarios may hold a card that must stop the run before
    // any write; the rest exercise the paths that write.
    let wild = rng.below(4) == 0;
    let specs = (0..cards)
        .map(|_| {
            let device = rng.pick(&DEVICES);
            let stock = model_for(device).unwrap().stock_ndiv;
            let start = start_ndiv(rng, stock, wild);
            (device, start, fault(rng, start, wild))
        })
        .collect();
    let goal = rng.pick(&[Goal::Daily, Goal::Sprint, Goal::Stock]);
    let cancel = match rng.below(4) {
        0 => Cancel::AfterWrites(rng.below(2 * cards as u64 + 1) as usize),
        1 => Cancel::AtTime(Duration::from_micros(rng.below(700_000))),
        _ => Cancel::Never,
    };
    Scenario {
        specs,
        goal,
        cancel,
    }
}

/// Mostly stock, sometimes already overclocked; when `wild`, now and then a
/// clock this tool never sets.
fn start_ndiv(rng: &mut Rng, stock: u8, wild: bool) -> u8 {
    if wild && rng.below(8) == 0 {
        return rng.pick(&[59, 64, 81, 90]);
    }
    rng.pick(&[
        stock,
        stock,
        stock,
        DAILY_NDIV.max(stock),
        SPRINT_NDIV.max(stock),
    ])
}

/// Healthy 80% of the time. Only `wild` scenarios get faults that the
/// survey catches before anything is written.
fn fault(rng: &mut Rng, start: u8, wild: bool) -> Fault {
    if rng.below(10) < 8 {
        return Fault::Healthy;
    }
    let lock_ceiling = (66 + rng.below(15) as u8).max(start);
    let during_run = [
        Fault::NoLockAbove(lock_ceiling),
        Fault::HangOnWrite,
        Fault::SlowMeter,
        Fault::Preempted,
        Fault::IgnoresWrites,
        Fault::LocksOnce,
        Fault::HangOnMeter,
    ];
    let at_survey = [
        Fault::Mixed,
        Fault::DeadPartition,
        Fault::NotGv100,
        Fault::OddDividers,
    ];
    if wild && rng.below(2) == 0 {
        rng.pick(&at_survey)
    } else {
        rng.pick(&during_run)
    }
}

fn target(goal: Goal, stock: u8) -> u8 {
    match goal {
        Goal::Daily => DAILY_NDIV.max(stock),
        Goal::Sprint => SPRINT_NDIV.max(stock),
        Goal::Stock => stock,
    }
}

/// A card that must stop the run before anything is written.
fn must_refuse(device: u16, start: u8, fault: Fault, goal: Goal) -> bool {
    let stock = model_for(device).unwrap().stock_ndiv;
    let ceiling = match goal {
        Goal::Stock => RESTORE_FROM_MAX_NDIV,
        Goal::Daily | Goal::Sprint => SPRINT_NDIV,
    };
    let known = (stock..=ceiling.max(stock)).contains(&start);
    !known
        || matches!(
            fault,
            Fault::Mixed | Fault::DeadPartition | Fault::NotGv100 | Fault::OddDividers
        )
        || matches!(fault, Fault::NoLockAbove(max) if start > max)
}

#[test]
fn every_outcome_matches_the_hardware_it_leaves() {
    let mut kinds = [0usize; 4];
    let runs =
        std::env::var("CHAOS_RUNS").map_or(RUNS, |n| n.parse().expect("CHAOS_RUNS is a number"));
    for seed in 0..runs {
        let mut rng = Rng::new(seed);
        let s = scenario(&mut rng);
        let rig = Rig::new(&s.specs);
        let starts = rig.ndivs();
        let cancelled = || match s.cancel {
            Cancel::Never => false,
            Cancel::AfterWrites(n) => rig.write_count() >= n,
            Cancel::AtTime(t) => rig.clock.now() >= t,
        };
        let (outcome, lines) = rig.go(s.goal, &cancelled);
        let context = format!("seed {seed}: {s:?} -> {outcome:?}");
        check_writes(&s, &rig, &context);
        check_hardware(&s, &rig, &starts, &outcome, &context);
        check_expectations(&s, &outcome, &context);
        check_lines(&lines, &outcome, &context);
        kinds[kind_index(outcome.kind)] += 1;
    }
    // Every kind of ending must actually have been exercised.
    assert!(
        kinds.iter().all(|&n| n >= 100),
        "done/refused/rolled-back/unknown {kinds:?}"
    );
    println!("done/refused/rolled-back/unknown over {runs} runs: {kinds:?}");
}

#[test]
fn dry_run_never_writes_a_register_across_random_scenarios() {
    let mut dry_count = 0usize;
    let mut refused_count = 0usize;
    for seed in 0..500u64 {
        let mut rng = Rng::new(seed ^ 0xD00D_F00D);
        let s = scenario(&mut rng);
        let rig = Rig::new(&s.specs);
        let starts = rig.ndivs();
        let (outcome, lines) = rig.go_dry(s.goal, &common::never, true);
        let context = format!("dry seed {seed}: {s:?} -> {outcome:?}");
        assert_eq!(rig.write_count(), 0, "dry run wrote a register: {context}");
        assert_eq!(rig.ndivs(), starts, "dry run changed a clock: {context}");
        match outcome.kind {
            Kind::Dry => dry_count += 1,
            Kind::Refused => refused_count += 1,
            other => panic!("unexpected dry-run outcome {other:?}: {context}"),
        }
        check_lines(&lines, &outcome, &context);
    }
    assert!(dry_count > 100 && refused_count > 50);
}

fn kind_index(kind: Kind) -> usize {
    match kind {
        Kind::Done => 0,
        Kind::Refused => 1,
        Kind::RolledBack => 2,
        Kind::Unknown => 3,
        Kind::Dry => panic!("the chaos scenarios never dry-run"),
    }
}

/// Only the multicast coefficient is ever written, nothing is ever written
/// into a hung register bus, and a card that will not re-lock gets the
/// overclock only once.
fn check_writes(s: &Scenario, rig: &Rig, context: &str) {
    for (sim, &(_, _, fault)) in rig.sims.iter().zip(&s.specs) {
        let writes = sim.writes();
        assert!(
            writes.iter().all(|&(off, _)| off == HBMPLL_COEFF_MC),
            "{context}"
        );
        assert_eq!(
            sim.writes_while_hung(),
            0,
            "wrote into a hung bus: {context}"
        );
        if fault == Fault::LocksOnce {
            assert!(writes.len() <= 2, "overclock written twice: {context}");
        }
    }
}

/// A card the run could not put back: its bus hung, or it would not re-lock.
fn lost(s: &Scenario, rig: &Rig, i: usize) -> bool {
    let sim = &rig.sims[i];
    sim.hung() || (s.specs[i].2 == Fault::LocksOnce && sim.writes().len() >= 2)
}

/// The cards are where the outcome says.
fn check_hardware(s: &Scenario, rig: &Rig, starts: &[u8], outcome: &Outcome, context: &str) {
    let ends = rig.ndivs();
    match outcome.kind {
        Kind::Done => {
            for (&(device, _, _), &end) in s.specs.iter().zip(&ends) {
                let stock = model_for(device).unwrap().stock_ndiv;
                assert_eq!(end, target(s.goal, stock), "{context}");
            }
        }
        Kind::Refused => {
            assert_eq!(rig.write_count(), 0, "{context}");
            assert_eq!(ends, starts, "{context}");
        }
        Kind::RolledBack => assert_eq!(ends, starts, "{context}"),
        Kind::Unknown => {
            let lost = |i: usize| lost(s, rig, i);
            assert!(
                (0..ends.len()).any(lost),
                "unknown without a lost card: {context}"
            );
            for i in (0..ends.len()).filter(|&i| !lost(i)) {
                assert_eq!(ends[i], starts[i], "healthy card not put back: {context}");
            }
        }
        Kind::Dry => panic!("the chaos scenarios never dry-run"),
    }
}

/// Setups with a known answer get it.
fn check_expectations(s: &Scenario, outcome: &Outcome, context: &str) {
    let refuse = s
        .specs
        .iter()
        .any(|&(d, start, f)| must_refuse(d, start, f, s.goal));
    if refuse {
        assert_eq!(outcome.kind, Kind::Refused, "{context}");
        return;
    }
    let benign = s
        .specs
        .iter()
        .all(|&(_, _, f)| matches!(f, Fault::Healthy | Fault::Preempted));
    if benign && s.cancel == Cancel::Never {
        assert_eq!(outcome.kind, Kind::Done, "{context}");
    }
}

/// Progress only moves forward within 0..=1, and no text carries a control
/// character that could drive a terminal.
fn check_lines(lines: &[Line], outcome: &Outcome, context: &str) {
    let progress = common::steps(lines);
    assert!(
        progress.windows(2).all(|w| w[0] <= w[1]),
        "{progress:?} {context}"
    );
    assert!(
        progress.iter().all(|p| (0.0..=1.0).contains(p)),
        "{context}"
    );
    let texts = lines.iter().filter_map(|line| match line {
        Line::Step { text, .. } | Line::Note(text) => Some(text.as_str()),
        _ => None,
    });
    for text in texts.chain([outcome.summary.as_str()]) {
        assert!(
            !text.is_empty() && !text.chars().any(char::is_control),
            "{text:?} {context}"
        );
    }
}
