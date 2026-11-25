//! A run that is told to stop puts every card back: SIGINT (Ctrl-C),
//! SIGTERM, and SIGHUP (what a dropped ssh session sends).

mod common;

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};

/// Start a 4-card simulated run and wait until it has changed a card: the
/// 500 ms measurement is next, so a signal now lands mid-run.
#[expect(
    clippy::zombie_processes,
    reason = "the caller waits on the child it gets back; the failure path waits before panicking"
)]
fn start_and_wait_for_a_change() -> (Child, BufReader<std::process::ChildStdout>, String) {
    let mut child = Command::new(common::bin())
        .args(["apply", "--simulate", "4"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn v100-unlock");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut seen = String::new();
    loop {
        let mut line = String::new();
        let read = stdout.read_line(&mut line);
        if !matches!(read, Ok(n) if n > 0) {
            let status = child.wait();
            panic!("run ended ({read:?}, {status:?}) before changing a card:\n{seen}");
        }
        seen.push_str(&line);
        if line.contains("locked in") {
            return (child, stdout, seen);
        }
    }
}

fn finish(
    mut child: Child,
    mut stdout: BufReader<std::process::ChildStdout>,
    mut seen: String,
) -> (i32, String, String) {
    stdout.read_to_string(&mut seen).unwrap();
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    (
        child.wait().unwrap().code().expect("exited, not killed"),
        seen,
        err,
    )
}

fn send(child: &Child, signal: libc::c_int) {
    // SAFETY: kill(2) on a child process we own.
    assert_eq!(unsafe { libc::kill(child.id() as libc::pid_t, signal) }, 0);
}

fn assert_put_back(code: i32, out: &str, err: &str) {
    assert_eq!(code, 3, "{out}{err}");
    assert!(
        err.contains("rolled back: cancelled; every card is back where it started"),
        "{err}"
    );
    assert!(
        out.contains("04:00.0 · V100-SXM2-16GB · back at 877.5 MHz"),
        "{out}"
    );
    assert!(!out.contains("done:"), "{out}");
}

#[test]
fn ctrl_c_mid_run_puts_every_card_back() {
    let (child, stdout, seen) = start_and_wait_for_a_change();
    send(&child, libc::SIGINT);
    send(&child, libc::SIGINT); // an impatient second Ctrl-C must not abandon the roll-back
    let (code, out, err) = finish(child, stdout, seen);
    assert_put_back(code, &out, &err);
}

#[test]
fn sigterm_mid_run_puts_every_card_back() {
    let (child, stdout, seen) = start_and_wait_for_a_change();
    send(&child, libc::SIGTERM);
    let (code, out, err) = finish(child, stdout, seen);
    assert_put_back(code, &out, &err);
}

#[test]
fn a_dropped_ssh_session_mid_run_puts_every_card_back() {
    let (child, stdout, seen) = start_and_wait_for_a_change();
    send(&child, libc::SIGHUP);
    let (code, out, err) = finish(child, stdout, seen);
    assert_put_back(code, &out, &err);
}

#[test]
fn a_reader_that_goes_away_does_not_stop_the_roll_back() {
    // The front end closes our stdin (cancel) and stops reading our stdout:
    // output fails with EPIPE, and the run must still finish putting cards back.
    let mut child = Command::new(common::bin())
        .args(["apply", "--simulate", "4", "--events"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn v100-unlock");
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.contains("locked in") {
        line.clear();
        assert_ne!(stdout.read_line(&mut line).unwrap(), 0, "run ended early");
    }
    drop(child.stdin.take());
    drop(stdout);
    let code = child
        .wait()
        .unwrap()
        .code()
        .expect("exited, not killed by SIGPIPE");
    assert_eq!(code, 3);
}
