mod common;

use std::io::Read;
use std::process::{Command, Stdio};

use v100_unlock::cli::{parse, Cli, Command as Cmd};
use v100_unlock::meter::TOLERANCE_MHZ;
use v100_unlock::protocol::{self, Line, HEADER};
use v100_unlock::run::Goal;

fn args(line: &str) -> Vec<String> {
    line.split_whitespace().map(String::from).collect()
}

fn cli(line: &str) -> Result<Cli, String> {
    parse(&args(line))
}

#[test]
fn commands_and_options_parse() {
    assert_eq!(cli("apply").unwrap().command, Cmd::Run(Goal::Daily));
    assert_eq!(
        cli("apply --sprint").unwrap().command,
        Cmd::Run(Goal::Sprint)
    );
    assert_eq!(cli("restore").unwrap().command, Cmd::Run(Goal::Stock));
    assert_eq!(cli("status").unwrap().command, Cmd::Status);
    assert_eq!(cli("--help").unwrap().command, Cmd::Help);

    let full = cli("apply --events --simulate 4 --card 0000:05:00.0").unwrap();
    assert!(full.events);
    assert_eq!(full.simulate, Some(4));
    assert_eq!(full.cards.len(), 1);
}

#[test]
fn the_same_card_twice_is_one_card() {
    let twice = cli("apply --card 0000:05:00.0 --card 0000:05:00.0").unwrap();
    assert_eq!(twice.cards.len(), 1);
}

#[test]
fn bad_command_lines_are_refused() {
    for bad in [
        "",
        "overclock",
        "restore --sprint",
        "status --events",
        "apply --card",
        "apply --card 08:00.0",
        "apply --simulate 0",
        "apply --simulate 65",
        "apply --simulate many",
        "apply --turbo",
    ] {
        assert!(cli(bad).is_err(), "{bad:?}");
    }
}

/// Run the binary. With `keep_stdin_open`, stdin is a pipe held open until it
/// exits. Otherwise stdin is /dev/null, which `--events` reads as an immediate
/// cancel. (Not a dropped pipe: on macOS a sibling test's child, spawned at the
/// same moment, can inherit the pipe's write end and keep it open.)
fn v100_unlock(line: &str, keep_stdin_open: bool) -> (i32, String, String) {
    let stdin = if keep_stdin_open {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = Command::new(common::bin())
        .args(args(line))
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn v100-unlock");
    let held_open = child.stdin.take();
    let out = read_all(child.stdout.take());
    let err = read_all(child.stderr.take());
    let code = child.wait().unwrap().code().unwrap();
    drop(held_open);
    (code, out, err)
}

fn read_all(pipe: Option<impl Read>) -> String {
    let mut text = String::new();
    pipe.expect("piped").read_to_string(&mut text).unwrap();
    text
}

#[test]
fn events_mode_streams_a_parseable_run() {
    let (code, out, _) = v100_unlock("apply --simulate 4 --events", true);
    assert_eq!(code, 0, "{out}");
    let lines: Vec<Line> = out.lines().map(|l| protocol::parse(l).unwrap()).collect();
    assert_eq!(out.lines().next(), Some(HEADER));
    assert_eq!(
        out.lines().last(),
        Some("end ok hbm 877.5 → 1053.0 MHz (+20%) · 4 cards")
    );
    assert!(
        lines
            .iter()
            .filter(|l| matches!(l, Line::Step { .. }))
            .count()
            >= 12
    );
}

#[test]
fn closing_stdin_cancels_an_events_run() {
    // Whether the cancel lands before or after the first write is a race;
    // either way the end line says which, and the exit code agrees.
    let (code, out, _) = v100_unlock("apply --simulate 2 --events", false);
    let last = out.lines().last().unwrap();
    match code {
        2 => assert_eq!(last, "end refused cancelled; nothing was changed"),
        3 => assert!(
            last.starts_with("end rolled-back cancelled; every card is back"),
            "{last}"
        ),
        _ => panic!("exit {code}: {out}"),
    }
}

#[test]
fn human_mode_prints_the_steps_the_receipt_and_the_result() {
    let (code, out, _) = v100_unlock("apply --simulate 1", false);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("found V100-SXM2-16GB\n"), "{out}");
    assert!(
        out.contains("\n04:00.0 · V100-SXM2-16GB · 877.5 → 1053.0 MHz"),
        "{out}"
    );
    assert!(
        out.ends_with("done: hbm 877.5 → 1053.0 MHz (+20%) · 1 card\n"),
        "{out}"
    );
}

#[test]
fn card_selects_a_subset() {
    let (code, out, _) = v100_unlock("apply --simulate 3 --card 0000:05:00.0 --events", true);
    assert_eq!(code, 0, "{out}");
    assert!(out.ends_with("· 1 card\n"), "{out}");
}

#[test]
fn a_card_that_is_not_there_is_refused() {
    let (code, _, err) = v100_unlock("apply --simulate 2 --card 0000:09:00.0", false);
    assert_eq!(code, 2);
    assert!(err.contains("no Tesla V100 at 0000:09:00.0"), "{err}");
}

#[test]
fn status_reads_every_card() {
    let (code, out, _) = v100_unlock("status --simulate 2", false);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.lines().count(), 2, "{out}");
    for line in out.lines() {
        assert!(line.contains("877.5 MHz (NDIV 65)"), "{line}");
        let metered: f64 = line.rsplit("metered ").next().unwrap().parse().unwrap();
        // Wall-clock jitter moves a simulated reading by ~0.1 MHz, as on hardware.
        assert!((metered - 877.5).abs() <= TOLERANCE_MHZ, "{line}");
    }
}

#[test]
fn a_usage_error_exits_2_and_points_at_help() {
    let (code, _, err) = v100_unlock("apply --turbo", false);
    assert_eq!(code, 2);
    assert!(err.contains("--help"), "{err}");
}

#[test]
fn real_hardware_needs_root() {
    if unsafe { libc::geteuid() } == 0 {
        return; // as root this would touch real cards
    }
    let (code, _, err) = v100_unlock("apply", false);
    assert_eq!(code, 2);
    assert!(err.contains("sudo"), "{err}");
}
