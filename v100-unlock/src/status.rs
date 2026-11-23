//! `v100-unlock status`: each card's memory clock, read-only.

use crate::bar0::{Bar0, Regs};
use crate::clock::Clock;
use crate::error::Error;
use crate::meter::{mean, measure, on_target, Rates};
use crate::pll::survey;
use crate::regs::Coeff;
use crate::run::Unit;

/// One line per card, and whether every card answered.
pub fn status<B: Bar0>(units: &[Unit<B>], clock: &dyn Clock) -> (Vec<String>, bool) {
    let surveys: Vec<Result<Coeff, Error>> = units.iter().map(|u| survey(&u.regs)).collect();
    let healthy: Vec<&Regs<B>> = units
        .iter()
        .zip(&surveys)
        .filter(|(_, survey)| survey.is_ok())
        .map(|(unit, _)| &unit.regs)
        .collect();
    let mut metered = measure(&healthy, clock).into_iter();
    let rows: Vec<(String, bool)> = units
        .iter()
        .zip(&surveys)
        .map(|(unit, survey)| row(unit, survey, &mut metered))
        .collect();
    let all_ok = rows.iter().all(|(_, ok)| *ok);
    (rows.into_iter().map(|(line, _)| line).collect(), all_ok)
}

/// Healthy cards take the next measurement, in order.
fn row<B>(
    unit: &Unit<B>,
    survey: &Result<Coeff, Error>,
    metered: &mut impl Iterator<Item = Result<Rates, Error>>,
) -> (String, bool) {
    let who = format!("{} · {}", unit.card.bdf.short(), unit.card.model.name);
    let coeff = match survey {
        Ok(coeff) => *coeff,
        Err(e) => return (format!("{who} · {e}"), false),
    };
    match metered.next() {
        Some(Ok(rates)) => {
            let line = format!(
                "{who} · {:.1} MHz (NDIV {}) · metered {:.1}",
                coeff.mhz(),
                coeff.ndiv(),
                mean(&rates)
            );
            if on_target(&rates, coeff.mhz()) {
                (line, true)
            } else {
                (format!("{line}, expected {:.1}", coeff.mhz()), false)
            }
        }
        Some(Err(e)) => (format!("{who} · {e}"), false),
        None => (format!("{who} · not measured"), false),
    }
}
