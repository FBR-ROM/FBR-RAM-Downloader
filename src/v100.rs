//! The real hack: runs `v100-unlock apply --events` as root, on this machine
//! or on a V100 host over ssh, and shows its progress.
//!
//! Closing v100-unlock's stdin cancels it, and it puts every card back. So
//! quitting the cracktro, a crash, or a dropped connection can never leave a
//! card half-changed.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use v100_unlock::outcome::{Kind, Outcome};
use v100_unlock::protocol::{self, Line, HEADER};

use crate::hack::{Hack, HackState};

/// Where v100-unlock is installed. The sudoers rule names this exact path.
const AGENT: &str = "/usr/local/sbin/v100-unlock";
/// Each status line stays up at least this long, so a run that takes half a
/// second still reads as a sequence of steps.
const DWELL: Duration = Duration::from_millis(600);
/// How long to wait for v100-unlock to start (ssh connect, a sudo prompt).
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// How long to wait, after quitting, for the cards to be put back.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// A line from v100-unlock, or one it could not have sent.
type Received = Result<Line, String>;

pub struct V100Hack {
    host: Option<String>,
    sprint: bool,
    dry_run: bool,
    show: Presenter,
    started: Instant,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    lines: Option<Receiver<Received>>,
}

impl V100Hack {
    /// `host`: a V100 machine to reach over ssh; `None` for this machine.
    pub fn new(host: Option<String>, sprint: bool, dry_run: bool, fast: bool) -> Self {
        let dwell = if fast { Duration::ZERO } else { DWELL };
        Self {
            host,
            sprint,
            dry_run,
            show: Presenter::new(dwell),
            started: Instant::now(),
            child: None,
            stdin: None,
            lines: None,
        }
    }

    fn place(&self) -> String {
        self.host.clone().unwrap_or_else(|| "this machine".into())
    }

    fn command(&self) -> Result<Command, String> {
        // The cracktro always drives the real agent with the machine-readable
        // protocol; `--demo` is handled by StubHack, never the agent.
        agent_command(&self.host, self.sprint, self.dry_run, false, true)
    }

    /// Wait for v100-unlock to answer, and for its first word after that, so
    /// a refusal (no cards, a card in a bad state) is reported before the
    /// cracktro takes over the screen.
    fn handshake(&mut self, lines: &Receiver<Received>, stderr: JoinHandle<String>) -> Result<(), String> {
        let place = self.place();
        let first = lines.recv_timeout(START_TIMEOUT);
        match first {
            Ok(Ok(Line::Header)) => {}
            Ok(Err(raw)) if raw.starts_with("v100-unlock ") => {
                return Err(format!("v100-unlock on {place} speaks {raw:?}, this cracktro speaks {HEADER:?}: update one"));
            }
            _ => {
                self.stop_child();
                return Err(explain(&stderr.join().unwrap_or_default(), &place));
            }
        }
        match lines.recv_timeout(START_TIMEOUT) {
            Ok(Ok(Line::End(outcome))) if outcome.kind != Kind::Done => Err(outcome.summary),
            Ok(Ok(line)) => {
                self.show.push(line);
                Ok(())
            }
            _ => Err(format!("v100-unlock on {place} stopped answering")),
        }
    }

    fn stop_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn now(&self) -> Duration {
        self.started.elapsed()
    }

    /// How a run ends when v100-unlock's output stops before it says.
    fn lost(&self) -> Outcome {
        let what = match &self.host {
            Some(host) => format!("lost the connection to {host} mid-run"),
            None => "lost v100-unlock mid-run".into(),
        };
        Outcome::new(Kind::Unknown, format!("{what}: check the cards with `v100-unlock status`"))
    }
}

/// Turn what v100-unlock or ssh printed on stderr into one plain sentence.
fn explain(stderr: &str, place: &str) -> String {
    let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output");
    if stderr.contains("password is required") {
        format!("sudo on {place} wants a password for v100-unlock: install its sudoers rule (see README)")
    } else if stderr.contains(AGENT) && (stderr.contains("not found") || stderr.contains("No such file")) {
        format!("v100-unlock is not installed at {AGENT} on {place} (see README)")
    } else {
        format!("could not start v100-unlock on {place}: {}", protocol::clean(last))
    }
}

/// The agent a real (register-writing) run uses: `FBR_V100_AGENT` if the
/// installer set it (a custom `--prefix`), else the standard installed
/// location named by the sudoers rule.
fn real_agent_path() -> String {
    if let Ok(p) = std::env::var("FBR_V100_AGENT") {
        if !p.is_empty() {
            return p;
        }
    }
    AGENT.to_string()
}

/// The agent a simulated run uses: `FBR_V100_AGENT`, then the standard
/// location, then next to this binary — the demo bundle ships both binaries
/// together, so `--headless --demo` works straight from it.
fn demo_agent_path() -> String {
    if let Ok(p) = std::env::var("FBR_V100_AGENT") {
        if !p.is_empty() {
            return p;
        }
    }
    let installed = std::path::Path::new(AGENT);
    if installed.exists() {
        return AGENT.to_string();
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("v100-unlock");
            if sibling.exists() {
                return sibling.to_string_lossy().into_owned();
            }
        }
    }
    AGENT.to_string()
}

/// Build the command that runs the V100 agent, on this machine (via sudo) or
/// on `host` over ssh. Shared by the cracktro (which reads the `--events`
/// protocol) and `--headless` (which reads plain text).
///
/// `events` selects the machine-readable protocol; `demo` runs a simulated
/// 4-card run instead of touching hardware.
pub fn agent_command(
    host: &Option<String>,
    sprint: bool,
    dry_run: bool,
    demo: bool,
    events: bool,
) -> Result<Command, String> {
    let mut args = vec!["apply"];
    if events {
        args.push("--events");
    }
    if sprint {
        args.push("--sprint");
    }
    if dry_run {
        args.push("--dry-run");
    }
    if demo {
        args.push("--demo");
    }
    match host {
        Some(host) if host.is_empty() || host.starts_with('-') || host.contains(char::is_whitespace) => {
            Err(format!("{host:?} is not a host name"))
        }
        Some(host) => {
            // On the remote, the agent is the installed one.
            let mut ssh = Command::new("ssh");
            // BatchMode: fail instead of prompting under the cracktro's screen.
            // ServerAlive: notice a dead link within about 15 s instead of hanging.
            ssh.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]);
            ssh.args(["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2"]);
            ssh.args(["--", host, "sudo", "-n"]);
            ssh.arg(AGENT);
            ssh.args(args);
            Ok(ssh)
        }
        None if !cfg!(target_os = "linux") && !demo => {
            Err("no V100 here: use --host to reach a V100 machine, or --demo".into())
        }
        None => {
            // A simulated run touches no hardware and needs no root, so it runs
            // the agent directly (the demo bundle ships it next to the binary).
            // A real run writes the registers, so it goes through sudo to the
            // installed, sudoers-named agent.
            if demo {
                let agent = demo_agent_path();
                let mut direct = Command::new(&agent);
                direct.args(args);
                Ok(direct)
            } else {
                let agent = real_agent_path();
                let mut sudo = Command::new("sudo");
                sudo.arg(&agent);
                sudo.args(args);
                Ok(sudo)
            }
        }
    }
}

/// `--headless`: run the V100 agent and stream its plain-text output to
/// stdout, returning the agent's exit code. No terminal, no cracktro, no
/// pacing — the same all-or-nothing run, for scripts and systemd.
pub fn run_headless(
    host: &Option<String>,
    sprint: bool,
    dry_run: bool,
    demo: bool,
) -> std::process::ExitCode {
    let mut cmd = match agent_command(host, sprint, dry_run, demo, false) {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // Inherit the caller's stdin/stdout/stderr: the run streams live, and a
    // closed stdin (Ctrl-C, a dropped pipe) still cancels it and puts the
    // cards back, exactly as under the cracktro.
    let status = match cmd.status() {
        Ok(status) => status,
        Err(e) => {
            eprintln!("could not start v100-unlock: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // The agent's codes (0 at target, 2 untouched, 3 put back, 4 unknown)
    // all fit a u8; anything else is clamped to 1.
    std::process::ExitCode::from(status.code().unwrap_or(1).min(255) as u8)
}

/// Read v100-unlock's stdout on a thread; lines go to the channel.
fn read_lines(stdout: impl Read + Send + 'static) -> Receiver<Received> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for raw in BufReader::new(stdout).lines() {
            let Ok(raw) = raw else { break };
            let line = protocol::parse(&raw).map_err(|_| protocol::clean(&raw));
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

impl Hack for V100Hack {
    fn start(&mut self) -> Result<(), String> {
        let mut child = self
            .command()?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start v100-unlock on {}: {e}", self.place()))?;
        let lines = read_lines(child.stdout.take().expect("piped stdout"));
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stderr = thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        self.stdin = child.stdin.take();
        self.child = Some(child);
        self.handshake(&lines, stderr)?;
        self.lines = Some(lines);
        self.started = Instant::now();
        Ok(())
    }

    fn poll(&mut self) -> HackState {
        let mut closed = false;
        if let Some(lines) = &self.lines {
            loop {
                match lines.try_recv() {
                    Ok(line) => self.show.push(line.unwrap_or_else(|raw| Line::Note(format!("(unexpected) {raw}")))),
                    Err(TryRecvError::Empty) => break,
                    // Every line has been read, so a missing end is really missing.
                    Err(TryRecvError::Disconnected) => {
                        closed = true;
                        break;
                    }
                }
            }
        }
        if closed && self.show.outcome.is_none() {
            self.show.push(Line::End(self.lost()));
        }
        self.show.tick(self.now())
    }

    fn finish(&mut self) -> Result<(), String> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if self.show.outcome.is_none() {
            say("stopping: putting the cards back...");
        }
        drop(self.stdin.take()); // closing stdin cancels a run that has not ended
        if let Some(lines) = self.lines.take() {
            let deadline = Instant::now() + STOP_TIMEOUT;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                match lines.recv_timeout(left) {
                    Ok(Ok(line)) => self.show.push(line),
                    Ok(Err(_)) => {}
                    Err(_) => break,
                }
            }
        }
        let _ = child.kill(); // already exited unless it hung past the deadline
        let _ = child.wait();
        for note in &self.show.receipt {
            say(note);
        }
        // A failure the screen showed ends the cracktro with it as the error,
        // which main prints; say every other ending here.
        if self.show.state.error.is_none() {
            let outcome = self.show.outcome.clone().unwrap_or_else(|| self.lost());
            say(&receipt_end(&outcome));
        }
        Ok(())
    }
}

fn receipt_end(outcome: &Outcome) -> String {
    let verdict = match outcome.kind {
        Kind::Done => "done",
        Kind::Dry => "dry run",
        Kind::Refused => "nothing changed",
        Kind::RolledBack => "rolled back",
        Kind::Unknown => "UNKNOWN STATE",
    };
    format!("{verdict}: {}", outcome.summary)
}

fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}

/// Paces v100-unlock's lines into the cracktro's state: each step is shown
/// for at least the dwell time, receipt lines are collected as they come,
/// and the end is shown last. The screen trails the real work, so what
/// v100-unlock has reported (`outcome`) is kept apart from what is shown.
pub struct Presenter {
    dwell: Duration,
    queue: VecDeque<Line>,
    shown_at: Option<Duration>,
    state: HackState,
    receipt: Vec<String>,
    /// How the run ended, as soon as v100-unlock says so.
    outcome: Option<Outcome>,
}

impl Presenter {
    pub fn new(dwell: Duration) -> Self {
        Self {
            dwell,
            queue: VecDeque::new(),
            shown_at: None,
            state: HackState {
                progress: 0.0,
                status: "connecting".into(),
                done: false,
                error: None,
                done_message: None,
            },
            receipt: Vec::new(),
            outcome: None,
        }
    }

    pub fn push(&mut self, line: Line) {
        match line {
            Line::Note(text) => self.receipt.push(text),
            Line::Header => {}
            Line::End(outcome) => {
                self.outcome = Some(outcome.clone());
                self.queue.push_back(Line::End(outcome));
            }
            step => self.queue.push_back(step),
        }
    }

    /// The state to draw at `now`, showing the next line once the current one
    /// has been up for the dwell time.
    pub fn tick(&mut self, now: Duration) -> HackState {
        let due = self.shown_at.is_none_or(|at| now.saturating_sub(at) >= self.dwell);
        if due {
            if let Some(line) = self.queue.pop_front() {
                self.show(line);
                self.shown_at = Some(now);
            }
        }
        self.state.clone()
    }

    fn show(&mut self, line: Line) {
        match line {
            Line::Step { progress, text } => {
                self.state.progress = progress;
                self.state.status = text;
            }
            Line::End(outcome) => match outcome.kind {
                Kind::Done | Kind::Dry => {
                    self.state.progress = 1.0;
                    self.state.status = outcome.summary.clone();
                    self.state.done = true;
                    // The receipt's headline is the completion message:
                    // "hbm 877.5 → 1053.0 MHz (+20%) · 4 cards", or the
                    // "dry run: ... · nothing written" line for a dry run.
                    self.state.done_message = Some(outcome.summary);
                }
                _ => self.state.error = Some(receipt_end(&outcome)),
            },
            Line::Header | Line::Note(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(progress: f32, text: &str) -> Line {
        Line::Step { progress, text: text.into() }
    }

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn each_step_stays_up_for_the_dwell_time() {
        let mut show = Presenter::new(DWELL);
        show.push(step(0.1, "found 4 × V100-SXM2-16GB"));
        show.push(step(0.2, "card 1/4 · 04:00.0 · 877.5 MHz"));
        assert_eq!(show.tick(at(0)).status, "found 4 × V100-SXM2-16GB");
        assert_eq!(show.tick(at(599)).status, "found 4 × V100-SXM2-16GB");
        let state = show.tick(at(600));
        assert_eq!((state.status.as_str(), state.progress), ("card 1/4 · 04:00.0 · 877.5 MHz", 0.2));
    }

    #[test]
    fn the_end_comes_last_and_receipt_lines_do_not_wait() {
        let mut show = Presenter::new(DWELL);
        show.push(step(0.95, "card 4/4 · 07:00.0 · metered 1053.0 MHz"));
        show.push(Line::Note("04:00.0 · V100-SXM2-16GB · 877.5 → 1053.0 MHz".into()));
        show.push(Line::End(Outcome::new(Kind::Done, "hbm 877.5 → 1053.0 MHz (+20%) · 4 cards")));
        assert!(!show.tick(at(0)).done, "the last step is shown first");
        assert_eq!(show.receipt.len(), 1);
        assert!(show.outcome.is_some(), "the end counts as soon as it arrives");
        let end = show.tick(at(600));
        assert!(end.done && end.progress == 1.0 && end.error.is_none());
        assert_eq!(end.status, "hbm 877.5 → 1053.0 MHz (+20%) · 4 cards");
    }

    #[test]
    fn a_failed_run_becomes_an_error_naming_what_happened() {
        let mut show = Presenter::new(Duration::ZERO);
        show.push(Line::End(Outcome::new(Kind::RolledBack, "06:00.0: memory PLLs did not lock")));
        let state = show.tick(at(0));
        assert!(!state.done);
        assert_eq!(state.error.as_deref(), Some("rolled back: 06:00.0: memory PLLs did not lock"));
    }

    /// A hack whose v100-unlock sent `lines` and then closed its output.
    fn hack_that_read(lines: Vec<Line>) -> V100Hack {
        let (tx, rx) = mpsc::channel();
        for line in lines {
            tx.send(Ok(line)).unwrap();
        }
        let mut hack = V100Hack::new(Some("c4130".into()), false, false, true);
        hack.lines = Some(rx);
        hack
    }

    #[test]
    fn output_that_stops_before_the_end_is_a_lost_connection() {
        let mut hack = hack_that_read(vec![step(0.2, "card 1/4 · 04:00.0 · 1053.0 MHz · locked in 0.1 ms")]);
        assert_eq!(hack.poll().status, "card 1/4 · 04:00.0 · 1053.0 MHz · locked in 0.1 ms");
        assert_eq!(
            hack.poll().error.as_deref(),
            Some("UNKNOWN STATE: lost the connection to c4130 mid-run: check the cards with `v100-unlock status`")
        );
    }

    #[test]
    fn a_run_that_ended_is_never_lost_however_its_exit_races_its_output() {
        let mut hack = hack_that_read(vec![Line::End(Outcome::new(Kind::Done, "hbm at target · 4 cards"))]);
        for _ in 0..3 {
            let state = hack.poll();
            assert!(state.done && state.error.is_none(), "{state:?}");
        }
    }

    #[test]
    fn sudo_and_install_problems_are_explained() {
        let password = "sudo: a password is required\n";
        assert!(explain(password, "c4130").contains("install its sudoers rule"));
        let missing = "sudo: /usr/local/sbin/v100-unlock: command not found\n";
        assert!(explain(missing, "c4130").contains("not installed at /usr/local/sbin/v100-unlock on c4130"));
        let ssh = "ssh: connect to host c4130 port 22: Connection refused\n";
        assert_eq!(explain(ssh, "c4130"), "could not start v100-unlock on c4130: ssh: connect to host c4130 port 22: Connection refused");
    }

    #[test]
    fn a_host_cannot_smuggle_in_ssh_options() {
        for host in ["-oProxyCommand=sh", "", "c4130 rm"] {
            let hack = V100Hack::new(Some(host.into()), false, false, false);
            assert!(hack.command().is_err(), "{host:?}");
        }
        assert!(V100Hack::new(Some("c4130".into()), false, false, false).command().is_ok());
    }

    fn program_args(cmd: &Command) -> (String, Vec<String>) {
        (
            cmd.get_program().to_string_lossy().into_owned(),
            cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect(),
        )
    }

    #[test]
    fn the_cracktro_drives_the_installed_agent_with_the_protocol() {
        let (prog, args) = program_args(&agent_command(&None, false, false, false, true).unwrap());
        assert_eq!(prog, "sudo");
        assert_eq!(args, vec![AGENT, "apply", "--events"]);
    }

    #[test]
    fn a_real_headless_run_goes_through_sudo_too() {
        let (prog, args) = program_args(&agent_command(&None, true, true, false, false).unwrap());
        assert_eq!(prog, "sudo");
        assert_eq!(args, vec![AGENT, "apply", "--sprint", "--dry-run"]);
    }

    #[test]
    fn a_simulated_headless_run_needs_no_sudo() {
        let (prog, args) = program_args(&agent_command(&None, false, false, true, false).unwrap());
        // No sudo: a simulated run touches no hardware. The agent is the
        // installed one or the one next to this binary.
        assert_ne!(prog, "sudo");
        assert!(prog.ends_with("v100-unlock"), "{prog}");
        assert_eq!(args, vec!["apply", "--demo"]);
    }

    #[test]
    fn a_remote_run_reaches_the_installed_agent_over_ssh() {
        let (prog, args) = program_args(&agent_command(&Some("c4130".into()), false, false, false, true).unwrap());
        assert_eq!(prog, "ssh");
        // ... -- c4130 sudo -n <agent> apply --events
        let tail: Vec<&str> = args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
        assert!(tail.contains(&"c4130"), "{tail:?}");
        assert!(tail.contains(&"sudo"), "{tail:?}");
        assert!(tail.contains(&AGENT), "{tail:?}");
        assert!(tail.contains(&"--events"), "{tail:?}");
    }
}
