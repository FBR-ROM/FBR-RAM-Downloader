//! The GV100 registers behind the HBM memory clock.

use std::ops::Range;

/// BAR0 is 16 MiB on every GV100 board.
pub const BAR0_SIZE: usize = 16 << 20;

/// Chip ID register, read before and after every step as a canary.
pub const PMC_BOOT_0: u32 = 0x00_0000;
/// What [`PMC_BOOT_0`] reads on a GV100.
pub const BOOT0_GV100: u32 = 0x1400_00A1;

/// Multicast HBM PLL coefficient: one write reaches all 16 memory partitions.
/// It is write-only. Reading it hangs the GV100 register bus (the PRI ring).
pub const HBMPLL_COEFF_MC: u32 = 0x98_BC98;
/// The multicast window around [`HBMPLL_COEFF_MC`]. Nothing in it is ever read.
pub const NO_READ: Range<u32> = 0x98_8000..0x98_C000;

/// Memory partitions (FBPAs) on every Tesla V100: four HBM2 stacks.
pub const FBPA_COUNT: usize = 16;
const FBPA_CFG: u32 = 0x90_3C90;
const FBPA_COEFF: u32 = 0x90_3C98;
const FBPA_STRIDE: u32 = 0x4000;
/// PLL_LOCK in a partition's PLL CFG register.
pub const CFG_PLL_LOCK: u32 = 1 << 5;

/// HBM PLLs, each driving two partitions, each with a free-running 36-bit
/// DRAM cycle counter.
pub const HBM_PLL_COUNT: usize = 8;
const DRAM_COUNTER: u32 = 0x13_0000;
const DRAM_COUNTER_STRIDE: u32 = 0x100;

/// PLL reference clock.
pub const REF_MHZ: f64 = 27.0;
/// The PLL dividers every GV100 board boots with: its memory clock is set by
/// NDIV alone, 13.5 MHz a step.
pub const STOCK_MDIV: u8 = 2;
pub const STOCK_PLDIV: u8 = 1;

/// Unicast PLL CFG register of memory partition `i`.
pub fn fbpa_cfg(i: usize) -> u32 {
    FBPA_CFG + i as u32 * FBPA_STRIDE
}

/// Unicast PLL coefficient register of memory partition `i`.
pub fn fbpa_coeff(i: usize) -> u32 {
    FBPA_COEFF + i as u32 * FBPA_STRIDE
}

/// Low 32 bits of HBM PLL `j`'s DRAM cycle counter.
pub fn dram_counter_lo(j: usize) -> u32 {
    DRAM_COUNTER + j as u32 * DRAM_COUNTER_STRIDE + 0x34
}

/// High bits (35:32) of HBM PLL `j`'s DRAM cycle counter.
pub fn dram_counter_hi(j: usize) -> u32 {
    DRAM_COUNTER + j as u32 * DRAM_COUNTER_STRIDE + 0x38
}

/// All-ones (BAR not answering) or 0xBADxxxxx (a PRI error) is not a register value.
pub fn is_bus_error(value: u32) -> bool {
    value == u32::MAX || value & 0xFFF0_0000 == 0xBAD0_0000
}

/// An HBM PLL coefficient: PLDIV in bits 21:16, NDIV in 15:8, MDIV in 7:0.
/// Memory clock = 27 MHz × NDIV / MDIV / PLDIV, so stock `0x14102` is 877.5 MHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coeff(pub u32);

impl Coeff {
    pub fn ndiv(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub fn mdiv(self) -> u8 {
        self.0 as u8
    }

    pub fn pldiv(self) -> u8 {
        ((self.0 >> 16) & 0x3F) as u8
    }

    /// The same coefficient with a new NDIV. Everything else is kept.
    pub fn with_ndiv(self, ndiv: u8) -> Coeff {
        Coeff((self.0 & !0xFF00) | (u32::from(ndiv) << 8))
    }

    pub fn mhz(self) -> f64 {
        let divider = f64::from(self.mdiv().max(1)) * f64::from(self.pldiv().max(1));
        REF_MHZ * f64::from(self.ndiv()) / divider
    }
}
