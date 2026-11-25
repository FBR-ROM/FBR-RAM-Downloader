//! v100-unlock: live HBM2 memory overclock for NVIDIA Tesla V100 (GV100).
//!
//! One write of a new PLL coefficient to the multicast register moves the memory
//! clock of all 16 memory partitions at once. The tool then waits for every PLL to
//! lock and measures the real clock from the DRAM cycle counters. Every selected
//! card changes, or none does. No VBIOS flash, no reboot; a reboot returns stock.

pub mod app;
pub mod bar0;
pub mod card;
pub mod cli;
pub mod clock;
pub mod error;
mod host;
pub mod meter;
pub mod mmap;
pub mod outcome;
pub mod pll;
pub mod protocol;
pub mod regs;
pub mod run;
pub mod sim;
pub mod status;
mod text;
