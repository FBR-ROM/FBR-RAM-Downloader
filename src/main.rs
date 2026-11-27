//! FBR RAM Downloader — C64/Game Boy cracktro entry point.
//!
//! Runs the terminal cracktro: a warp starfield, a shimmering border title,
//! three 3D models that cycle as they spin, a "DOWNLOADING RAM" bar, and a
//! falling-letters celebration on completion. All in the 4-color FBR Game Boy
//! palette. The hack is pluggable via the [`hack::Hack`] trait (see
//! [`make_hack`]): the real one is [`v100::V100Hack`], which raises the HBM
//! memory clock of every Tesla V100 here, or on `--host`; `--demo` runs the
//! simulated [`hack::StubHack`]. Resizes live with the terminal (btop-style).
//!
//! Flags:
//!   --host HOST      run on the V100 machine HOST, over ssh
//!   --sprint         1080 MHz instead of the daily 1053 MHz
//!   --dry-run        survey and report, write nothing
//!   --fast           skip the pacing of the status lines
//!   --demo           the simulated download, no hardware
//!   --headless       run the V100 agent, stream its plain text, no cracktro
//!   --dump N [W H]   print N frames to stdout (for verification / GIF)
//!   --layout [W H]   print the computed layout for a size and exit
//!   --grey           use the Grey Quarttrochrome palette (default: Green)
//!   --dark/--light   force the terminal theme (default: auto-detect)
//!   --music FILE     play a specific track (else auto-detect music/)
//!   --no-music       disable music

mod canvas;
mod hack;
mod model_data;
mod music;
mod scene;
mod tui;
mod v100;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(33); // ~30 fps

const USAGE: &str = "\
usage: fbr-ram-downloader [options]

  Live HBM2 memory clock unlock for every NVIDIA Tesla V100 (1053 MHz daily,
  1080 MHz sprint), wrapped in a 4-color Game Boy cracktro.

options:
  --demo             simulated download: no hardware, no root
  --headless         run v100-unlock in plain text (scripts, systemd)
  --host HOST        run on the V100 machine HOST over ssh
  --dry-run          survey and report, write nothing
  --sprint           1080 MHz (+23%) instead of daily 1053 MHz (+20%)
  --fast             skip status-line pacing in the cracktro
  --grey             use the Grey Quarttrochrome palette (default: Green)
  --dark / --light   force terminal theme instead of auto-detecting
  --music FILE       play a specific audio track (default: auto-detect music/)
  --no-music         start with music off
  --dump [N [W H]]   print N frames at WxH to stdout and exit
  --layout [W H]     print computed layout for WxH and exit
  -V, --version      print version and exit
  -h, --help         this help

keys: q / Esc / Ctrl-C quit, m mute/unmute, mouse drag spins the 3D model
";

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Help,
    Version,
    Run(Args),
}

#[derive(Debug, PartialEq, Eq)]
struct Args {
    dump: Option<(usize, usize, usize)>,
    grey: bool,
    music: Option<String>,
    no_music: bool,
    /// `--layout [W H]`: print the computed layout and exit (debug aid).
    layout: bool,
    layout_size: Option<(usize, usize)>,
    /// `--dark` / `--light`: force the terminal theme instead of auto-detecting.
    /// `Some(true)` = dark, `Some(false)` = light, `None` = auto-detect.
    theme: Option<bool>,
    /// `--demo`: the simulated download instead of the real hack.
    demo: bool,
    /// `--host HOST`: the V100 machine to reach over ssh.
    host: Option<String>,
    /// `--sprint`: 1080 MHz instead of 1053.
    sprint: bool,
    /// `--dry-run`: survey and report, but write nothing.
    dry_run: bool,
    /// `--fast`: no pacing of the status lines.
    fast: bool,
    /// `--headless`: run the V100 agent and stream its plain text; no cracktro.
    headless: bool,
}

fn parse_args_from<I: IntoIterator<Item = String>>(argv: I) -> Result<Action, String> {
    let argv: Vec<String> = argv.into_iter().collect();
    let mut args = Args {
        dump: None,
        grey: false,
        music: None,
        no_music: false,
        layout: false,
        layout_size: None,
        theme: None,
        demo: false,
        host: None,
        sprint: false,
        dry_run: false,
        fast: false,
        headless: false,
    };
    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].as_str();
        match arg {
            "-h" | "--help" => return Ok(Action::Help),
            "-V" | "--version" => return Ok(Action::Version),
            "--dump" => {
                if let Some(n) = argv.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
                    if let Some(w) = argv.get(i + 2).and_then(|s| s.parse::<usize>().ok()) {
                        let Some(h) = argv.get(i + 3).and_then(|s| s.parse::<usize>().ok()) else {
                            return Err("--dump width requires a height".into());
                        };
                        if w == 0 || h == 0 {
                            return Err("--dump dimensions must be positive".into());
                        }
                        args.dump = Some((n, w, h));
                        i += 4;
                    } else {
                        args.dump = Some((n, 80, 24));
                        i += 2;
                    }
                } else {
                    args.dump = Some((30, 80, 24));
                    i += 1;
                }
            }
            "--layout" => {
                args.layout = true;
                if let Some(w) = argv.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
                    let Some(h) = argv.get(i + 2).and_then(|s| s.parse::<usize>().ok()) else {
                        return Err("--layout width requires a height".into());
                    };
                    if w == 0 || h == 0 {
                        return Err("--layout dimensions must be positive".into());
                    }
                    args.layout_size = Some((w, h));
                    i += 3;
                } else {
                    i += 1;
                }
            }
            "--grey" => {
                args.grey = true;
                i += 1;
            }
            "--dark" => {
                args.theme = Some(true);
                i += 1;
            }
            "--light" => {
                args.theme = Some(false);
                i += 1;
            }
            "--no-music" => {
                args.no_music = true;
                i += 1;
            }
            "--demo" => {
                args.demo = true;
                i += 1;
            }
            "--sprint" => {
                args.sprint = true;
                i += 1;
            }
            "--dry-run" => {
                args.dry_run = true;
                i += 1;
            }
            "--fast" => {
                args.fast = true;
                i += 1;
            }
            "--headless" => {
                args.headless = true;
                i += 1;
            }
            "--host" => {
                let Some(h) = argv.get(i + 1) else {
                    return Err("--host needs a host name".into());
                };
                if h.is_empty() || h.starts_with('-') || h.contains(char::is_whitespace) {
                    return Err(format!("{h:?} is not a host name"));
                }
                args.host = Some(h.clone());
                i += 2;
            }
            _ if arg.starts_with("--host=") => {
                let h = &arg["--host=".len()..];
                if h.is_empty() || h.starts_with('-') || h.contains(char::is_whitespace) {
                    return Err(format!("{h:?} is not a host name"));
                }
                args.host = Some(h.to_string());
                i += 1;
            }
            "--music" => {
                let Some(f) = argv.get(i + 1) else {
                    return Err("--music needs a file path".into());
                };
                if f.is_empty() {
                    return Err("--music needs a file path".into());
                }
                args.music = Some(f.clone());
                i += 2;
            }
            _ if arg.starts_with("--music=") => {
                let f = &arg["--music=".len()..];
                if f.is_empty() {
                    return Err("--music needs a file path".into());
                }
                args.music = Some(f.to_string());
                i += 1;
            }
            other => return Err(format!("unknown option {other:?} (see --help)")),
        }
    }
    Ok(Action::Run(args))
}

/// The hack to run. This is the single integration point: the starfield, 3D
/// model, loader, scroller and blow-away ending are all driven purely by
/// `HackState`, so nothing else knows which hack is running.
fn make_hack(args: &Args) -> Box<dyn hack::Hack> {
    if args.demo {
        return Box::new(hack::StubHack::new());
    }
    Box::new(v100::V100Hack::new(args.host.clone(), args.sprint, args.dry_run, args.fast))
}

/// Errors are printed as the plain sentence they are (a `main` that returns
/// `Result` prints their debug form, in quotes).
fn main() -> std::process::ExitCode {
    let args = match parse_args_from(std::env::args()) {
        Ok(Action::Help) => {
            print!("{USAGE}");
            return std::process::ExitCode::SUCCESS;
        }
        Ok(Action::Version) => {
            println!("fbr-ram-downloader {}", env!("CARGO_PKG_VERSION"));
            return std::process::ExitCode::SUCCESS;
        }
        Ok(Action::Run(args)) => args,
        Err(e) => {
            eprintln!("fbr-ram-downloader: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    // `--headless` runs the V100 agent and streams its plain text, returning
    // the agent's own exit code (0 at target, 2 untouched, 3 put back, 4
    // unknown). No terminal, no cracktro — for scripts and systemd.
    if args.headless {
        return v100::run_headless(&args.host, args.sprint, args.dry_run, args.demo);
    }
    match run_with(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_with(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let palette = if args.grey {
        canvas::Palette::grey()
    } else {
        canvas::Palette::green()
    };

    // Layout probe: print the computed responsive layout for a size and exit.
    if args.layout {
        let (w, h) = match args.layout_size {
            Some(wh) => wh,
            None => {
                let t = crossterm::terminal::size()?;
                (t.0 as usize, t.1 as usize)
            }
        };
        print!("{}", tui::describe_layout(w, h));
        return Ok(());
    }

    // Headless mode: render N frames to stdout (for verification / GIF).
    if let Some((n, w, h)) = args.dump {
        return dump_frames(n, w, h, palette);
    }

    // Detect the terminal's background color and remap any palette shade that
    // would be invisible against it. `--dark`/`--light` override the
    // auto-detection by assuming a black or white background.
    let bg: Option<(u8, u8, u8)> = match args.theme {
        Some(true) => Some((0, 0, 0)),
        Some(false) => Some((255, 255, 255)),
        None => detect_background(),
    };
    let palette = palette.remap_for(bg);

    let term = crossterm::terminal::size()?;
    let (term_w, term_h) = (term.0 as usize, term.1 as usize);

    let (sw, sh) = scene_size(term_w, term_h);
    let mut scene = scene::Scene::new(sw, sh);
    let mut cracktro = tui::Cracktro::new(term_w, term_h, palette);
    let mut hack = make_hack(&args);
    hack.start()?;

    // Start music (unless disabled). No-op if no track/player found.
    let mut music = if args.no_music {
        music::Music::none()
    } else {
        match music::Music::resolve(args.music.as_deref()) {
            Some(path) => music::Music::start(&path),
            None => music::Music::none(),
        }
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::cursor::Hide,
        crossterm::cursor::MoveTo(0, 0),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )?;

    // Ratatui owns the back-buffer and diffs it against the previous frame,
    // writing only the cells that changed. It auto-resizes its buffers to the
    // real terminal size on every draw, so resizes are handled for free.
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run_loop(
        &mut scene,
        &mut cracktro,
        &mut hack,
        &mut music,
        &mut terminal,
        &mut (term_w, term_h),
    );

    // Always restore the terminal and stop music, even on error. Teardown is
    // best-effort: a failure here must not mask the real result or skip the
    // music/hack cleanup.
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    music.stop();
    let _ = hack.finish();

    result
}

fn dump_frames(n: usize, w: usize, h: usize, palette: canvas::Palette) -> Result<(), Box<dyn std::error::Error>> {
    let (sw, sh) = scene_size(w, h);
    let mut scene = scene::Scene::new(sw, sh);
    let mut cracktro = tui::Cracktro::new(w, h, palette);
    // Headless frames are for inspecting the visuals: always the stub.
    let mut hack: Box<dyn hack::Hack> = Box::new(hack::StubHack::new());
    hack.start()?;
    let dt = 0.033;
    for i in 0..n {
        scene.advance(dt);
        let state = hack.poll();
        let cv = cracktro.render(dt, &scene.render(), &state, w, h);
        println!("{}", cv.to_ansi());
        if i + 1 < n {
            println!();
        }
    }
    let _ = hack.finish();
    Ok(())
}

fn scene_size(term_w: usize, term_h: usize) -> (usize, usize) {
    // Logo region in cells (shared with the TUI layout): one brightness sample
    // per cell.
    tui::logo_cells(term_w, term_h)
}

/// Detect the terminal's background color, so the palette can avoid any shade
/// that would be invisible against it.
///
/// Asks the terminal for its background color via the OSC 11 query
/// (`ESC ] 11 ; ? BEL`) and reads the response from the input stream. Most
/// modern terminals (kitty, WezTerm, iTerm2, Ghostty, foot, xterm, VTE) answer
/// with an `rgb:RR/GG/BB` string. If the terminal doesn't respond within the
/// timeout, we return `None` and the palette falls back to a set of shades
/// that is visible on both a black and a white background.
fn detect_background() -> Option<(u8, u8, u8)> {
    use std::io::IsTerminal;

    let stdin = io::stdin();
    let stdout = io::stdout();
    // Only query if we have a real TTY on both ends; otherwise the response
    // would never come and we'd just wait for the timeout.
    if !stdin.is_terminal() || !stdout.is_terminal() {
        return None;
    }

    // The query needs raw input to read the response without a newline.
    // Remember whether raw mode was already on so we can restore it.
    let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    if !was_raw {
        if let Err(_) = crossterm::terminal::enable_raw_mode() {
            return None;
        }
    }
    let result = query_background();
    if !was_raw {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    result
}

/// Send the OSC 11 background-color query and parse the response.
///
/// Uses a non-blocking read with a hard deadline so a terminal that never
/// answers (or answers slowly) can't hang startup.
fn query_background() -> Option<(u8, u8, u8)> {
    use std::io::{Read, Write};
    use std::os::unix::io::AsRawFd;
    use std::time::{Duration, Instant};

    let mut stdout = io::stdout();
    let _ = stdout.write_all(b"\x1b]11;?\x07");
    let _ = stdout.flush();

    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    // Make stdin non-blocking for the duration of the query, then restore the
    // original flags on every exit path.
    let orig = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if orig < 0 {
        return None; // can't query the flags; remap will be theme-safe
    }
    unsafe { libc::fcntl(fd, libc::F_SETFL, orig | libc::O_NONBLOCK) };

    let deadline = Instant::now() + Duration::from_millis(150);
    let mut buf = [0u8; 256];
    let mut got = 0usize;
    let bg = loop {
        if let Some(rgb) = parse_osc11(&buf[..got]) {
            break Some(rgb);
        }
        if Instant::now() >= deadline {
            break None; // no response: remap will be theme-safe
        }
        // Wait up to 10ms for data, so we don't busy-spin.
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let n = unsafe { libc::poll(&mut pollfd, 1, 10) };
        if n > 0 && (pollfd.revents & libc::POLLIN) != 0 {
            let mut handle = stdin.lock();
            match handle.read(&mut buf[got..]) {
                Ok(0) => break None,
                Ok(n) => {
                    got += n;
                    if got >= buf.len() {
                        break None;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break None,
            }
        }
    };
    // Restore the original flags.
    unsafe { libc::fcntl(fd, libc::F_SETFL, orig) };
    bg
}

/// Parse a complete `rgb:RR/GG/BB` payload out of an OSC 11 response.
///
/// Requires an OSC terminator (`BEL` `\x07`, `ST` `\x1b\\`, or 8-bit `ST`
/// `\u{9c}`) after the RGB triplet so a partial read never misparses a
/// truncated blue channel. Scales 1-, 2-, 3-, or 4-digit hex channels to 8-bit.
fn parse_osc11(data: &[u8]) -> Option<(u8, u8, u8)> {
    let s = String::from_utf8_lossy(data);
    let start = s.find("rgb:")? + 4;
    let rest = &s[start..];
    let term = [
        rest.find('\x07'),
        rest.find("\x1b\\"),
        rest.find('\u{9c}'),
    ]
    .into_iter()
    .flatten()
    .min()?;
    let payload = &rest[..term];
    let parts: Vec<&str> = payload.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let to8 = |p: &str| -> Option<u8> {
        let p = p.trim();
        if p.is_empty() || p.len() > 4 || !p.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(p, 16).ok()?;
        match p.len() {
            1 => Some((v * 17) as u8),
            2 => Some(v as u8),
            3 => Some(((v * 255) / 0xfff) as u8),
            4 => Some(((v * 255) / 0xffff) as u8),
            _ => None,
        }
    };
    Some((to8(parts[0])?, to8(parts[1])?, to8(parts[2])?))
}

fn run_loop(
    scene: &mut scene::Scene,
    cracktro: &mut tui::Cracktro,
    hack: &mut Box<dyn hack::Hack>,
    music: &mut music::Music,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    size: &mut (usize, usize),
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last = Instant::now();
    // Set when the hack first reports done; the celebration renders for a
    // hold window after this, then the program exits.
    let mut finished_at: Option<Instant> = None;
    // Last mouse position for drag deltas.
    let mut last_mouse: Option<(u16, u16)> = None;

    loop {
        // Trust the terminal's real size every frame. Some terminals and
        // multiplexers do not deliver resize events reliably, and a stale size
        // would leave the composition mis-framed. `terminal::size` is a cheap
        // ioctl, so polling it here keeps the scene/cracktro rasterization in
        // sync with the real terminal. Ratatui's `draw` independently
        // auto-resizes its own back-buffer to the same size.
        if let Ok((tw, th)) = crossterm::terminal::size() {
            let (tw, th) = (tw as usize, th as usize);
            if (tw, th) != *size && tw > 0 && th > 0 {
                *size = (tw, th);
                let (sw, sh) = scene_size(tw, th);
                scene.set_size(sw, sh);
                cracktro.set_size(tw, th);
            }
        }

        // Drain all pending events (keeps the frame rate steady).
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q' | 'Q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c' | 'C') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char('m' | 'M') => music.toggle(),
                    _ => {}
                },
                // Resize is handled by the per-frame size poll above; the
                // event is ignored here.
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                        scene.mouse_down();
                        last_mouse = Some((m.column, m.row));
                    }
                    MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                        if let Some((lx, ly)) = last_mouse {
                            scene.mouse_drag(
                                (m.column as i32) - (lx as i32),
                                (m.row as i32) - (ly as i32),
                            );
                        }
                        last_mouse = Some((m.column, m.row));
                    }
                    MouseEventKind::Up(_) => {
                        scene.mouse_up();
                        last_mouse = None;
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f32().min(0.1);
        last = now;

        scene.advance(dt);
        let state = hack.poll();
        if state.error.is_some() {
            let cv = cracktro.render(dt, &scene.render(), &state, size.0, size.1);
            let _ = terminal.draw(|f| cv.to_buffer(f.buffer_mut()));
            return Err(state.error.unwrap().into());
        }
        if state.done && finished_at.is_none() {
            finished_at = Some(Instant::now());
        }

        // Render the frame into a canvas, then hand it to ratatui. `draw`
        // auto-resizes the back-buffer to the real terminal size, diffs it
        // against the previous frame, and writes only the changed cells — so
        // resizes reflow cleanly with no blank flash and no full-screen
        // reprint.
        let cv = cracktro.render(dt, &scene.render(), &state, size.0, size.1);
        terminal.draw(|f| cv.to_buffer(f.buffer_mut()))?;

        if let Some(t0) = finished_at {
            // Keep animating (the freeze + blow-away celebration) for the
            // hold window, then exit.
            if t0.elapsed() >= Duration::from_millis(3500) {
                return Ok(());
            }
        }

        // Sleep the remainder of the tick.
        let elapsed = now.elapsed();
        if elapsed < TICK {
            std::thread::sleep(TICK - elapsed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Action, String> {
        let mut argv = vec!["fbr-ram-downloader".to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        parse_args_from(argv)
    }

    #[test]
    fn help_and_version_flags_return_early() {
        assert_eq!(parse(&["--help"]), Ok(Action::Help));
        assert_eq!(parse(&["-h"]), Ok(Action::Help));
        assert_eq!(parse(&["--version"]), Ok(Action::Version));
        assert_eq!(parse(&["-V"]), Ok(Action::Version));
    }

    #[test]
    fn parses_all_valid_flags() {
        let Action::Run(a) = parse(&[
            "--demo",
            "--headless",
            "--sprint",
            "--dry-run",
            "--fast",
            "--grey",
            "--dark",
            "--no-music",
            "--host",
            "c4130",
            "--music=track.mp3",
        ])
        .unwrap() else {
            panic!("expected Run");
        };
        assert!(a.demo && a.headless && a.sprint && a.dry_run && a.fast && a.grey && a.no_music);
        assert_eq!(a.theme, Some(true));
        assert_eq!(a.host.as_deref(), Some("c4130"));
        assert_eq!(a.music.as_deref(), Some("track.mp3"));

        let Action::Run(b) = parse(&["--light", "--host=gpu01", "--music", "song.ogg"]).unwrap()
        else {
            panic!("expected Run");
        };
        assert_eq!(b.theme, Some(false));
        assert_eq!(b.host.as_deref(), Some("gpu01"));
        assert_eq!(b.music.as_deref(), Some("song.ogg"));
    }

    #[test]
    fn dump_and_layout_do_not_swallow_following_flags() {
        let Action::Run(a) = parse(&["--dump", "--grey"]).unwrap() else {
            panic!("expected Run");
        };
        assert_eq!(a.dump, Some((30, 80, 24)));
        assert!(a.grey);

        let Action::Run(b) = parse(&["--dump", "60", "--grey"]).unwrap() else {
            panic!("expected Run");
        };
        assert_eq!(b.dump, Some((60, 80, 24)));
        assert!(b.grey);

        let Action::Run(c) = parse(&["--dump", "12", "100", "30", "--grey"]).unwrap() else {
            panic!("expected Run");
        };
        assert_eq!(c.dump, Some((12, 100, 30)));
        assert!(c.grey);

        let Action::Run(d) = parse(&["--layout", "--grey"]).unwrap() else {
            panic!("expected Run");
        };
        assert!(d.layout && d.layout_size.is_none() && d.grey);

        let Action::Run(e) = parse(&["--layout", "90", "28"]).unwrap() else {
            panic!("expected Run");
        };
        assert_eq!(e.layout_size, Some((90, 28)));
    }

    #[test]
    fn rejects_unknown_and_malformed_flags() {
        for bad in [
            &["--dryrun"][..],
            &["foo"][..],
            &["--dump", "10", "80"][..],
            &["--dump", "10", "0", "24"][..],
            &["--layout", "80"][..],
            &["--layout", "80", "0"][..],
            &["--host"][..],
            &["--host", ""][..],
            &["--host", "--sprint"][..],
            &["--host=bad host"][..],
            &["--music"][..],
            &["--music="][..],
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn parse_osc11_handles_bel_st_8bit_16bit_and_incomplete_payloads() {
        assert_eq!(parse_osc11(b"\x1b]11;rgb:ff/80/00\x07"), Some((0xff, 0x80, 0x00)));
        assert_eq!(
            parse_osc11(b"\x1b]11;rgb:1e1e/1e1e/2e2e\x07"),
            Some((0x1e, 0x1e, 0x2e))
        );
        assert_eq!(
            parse_osc11(b"\x1b]11;rgb:ffff/0000/8080\x1b\\"),
            Some((0xff, 0x00, 0x80))
        );
        assert_eq!(
            parse_osc11(b"junk\x1b]11;rgb:f/0/8\x07trailing"),
            Some((0xff, 0x00, 0x88))
        );
        assert_eq!(
            parse_osc11(b"\x1b]11;rgb:fff/000/800\x07"),
            Some((0xff, 0x00, 0x7f))
        );
        // Incomplete (no terminator yet): must return None so read loop waits.
        assert_eq!(parse_osc11(b"\x1b]11;rgb:1e1e/1e1e/2e"), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:ff/80/00"), None);
        // Malformed payloads.
        assert_eq!(parse_osc11(b"\x1b]11;rgb:zz/00/00\x07"), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:ff/00\x07"), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:ff/00/00/00\x07"), None);
    }
}
