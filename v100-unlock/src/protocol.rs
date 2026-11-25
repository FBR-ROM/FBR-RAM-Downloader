//! The `--events` output: one line per step, for a front end to read.
//!
//! ```text
//! v100-unlock 1
//! step 0.150 card 1/4 · 04:00.0 · 877.5 MHz
//! note 04:00.0 · V100-SXM2-16GB · 877.5 → 1053.0 MHz · metered 1053.0
//! end ok hbm 877.5 → 1053.0 MHz (+20%) · 4 cards
//! ```
//!
//! Text is stripped of control characters on both ends, so a line can never
//! carry a terminal escape sequence.

use crate::outcome::{Kind, Outcome};

/// First line of the stream; the number is the protocol version.
pub const HEADER: &str = "v100-unlock 1";
/// Longest text a line carries.
pub const MAX_TEXT: usize = 400;

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Header,
    /// Progress from 0 to 1, and what is happening.
    Step {
        progress: f32,
        text: String,
    },
    /// One line of the receipt.
    Note(String),
    End(Outcome),
}

pub fn encode(line: &Line) -> String {
    match line {
        Line::Header => HEADER.to_string(),
        Line::Step { progress, text } => {
            format!("step {:.3} {}", progress.clamp(0.0, 1.0), clean(text))
        }
        Line::Note(text) => format!("note {}", clean(text)),
        Line::End(outcome) => format!("end {} {}", outcome.kind.as_str(), clean(&outcome.summary)),
    }
}

pub fn parse(raw: &str) -> Result<Line, String> {
    let line = raw.trim_end_matches(['\r', '\n']);
    if line == HEADER {
        return Ok(Line::Header);
    }
    let (word, rest) = line.split_once(' ').ok_or_else(|| rejected(line))?;
    match word {
        "step" => parse_step(rest).ok_or_else(|| rejected(line)),
        "note" => Ok(Line::Note(clean(rest))),
        "end" => parse_end(rest).ok_or_else(|| rejected(line)),
        _ => Err(rejected(line)),
    }
}

/// Text with control characters removed, at most [`MAX_TEXT`] characters.
pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT)
        .collect()
}

fn parse_step(rest: &str) -> Option<Line> {
    let (number, text) = rest.split_once(' ').unwrap_or((rest, ""));
    let progress: f32 = number.parse().ok()?;
    (0.0..=1.0).contains(&progress).then(|| Line::Step {
        progress,
        text: clean(text),
    })
}

fn parse_end(rest: &str) -> Option<Line> {
    let (word, text) = rest.split_once(' ').unwrap_or((rest, ""));
    Kind::parse(word).map(|kind| Line::End(Outcome::new(kind, clean(text))))
}

fn rejected(line: &str) -> String {
    format!("not a v100-unlock line: {:?}", clean(line))
}
