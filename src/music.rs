//! Optional background music for the cracktro.
//!
//! Looks for a track in `music/` (or an explicit `--music FILE`) and plays it
//! through the first available player, looping. If no player is installed it
//! silently no-ops (the cracktro still runs). Stopped cleanly on exit.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const PLAYERS: &[&str] = &["mpv", "ffplay", "paplay", "aplay"];
const EXTS: &[&str] = &["mp3", "ogg", "flac", "wav", "m4a", "opus", "aac"];

pub struct Music {
    child: Option<Child>,
    /// The resolved track, remembered so the player can be restarted after a
    /// toggle. None when there is no track (or `--no-music`).
    track: Option<PathBuf>,
}

impl Music {
    /// A no-op music handle (no track / no player).
    pub fn none() -> Self {
        Music { child: None, track: None }
    }
    /// Resolve the track to play: explicit path, else the first audio file in
    /// `music/`. Returns None if nothing is found.
    pub fn resolve(explicit: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = explicit {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
            eprintln!("music: file not found: {p}");
            return None;
        }
        let dir = Path::new("music");
        if !dir.is_dir() {
            return None;
        }
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| EXTS.contains(&e.to_lowercase().as_str()))
                        .unwrap_or(false)
            })
            .collect();
        entries.sort();
        entries.into_iter().next()
    }

    /// Start playing `path` (looped) if a player is available.
    pub fn start(path: &Path) -> Self {
        let mut m = Music { child: None, track: Some(path.to_path_buf()) };
        m.spawn();
        m
    }

    /// Spawn the player for the remembered track (no-op without one).
    fn spawn(&mut self) {
        let Some(path) = self.track.clone() else {
            return;
        };
        let path = path.as_path();
        for player in PLAYERS {
            if which(player).is_none() {
                continue;
            }
            let p = path.to_string_lossy().into_owned();
            let args = match *player {
                "mpv" => vec![
                    "--loop",
                    "inf",
                    "--really-quiet",
                    "--no-video",
                    p.as_str(),
                ],
                "ffplay" => vec![
                    "-loop",
                    "-1",
                    "-nodisp",
                    "-autoexit",
                    "-loglevel",
                    "quiet",
                    p.as_str(),
                ],
                // paplay / aplay: no loop flag; we re-spawn on exit.
                _ => vec![p.as_str()],
            };
            match Command::new(player)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => {
                    eprintln!("music: playing {} via {player}", path.display());
                    self.child = Some(child);
                    return;
                }
                Err(e) => eprintln!("music: failed to start {player}: {e}"),
            }
        }
        eprintln!(
            "music: no player found (tried {}); install mpv or ffplay",
            PLAYERS.join(", ")
        );
    }

    /// Toggle playback: stop if playing, restart the remembered track if not.
    /// A no-op when there is no track (no `music/` file or `--no-music`).
    pub fn toggle(&mut self) {
        if self.child.is_some() {
            self.stop();
            eprintln!("music: off");
        } else if self.track.is_some() {
            self.spawn();
            eprintln!("music: on");
        }
    }

    /// Stop the player.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Music {
    fn drop(&mut self) {
        self.stop();
    }
}

fn which(bin: &str) -> Option<PathBuf> {
    let var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&var) {
        let cand = dir.join(bin);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}
