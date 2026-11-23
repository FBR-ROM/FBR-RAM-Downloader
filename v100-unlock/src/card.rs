//! Finding the Tesla V100s in a machine.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::error::Error;

/// Where Linux lists PCI devices.
pub const SYSFS_PCI: &str = "/sys/bus/pci/devices";
const NVIDIA: u16 = 0x10de;

/// A Tesla V100 board, by PCI device ID.
#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    pub device_id: u16,
    pub name: &'static str,
    /// The memory clock it ships at (13.5 MHz per step at stock dividers).
    pub stock_ndiv: u8,
}

const fn v100(device_id: u16, name: &'static str) -> Model {
    Model {
        device_id,
        name,
        stock_ndiv: 65,
    }
}

/// Every Tesla V100 in the PCI ID database. All have four HBM2 stacks and 16
/// memory partitions, and ship at 877.5 MHz (900 GB/s), except the FHHL at
/// 810 MHz (829 GB/s) and the V100S at 1107 MHz (1134 GB/s).
pub const V100_MODELS: &[Model] = &[
    v100(0x1db0, "V100-SXM2-16GB"),
    v100(0x1db1, "V100-SXM2-16GB"),
    v100(0x1db2, "V100-DGXS-16GB"),
    Model {
        device_id: 0x1db3,
        name: "V100-FHHL-16GB",
        stock_ndiv: 60,
    },
    v100(0x1db4, "V100-PCIE-16GB"),
    v100(0x1db5, "V100-SXM2-32GB"),
    v100(0x1db6, "V100-PCIE-32GB"),
    v100(0x1db7, "V100-DGXS-32GB"),
    v100(0x1db8, "V100-SXM3-32GB"),
    v100(0x1dbd, "V100-DGX1-V"),
    v100(0x1df0, "V100-PG500-216"),
    v100(0x1df2, "V100-PG503-216"),
    v100(0x1df5, "V100-SXM2-16GB"),
    Model {
        device_id: 0x1df6,
        name: "V100S-PCIE-32GB",
        stock_ndiv: 82,
    },
];

pub fn model_for(device_id: u16) -> Option<&'static Model> {
    V100_MODELS.iter().find(|m| m.device_id == device_id)
}

/// A PCI address in sysfs form, `dddd:bb:dd.f`, lowercase.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bdf(String);

impl Bdf {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Without the usual `0000:` domain, as `nvidia-smi` prints it.
    pub fn short(&self) -> &str {
        self.0.strip_prefix("0000:").unwrap_or(&self.0)
    }
}

impl FromStr for Bdf {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = |part: &str, len: usize| {
            part.len() == len && part.chars().all(|c| c.is_ascii_hexdigit())
        };
        let fields = s.split_once(':').and_then(|(domain, rest)| {
            let (bus, rest) = rest.split_once(':')?;
            let (dev, func) = rest.split_once('.')?;
            Some((domain, bus, dev, func))
        });
        match fields {
            Some((domain, bus, dev, func))
                if hex(domain, 4)
                    && hex(bus, 2)
                    && hex(dev, 2)
                    && matches!(func.as_bytes(), [b'0'..=b'7']) =>
            {
                Ok(Bdf(s.to_ascii_lowercase()))
            }
            _ => Err(format!("{s:?} is not a PCI address like 0000:08:00.0")),
        }
    }
}

impl fmt::Display for Bdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One Tesla V100 in this machine.
#[derive(Clone, Debug)]
pub struct Card {
    pub bdf: Bdf,
    pub model: &'static Model,
}

impl Card {
    /// The file that maps this card's BAR0.
    pub fn resource0(&self, sysfs: &Path) -> PathBuf {
        sysfs.join(self.bdf.as_str()).join("resource0")
    }
}

/// Every Tesla V100 under `sysfs`, in PCI order. A machine without PCI sysfs
/// (macOS, a container) simply has none.
pub fn discover(sysfs: &Path) -> Result<Vec<Card>, Error> {
    let entries = match fs::read_dir(sysfs) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(format!("{}: {e}", sysfs.display()))),
    };
    let mut cards: Vec<Card> = entries
        .flatten()
        .filter_map(|e| card_at(&e.path()))
        .collect();
    cards.sort_by(|a, b| a.bdf.cmp(&b.bdf));
    Ok(cards)
}

fn card_at(dir: &Path) -> Option<Card> {
    let bdf: Bdf = dir.file_name()?.to_str()?.parse().ok()?;
    if read_hex(&dir.join("vendor"))? != NVIDIA {
        return None;
    }
    let model = model_for(read_hex(&dir.join("device"))?)?;
    Some(Card { bdf, model })
}

fn read_hex(path: &Path) -> Option<u16> {
    let text = fs::read_to_string(path).ok()?;
    u16::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}
