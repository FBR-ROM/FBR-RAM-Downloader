//! Reading and changing one card's memory clock.
//!
//! The change is a single write of the new coefficient to the multicast
//! register, then a wait for all 16 partition PLLs to lock. There is no
//! memory-clock-change alert, no self-refresh and no PLL disable/relock cycle.
//! If the PLLs do not lock, the previous coefficient is written back.

use std::time::Duration;

use crate::bar0::{Bar0, Regs};
use crate::clock::Clock;
use crate::error::Error;
use crate::regs::{
    fbpa_cfg, fbpa_coeff, is_bus_error, Coeff, CFG_PLL_LOCK, FBPA_COUNT, HBMPLL_COEFF_MC,
};

/// How long the PLLs get to lock after a coefficient write.
pub const LOCK_TIMEOUT: Duration = Duration::from_millis(500);
const LOCK_POLL: Duration = Duration::from_micros(20);

/// A coefficient change that locked.
#[derive(Clone, Copy, Debug)]
pub struct Locked {
    pub elapsed: Duration,
}

/// The card's current coefficient. Fails unless every partition answers, is
/// locked, and agrees.
pub fn survey<B: Bar0>(regs: &Regs<B>) -> Result<Coeff, Error> {
    regs.canary()?;
    let parts = (0..FBPA_COUNT)
        .map(|i| Ok((regs.read(fbpa_cfg(i))?, regs.read(fbpa_coeff(i))?)))
        .collect::<Result<Vec<(u32, u32)>, Error>>()?;
    regs.canary()?;
    let unhealthy = |&(cfg, coeff): &(u32, u32)| {
        is_bus_error(cfg) || is_bus_error(coeff) || cfg & CFG_PLL_LOCK == 0
    };
    if let Some(index) = parts.iter().position(unhealthy) {
        let (cfg, coeff) = parts[index];
        return Err(Error::Partition { index, cfg, coeff });
    }
    let first = parts[0].1;
    if parts.iter().any(|&(_, coeff)| coeff != first) {
        return Err(Error::Mixed {
            coeffs: parts.iter().map(|&(_, c)| c).collect(),
        });
    }
    Ok(Coeff(first))
}

/// Move the card from `from` to `to`. On failure the card is back at `from`,
/// unless the error is [`Error::Stranded`].
pub fn set_coeff<B: Bar0>(
    regs: &Regs<B>,
    from: Coeff,
    to: Coeff,
    clock: &dyn Clock,
) -> Result<Locked, Error> {
    match program(regs, to, clock) {
        Ok(locked) => Ok(locked),
        Err(e) if e.is_fatal() => Err(Error::Stranded(Box::new(e))),
        Err(e) => match program(regs, from, clock) {
            Ok(_) => Err(e),
            Err(_) => Err(Error::Stranded(Box::new(e))),
        },
    }
}

/// Put a card back at `coeff` after a failed run. Unlike [`set_coeff`] there
/// is nothing to fall back to: if the write does not take, the card is
/// [`Error::Stranded`].
pub fn restore<B: Bar0>(regs: &Regs<B>, coeff: Coeff, clock: &dyn Clock) -> Result<Locked, Error> {
    program(regs, coeff, clock).map_err(|e| match e {
        Error::Stranded(_) => e,
        other => Error::Stranded(Box::new(other)),
    })
}

/// One multicast write, then wait for lock and confirm the coefficient stuck.
fn program<B: Bar0>(regs: &Regs<B>, coeff: Coeff, clock: &dyn Clock) -> Result<Locked, Error> {
    regs.canary()?; // never write into a hung register bus
    let start = clock.now();
    regs.write(HBMPLL_COEFF_MC, coeff.0)?;
    regs.read(fbpa_coeff(0))?; // fence: read back through the same block
    regs.canary()?;
    wait_for_lock(regs, coeff, clock, start)?;
    let elapsed = clock.now().saturating_sub(start);
    let now = survey(regs)?;
    if now != coeff {
        return Err(Error::Readback {
            want: coeff.0,
            got: now.0,
        });
    }
    Ok(Locked { elapsed })
}

fn wait_for_lock<B: Bar0>(
    regs: &Regs<B>,
    coeff: Coeff,
    clock: &dyn Clock,
    start: Duration,
) -> Result<(), Error> {
    loop {
        if all_locked(regs)? {
            return Ok(());
        }
        if clock.now().saturating_sub(start) >= LOCK_TIMEOUT {
            regs.canary()?;
            return Err(Error::NotLocked { coeff: coeff.0 });
        }
        clock.sleep(LOCK_POLL);
    }
}

fn all_locked<B: Bar0>(regs: &Regs<B>) -> Result<bool, Error> {
    for i in 0..FBPA_COUNT {
        let cfg = regs.read(fbpa_cfg(i))?;
        if is_bus_error(cfg) || cfg & CFG_PLL_LOCK == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}
