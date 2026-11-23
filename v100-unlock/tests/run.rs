mod common;

use std::time::Duration;

use common::{never, notes, steps, Rig, OK, V100, V100S};
use v100_unlock::clock::Clock;
use v100_unlock::outcome::Kind;
use v100_unlock::regs::Coeff;
use v100_unlock::run::Goal;
use v100_unlock::sim::Fault;

#[test]
fn any_number_of_cards_goes_to_the_daily_clock() {
    for n in [1, 2, 4, 8, 16] {
        let rig = Rig::v100s(n);
        let (outcome, _) = rig.go(Goal::Daily, &never);
        assert_eq!(outcome.kind, Kind::Done, "{n} cards: {}", outcome.summary);
        assert_eq!(rig.ndivs(), vec![78; n]);
        assert_eq!(rig.write_count(), n, "one write per card");
    }
}

#[test]
fn the_summary_reports_the_change() {
    let (four, _) = Rig::v100s(4).go(Goal::Daily, &never);
    let (one, _) = Rig::v100s(1).go(Goal::Daily, &never);
    assert_eq!(four.summary, "hbm 877.5 → 1053.0 MHz (+20%) · 4 cards");
    assert_eq!(one.summary, "hbm 877.5 → 1053.0 MHz (+20%) · 1 card");
}

#[test]
fn progress_only_moves_forward_and_stays_in_range() {
    let (_, lines) = Rig::v100s(4).go(Goal::Daily, &never);
    let steps = steps(&lines);
    assert!(steps.len() >= 4, "{steps:?}");
    assert!(steps.windows(2).all(|w| w[0] <= w[1]), "{steps:?}");
    assert!(steps.iter().all(|p| (0.0..=1.0).contains(p)), "{steps:?}");
}

#[test]
fn the_receipt_lists_every_card() {
    let (_, lines) = Rig::v100s(2).go(Goal::Daily, &never);
    let notes = notes(&lines);
    assert!(
        notes[0].starts_with("04:00.0 · V100-SXM2-16GB · 877.5 → 1053.0 MHz"),
        "{notes:?}"
    );
    assert!(
        notes[1].starts_with("05:00.0 · V100-SXM2-16GB"),
        "{notes:?}"
    );
    assert!(notes.last().unwrap().contains("reboot"), "{notes:?}");
}

#[test]
fn cards_already_at_the_target_are_left_alone() {
    let rig = Rig::new(&[(V100, 78, OK), (V100, 78, OK)]);
    let (outcome, _) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::Done);
    assert_eq!(outcome.summary, "already at 1053.0 MHz · 2 cards");
    assert_eq!(rig.write_count(), 0);
}

#[test]
fn a_v100s_is_never_clocked_below_its_stock_1107_mhz() {
    let rig = Rig::new(&[(V100, 65, OK), (V100S, 82, OK)]);
    let (outcome, _) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::Done, "{}", outcome.summary);
    assert_eq!(rig.ndivs(), [78, 82]);
    assert_eq!(
        outcome.summary,
        "hbm 877.5 → 1053.0 MHz (+20%) · 1 card · 1 already there"
    );
}

#[test]
fn sprint_goes_to_1080_mhz() {
    let rig = Rig::v100s(2);
    let (outcome, _) = rig.go(Goal::Sprint, &never);
    assert_eq!(rig.ndivs(), [80, 80]);
    assert_eq!(outcome.summary, "hbm 877.5 → 1080.0 MHz (+23%) · 2 cards");
}

#[test]
fn restore_returns_cards_to_stock() {
    let rig = Rig::new(&[(V100, 78, OK), (V100, 80, OK)]);
    let (outcome, lines) = rig.go(Goal::Stock, &never);
    assert_eq!(outcome.kind, Kind::Done);
    assert_eq!(rig.ndivs(), [65, 65]);
    assert!(
        !notes(&lines).iter().any(|n| n.contains("reboot")),
        "no volatility note"
    );
}

#[test]
fn one_card_failing_puts_every_card_back() {
    let rig = Rig::new(&[
        (V100, 65, OK),
        (V100, 65, OK),
        (V100, 65, Fault::NoLockAbove(70)),
        (V100, 65, OK),
    ]);
    let (outcome, _) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::RolledBack, "{}", outcome.summary);
    assert!(outcome.summary.contains("06:00.0"), "{}", outcome.summary);
    assert_eq!(rig.ndivs(), [65; 4]);
    assert!(
        rig.sims[3].writes().is_empty(),
        "the card after the failure is never touched"
    );
}

#[test]
fn a_clock_that_measures_wrong_puts_every_card_back() {
    let rig = Rig::new(&[(V100, 65, OK), (V100, 65, Fault::SlowMeter), (V100, 65, OK)]);
    let (outcome, _) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::RolledBack);
    assert!(outcome.summary.contains("metered"), "{}", outcome.summary);
    assert_eq!(rig.ndivs(), [65; 3]);
}

#[test]
fn cancelling_mid_run_puts_every_card_back() {
    let rig = Rig::v100s(4);
    let after_first_write = || !rig.sims[0].writes().is_empty();
    let (outcome, _) = rig.go(Goal::Daily, &after_first_write);
    assert_eq!(outcome.kind, Kind::RolledBack);
    assert!(outcome.summary.contains("cancelled"), "{}", outcome.summary);
    assert_eq!(rig.ndivs(), [65; 4]);
}

#[test]
fn a_caller_that_leaves_during_the_measurement_still_gets_every_card_back() {
    let rig = Rig::v100s(2);
    // The measurement sleeps 500 ms of simulated time; cancel partway through it.
    let mid_measurement = || rig.clock.now() > Duration::from_millis(250);
    let (outcome, _) = rig.go(Goal::Daily, &mid_measurement);
    assert_eq!(outcome.kind, Kind::RolledBack, "{}", outcome.summary);
    assert_eq!(rig.ndivs(), [65; 2]);
}

#[test]
fn a_hung_card_means_a_power_cycle() {
    let rig = Rig::new(&[(V100, 65, OK), (V100, 65, Fault::HangOnWrite)]);
    let (outcome, lines) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::Unknown);
    assert_eq!(rig.sims[0].ndiv(), 65, "the healthy card is still put back");
    assert!(
        notes(&lines).iter().any(|n| n.contains("power-cycle")),
        "{lines:?}"
    );
}

#[test]
fn unexpected_starting_clocks_are_refused_without_writing() {
    for ndiv in [60, 81, 90] {
        let rig = Rig::new(&[(V100, ndiv, OK)]);
        let (outcome, _) = rig.go(Goal::Daily, &never);
        assert_eq!(outcome.kind, Kind::Refused, "ndiv {ndiv}");
        assert_eq!(rig.write_count(), 0);
    }
}

#[test]
fn a_bad_survey_on_any_card_refuses_the_whole_run() {
    let rig = Rig::new(&[(V100, 65, OK), (V100, 65, Fault::Mixed)]);
    let (outcome, _) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::Refused);
    assert_eq!(rig.write_count(), 0);
}

#[test]
fn no_cards_is_a_refusal() {
    let (outcome, _) = Rig::v100s(0).go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::Refused);
    assert!(
        outcome.summary.contains("no Tesla V100"),
        "{}",
        outcome.summary
    );
}

#[test]
fn putting_a_card_back_never_writes_the_overclock_again() {
    // 04 locks at 1053 MHz but will not lock again; 05 never locks above 70.
    let rig = Rig::new(&[
        (V100, 65, Fault::LocksOnce),
        (V100, 65, Fault::NoLockAbove(70)),
    ]);
    let (outcome, _) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::Unknown, "{}", outcome.summary);
    let written: Vec<u8> = rig.sims[0]
        .writes()
        .iter()
        .map(|&(_, v)| Coeff(v).ndiv())
        .collect();
    assert_eq!(
        written,
        [78, 65],
        "the roll-back writes stock once and stops"
    );
}

#[test]
fn a_card_already_at_the_target_is_still_measured() {
    let rig = Rig::new(&[(V100, 78, Fault::SlowMeter)]);
    let (outcome, _) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::Refused, "{}", outcome.summary);
    assert!(outcome.summary.contains("metered"), "{}", outcome.summary);
    assert_eq!(rig.write_count(), 0);
}

#[test]
fn a_bus_that_hangs_during_the_measurement_is_never_written_again() {
    let rig = Rig::new(&[(V100, 65, OK), (V100, 65, Fault::HangOnMeter)]);
    let (outcome, _) = rig.go(Goal::Daily, &never);
    assert_eq!(outcome.kind, Kind::Unknown, "{}", outcome.summary);
    assert!(
        outcome
            .summary
            .starts_with("a card is in an unknown state, power-cycle the host: "),
        "{}",
        outcome.summary
    );
    assert_eq!(rig.sims[1].writes().len(), 1, "no write into the hung card");
    assert_eq!(rig.sims[0].ndiv(), 65, "the healthy card is put back");
}

#[test]
fn cancelling_before_any_write_changes_nothing() {
    let rig = Rig::v100s(3);
    let (outcome, _) = rig.go(Goal::Daily, &|| true);
    assert_eq!(outcome.kind, Kind::Refused, "{}", outcome.summary);
    assert!(
        outcome.summary.starts_with("cancelled"),
        "{}",
        outcome.summary
    );
    assert_eq!(rig.write_count(), 0);
}

#[test]
fn restore_brings_a_card_down_from_any_clock_up_to_1107_mhz() {
    let rig = Rig::new(&[(V100, 81, OK), (V100, 82, OK)]);
    let (outcome, _) = rig.go(Goal::Stock, &never);
    assert_eq!(outcome.kind, Kind::Done, "{}", outcome.summary);
    assert_eq!(rig.ndivs(), [65, 65]);
}

#[test]
fn a_card_with_unusual_pll_dividers_is_refused() {
    for goal in [Goal::Daily, Goal::Stock] {
        let rig = Rig::new(&[(V100, 65, Fault::OddDividers)]);
        let (outcome, _) = rig.go(goal, &never);
        assert_eq!(outcome.kind, Kind::Refused, "{goal:?}: {}", outcome.summary);
        assert!(outcome.summary.contains("dividers"), "{}", outcome.summary);
        assert_eq!(rig.write_count(), 0);
    }
}

#[test]
fn cards_from_different_clocks_are_summarised_by_where_they_end() {
    let (back, _) = Rig::new(&[(V100, 81, OK), (V100, 78, OK)]).go(Goal::Stock, &never);
    assert_eq!(back.summary, "hbm back to 877.5 MHz · 2 cards");
    let (up, _) = Rig::new(&[(V100, 65, OK), (V100, 80, OK)]).go(Goal::Daily, &never);
    assert_eq!(up.summary, "hbm at 1053.0 MHz · 2 cards");
}
