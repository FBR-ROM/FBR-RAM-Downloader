//! One run across every selected card: survey, change, measure. All or nothing.
//!
//! Every card is surveyed before anything is written. Cards are changed one at
//! a time, then every card is measured, changed or not. If any card fails, or
//! the caller cancels, every card that was changed is put back.

use crate::bar0::{Bar0, Regs};
use crate::card::{Card, Model};
use crate::clock::Clock;
use crate::error::Error;
use crate::meter::{mean, measure, on_target};
use crate::outcome::{Kind, Outcome};
use crate::pll::{restore, set_coeff, survey};
use crate::protocol::Line;
use crate::regs::{Coeff, STOCK_MDIV, STOCK_PLDIV};
use crate::text;

/// The daily clock: 1053 MHz, memtest-clean and byte-identical in serving.
pub const DAILY_NDIV: u8 = 78;
/// The sprint clock: 1080 MHz, for short benchmark runs.
pub const SPRINT_NDIV: u8 = 80;
/// The highest clock `restore` brings a card down from: 1107 MHz, the V100S
/// rating and the top of the v100-overclock ladders.
pub const RESTORE_FROM_MAX_NDIV: u8 = 82;

/// Where the progress bar stands after each phase.
const SURVEYED: f32 = 0.15;
const CHANGED: f32 = 0.6;
const METERED: f32 = 0.95;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    Daily,
    Sprint,
    Stock,
}

impl Goal {
    /// The NDIV this goal means for a model. A card is never set below the
    /// clock it ships at (the V100S ships at 1107 MHz).
    fn ndiv_for(self, model: &Model) -> u8 {
        match self {
            Goal::Daily => DAILY_NDIV.max(model.stock_ndiv),
            Goal::Sprint => SPRINT_NDIV.max(model.stock_ndiv),
            Goal::Stock => model.stock_ndiv,
        }
    }
}

/// One card and its registers.
pub struct Unit<B> {
    pub card: Card,
    pub regs: Regs<B>,
}

#[derive(Clone, Copy)]
struct Plan {
    from: Coeff,
    to: Coeff,
}

impl Plan {
    fn changes(self) -> bool {
        self.from != self.to
    }
}

struct Io<'a> {
    clock: &'a dyn Clock,
    cancelled: &'a dyn Fn() -> bool,
    emit: &'a mut dyn FnMut(Line),
}

impl Io<'_> {
    fn step(&mut self, progress: f32, text: String) {
        (self.emit)(Line::Step { progress, text });
    }

    fn note(&mut self, text: String) {
        (self.emit)(Line::Note(text));
    }
}

/// Take every unit to `goal`. Progress and the receipt go to `emit`;
/// `cancelled` is checked between cards. With `dry_run`, the cards are
/// surveyed and reported but nothing is written.
pub fn run<B: Bar0>(
    units: &[Unit<B>],
    goal: Goal,
    clock: &dyn Clock,
    cancelled: &dyn Fn() -> bool,
    dry_run: bool,
    emit: &mut dyn FnMut(Line),
) -> Outcome {
    if units.is_empty() {
        return Outcome::new(Kind::Refused, "no Tesla V100 found");
    }
    let mut io = Io {
        clock,
        cancelled,
        emit,
    };
    let cards: Vec<&Card> = units.iter().map(|u| &u.card).collect();
    io.step(0.0, text::found(&cards));
    plan(units, goal, &mut io)
        .and_then(|plans| {
            if dry_run {
                return Ok(finish(units, &plans, &[], goal, true, &mut io));
            }
            let touched = change(units, &plans, &mut io)?;
            let metered = verify(units, &plans, &touched, &mut io)?;
            Ok(finish(units, &plans, &metered, goal, false, &mut io))
        })
        .unwrap_or_else(|outcome| outcome)
}

/// Survey every card before writing anything.
fn plan<B: Bar0>(units: &[Unit<B>], goal: Goal, io: &mut Io) -> Result<Vec<Plan>, Outcome> {
    let total = units.len();
    units
        .iter()
        .enumerate()
        .map(|(i, unit)| {
            let from = survey(&unit.regs).map_err(|e| refuse(text::failed(&unit.card, &e)))?;
            check_known(&unit.card, from, goal)?;
            let plan = Plan {
                from,
                to: from.with_ndiv(goal.ndiv_for(unit.card.model)),
            };
            let label = text::label(&unit.card, i, total);
            io.step(
                span(0.0, SURVEYED, i + 1, total),
                text::surveyed(&label, from, plan.changes()),
            );
            Ok(plan)
        })
        .collect()
}

/// Only start from a state this tool understands: the stock PLL dividers and
/// a clock from the card's stock up to sprint. `restore` also brings a card
/// down from anything up to [`RESTORE_FROM_MAX_NDIV`].
fn check_known(card: &Card, from: Coeff, goal: Goal) -> Result<(), Outcome> {
    let who = card.bdf.short();
    if (from.mdiv(), from.pldiv()) != (STOCK_MDIV, STOCK_PLDIV) {
        return Err(refuse(format!(
            "{who}: unexpected PLL dividers (MDIV {}, PLDIV {}); reboot to reset them",
            from.mdiv(),
            from.pldiv()
        )));
    }
    let stock = card.model.stock_ndiv;
    let ceiling = match goal {
        Goal::Stock => RESTORE_FROM_MAX_NDIV,
        Goal::Daily | Goal::Sprint => SPRINT_NDIV,
    }
    .max(stock);
    if (stock..=ceiling).contains(&from.ndiv()) {
        return Ok(());
    }
    Err(refuse(format!(
        "{who}: memory clock is {:.1} MHz, outside {:.1}–{:.1} MHz; reboot to reset it",
        from.mhz(),
        from.with_ndiv(stock).mhz(),
        from.with_ndiv(ceiling).mhz()
    )))
}

/// Change every card that needs it, one at a time. Returns the cards changed.
fn change<B: Bar0>(units: &[Unit<B>], plans: &[Plan], io: &mut Io) -> Result<Vec<usize>, Outcome> {
    let todo: Vec<usize> = (0..plans.len()).filter(|&i| plans[i].changes()).collect();
    let mut touched = Vec::with_capacity(todo.len());
    for (k, &i) in todo.iter().enumerate() {
        if (io.cancelled)() {
            return Err(roll_back(units, plans, &touched, &[], "cancelled", io));
        }
        let (unit, plan) = (&units[i], plans[i]);
        match set_coeff(&unit.regs, plan.from, plan.to, io.clock) {
            Ok(locked) => {
                let label = text::label(&unit.card, i, units.len());
                io.step(
                    span(SURVEYED, CHANGED, k + 1, todo.len()),
                    text::locked(&label, plan.to, locked.elapsed),
                );
                touched.push(i);
            }
            Err(e) => {
                // The failed change wrote this card, so it counts as touched;
                // set_coeff has already put it back unless it is stranded.
                let reason = text::failed(&unit.card, &e);
                if matches!(e, Error::Stranded(_)) {
                    return Err(roll_back(units, plans, &touched, &[i], &reason, io));
                }
                touched.push(i);
                return Err(roll_back(units, plans, &touched, &[], &reason, io));
            }
        }
    }
    Ok(touched)
}

/// Measure every card in one window, changed or not: a card is at the target
/// only when its clock says so. Returns each card's metered MHz.
fn verify<B: Bar0>(
    units: &[Unit<B>],
    plans: &[Plan],
    touched: &[usize],
    io: &mut Io,
) -> Result<Vec<f64>, Outcome> {
    let regs: Vec<&Regs<B>> = units.iter().map(|unit| &unit.regs).collect();
    let results = measure(&regs, io.clock);
    // Checked after the 500 ms window: a caller gone by now must not be told
    // "done" while the cards stay changed.
    if (io.cancelled)() {
        return Err(roll_back(units, plans, touched, &[], "cancelled", io));
    }
    let mut metered = Vec::with_capacity(units.len());
    for (i, result) in results.into_iter().enumerate() {
        let (card, want) = (&units[i].card, plans[i].to.mhz());
        let (stranded, reason) = match result {
            Ok(rates) if on_target(&rates, want) => {
                let label = text::label(card, i, units.len());
                io.step(
                    span(CHANGED, METERED, i + 1, units.len()),
                    text::metered(&label, mean(&rates)),
                );
                metered.push(mean(&rates));
                continue;
            }
            Ok(rates) => (vec![], text::off_target(card, mean(&rates), want)),
            Err(e) if e.is_fatal() => (vec![i], text::failed(card, &e)),
            Err(e) => (vec![], text::failed(card, &e)),
        };
        return Err(roll_back(units, plans, touched, &stranded, &reason, io));
    }
    Ok(metered)
}

/// Put every touched card back by writing its original coefficient once, or
/// not at all if its registers already show it. Cards in `stranded` are not
/// touched again: their register bus is hung. The outcome names the state the
/// cards are left in.
fn roll_back<B: Bar0>(
    units: &[Unit<B>],
    plans: &[Plan],
    touched: &[usize],
    stranded: &[usize],
    reason: &str,
    io: &mut Io,
) -> Outcome {
    let lost: Vec<bool> = units
        .iter()
        .zip(plans)
        .enumerate()
        .map(|(i, (unit, plan))| {
            let was_changed = touched.contains(&i);
            let already_back = || survey(&unit.regs) == Ok(plan.from);
            let lost = stranded.contains(&i)
                || (was_changed
                    && !already_back()
                    && restore(&unit.regs, plan.from, io.clock).is_err());
            io.note(match (lost, was_changed) {
                (true, _) => text::stranded(&unit.card),
                (false, true) => text::put_back(&unit.card, plan.from),
                (false, false) => text::unchanged(&unit.card, plan.from, None),
            });
            lost
        })
        .collect();
    if lost.contains(&true) {
        Outcome::new(
            Kind::Unknown,
            format!("a card is in an unknown state, power-cycle the host: {reason}"),
        )
    } else if touched.is_empty() {
        Outcome::new(Kind::Refused, format!("{reason}; nothing was changed"))
    } else {
        Outcome::new(
            Kind::RolledBack,
            format!("{reason}; every card is back where it started"),
        )
    }
}

/// The receipt, one line per card, and the headline. A dry run reports what
/// would happen and ends `Dry`; nothing was written, so there is no
/// volatility note.
fn finish<B: Bar0>(
    units: &[Unit<B>],
    plans: &[Plan],
    metered: &[f64],
    goal: Goal,
    dry: bool,
    io: &mut Io,
) -> Outcome {
    if dry {
        for (unit, plan) in units.iter().zip(plans) {
            io.note(text::would_change(&unit.card, plan.from, plan.to));
        }
    } else {
        for ((unit, plan), &mhz) in units.iter().zip(plans).zip(metered) {
            io.note(if plan.changes() {
                text::changed(&unit.card, plan.from, plan.to, mhz)
            } else {
                text::unchanged(&unit.card, plan.from, Some(mhz))
            });
        }
    }
    if !dry && goal != Goal::Stock {
        io.note(text::VOLATILE.to_string());
    }
    let pairs: Vec<(Coeff, Coeff)> = plans.iter().map(|p| (p.from, p.to)).collect();
    let kind = if dry { Kind::Dry } else { Kind::Done };
    let summary = if dry {
        text::dry_headline(&pairs, goal == Goal::Stock)
    } else {
        text::headline(&pairs, goal == Goal::Stock)
    };
    Outcome::new(kind, summary)
}

fn refuse(reason: String) -> Outcome {
    Outcome::new(Kind::Refused, reason)
}

fn span(lo: f32, hi: f32, done: usize, total: usize) -> f32 {
    lo + (hi - lo) * done as f32 / total.max(1) as f32
}
