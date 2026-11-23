//! The command line.

use crate::card::Bdf;
use crate::run::Goal;

pub const USAGE: &str = "\
usage: v100-unlock <command> [options]

commands:
  status            show each Tesla V100's memory clock (read-only)
  apply             raise the memory clock to 1053 MHz (+20%)
  apply --sprint    raise it to 1080 MHz (+23%), for short runs
  restore           put the stock memory clock back

options:
  --card BDF        only this card, e.g. 0000:08:00.0 (repeatable)
  --events          machine-readable progress; closing stdin cancels
  --simulate N      N simulated cards: no hardware, no root
  -h, --help        this help

Every selected card changes, or none does. The change is live and volatile:
a reboot puts stock clocks back.

exit status: 0 at target, 2 nothing changed, 3 put back,
             4 a card is in an unknown state (power-cycle the host)
";

/// Most simulated cards `--simulate` accepts.
pub const MAX_SIMULATED: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Run(Goal),
    Help,
}

#[derive(Debug, PartialEq)]
pub struct Cli {
    pub command: Command,
    /// Cards to use; empty means every Tesla V100.
    pub cards: Vec<Bdf>,
    pub events: bool,
    pub simulate: Option<usize>,
}

pub fn parse(args: &[String]) -> Result<Cli, String> {
    let mut rest = args.iter();
    let command = match rest.next().map(String::as_str) {
        Some("status") => Command::Status,
        Some("apply") => Command::Run(Goal::Daily),
        Some("restore") => Command::Run(Goal::Stock),
        Some("-h" | "--help" | "help") => Command::Help,
        Some(other) => return Err(format!("unknown command {other:?}")),
        None => return Err("missing command".into()),
    };
    let mut cli = Cli {
        command,
        cards: Vec::new(),
        events: false,
        simulate: None,
    };
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--sprint" if cli.command == Command::Run(Goal::Daily) => {
                cli.command = Command::Run(Goal::Sprint)
            }
            "--sprint" => return Err("--sprint goes with apply".into()),
            "--card" => cli.cards.push(value(rest.next(), "--card")?.parse()?),
            "--events" => cli.events = true,
            "--simulate" => cli.simulate = Some(simulated(value(rest.next(), "--simulate")?)?),
            "-h" | "--help" => cli.command = Command::Help,
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    if cli.events && cli.command == Command::Status {
        return Err("--events goes with apply or restore".into());
    }
    cli.cards.sort();
    cli.cards.dedup();
    Ok(cli)
}

fn value<'a>(arg: Option<&'a String>, option: &str) -> Result<&'a str, String> {
    arg.map(String::as_str)
        .ok_or_else(|| format!("{option} needs a value"))
}

fn simulated(count: &str) -> Result<usize, String> {
    match count.parse() {
        Ok(n) if (1..=MAX_SIMULATED).contains(&n) => Ok(n),
        _ => Err(format!(
            "--simulate takes 1 to {MAX_SIMULATED} cards, not {count:?}"
        )),
    }
}
