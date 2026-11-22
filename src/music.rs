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
