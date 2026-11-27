//! Release artifact, hermetic dynamic-linker, and installer (`install.sh`) tests.
//!
//! Skipped inside Nix build sandboxes (`NIX_BUILD_TOP` set) where `demo/` is
//! intentionally excluded from `cleanSrc`. Everywhere else (local dev, CI),
//! these tests enforce that the committed `demo/` bundle is 100% self-contained
//! and that `install.sh` installs, verifies, updates, and uninstalls cleanly.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

fn in_nix_sandbox() -> bool {
    std::env::var_os("NIX_BUILD_TOP").is_some()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fbr-release-test-{tag}-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

#[test]
fn demo_bundle_files_and_elf_dependencies_are_complete() {
    if in_nix_sandbox() {
        return;
    }
    let demo = repo_root().join("demo");
    let lib = demo.join("lib");

    for rel in [
        "fbr-ram-downloader",
        "v100-unlock",
        "run.sh",
        "music/Arcane Magic.mp3",
        "lib/ld-linux-x86-64.so.2",
        "lib/libc.so.6",
        "lib/libm.so.6",
        "lib/libpthread.so.0",
        "lib/librt.so.1",
        "lib/libgcc_s.so.1",
        "lib/libasound.so.2",
    ] {
        let p = demo.join(rel);
        assert!(p.is_file(), "missing bundle artifact: {}", p.display());
    }

    if let Some(readelf) = find_in_path("readelf") {
        // v100-unlock must be a fully static binary (no PT_INTERP, no DT_NEEDED).
        let ph = Command::new(&readelf)
            .args(["-l"])
            .arg(demo.join("v100-unlock"))
            .output()
            .unwrap();
        let ph_text = String::from_utf8_lossy(&ph.stdout);
        assert!(
            !ph_text.contains("INTERP"),
            "demo/v100-unlock must not have PT_INTERP:\n{ph_text}"
        );
        let dyn_agent = Command::new(&readelf)
            .args(["-d"])
            .arg(demo.join("v100-unlock"))
            .output()
            .unwrap();
        let dyn_agent_text = String::from_utf8_lossy(&dyn_agent.stdout);
        assert!(
            !dyn_agent_text.contains("(NEEDED)"),
            "demo/v100-unlock must have no DT_NEEDED:\n{dyn_agent_text}"
        );

        // Every DT_NEEDED of demo/fbr-ram-downloader and every bundled .so must
        // exist inside demo/lib/.
        let mut to_check = vec![demo.join("fbr-ram-downloader")];
        for entry in fs::read_dir(&lib).unwrap().flatten() {
            let p = entry.path();
            if p.is_file() {
                to_check.push(p);
            }
        }
        for elf in to_check {
            let out = Command::new(&readelf).args(["-d"]).arg(&elf).output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.contains("(NEEDED)") {
                    if let Some(start) = line.find('[') {
                        if let Some(end) = line[start + 1..].find(']') {
                            let soname = &line[start + 1..start + 1 + end];
                            assert!(
                                lib.join(soname).is_file(),
                                "{} needs {soname}, which is missing from demo/lib/",
                                elf.display()
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn demo_bundle_runs_hermetically_with_zero_host_library_leaks() {
    if in_nix_sandbox() {
        return;
    }
    let demo = repo_root().join("demo");
    let lib = demo.join("lib");
    let lib_str = lib.to_string_lossy().into_owned();
    let loader = lib.join("ld-linux-x86-64.so.2");
    let bin = demo.join("fbr-ram-downloader");

    for args in [
        &["--layout", "80", "24"][..],
        &["--dump", "2", "80", "24"][..],
        &["--headless", "--demo", "--dry-run"][..],
    ] {
        let out = Command::new(&loader)
            .env("LD_DEBUG", "libs")
            .env("FBR_BUNDLED_ALSA", "1")
            .args(["--inhibit-cache", "--inhibit-rpath", "", "--library-path"])
            .arg(&lib)
            .arg(&bin)
            .args(args)
            .output()
            .expect("run bundled loader");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(0),
            "hermetic run {args:?} failed:\nstdout={stdout}\nstderr={stderr}"
        );
        // Every `calling init: <path>` must resolve inside demo/lib/.
        for line in stderr.lines() {
            if let Some(rest) = line.split("calling init: ").nth(1) {
                let loaded = rest.trim();
                assert!(
                    loaded.contains(&lib_str) || loaded.ends_with("ld-linux-x86-64.so.2"),
                    "host library leak detected in {args:?}: {loaded}"
                );
            }
        }
    }

    // demo/run.sh must work regardless of current working directory.
    let run_out = Command::new(demo.join("run.sh"))
        .current_dir("/tmp")
        .args(["--dump", "2", "80", "24"])
        .output()
        .expect("run demo/run.sh from /tmp");
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "demo/run.sh from /tmp failed: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
}

#[test]
fn installer_cli_validation_and_dry_run() {
    if in_nix_sandbox() {
        return;
    }
    let script = repo_root().join("install.sh");

    let help = Command::new(&script).arg("--help").output().unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("install.sh"));

    let bad_opt = Command::new(&script).arg("--nope").output().unwrap();
    assert_eq!(bad_opt.status.code(), Some(2));

    let rel_prefix = Command::new(&script)
        .args(["--dry-run", "--prefix", "relative/path", "--user", "alice"])
        .output()
        .unwrap();
    assert_eq!(rel_prefix.status.code(), Some(2));

    let root_user = Command::new(&script)
        .args(["--dry-run", "--prefix", "/tmp/fbr-unused", "--user", "root"])
        .output()
        .unwrap();
    assert_eq!(root_user.status.code(), Some(1));

    let bad_user = Command::new(&script)
        .args([
            "--dry-run",
            "--prefix",
            "/tmp/fbr-unused",
            "--user",
            "bad name",
        ])
        .output()
        .unwrap();
    assert_eq!(bad_user.status.code(), Some(2));

    // Dry run succeeds without root and touches nothing.
    let tmp = temp_dir("dry");
    let prefix = tmp.join("prefix");
    let dry = Command::new(&script)
        .args(["--dry-run", "--prefix"])
        .arg(&prefix)
        .args(["--user", "alice"])
        .output()
        .unwrap();
    assert_eq!(
        dry.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(!prefix.exists(), "dry-run must not create prefix");
    let _ = fs::remove_dir_all(&tmp);
}

fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn installer_full_sandboxed_install_reinstall_and_uninstall() {
    if in_nix_sandbox() {
        return;
    }
    let script = repo_root().join("install.sh");
    let real_install = find_in_path("install").expect("real `install` on PATH");

    let tmp = temp_dir("sandbox-install");
    let shim = tmp.join("shim");
    let prefix = tmp.join("prefix");
    let sudoers_dir = tmp.join("sudoers.d");
    fs::create_dir_all(&shim).unwrap();

    // Fake `sudo`: strip `-n` and run the command directly as the current user.
    write_executable(
        &shim.join("sudo"),
        "#!/usr/bin/env bash\nif [[ \"${1:-}\" == \"-n\" ]]; then shift; fi\nexec \"$@\"\n",
    );
    // Fake `install`: strip `-o <user>` and `-g <group>` so unprivileged tests
    // can exercise the exact install.sh code path, including mode 0440.
    write_executable(
        &shim.join("install"),
        &format!(
            "#!/usr/bin/env bash\nargs=()\nwhile [[ $# -gt 0 ]]; do\n  case \"$1\" in\n    -o|-g) shift 2 ;;\n    *) args+=(\"$1\"); shift ;;\n  esac\ndone\nexec {:?} \"${{args[@]}}\"\n",
            real_install
        ),
    );
    // Fake `visudo`: verify the rule line has the expected NOPASSWD syntax.
    write_executable(
        &shim.join("visudo"),
        "#!/usr/bin/env bash\ngrep -q 'ALL=(root) NOPASSWD:' \"$2\"\n",
    );

    let orig_path = std::env::var("PATH").unwrap_or_default();
    let test_path = format!("{}:{orig_path}", shim.display());

    // 1. Fresh install.
    let res = Command::new(&script)
        .env("PATH", &test_path)
        .env("FBR_SUDOERS_DIR", &sudoers_dir)
        .args(["--prefix"])
        .arg(&prefix)
        .args(["--user", "testuser"])
        .output()
        .expect("run sandboxed install.sh");
    assert_eq!(
        res.status.code(),
        Some(0),
        "install failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );

    let launcher = prefix.join("bin/fbr-ram-downloader");
    let agent = prefix.join("sbin/v100-unlock");
    let sudoers = sudoers_dir.join("fbr-ram-downloader");
    assert!(launcher.is_file());
    assert!(agent.is_file());
    assert!(sudoers.is_file());
    assert_eq!(
        fs::metadata(&sudoers).unwrap().permissions().mode() & 0o777,
        0o440
    );
    assert_eq!(
        fs::read_to_string(&sudoers).unwrap().trim(),
        format!("testuser ALL=(root) NOPASSWD: {}", agent.display())
    );

    // 2. Installed launcher works for both --headless --demo and --dump.
    let dump = Command::new(&launcher)
        .args(["--dump", "2", "80", "24"])
        .output()
        .unwrap();
    assert_eq!(dump.status.code(), Some(0));

    // 3. Re-running install.sh is idempotent.
    let res2 = Command::new(&script)
        .env("PATH", &test_path)
        .env("FBR_SUDOERS_DIR", &sudoers_dir)
        .args(["--prefix"])
        .arg(&prefix)
        .args(["--user", "testuser"])
        .output()
        .unwrap();
    assert_eq!(
        res2.status.code(),
        Some(0),
        "re-install failed: {}",
        String::from_utf8_lossy(&res2.stderr)
    );

    // 4. --uninstall removes every installed file and sudoers rule.
    let un = Command::new(&script)
        .env("PATH", &test_path)
        .env("FBR_SUDOERS_DIR", &sudoers_dir)
        .args(["--uninstall", "--prefix"])
        .arg(&prefix)
        .output()
        .unwrap();
    assert_eq!(
        un.status.code(),
        Some(0),
        "uninstall failed: {}",
        String::from_utf8_lossy(&un.stderr)
    );
    assert!(!launcher.exists());
    assert!(!agent.exists());
    assert!(!prefix.join("lib/fbr-ram-downloader").exists());
    assert!(!sudoers.exists());

    let _ = fs::remove_dir_all(&tmp);
}
