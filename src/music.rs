//! Self-contained background music for the cracktro.
//!
//! Decodes and plays a track from `music/` (or an explicit `--music FILE`)
//! in-process via [`rodio`], looping. No external player (mpv/ffplay) is
//! required — the binary is fully self-contained. If no audio device is
//! available it silently no-ops (the cracktro still runs). Stopped cleanly on
//! exit.

use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rodio::{source::Source, Decoder, OutputStream, OutputStreamBuilder, Sink};

const EXTS: &[&str] = &["mp3", "ogg", "flac", "wav", "m4a", "opus", "aac"];

/// Recorded once before this module ever touches `ALSA_CONFIG_PATH`, so a
/// user-provided override always wins while our own multi-card retries can
/// update the variable freely.
static USER_ALSA_CONFIG: OnceLock<bool> = OnceLock::new();

/// Background PCM streamer to PipeWire (`pw-cat`) or PulseAudio (`pacat`).
/// Used when bundled ALSA cannot load host sound-server plugins across glibc
/// versions, so audio mixes cleanly with the desktop instead of hitting EBUSY
/// on exclusive `hw:0,0`.
struct PipePlayer {
    playing: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    child_pid: u32,
    thread: Option<JoinHandle<()>>,
}

impl PipePlayer {
    fn start(decoder: Decoder<BufReader<File>>) -> Result<Self, String> {
        let rate = decoder.sample_rate();
        let channels = decoder.channels();
        let mut child = spawn_sound_server_pipe(rate, channels)?;
        let stdin = child.stdin.take().ok_or_else(|| "no pipe stdin".to_string())?;
        let child_pid = child.id();

        let playing = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let p_flag = Arc::clone(&playing);
        let s_flag = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let mut stdin = stdin;
            let mut buf = Vec::with_capacity(1024 * 4);
            let mut samples = decoder.repeat_infinite();
            while !s_flag.load(Ordering::Relaxed) {
                if !p_flag.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                buf.clear();
                for _ in 0..1024 {
                    if let Some(s) = samples.next() {
                        buf.extend_from_slice(&s.to_le_bytes());
                    } else {
                        break;
                    }
                }
                if buf.is_empty() || stdin.write_all(&buf).is_err() {
                    break;
                }
            }
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
        });

        Ok(PipePlayer {
            playing,
            stop,
            child_pid,
            thread: Some(handle),
        })
    }

    fn set_playing(&self, on: bool) {
        self.playing.store(on, Ordering::Relaxed);
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child_pid as libc::pid_t, libc::SIGTERM);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for PipePlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Spawn `pw-cat` (PipeWire) or `pacat` (PulseAudio) accepting raw `f32le` PCM
/// on stdin. Returns `Err` if neither is installed or if the process exits
/// immediately (no running sound server).
fn spawn_sound_server_pipe(rate: u32, channels: u16) -> Result<Child, String> {
    let rate_s = rate.to_string();
    let ch_s = channels.to_string();
    let candidates: [(&str, Vec<String>); 2] = [
        (
            "pw-cat",
            vec![
                "--playback".into(),
                "--raw".into(),
                "--format".into(),
                "f32".into(),
                "--rate".into(),
                rate_s.clone(),
                "--channels".into(),
                ch_s.clone(),
                "-".into(),
            ],
        ),
        (
            "pacat",
            vec![
                "--playback".into(),
                "--raw".into(),
                "--format=float32le".into(),
                format!("--rate={rate_s}"),
                format!("--channels={ch_s}"),
            ],
        ),
    ];

    for (prog, args) in candidates {
        let Ok(mut child) = Command::new(prog)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        // Give the client a brief moment to fail if no daemon is running.
        thread::sleep(Duration::from_millis(20));
        if matches!(child.try_wait(), Ok(None)) {
            return Ok(child);
        }
        let _ = child.wait();
    }
    Err("no PipeWire or PulseAudio stream available".into())
}

pub struct Music {
    /// Kept alive for the duration of playback; dropping it stops the audio
    /// thread.
    stream: Option<OutputStream>,
    /// Drives the looping source; dropping it stops the track.
    sink: Option<Sink>,
    /// PipeWire/PulseAudio raw PCM streamer when direct ALSA is bypassed/busy.
    pipe: Option<PipePlayer>,
    /// The resolved track, remembered so playback can be restarted after a
    /// toggle. None when there is no track (or `--no-music`).
    track: Option<PathBuf>,
    /// Whether playback is currently active (vs. paused).
    playing: bool,
    /// Temporary ALSA config file written for bundled/fallback hardware access,
    /// removed on stop/drop.
    temp_alsa_conf: Option<PathBuf>,
}

impl Music {
    /// A no-op music handle (no track / no audio device).
    pub fn none() -> Self {
        Music {
            stream: None,
            sink: None,
            pipe: None,
            track: None,
            playing: false,
            temp_alsa_conf: None,
        }
    }

    /// Resolve the track to play: explicit path, else the first audio file in
    /// `music/`. Searches `./music`, next to `argv[0]` (when launched through a
    /// bundled glibc loader, where `current_exe()` points at the loader), next
    /// to `current_exe()` (when launched via `PATH`), and `./demo/music` (fresh
    /// repo checkout). Returns `None` if nothing is found.
    pub fn resolve(explicit: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = explicit {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
            eprintln!("music: file not found: {p}");
            return None;
        }
        let mut dirs = vec![PathBuf::from("music")];
        if let Some(arg0) = std::env::args().next() {
            if let Some(dir) = Path::new(&arg0).parent() {
                if !dir.as_os_str().is_empty() {
                    let d = dir.join("music");
                    if !dirs.contains(&d) {
                        dirs.push(d);
                    }
                }
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let d = dir.join("music");
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
        let demo_music = PathBuf::from("demo/music");
        if !dirs.contains(&demo_music) {
            dirs.push(demo_music);
        }
        for dir in dirs {
            if let Some(track) = first_track_in(&dir) {
                return Some(track);
            }
        }
        None
    }

    /// Start playing `path` (looped) on the default audio device. Silently
    /// degrades to no-op if no audio device is available or the file cannot be
    /// decoded.
    pub fn start(path: &Path) -> Self {
        let mut m = Music {
            stream: None,
            sink: None,
            pipe: None,
            track: Some(path.to_path_buf()),
            playing: false,
            temp_alsa_conf: None,
        };
        let _ = m.spawn();
        m
    }

    /// Open the audio device and start the looping track. Silent: may be called
    /// from the key handler while the TUI owns the terminal.
    fn spawn(&mut self) -> Result<(), String> {
        let Some(path) = self.track.clone() else {
            return Err("no track".into());
        };
        let path = path.as_path();

        let file =
            File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let decoder = Decoder::try_from(BufReader::new(file))
            .map_err(|e| format!("cannot decode {}: {e}", path.display()))?;

        let user_override =
            *USER_ALSA_CONFIG.get_or_init(|| std::env::var_os("ALSA_CONFIG_PATH").is_some());
        let bundled = std::env::var("FBR_BUNDLED_ALSA").as_deref() == Ok("1");

        // On bundled builds without an explicit ALSA_CONFIG_PATH, prefer the
        // host sound server (`pw-cat`/`pacat`) first so audio mixes cleanly
        // with PipeWire/PulseAudio instead of hitting EBUSY on direct `hw:0,0`.
        if bundled && !user_override {
            match PipePlayer::start(decoder) {
                Ok(player) => {
                    self.pipe = Some(player);
                    self.playing = true;
                    return Ok(());
                }
                Err(_) => {
                    // Fall through to direct ALSA hardware below; re-open decoder.
                    let file2 = File::open(path)
                        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
                    let decoder2 = Decoder::try_from(BufReader::new(file2))
                        .map_err(|e| format!("cannot decode {}: {e}", path.display()))?;
                    return self.spawn_alsa(decoder2);
                }
            }
        }

        // Native builds: try rodio's default stream first, then sound-server
        // pipe, then direct ALSA hardware cards.
        match self.spawn_alsa(decoder) {
            Ok(()) => Ok(()),
            Err(alsa_err) => {
                if let Ok(file2) = File::open(path) {
                    if let Ok(decoder2) = Decoder::try_from(BufReader::new(file2)) {
                        if let Ok(player) = PipePlayer::start(decoder2) {
                            self.pipe = Some(player);
                            self.playing = true;
                            return Ok(());
                        }
                    }
                }
                Err(alsa_err)
            }
        }
    }

    fn spawn_alsa(&mut self, decoder: Decoder<BufReader<File>>) -> Result<(), String> {
        let stream = self.open_stream()?;
        let sink = Sink::connect_new(&stream.mixer());
        sink.append(decoder.repeat_infinite());
        sink.play();

        self.stream = Some(stream);
        self.sink = Some(sink);
        self.playing = true;
        Ok(())
    }

    /// Open an output stream.
    ///
    /// - If the caller set `ALSA_CONFIG_PATH`, honour it verbatim.
    /// - On native builds (`FBR_BUNDLED_ALSA` != `1`), try the system default
    ///   first so PipeWire/PulseAudio works normally, falling back to direct
    ///   ALSA hardware cards only if that fails.
    /// - On bundled builds (`FBR_BUNDLED_ALSA=1`), bypass host ALSA plugins
    ///   (which cannot load across glibc versions) and try each candidate
    ///   hardware card directly.
    fn open_stream(&mut self) -> Result<OutputStream, String> {
        let user_override =
            *USER_ALSA_CONFIG.get_or_init(|| std::env::var_os("ALSA_CONFIG_PATH").is_some());
        let bundled = std::env::var("FBR_BUNDLED_ALSA").as_deref() == Ok("1");

        if user_override {
            return OutputStreamBuilder::open_default_stream()
                .map_err(|e| format!("no audio device ({e})"));
        }

        if !bundled {
            if let Ok(stream) = OutputStreamBuilder::open_default_stream() {
                return Ok(stream);
            }
        }

        let mut last_err = String::from("no audio device");
        for card in candidate_cards() {
            if let Some(conf_path) = write_hw_alsa_config(card) {
                self.temp_alsa_conf = Some(conf_path);
                match OutputStreamBuilder::open_default_stream() {
                    Ok(stream) => return Ok(stream),
                    Err(e) => last_err = format!("no audio device ({e})"),
                }
            }
        }
        Err(last_err)
    }

    /// Toggle playback: pause if playing, resume if paused, start if stopped.
    /// A no-op when there is no track (no `music/` file or `--no-music`).
    /// Silent: runs from the key handler while the TUI owns the terminal.
    pub fn toggle(&mut self) {
        if let Some(sink) = &self.sink {
            if self.playing {
                sink.pause();
                self.playing = false;
            } else {
                sink.play();
                self.playing = true;
            }
        } else if let Some(pipe) = &self.pipe {
            self.playing = !self.playing;
            pipe.set_playing(self.playing);
        } else if self.track.is_some() {
            let _ = self.spawn();
        }
    }

    /// Stop the player, release the audio device, and remove any temporary
    /// ALSA config written by this process.
    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        if let Some(mut pipe) = self.pipe.take() {
            pipe.stop();
        }
        self.stream = None;
        self.playing = false;
        if let Some(path) = self.temp_alsa_conf.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for Music {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The first audio file in `dir` (alphabetically), or `None`.
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

/// Candidate ALSA card indices in priority order: `FBR_AUDIO_CARD` if set,
/// then the sound server's default output card, then every card with a
/// playback device. Deduplicated, falling back to `[0]` if none are detected.
fn candidate_cards() -> Vec<u32> {
    let mut cards: Vec<u32> = Vec::new();
    let push = |c: u32, cards: &mut Vec<u32>| {
        if !cards.contains(&c) {
            cards.push(c);
        }
    };
    if let Ok(s) = std::env::var("FBR_AUDIO_CARD") {
        if let Ok(c) = s.parse::<u32>() {
            push(c, &mut cards);
        }
    }
    if let Some(c) = default_output_card() {
        push(c, &mut cards);
    }
    for c in playback_cards() {
        push(c, &mut cards);
    }
    if cards.is_empty() {
        cards.push(0);
    }
    cards
}

/// Write a per-user/per-process direct-hardware ALSA config for `card` and
/// point `ALSA_CONFIG_PATH` at it.
fn write_hw_alsa_config(card: u32) -> Option<PathBuf> {
    let conf = format!(
        "pcm.!default {{\n    type hw\n    card {card}\n    device 0\n}}\n\
         ctl.!default {{\n    type hw\n    card {card}\n}}\n"
    );
    #[cfg(unix)]
    let uid = unsafe { libc::geteuid() };
    #[cfg(not(unix))]
    let uid = 0u32;
    let path = std::env::temp_dir().join(format!(
        "fbr-ram-downloader-asound-{uid}-{}.conf",
        std::process::id()
    ));
    std::fs::write(&path, conf).ok()?;
    std::env::set_var("ALSA_CONFIG_PATH", &path);
    Some(path)
}

/// The card the sound server routes its default output to. Queries `wpctl`
/// (PipeWire) or `pactl` (PulseAudio), then maps the sink name to a card via
/// `/sys/class/sound`.
fn default_output_card() -> Option<u32> {
    let sink = default_sink_name()?;
    let cards = sound_cards()?;
    map_sink_to_card_in(&sink, &cards)
}

/// The default output sink name from the running sound server, e.g.
/// `alsa_output.usb-Apple_Inc._Studio_Display_...-02.analog-stereo`.
fn default_sink_name() -> Option<String> {
    if let Ok(out) = std::process::Command::new("wpctl")
        .args(["list", "audio", "sinks"])
        .output()
    {
        if out.status.success() {
            if let Some(sink) = parse_wpctl_default_sink(&String::from_utf8_lossy(&out.stdout)) {
                return Some(sink);
            }
        }
    }
    if let Ok(out) = std::process::Command::new("pactl").arg("info").output() {
        if out.status.success() {
            if let Some(sink) = parse_pactl_default_sink(&String::from_utf8_lossy(&out.stdout)) {
                return Some(sink);
            }
        }
    }
    None
}

/// Extract the default `alsa_output...` sink from `wpctl list audio sinks`.
fn parse_wpctl_default_sink(text: &str) -> Option<String> {
    for line in text.lines() {
        if line.contains('*') {
            for tok in line.split_whitespace() {
                if tok.starts_with("alsa_output") {
                    return Some(tok.to_string());
                }
            }
        }
    }
    None
}

/// Extract the `Default Sink:` name from `pactl info`.
fn parse_pactl_default_sink(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("Default Sink:") {
            let name = rest.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Map a sink name to a card number given `(card, device_path, id)` entries.
fn map_sink_to_card_in(sink: &str, cards: &[(u32, String, String)]) -> Option<u32> {
    let s = sink.to_ascii_lowercase();
    let stripped = s.strip_prefix("alsa_output.").unwrap_or(&s);
    let cut = stripped
        .find(".analog")
        .or_else(|| stripped.find(".digital"))
        .unwrap_or(stripped.len());
    let devname = &stripped[..cut];

    if let Some(bdf) = extract_pci_bdf(devname) {
        for (card, dev, _) in cards {
            if dev.to_ascii_lowercase().contains(&bdf) {
                return Some(*card);
            }
        }
    }
    let devn: String = devname
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    for (card, _, id) in cards {
        let idn: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if !idn.is_empty() && devn.contains(&idn) {
            return Some(*card);
        }
    }
    None
}

/// `(card, device_path, id)` for every card in `/sys/class/sound`.
fn sound_cards() -> Option<Vec<(u32, String, String)>> {
    let entries = std::fs::read_dir("/sys/class/sound").ok()?;
    let mut cards = Vec::new();
    for e in entries.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(card_str) = name.strip_prefix("card") else {
            continue;
        };
        let Ok(card) = card_str.parse::<u32>() else {
            continue;
        };
        let dir = e.path();
        let dev = std::fs::read_link(dir.join("device"))
            .ok()
            .and_then(|p| std::fs::canonicalize(dir.join(p)).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let id = std::fs::read_to_string(dir.join("id"))
            .unwrap_or_default()
            .trim()
            .to_string();
        cards.push((card, dev, id));
    }
    if cards.is_empty() {
        return None;
    }
    cards.sort_by_key(|(c, _, _)| *c);
    Some(cards)
}

/// `pci-0000_00_1f.3` -> `0000:00:1f.3` (the PCI BDF, as it appears in a
/// `/sys/devices` path).
fn extract_pci_bdf(devname: &str) -> Option<String> {
    let start = devname.find("pci-")? + 4;
    let rest = &devname[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.')
        .unwrap_or(rest.len());
    let raw = &rest[..end];
    if raw.is_empty() {
        return None;
    }
    Some(raw.replace('_', ":"))
}

/// Every card that has a playback device, lowest first, from
/// `/sys/class/sound` (`pcmC{card}D{dev}p`), falling back to
/// `/proc/asound/cards`.
fn playback_cards() -> Vec<u32> {
    if let Ok(entries) = std::fs::read_dir("/sys/class/sound") {
        let names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let cards = playback_cards_from_names(&refs);
        if !cards.is_empty() {
            return cards;
        }
    }
    if let Ok(text) = std::fs::read_to_string("/proc/asound/cards") {
        return playback_cards_from_proc(&text);
    }
    Vec::new()
}

/// Parse playback card numbers from `/sys/class/sound` entry names
/// (`pcmC<card>D<dev>p`). Sorted and deduplicated.
fn playback_cards_from_names(names: &[&str]) -> Vec<u32> {
    let mut cards: Vec<u32> = names
        .iter()
        .filter_map(|name| {
            let body = name.strip_prefix("pcmC")?.strip_suffix('p')?;
            let (card_str, dev_str) = body.split_once('D')?;
            let card = card_str.parse::<u32>().ok()?;
            let _dev = dev_str.parse::<u32>().ok()?;
            Some(card)
        })
        .collect();
    cards.sort_unstable();
    cards.dedup();
    cards
}

/// Parse card numbers from `/proc/asound/cards` (` 0 [PCH] : HDA-Intel ...`).
fn playback_cards_from_proc(text: &str) -> Vec<u32> {
    let mut cards = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some((first, _)) = trimmed.split_once(' ') {
            if let Ok(card) = first.parse::<u32>() {
                cards.push(card);
            }
        }
    }
    cards.sort_unstable();
    cards.dedup();
    cards
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TMP_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fbr-music-test-{tag}-{}-{}",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn playback_cards_parses_single_and_multi_digit_sysfs_names() {
        let names = [
            "card0",
            "controlC0",
            "pcmC0D0c",
            "pcmC0D0p",
            "pcmC0D10p",
            "pcmC2D1p",
            "pcmC12D3p",
            "pcmC12D0c",
            "pcmCXD0p",
            "pcmC1DXp",
            "timer",
        ];
        assert_eq!(playback_cards_from_names(&names), vec![0, 2, 12]);
        assert!(playback_cards_from_names(&["pcmC0D0c", "card0", "timer"]).is_empty());
    }

    #[test]
    fn playback_cards_parses_proc_asound_cards() {
        let sample = "\
 0 [PCH            ]: HDA-Intel - HDA Intel PCH
                      HDA Intel PCH at 0xa1230000 irq 145
 2 [Display        ]: USB-Audio - Studio Display
                      Apple Inc. Studio Display at usb-0000:00:14.0-1
";
        assert_eq!(playback_cards_from_proc(sample), vec![0, 2]);
        assert!(playback_cards_from_proc("").is_empty());
    }

    #[test]
    fn wpctl_and_pactl_default_sink_parsers() {
        let wpctl = "\
Audio
 ├─ Sinks:
 │      44. alsa_output.pci-0000_00_1f.3.analog-stereo
 │  *   52. alsa_output.usb-Apple_Inc._Studio_Display-02.analog-stereo [vol: 0.70]
";
        assert_eq!(
            parse_wpctl_default_sink(wpctl).as_deref(),
            Some("alsa_output.usb-Apple_Inc._Studio_Display-02.analog-stereo")
        );
        assert_eq!(parse_wpctl_default_sink("no default sink here"), None);

        let pactl = "\
Server String: /run/user/1000/pulse/native
Default Sink: alsa_output.pci-0000_00_1f.3.analog-stereo
Default Source: alsa_input.pci-0000_00_1f.3.analog-stereo
";
        assert_eq!(
            parse_pactl_default_sink(pactl).as_deref(),
            Some("alsa_output.pci-0000_00_1f.3.analog-stereo")
        );
        assert_eq!(parse_pactl_default_sink("Default Sink:   \n"), None);
    }

    #[test]
    fn extract_pci_bdf_and_sink_mapping() {
        assert_eq!(
            extract_pci_bdf("pci-0000_00_1f.3").as_deref(),
            Some("0000:00:1f.3")
        );
        assert_eq!(extract_pci_bdf("usb-Apple_Inc"), None);

        let cards = vec![
            (
                0,
                "/sys/devices/pci0000:00/0000:00:1f.3/sound/card0".into(),
                "PCH".into(),
            ),
            (
                2,
                "/sys/devices/pci0000:00/0000:00:14.0/usb1/sound/card2".into(),
                "Display".into(),
            ),
        ];
        assert_eq!(
            map_sink_to_card_in("alsa_output.pci-0000_00_1f.3.analog-stereo", &cards),
            Some(0)
        );
        assert_eq!(
            map_sink_to_card_in(
                "alsa_output.usb-Apple_Inc._Studio_Display_1234-02.analog-stereo",
                &cards
            ),
            Some(2)
        );
        assert_eq!(
            map_sink_to_card_in("alsa_output.pci-0000_09_00.1.hdmi-stereo", &cards),
            None
        );
    }

    #[test]
    fn first_track_in_picks_first_supported_audio_file_alphabetically() {
        let dir = temp_dir("tracks");
        std::fs::write(dir.join("README.txt"), "not audio").unwrap();
        std::fs::write(dir.join("z_track.MP3"), "fake").unwrap();
        std::fs::write(dir.join("a_track.flac"), "fake").unwrap();

        assert_eq!(first_track_in(&dir), Some(dir.join("a_track.flac")));
        assert_eq!(Music::resolve(Some(dir.join("z_track.MP3").to_str().unwrap())), Some(dir.join("z_track.MP3")));
        assert_eq!(Music::resolve(Some(dir.join("missing.mp3").to_str().unwrap())), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn music_none_toggle_and_stop_are_safe_noops() {
        let mut m = Music::none();
        m.toggle();
        m.stop();
        assert!(!m.playing);
    }
}
