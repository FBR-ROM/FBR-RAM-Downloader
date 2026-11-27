//! End-to-end CLI tests for the `fbr-ram-downloader` binary.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_fbr-ram-downloader")
}

/// Find the `v100-unlock` binary built alongside `fbr-ram-downloader` in the
/// same target dir, falling back to the committed `demo/v100-unlock`.
fn agent_bin() -> PathBuf {
    let exe = PathBuf::from(bin());
    let sibling = exe.parent().unwrap().join("v100-unlock");
    if sibling.is_file() {
        return sibling;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("demo/v100-unlock")
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn fbr-ram-downloader");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn help_and_version_exit_zero_cleanly() {
    for flag in ["-h", "--help"] {
        let (code, out, err) = run(&[flag]);
        assert_eq!(code, 0, "{flag}: {err}");
        assert!(out.contains("usage: fbr-ram-downloader"), "{out}");
        assert!(err.is_empty(), "{err}");
    }
    for flag in ["-V", "--version"] {
        let (code, out, err) = run(&[flag]);
        assert_eq!(code, 0, "{flag}: {err}");
        assert_eq!(
            out.trim(),
            format!("fbr-ram-downloader {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(err.is_empty(), "{err}");
    }
}

#[test]
fn unknown_and_malformed_flags_exit_two_without_running_anything() {
    for bad in [
        &["--dryrun"][..],
        &["--bogus"][..],
        &["--host"][..],
        &["--music"][..],
        &["--dump", "5", "80"][..],
    ] {
        let (code, out, err) = run(bad);
        assert_eq!(code, 2, "{bad:?}: stdout={out:?} stderr={err:?}");
        assert!(out.is_empty(), "{bad:?}: {out}");
        assert!(!err.is_empty(), "{bad:?}: expected error on stderr");
    }
}

#[test]
fn layout_probe_prints_expected_geometry() {
    let (code, out, err) = run(&["--layout", "80", "24"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("terminal 80x24"), "{out}");
    assert!(out.contains("too_small false"), "{out}");

    let (code2, out2, err2) = run(&["--layout", "20", "8"]);
    assert_eq!(code2, 0, "{err2}");
    assert!(out2.contains("terminal 20x8"), "{out2}");
    assert!(out2.contains("too_small true"), "{out2}");
}

#[test]
fn dump_is_deterministic_and_honors_trailing_flags() {
    let (c1, out1, err1) = run(&["--dump", "4", "80", "24"]);
    let (c2, out2, err2) = run(&["--dump", "4", "80", "24"]);
    assert_eq!(c1, 0, "{err1}");
    assert_eq!(c2, 0, "{err2}");
    assert_eq!(out1, out2, "--dump must be byte-for-byte deterministic");
    let frames: Vec<&str> = out1.split("\n\n").filter(|f| !f.trim().is_empty()).collect();
    assert_eq!(frames.len(), 4);
    assert_eq!(frames[0].lines().count(), 24);
    assert!(!out1.contains("\x1b[48;"), "must never emit background colors");

    // Trailing --grey must not be swallowed by --dump.
    let (cg, outg, errg) = run(&["--dump", "2", "60", "20", "--grey"]);
    assert_eq!(cg, 0, "{errg}");
    // Grey palette shade 1 is (178, 175, 184).
    assert!(
        outg.contains("\x1b[38;2;178;175;184m"),
        "expected Grey palette ANSI color"
    );
}

#[test]
fn headless_demo_modes_run_end_to_end() {
    let agent = agent_bin();
    assert!(
        agent.is_file(),
        "v100-unlock binary missing at {}",
        agent.display()
    );

    for (args, expected_tail) in [
        (
            &["--headless", "--demo", "--dry-run"][..],
            "dry run: hbm 877.5 → 1053.0 MHz (+20%) · 4 cards · nothing written",
        ),
        (
            &["--headless", "--demo"][..],
            "done: hbm 877.5 → 1053.0 MHz (+20%) · 4 cards",
        ),
        (
            &["--headless", "--demo", "--sprint"][..],
            "done: hbm 877.5 → 1080.0 MHz (+23%) · 4 cards",
        ),
    ] {
        let out = Command::new(bin())
            .env("FBR_V100_AGENT", &agent)
            .args(args)
            .output()
            .expect("spawn headless demo");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?} failed: stdout={stdout}\nstderr={stderr}"
        );
        assert!(
            stdout.trim_end().ends_with(expected_tail),
            "{args:?}: expected tail {expected_tail:?}, got:\n{stdout}"
        );
    }
}
