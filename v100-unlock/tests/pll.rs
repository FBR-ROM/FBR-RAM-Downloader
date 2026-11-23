use std::time::Duration;

use v100_unlock::bar0::{Access, Regs};
use v100_unlock::error::Error;
use v100_unlock::meter::{measure, on_target, rates};
use v100_unlock::pll::{set_coeff, survey};
use v100_unlock::regs::{Coeff, HBMPLL_COEFF_MC};
use v100_unlock::sim::{Fault, SimCard, SimClock};

const STOCK: Coeff = Coeff(0x0001_4102);
const DAILY: Coeff = Coeff(0x0001_4E02);
const MC: u32 = HBMPLL_COEFF_MC;

#[test]
fn survey_reads_the_one_locked_coefficient() {
    let clock = SimClock::new();
    let sim = SimCard::v100(&clock);
    assert_eq!(survey(&Regs::new(&sim, Access::ReadOnly)), Ok(STOCK));
}

#[test]
fn survey_refuses_partitions_that_disagree_or_do_not_answer() {
    let clock = SimClock::new();
    let mixed = SimCard::new(&clock, 0x1db1, 65, Fault::Mixed);
    let dead = SimCard::new(&clock, 0x1db1, 65, Fault::DeadPartition);
    assert!(matches!(
        survey(&Regs::new(&mixed, Access::ReadOnly)),
        Err(Error::Mixed { .. })
    ));
    assert!(matches!(
        survey(&Regs::new(&dead, Access::ReadOnly)),
        Err(Error::Partition { index: 5, .. })
    ));
}

#[test]
fn a_clock_change_is_one_multicast_write() {
    let clock = SimClock::new();
    let sim = SimCard::v100(&clock);
    let regs = Regs::new(&sim, Access::ReadWrite);

    let locked = set_coeff(&regs, STOCK, DAILY, &clock).unwrap();

    assert_eq!(sim.writes(), [(MC, DAILY.0)]);
    assert_eq!(survey(&regs), Ok(DAILY));
    assert!(
        locked.elapsed < Duration::from_millis(1),
        "{:?}",
        locked.elapsed
    );
}

#[test]
fn a_pll_that_will_not_lock_is_put_back() {
    let clock = SimClock::new();
    let sim = SimCard::new(&clock, 0x1db1, 65, Fault::NoLockAbove(70));
    let regs = Regs::new(&sim, Access::ReadWrite);

    let err = set_coeff(&regs, STOCK, DAILY, &clock).unwrap_err();

    assert_eq!(err, Error::NotLocked { coeff: DAILY.0 });
    assert_eq!(sim.writes(), [(MC, DAILY.0), (MC, STOCK.0)]);
    assert_eq!(survey(&regs), Ok(STOCK));
}

#[test]
fn a_hung_register_bus_stops_all_access_to_that_card() {
    let clock = SimClock::new();
    let sim = SimCard::new(&clock, 0x1db1, 65, Fault::HangOnWrite);
    let regs = Regs::new(&sim, Access::ReadWrite);

    let err = set_coeff(&regs, STOCK, DAILY, &clock).unwrap_err();

    assert!(matches!(err, Error::Stranded(_)), "{err}");
    assert_eq!(
        sim.writes(),
        [(MC, DAILY.0)],
        "no restore write into a hung bus"
    );
}

#[test]
fn rates_survive_the_36_bit_counter_wrap() {
    let before = [(1u64 << 36) - 1_000; 8];
    let after = [500; 8];
    for mhz in rates(&before, &after, 1e-6) {
        assert!((mhz - 1500.0).abs() < 1e-6, "{mhz}");
    }
}

#[test]
fn the_meter_reads_the_live_memory_clock() {
    let clock = SimClock::new();
    let sim = SimCard::v100(&clock);
    let regs = Regs::new(&sim, Access::ReadWrite);

    let before = measure(&[&regs], &clock).remove(0).unwrap();
    set_coeff(&regs, STOCK, DAILY, &clock).unwrap();
    let after = measure(&[&regs], &clock).remove(0).unwrap();

    assert!(on_target(&before, 877.5), "{before:?}");
    assert!(on_target(&after, 1053.0), "{after:?}");
}

#[test]
fn on_target_allows_one_mhz_on_every_pll() {
    assert!(on_target(&[1053.9; 8], 1053.0));
    let one_low = [
        1053.0, 1053.0, 1053.0, 1053.0, 1053.0, 1053.0, 1053.0, 1051.9,
    ];
    assert!(!on_target(&one_low, 1053.0));
}

#[test]
fn a_preempted_counter_read_does_not_skew_the_measurement() {
    let clock = SimClock::new();
    let sim = SimCard::new(&clock, 0x1db1, 65, Fault::Preempted);
    let regs = Regs::new(&sim, Access::ReadOnly);

    let rates = measure(&[&regs], &clock).remove(0).unwrap();

    for mhz in rates {
        assert!((mhz - 877.5).abs() < 0.01, "{rates:?}");
    }
}

#[test]
fn a_write_that_does_not_stick_is_caught() {
    let clock = SimClock::new();
    let sim = SimCard::new(&clock, 0x1db1, 65, Fault::IgnoresWrites);
    let regs = Regs::new(&sim, Access::ReadWrite);

    let err = set_coeff(&regs, STOCK, DAILY, &clock).unwrap_err();

    assert_eq!(
        err,
        Error::Readback {
            want: DAILY.0,
            got: STOCK.0
        }
    );
    assert_eq!(survey(&regs), Ok(STOCK));
}

#[test]
fn status_reports_a_broken_card_without_hiding_the_good_ones() {
    use v100_unlock::card::{model_for, Card};
    use v100_unlock::run::Unit;
    use v100_unlock::status::status;

    let clock = SimClock::new();
    let good = SimCard::v100(&clock);
    let dead = SimCard::new(&clock, 0x1db1, 65, Fault::DeadPartition);
    let tuned = SimCard::new(&clock, 0x1db1, 78, Fault::Healthy);
    let unit = |bus: u8, sim| Unit {
        card: Card {
            bdf: format!("0000:{bus:02x}:00.0").parse().unwrap(),
            model: model_for(0x1db1).unwrap(),
        },
        regs: Regs::new(sim, Access::ReadOnly),
    };
    let units = [unit(4, &good), unit(6, &dead), unit(8, &tuned)];

    let (lines, all_ok) = status(&units, &clock);

    assert!(!all_ok);
    assert!(
        lines[0].starts_with("04:00.0 · V100-SXM2-16GB · 877.5 MHz (NDIV 65) · metered 877.5"),
        "{lines:?}"
    );
    assert!(
        lines[1].contains("memory partition 5 is not answering"),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with("08:00.0 · V100-SXM2-16GB · 1053.0 MHz (NDIV 78) · metered 1053.0"),
        "{lines:?}"
    );
    assert!(good.writes().is_empty() && dead.writes().is_empty() && tuned.writes().is_empty());
}

#[test]
fn status_flags_a_clock_that_measures_wrong() {
    use v100_unlock::card::{model_for, Card};
    use v100_unlock::run::Unit;
    use v100_unlock::status::status;

    let clock = SimClock::new();
    let slow = SimCard::new(&clock, 0x1db1, 65, Fault::SlowMeter);
    let units = [Unit {
        card: Card {
            bdf: "0000:04:00.0".parse().unwrap(),
            model: model_for(0x1db1).unwrap(),
        },
        regs: Regs::new(&slow, Access::ReadOnly),
    }];

    let (lines, all_ok) = status(&units, &clock);

    assert!(!all_ok);
    assert!(lines[0].ends_with("expected 877.5"), "{lines:?}");
}
