mod common;

use std::fs;
use std::path::Path;

use v100_unlock::card::{discover, model_for, Bdf};
use v100_unlock::regs::Coeff;

#[test]
fn the_stock_coefficient_decodes_to_877_5_mhz() {
    let stock = Coeff(0x0001_4102);
    assert_eq!((stock.ndiv(), stock.mdiv(), stock.pldiv()), (65, 2, 1));
    assert!((stock.mhz() - 877.5).abs() < 1e-9);
}

#[test]
fn with_ndiv_changes_only_the_multiplier() {
    let stock = Coeff(0x0001_4102);
    let daily = stock.with_ndiv(78);
    assert_eq!(daily, Coeff(0x0001_4E02));
    assert!((daily.mhz() - 1053.0).abs() < 1e-9);
    assert_eq!(stock, Coeff(0x0001_4102), "the original is untouched");
}

#[test]
fn bdf_accepts_sysfs_names_and_rejects_everything_else() {
    let bdf: Bdf = "0000:08:00.0".parse().unwrap();
    assert_eq!(bdf.as_str(), "0000:08:00.0");
    assert_eq!(bdf.short(), "08:00.0");
    assert_eq!(
        "0000:3B:00.0".parse::<Bdf>().unwrap().as_str(),
        "0000:3b:00.0"
    );
    assert_eq!(
        "0001:08:00.0".parse::<Bdf>().unwrap().short(),
        "0001:08:00.0"
    );
    for bad in [
        "08:00.0",
        "0000:08:00.8",
        "0000:08:00",
        "../../etc/passwd",
        "0000:08:00.0 ",
        "0000:g8:00.0",
        "0000.08:00:0",
        "",
    ] {
        assert!(bad.parse::<Bdf>().is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn every_tesla_v100_is_known_and_other_gv100_boards_are_not() {
    let v100s = [
        0x1db0, 0x1db1, 0x1db2, 0x1db3, 0x1db4, 0x1db5, 0x1db6, 0x1db7, 0x1db8, 0x1dbd, 0x1df0,
        0x1df2, 0x1df5, 0x1df6,
    ];
    for id in v100s {
        assert!(model_for(id).is_some(), "{id:04x}");
    }
    // Titan V, Quadro GV100, CMP 100-200/210, engineering sample.
    for id in [0x1d81, 0x1dba, 0x1d83, 0x1d84, 0x1dc1, 0x1df4, 0x1dbe] {
        assert!(model_for(id).is_none(), "{id:04x}");
    }
    assert_eq!(model_for(0x1db1).unwrap().stock_ndiv, 65);
    assert_eq!(
        model_for(0x1db3).unwrap().stock_ndiv,
        60,
        "the FHHL ships at 810 MHz"
    );
    assert_eq!(
        model_for(0x1df6).unwrap().stock_ndiv,
        82,
        "V100S ships at 1107 MHz"
    );
}

fn fake_device(root: &Path, bdf: &str, vendor: &str, device: &str) {
    let dir = root.join(bdf);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("vendor"), format!("{vendor}\n")).unwrap();
    fs::write(dir.join("device"), format!("{device}\n")).unwrap();
}

#[test]
fn discover_finds_only_tesla_v100s_in_pci_order() {
    let root = common::temp_dir("discover");
    fake_device(&root, "0000:08:00.0", "0x10de", "0x1db1");
    fake_device(&root, "0000:04:00.0", "0x10de", "0x1db5");
    fake_device(&root, "0000:05:00.0", "0x10de", "0x1d81"); // Titan V
    fake_device(&root, "0000:00:02.0", "0x8086", "0x1db1"); // not NVIDIA
    fs::create_dir_all(root.join("not-a-bdf")).unwrap();

    let found: Vec<(String, &str)> = discover(&root)
        .unwrap()
        .iter()
        .map(|c| (c.bdf.short().to_string(), c.model.name))
        .collect();

    assert_eq!(
        found,
        [
            ("04:00.0".to_string(), "V100-SXM2-32GB"),
            ("08:00.0".to_string(), "V100-SXM2-16GB"),
        ]
    );
}

#[test]
fn a_machine_without_pci_sysfs_has_no_cards() {
    let none = discover(Path::new("/nonexistent/sys/bus/pci/devices")).unwrap();
    assert!(none.is_empty());
}

/// On a V100 host: `EXPECT_V100S=4 cargo test --test model -- --ignored`.
/// Discovery reads only world-readable sysfs files, so it needs no root.
#[test]
#[ignore = "reads this machine's PCI devices"]
fn discovers_the_cards_in_this_machine() {
    let cards = discover(Path::new(v100_unlock::card::SYSFS_PCI)).unwrap();
    for card in &cards {
        println!("{} {}", card.bdf, card.model.name);
    }
    let expected: usize = std::env::var("EXPECT_V100S")
        .expect("set EXPECT_V100S to the number of V100s in this machine")
        .parse()
        .unwrap();
    assert_eq!(cards.len(), expected);
}
