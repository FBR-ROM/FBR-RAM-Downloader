//! Self-contained background music for the cracktro.
//!
//! Decodes and plays a track from `music/` (or an explicit `--music FILE`)
//! in-process via [`rodio`], looping. No external player (mpv/ffplay) is
//! required — the binary is fully self-contained. If no audio device is
//! available it silently no-ops (the cracktro still runs). Stopped cleanly on
//! exit.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, source::Source};

const EXTS: &[&str] = &["mp3", "ogg", "flac", "wav", "m4a", "opus", "aac"];

pub struct Music {
    /// Kept alive for the duration of playback; dropping it stops the audio
    /// thread.
    stream: Option<OutputStream>,
    /// Drives the looping source; dropping it stops the track.
    sink: Option<Sink>,
    /// The resolved track, remembered so playback can be restarted after a
    /// toggle. None when there is no track (or `--no-music`).
    track: Option<PathBuf>,
    /// Whether the sink is currently playing (vs. paused).
    playing: bool,
}

impl Music {
    /// A no-op music handle (no track / no audio device).
    pub fn none() -> Self {
        Music { stream: None, sink: None, track: None, playing: false }
    }

    /// Resolve the track to play: explicit path, else the first audio file in
    /// `music/`. The `music/` dir is looked up relative to the current working
    /// directory first (dev), then next to the binary (the self-contained
    /// `demo/` bundle). Returns None if nothing is found.
    pub fn resolve(explicit: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = explicit {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
            eprintln!("music: file not found: {p}");
            return None;
        }
        // Candidate music dirs: cwd first, then the binary's directory.
        // `argv[0]` is the binary's path as invoked; `current_exe()` is a
        // fallback, but it is NOT used first: when the binary is launched
        // through a bundled glibc loader (demo/run.sh), `current_exe()`
        // reports the *loader's* path, not the binary's.
        let mut dirs = vec![PathBuf::from("music")];
        let exe = std::env::args().next().map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok());
        if let Some(exe) = exe {
            if let Some(dir) = exe.parent() {
                let d = dir.join("music");
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
        for dir in dirs {
            if let Some(track) = first_track_in(&dir) {
                return Some(track);
            }
        }
        None
    }

    /// Start playing `path` (looped) on the default audio device.
    pub fn start(path: &Path) -> Self {
        let mut m = Music { stream: None, sink: None, track: Some(path.to_path_buf()), playing: false };
        m.spawn();
        m
    }

    /// Open the audio device and start the looping track (no-op without one).
    fn spawn(&mut self) {
        let Some(path) = self.track.clone() else {
            return;
        };
        let path = path.as_path();

        // Open the default output stream. If there's no audio device this
        // fails and we no-op (the cracktro still runs).
        let stream = match OutputStreamBuilder::open_default_stream() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("music: no audio device ({e}); running without music");
                return;
            }
        };

        // Decode the file and loop it.
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("music: cannot open {}: {e}", path.display());
                return;
            }
        };
        let decoder = match Decoder::try_from(BufReader::new(file)) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("music: cannot decode {}: {e}", path.display());
                return;
            }
        };

        let sink = Sink::connect_new(&stream.mixer());
        sink.append(decoder.repeat_infinite());
        sink.play();

        eprintln!("music: playing {} (in-process)", path.display());
        self.stream = Some(stream);
        self.sink = Some(sink);
        self.playing = true;
    }

    /// Toggle playback: pause if playing, resume if paused, start if stopped.
    /// A no-op when there is no track (no `music/` file or `--no-music`).
    pub fn toggle(&mut self) {
        match &self.sink {
            Some(sink) => {
                if self.playing {
                    sink.pause();
                    self.playing = false;
                    eprintln!("music: off");
                } else {
                    sink.play();
                    self.playing = true;
                    eprintln!("music: on");
                }
            }
            None if self.track.is_some() => {
                self.spawn();
            }
            None => {}
        }
    }

    /// Stop the player and release the audio device.
    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.stream = None;
        self.playing = false;
    }
}

impl Drop for Music {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The first audio file in `dir` (alphabetically), or None.
fn first_track_in(dir: &Path) -> Option<PathBuf> {
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
                    .map(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
                    .unwrap_or(false)
        })
        .collect();
    entries.sort();
    entries.into_iter().next()
}
