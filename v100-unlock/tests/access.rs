mod common;

use std::fs::OpenOptions;
use std::path::Path;

use v100_unlock::bar0::{Access, Bar0, Regs};
use v100_unlock::error::Error;
use v100_unlock::mmap::MmapBar0;
use v100_unlock::regs::{fbpa_coeff, BAR0_SIZE, HBMPLL_COEFF_MC, PMC_BOOT_0};
use v100_unlock::sim::{Fault, SimCard, SimClock};

#[test]
fn the_write_only_multicast_window_is_never_read() {
    let clock = SimClock::new();
    let sim = SimCard::v100(&clock);
    let regs = Regs::new(&sim, Access::ReadWrite);
    for off in [0x98_8000, HBMPLL_COEFF_MC, 0x98_BFFC] {
        assert_eq!(regs.read(off), Err(Error::Forbidden(off)));
    }
    assert!(regs.read(0x98_7FFC).is_ok(), "just below the window");
    assert!(regs.read(0x98_C000).is_ok(), "just above the window");
}

#[test]
fn misaligned_and_out_of_range_offsets_are_refused() {
    let clock = SimClock::new();
    let sim = SimCard::v100(&clock);
    let regs = Regs::new(&sim, Access::ReadWrite);
    let end = BAR0_SIZE as u32;
    assert_eq!(regs.read(0x90_3C91), Err(Error::Misaligned(0x90_3C91)));
    assert_eq!(regs.read(end), Err(Error::OutOfRange(end)));
    assert_eq!(regs.write(0x90_3C92, 0), Err(Error::Misaligned(0x90_3C92)));
    assert!(sim.writes().is_empty());
}

#[test]
fn a_read_only_mapping_refuses_writes() {
    let clock = SimClock::new();
    let sim = SimCard::v100(&clock);
    let regs = Regs::new(&sim, Access::ReadOnly);
    assert_eq!(regs.write(HBMPLL_COEFF_MC, 0), Err(Error::ReadOnly));
    assert!(sim.writes().is_empty());
}

#[test]
fn the_canary_is_pmc_boot_0() {
    let clock = SimClock::new();
    let v100 = SimCard::v100(&clock);
    assert_eq!(Regs::new(&v100, Access::ReadOnly).canary(), Ok(()));
    let other = SimCard::new(&clock, 0x1db1, 65, Fault::NotGv100);
    assert!(matches!(
        Regs::new(&other, Access::ReadOnly).canary(),
        Err(Error::Canary { .. })
    ));
}

fn bar_file(len: u64) -> std::path::PathBuf {
    let path = common::temp_dir("mmap").join("resource0");
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.set_len(len).unwrap();
    path
}

#[test]
fn the_mmap_backend_maps_a_16_mib_bar() {
    let path = bar_file(BAR0_SIZE as u64);
    {
        let regs = Regs::new(
            MmapBar0::open(&path, Access::ReadWrite).unwrap(),
            Access::ReadWrite,
        );
        regs.write(fbpa_coeff(3), 0x0001_4E02).unwrap();
    }
    let bar = MmapBar0::open(&path, Access::ReadOnly).unwrap();
    assert_eq!(bar.read32(fbpa_coeff(3)), 0x0001_4E02);
    assert_eq!(bar.read32(PMC_BOOT_0), 0);
}

#[test]
fn the_mmap_backend_rejects_a_file_smaller_than_bar0() {
    let path = bar_file(4096);
    assert!(matches!(
        MmapBar0::open(&path, Access::ReadOnly),
        Err(Error::Io(_))
    ));
}

#[test]
fn opening_a_missing_device_names_the_path() {
    let missing = Path::new("/nonexistent/0000:08:00.0/resource0");
    match MmapBar0::open(missing, Access::ReadOnly) {
        Err(Error::Io(msg)) => assert!(msg.contains("resource0"), "{msg}"),
        other => panic!("expected an Io error, got {:?}", other.err()),
    }
}

#[test]
#[should_panic(expected = "write-only multicast window")]
fn the_mmap_backend_itself_refuses_the_multicast_window() {
    let path = bar_file(BAR0_SIZE as u64);
    let bar = MmapBar0::open(&path, Access::ReadOnly).unwrap();
    bar.read32(HBMPLL_COEFF_MC);
}
