//! Everything that can go wrong talking to a card.

use std::fmt;

use crate::pll::LOCK_TIMEOUT;
use crate::regs::{Coeff, BOOT0_GV100};

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// A read inside the write-only multicast window.
    Forbidden(u32),
    Misaligned(u32),
    OutOfRange(u32),
    /// A write through a read-only mapping.
    ReadOnly,
    Io(String),
    /// PMC_BOOT_0 is wrong: not a GV100, or its register bus is hung.
    Canary {
        found: u32,
    },
    /// A memory partition did not answer, or its PLL is not locked.
    Partition {
        index: usize,
        cfg: u32,
        coeff: u32,
    },
    /// The memory partitions hold different coefficients.
    Mixed {
        coeffs: Vec<u32>,
    },
    /// The PLLs did not lock in time.
    NotLocked {
        coeff: u32,
    },
    /// The coefficient read back is not the one written.
    Readback {
        want: u32,
        got: u32,
    },
    /// The card could not be returned to its previous clock.
    Stranded(Box<Error>),
}

impl Error {
    /// True when the card must not be touched again before a power cycle.
    pub fn is_fatal(&self) -> bool {
        matches!(self, Error::Canary { .. } | Error::Stranded(_))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Forbidden(off) => write!(
                f,
                "refusing to read 0x{off:06X}: write-only multicast register (reading it hangs the register bus)"
            ),
            Error::Misaligned(off) => write!(f, "0x{off:06X} is not a 32-bit register offset"),
            Error::OutOfRange(off) => write!(f, "0x{off:06X} is outside BAR0"),
            Error::ReadOnly => write!(f, "BAR0 is mapped read-only"),
            Error::Io(msg) => write!(f, "{msg}"),
            Error::Canary { found } => write!(
                f,
                "PMC_BOOT_0 reads 0x{found:08X}, not 0x{BOOT0_GV100:08X}: not a GV100, or its register bus is hung"
            ),
            Error::Partition { index, cfg, coeff } => write!(
                f,
                "memory partition {index} is not answering or not locked (CFG 0x{cfg:08X}, COEFF 0x{coeff:08X})"
            ),
            Error::Mixed { coeffs } => write!(
                f,
                "memory partitions disagree on the clock ({}); reboot to reset",
                tally(coeffs)
            ),
            Error::NotLocked { coeff } => write!(
                f,
                "memory PLLs did not lock at coefficient 0x{coeff:08X} within {} ms",
                LOCK_TIMEOUT.as_millis()
            ),
            Error::Readback { want, got } => {
                write!(f, "coefficient reads 0x{got:08X} after writing 0x{want:08X}")
            }
            Error::Stranded(cause) => write!(f, "{cause}; the card could not be put back"),
        }
    }
}

impl std::error::Error for Error {}

/// `15 at 877.5 MHz, 1 at 891.0 MHz`: each distinct coefficient, in order of
/// first appearance, with how many partitions hold it.
fn tally(coeffs: &[u32]) -> String {
    let mut distinct: Vec<(u32, usize)> = Vec::new();
    for &coeff in coeffs {
        match distinct.iter_mut().find(|(c, _)| *c == coeff) {
            Some((_, count)) => *count += 1,
            None => distinct.push((coeff, 1)),
        }
    }
    let parts: Vec<String> = distinct
        .iter()
        .map(|&(coeff, count)| format!("{count} at {:.1} MHz", Coeff(coeff).mhz()))
        .collect();
    parts.join(", ")
}
