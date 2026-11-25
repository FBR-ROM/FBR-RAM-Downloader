//! The words a run prints: status lines, receipt lines and summaries.

use std::time::Duration;

use crate::card::Card;
use crate::error::Error;
use crate::regs::Coeff;

pub const VOLATILE: &str = "volatile: a reboot or `v100-unlock restore` puts stock clocks back";

/// `card 2/4 · 06:00.0`, or just `08:00.0` when there is one card.
pub fn label(card: &Card, index: usize, total: usize) -> String {
    if total == 1 {
        card.bdf.short().to_string()
    } else {
        format!("card {}/{} · {}", index + 1, total, card.bdf.short())
    }
}

pub fn found(cards: &[&Card]) -> String {
    let first = cards[0].model.name;
    let same = cards.iter().all(|c| c.model.name == first);
    match cards.len() {
        1 => format!("found {first}"),
        n if same => format!("found {n} × {first}"),
        n => format!("found {n} Tesla V100s"),
    }
}

pub fn surveyed(label: &str, from: Coeff, changes: bool) -> String {
    let tail = if changes { "" } else { " · already there" };
    format!("{label} · {:.1} MHz{tail}", from.mhz())
}

pub fn locked(label: &str, to: Coeff, elapsed: Duration) -> String {
    format!(
        "{label} · {:.1} MHz · locked in {:.1} ms",
        to.mhz(),
        elapsed.as_secs_f64() * 1e3
    )
}

pub fn metered(label: &str, mhz: f64) -> String {
    format!("{label} · metered {mhz:.1} MHz")
}

pub fn failed(card: &Card, err: &Error) -> String {
    format!("{}: {err}", card.bdf.short())
}

pub fn off_target(card: &Card, got: f64, want: f64) -> String {
    format!(
        "{}: metered {got:.1} MHz, expected {want:.1}",
        card.bdf.short()
    )
}

fn who(card: &Card) -> String {
    format!("{} · {}", card.bdf.short(), card.model.name)
}

pub fn changed(card: &Card, from: Coeff, to: Coeff, metered: f64) -> String {
    format!(
        "{} · {:.1} → {:.1} MHz · metered {metered:.1}",
        who(card),
        from.mhz(),
        to.mhz()
    )
}

/// A dry-run receipt line: what would happen, without the metered part.
pub fn would_change(card: &Card, from: Coeff, to: Coeff) -> String {
    if from == to {
        format!("{} · {:.1} MHz · unchanged", who(card), from.mhz())
    } else {
        format!(
            "{} · {:.1} → {:.1} MHz · would change",
            who(card),
            from.mhz(),
            to.mhz()
        )
    }
}

pub fn unchanged(card: &Card, at: Coeff, metered: Option<f64>) -> String {
    let line = format!("{} · {:.1} MHz · unchanged", who(card), at.mhz());
    match metered {
        Some(mhz) => format!("{line} · metered {mhz:.1}"),
        None => line,
    }
}

pub fn put_back(card: &Card, at: Coeff) -> String {
    format!("{} · back at {:.1} MHz", who(card), at.mhz())
}

pub fn stranded(card: &Card) -> String {
    format!("{} · state unknown: power-cycle the host", who(card))
}

/// The one-line result, e.g. `hbm 877.5 → 1053.0 MHz (+20%) · 4 cards`.
pub fn headline(pairs: &[(Coeff, Coeff)], to_stock: bool) -> String {
    let changed: Vec<(Coeff, Coeff)> = pairs.iter().copied().filter(|(a, b)| a != b).collect();
    let Some(&(from, to)) = changed.first() else {
        return already(pairs);
    };
    let kept = pairs.len() - changed.len();
    let tail = if kept > 0 {
        format!(" · {kept} already there")
    } else {
        String::new()
    };
    let same_from = changed.iter().all(|&(f, _)| f == from);
    let same_to = changed.iter().all(|&(_, t)| t == to);
    let what = match (same_from, same_to, to_stock) {
        (_, false, _) => "hbm at target".to_string(),
        (_, true, true) => format!("hbm back to {:.1} MHz", to.mhz()),
        (false, true, false) => format!("hbm at {:.1} MHz", to.mhz()),
        (true, true, false) => {
            let pct = (to.mhz() / from.mhz() - 1.0) * 100.0;
            format!("hbm {:.1} → {:.1} MHz ({pct:+.0}%)", from.mhz(), to.mhz())
        }
    };
    format!("{what} · {}{tail}", cards(changed.len()))
}

/// The one-line result of a dry run, e.g.
/// `dry run: hbm 877.5 → 1053.0 MHz (+20%) · 4 cards · nothing written`.
/// Mirrors [`headline`], but says what would happen and that nothing was
/// written.
pub fn dry_headline(pairs: &[(Coeff, Coeff)], to_stock: bool) -> String {
    let changed: Vec<(Coeff, Coeff)> = pairs.iter().copied().filter(|(a, b)| a != b).collect();
    if changed.is_empty() {
        // `already` already carries the card count.
        return format!("dry run: {} · nothing written", already(pairs));
    }
    let &(from, to) = changed.first().unwrap();
    let same_from = changed.iter().all(|&(f, _)| f == from);
    let same_to = changed.iter().all(|&(_, t)| t == to);
    let what = match (same_from, same_to, to_stock) {
        (_, false, _) => "hbm at target".to_string(),
        (_, true, true) => format!("hbm back to {:.1} MHz", to.mhz()),
        (false, true, false) => format!("hbm at {:.1} MHz", to.mhz()),
        (true, true, false) => {
            let pct = (to.mhz() / from.mhz() - 1.0) * 100.0;
            format!("hbm {:.1} → {:.1} MHz ({pct:+.0}%)", from.mhz(), to.mhz())
        }
    };
    format!("dry run: {what} · {} · nothing written", cards(changed.len()))
}

fn already(pairs: &[(Coeff, Coeff)]) -> String {
    let at = pairs[0].1;
    if pairs.iter().all(|&(_, to)| to == at) {
        format!("already at {:.1} MHz · {}", at.mhz(), cards(pairs.len()))
    } else {
        format!("already at target · {}", cards(pairs.len()))
    }
}

fn cards(n: usize) -> String {
    if n == 1 {
        "1 card".to_string()
    } else {
        format!("{n} cards")
    }
}
