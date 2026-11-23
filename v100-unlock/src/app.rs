//! The `v100-unlock` program.

use std::io::{self, Write};

use crate::bar0::{Access, Bar0, Regs};
use crate::card::{model_for, Card};
use crate::cli::{self, Cli, Command, USAGE};
use crate::clock::{Clock, SystemClock};
use crate::host::{self, AnyUnit};
use crate::outcome::{Kind, Outcome};
use crate::protocol::{encode, Line};
use crate::run::{run, Goal, Unit};
use crate::sim::{Fault, SimCard, SimClock};
use crate::status::status;

/// Simulated cards are V100-SXM2-16GBs at stock, from PCI bus 04 up.
const SIM_DEVICE: u16 = 0x1db1;
const SIM_STOCK_NDIV: u8 = 65;
const SIM_FIRST_BUS: usize = 4;

/// Run the program and return its exit status.
pub fn main(args: &[String]) -> i32 {
    let cli = match cli::parse(args) {
        Ok(cli) => cli,
        Err(e) => {
            eprint_line(&format!("v100-unlock: {e} (see --help)"));
            return 2;
        }
    };
    match cli.command {
        Command::Help => {
            print_line(USAGE.trim_end());
            0
        }
        Command::Status => show_status(&cli),
        Command::Run(goal) => change(&cli, goal),
    }
}

fn change(cli: &Cli, goal: Goal) -> i32 {
    host::watch_for_cancel(cli.events);
    let mut out = Output::new(cli.events);
    out.header();
    let outcome = with_cards(cli, Access::ReadWrite, |units, clock| {
        run(units, goal, clock, &host::cancelled, &mut |line| {
            out.line(&line)
        })
    })
    .unwrap_or_else(|refusal| refusal);
    out.end(&outcome);
    outcome.kind.exit_code()
}

/// 0 when every card answered, 1 when one did not, 2 when it could not start.
fn show_status(cli: &Cli) -> i32 {
    match with_cards(cli, Access::ReadOnly, |units, clock| status(units, clock)) {
        Ok((lines, all_ok)) if !lines.is_empty() => {
            for line in &lines {
                print_line(line);
            }
            if all_ok {
                0
            } else {
                1
            }
        }
        Ok(_) => {
            eprint_line("v100-unlock: no Tesla V100 found");
            2
        }
        Err(refusal) => {
            eprint_line(&format!("v100-unlock: {}", refusal.summary));
            2
        }
    }
}

/// Hand the selected cards, real or simulated, to `job`.
fn with_cards<T>(
    cli: &Cli,
    access: Access,
    job: impl FnOnce(&[AnyUnit], &dyn Clock) -> T,
) -> Result<T, Outcome> {
    if let Some(count) = cli.simulate {
        let clock = SimClock::realtime();
        let cards = host::select(simulated_cards(count), &cli.cards)?;
        let sims: Vec<SimCard> = cards
            .iter()
            .map(|_| SimCard::new(&clock, SIM_DEVICE, SIM_STOCK_NDIV, Fault::Healthy))
            .collect();
        let units: Vec<AnyUnit> = cards
            .into_iter()
            .zip(&sims)
            .map(|(card, sim)| Unit {
                card,
                regs: Regs::new(Box::new(sim) as Box<dyn Bar0>, access),
            })
            .collect();
        return Ok(job(&units, &clock));
    }
    host::require_root()?;
    let _lock = match access {
        Access::ReadWrite => Some(host::lock()?),
        Access::ReadOnly => None,
    };
    let units = host::open_cards(&cli.cards, access)?;
    Ok(job(&units, &SystemClock::new()))
}

fn simulated_cards(count: usize) -> Vec<Card> {
    let model = model_for(SIM_DEVICE).expect("the simulated device is a Tesla V100");
    (0..count)
        .map(|i| {
            let bdf = format!("0000:{:02x}:00.0", SIM_FIRST_BUS + i);
            let bdf = bdf
                .parse()
                .expect("simulated PCI addresses are well formed");
            Card { bdf, model }
        })
        .collect()
}

/// Where a run's lines go: plain text for people, or `--events` lines.
struct Output {
    events: bool,
    in_receipt: bool,
}

impl Output {
    fn new(events: bool) -> Self {
        Self {
            events,
            in_receipt: false,
        }
    }

    fn header(&self) {
        if self.events {
            print_line(&encode(&Line::Header));
        }
    }

    fn line(&mut self, line: &Line) {
        if self.events {
            return print_line(&encode(line));
        }
        match line {
            Line::Step { text, .. } => print_line(text),
            Line::Note(text) => {
                if !self.in_receipt {
                    print_line("");
                    self.in_receipt = true;
                }
                print_line(text);
            }
            Line::Header | Line::End(_) => {}
        }
    }

    fn end(&self, outcome: &Outcome) {
        if self.events {
            return print_line(&encode(&Line::End(outcome.clone())));
        }
        let verdict = match outcome.kind {
            Kind::Done => return print_line(&format!("\ndone: {}", outcome.summary)),
            Kind::Refused => "refused",
            Kind::RolledBack => "rolled back",
            Kind::Unknown => "UNKNOWN STATE",
        };
        eprint_line(&format!("{verdict}: {}", outcome.summary));
    }
}

/// Write a line, ignoring a closed pipe: the run must finish its roll-back
/// even if whoever was reading has gone.
fn print_line(text: &str) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{text}");
    let _ = stdout.flush();
}

fn eprint_line(text: &str) {
    let _ = writeln!(io::stderr().lock(), "{text}");
}
