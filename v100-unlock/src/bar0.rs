//! Checked access to a card's registers.
//!
//! [`Bar0`] is the raw 32-bit access a backend provides. [`Regs`] is the only way
//! the rest of the crate touches it, so every access is checked for alignment and
//! range, and nothing can read the write-only multicast window.

use crate::error::Error;
use crate::regs::{BAR0_SIZE, BOOT0_GV100, NO_READ, PMC_BOOT_0};

/// Raw register access, implemented by the mmap backend and the simulator.
pub trait Bar0 {
    fn read32(&self, off: u32) -> u32;
    fn write32(&self, off: u32, val: u32);
}

impl<T: Bar0 + ?Sized> Bar0 for &T {
    fn read32(&self, off: u32) -> u32 {
        (**self).read32(off)
    }

    fn write32(&self, off: u32, val: u32) {
        (**self).write32(off, val)
    }
}

impl<T: Bar0 + ?Sized> Bar0 for Box<T> {
    fn read32(&self, off: u32) -> u32 {
        (**self).read32(off)
    }

    fn write32(&self, off: u32, val: u32) {
        (**self).write32(off, val)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

/// A card's registers, behind the checks.
pub struct Regs<B> {
    bar: B,
    access: Access,
}

impl<B: Bar0> Regs<B> {
    pub fn new(bar: B, access: Access) -> Self {
        Self { bar, access }
    }

    pub fn read(&self, off: u32) -> Result<u32, Error> {
        check(off)?;
        if NO_READ.contains(&off) {
            return Err(Error::Forbidden(off));
        }
        Ok(self.bar.read32(off))
    }

    pub fn write(&self, off: u32, val: u32) -> Result<(), Error> {
        check(off)?;
        if self.access == Access::ReadOnly {
            return Err(Error::ReadOnly);
        }
        self.bar.write32(off, val);
        Ok(())
    }

    /// Fails unless PMC_BOOT_0 reads as a GV100.
    pub fn canary(&self) -> Result<(), Error> {
        match self.read(PMC_BOOT_0)? {
            BOOT0_GV100 => Ok(()),
            found => Err(Error::Canary { found }),
        }
    }
}

fn check(off: u32) -> Result<(), Error> {
    if !off.is_multiple_of(4) {
        return Err(Error::Misaligned(off));
    }
    if off as usize + 4 > BAR0_SIZE {
        return Err(Error::OutOfRange(off));
    }
    Ok(())
}
