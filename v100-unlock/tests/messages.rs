//! What people read when something goes wrong: every error names the
//! register or value involved, and every hint is the right one.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use v100_unlock::bar0::Access;
use v100_unlock::card::{discover, model_for, Card};
use v100_unlock::clock::{Clock, SystemClock};
use v100_unlock::error::Error;
use v100_unlock::mmap::MmapBar0;

#[test]
fn every_error_names_what_went_wrong() {
    let cases = [
        (Error::Forbidden(0x98_BC98), "0x98BC98"),
        (
            Error::Misaligned(0x90_3C91),
            "0x903C91 is not a 32-bit register offset",
        ),
        (Error::OutOfRange(0x100_0000), "0x1000000 is outside BAR0"),
        (Error::ReadOnly, "read-only"),
        (
            Error::Canary { found: u32::MAX },
            "PMC_BOOT_0 reads 0xFFFFFFFF, not 0x140000A1",
        ),
        (
            Error::Partition {
                index: 5,
                cfg: 0xBADF_5040,
                coeff: 0x0001_4102,
            },
            "memory partition 5",
        ),
        (
            Error::NotLocked { coeff: 0x0001_4E02 },
            "did not lock at coefficient 0x00014E02 within 500 ms",
        ),
        (
            Error::Readback {
                want: 0x0001_4E02,
                got: 0x0001_4102,
            },
            "reads 0x00014102 after writing 0x00014E02",
        ),
    ];
    for (error, expected) in cases {
        let text = error.to_string();
        assert!(
            text.contains(expected),
            "{text:?} should contain {expected:?}"
        );
    }
}

#[test]
fn disagreeing_partitions_are_summarised_in_mhz() {
    let mut coeffs = vec![0x0001_4102; 16];
    coeffs[3] = 0x0001_4202;
    let text = Error::Mixed { coeffs }.to_string();
    assert_eq!(text, "memory partitions disagree on the clock (15 at 877.5 MHz, 1 at 891.0 MHz); reboot to reset");
}

#[test]
fn a_stranded_card_says_it_could_not_be_put_back() {
    let text = Error::Stranded(Box::new(Error::Canary { found: u32::MAX })).to_string();
    assert!(text.ends_with("; the card could not be put back"), "{text}");
    assert!(Error::Stranded(Box::new(Error::ReadOnly)).is_fatal());
    assert!(!Error::NotLocked { coeff: 0 }.is_fatal());
}

#[test]
fn a_card_knows_its_bar_file_and_prints_its_full_address() {
    let card = Card {
        bdf: "0000:08:00.0".parse().unwrap(),
        model: model_for(0x1db1).unwrap(),
    };
    let sysfs = Path::new("/sys/bus/pci/devices");
    assert_eq!(
        card.resource0(sysfs),
        Path::new("/sys/bus/pci/devices/0000:08:00.0/resource0")
    );
    assert_eq!(card.bdf.to_string(), "0000:08:00.0");
}

#[test]
fn discovery_reports_a_sysfs_path_it_cannot_list() {
    let file = common::temp_dir("discover-file").join("not-a-directory");
    fs::write(&file, "").unwrap();
    assert!(matches!(discover(&file), Err(Error::Io(msg)) if msg.contains("not-a-directory")));
}

#[test]
fn a_bar_file_without_write_permission_says_it_needs_root() {
    if unsafe { libc::geteuid() } == 0 {
        return; // root ignores file permissions
    }
    let path = common::temp_dir("mmap-perm").join("resource0");
    fs::write(&path, "").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    match MmapBar0::open(&path, Access::ReadWrite) {
        Err(Error::Io(msg)) => assert!(
            msg.starts_with("open ") && msg.ends_with("(needs root)"),
            "{msg}"
        ),
        other => panic!("expected an Io error, got {:?}", other.err()),
    }
}

#[test]
fn the_system_clock_moves_forward_and_sleeps() {
    let clock = SystemClock::default();
    let before = clock.now();
    clock.sleep(Duration::from_millis(5));
    assert!(clock.now() - before >= Duration::from_millis(5));
}
