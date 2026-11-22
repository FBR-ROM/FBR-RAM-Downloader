//! FBR RAM Downloader — C64/Game Boy cracktro entry point.
//!
//! Runs the terminal cracktro: a warp starfield, a big 3D "FBR" title, a
//! color-cycled 3D logo, a "DOWNLOADING RAM" bar, and a falling-letters
//! celebration on completion. All in the 4-color FBR Game Boy palette.
//! The hack is pluggable via the [`hack::Hack`] trait; today it's the
//! simulated [`hack::StubHack`]. Resizes live with the terminal (btop-style).
//!
//! Flags:
//!   --dump N [W H]   headless: print N frames to stdout
//!   --grey           use the Grey Quarttrochrome palette (default: Green)
//!   --music FILE     play a specific track (else auto-detect music/)
//!   --no-music       disable music

mod canvas;
mod hack;
mod model_data;
mod music;
mod scene;
mod tui;

use crossterm::{
    event::{self, Event, KeyCode, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(33); // ~30 fps

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
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().collect();
    let mut args = Args {
        dump: None,
        grey: false,
        music: None,
        no_music: false,
        layout: false,
        layout_size: None,
        theme: None,
    };
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--dump" => {
                let n = argv.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(30);
                let w = argv.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(80);
                let h = argv.get(i + 3).and_then(|s| s.parse().ok()).unwrap_or(24);
                args.dump = Some((n, w, h));
                i += 4;
            }
            "--layout" => {
                args.layout = true;
                let w = argv.get(i + 1).and_then(|s| s.parse().ok());
                let h = argv.get(i + 2).and_then(|s| s.parse().ok());
                if let (Some(w), Some(h)) = (w, h) {
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
            "--music" => {
                if let Some(f) = argv.get(i + 1) {
                    args.music = Some(f.clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    args
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();
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

    // Detect the terminal's light/dark theme and remap the one palette shade
    // that would be invisible against the background. `--dark`/`--light`
    // override the auto-detection.
    let dark = match args.theme {
        Some(d) => d,
        None => detect_dark_mode(),
    };
    let palette = palette.remap_for(dark);

    let term = crossterm::terminal::size()?;
    let (term_w, term_h) = (term.0 as usize, term.1 as usize);

    let mut scene = scene::Scene::new(scene_size(term_w, term_h).0, scene_size(term_w, term_h).1);
    let mut cracktro = tui::Cracktro::new(term_w, term_h, palette);
    let mut hack: Box<dyn hack::Hack> = Box::new(hack::StubHack::new());
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

    // Always restore the terminal and stop music, even on error.
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    music.stop();
    let _ = hack.finish();

    result
}

fn dump_frames(n: usize, w: usize, h: usize, palette: canvas::Palette) -> Result<(), Box<dyn std::error::Error>> {
    let (sw, sh) = scene_size(w, h);
    let mut scene = scene::Scene::new(sw, sh);
    let mut cracktro = tui::Cracktro::new(w, h, palette);
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
    // Logo block in cells (shared with the TUI layout). The scene renders
    // half-block pixels internally: (units, 2 * rows) pixels.
    tui::logo_cells(term_w, term_h)
}

/// Detect whether the terminal has a dark background, so the palette can avoid
/// the one shade that would be invisible against it.
///
/// Asks the terminal for its background color via the OSC 11 query
/// (`ESC ] 11 ; ? BEL`) and reads the response from the input stream. Most
/// modern terminals (kitty, WezTerm, iTerm2, Ghostty, foot, xterm, VTE) answer
/// with an `rgb:RR/GG/BB` string. If the terminal doesn't respond within the
/// timeout, we fall back to assuming a dark background (the common case, and
/// the safe default for this cracktro's palette).
fn detect_dark_mode() -> bool {
    use std::io::IsTerminal;

    let stdin = io::stdin();
    let stdout = io::stdout();
    // Only query if we have a real TTY on both ends; otherwise the response
    // would never come and we'd just wait for the timeout.
    if !stdin.is_terminal() || !stdout.is_terminal() {
        return true;
    }

    // The query needs raw input to read the response without a newline.
    // Remember whether raw mode was already on so we can restore it.
    let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    if !was_raw {
        if let Err(_) = crossterm::terminal::enable_raw_mode() {
            return true;
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
fn query_background() -> bool {
    use std::io::{Read, Write};
    use std::os::unix::io::AsRawFd;
    use std::time::{Duration, Instant};

    let mut stdout = io::stdout();
    let _ = stdout.write_all(b"\x1b]11;?\x07");
    let _ = stdout.flush();

    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    // Make stdin non-blocking for the duration of the query.
    let orig = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    unsafe {
        libc::fcntl(fd, libc::F_SETFL, orig | libc::O_NONBLOCK);
    }

    let deadline = Instant::now() + Duration::from_millis(150);
    let mut buf = [0u8; 256];
    let mut got = 0usize;
    loop {
        if let Some(rgb) = parse_osc11(&buf[..got]) {
            // Restore blocking mode before returning.
            unsafe { libc::fcntl(fd, libc::F_SETFL, orig); }
            return is_dark_rgb(rgb);
        }
        if Instant::now() >= deadline {
            break;
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
                Ok(0) => break,
                Ok(n) => {
                    got += n;
                    if got >= buf.len() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
        }
    }
    // Restore blocking mode.
    unsafe { libc::fcntl(fd, libc::F_SETFL, orig); }
    // No response: assume dark.
    true
}

/// Parse an `rgb:RR/GG/BB` (hex) payload out of an OSC 11 response.
fn parse_osc11(data: &[u8]) -> Option<(u8, u8, u8)> {
    let s = String::from_utf8_lossy(data);
    let start = s.find("rgb:")? + 4;
    let rest = &s[start..];
    let parts: Vec<&str> = rest.splitn(3, '/').collect();
    if parts.len() < 3 {
        return None;
    }
    let r = u8::from_str_radix(parts[0].trim(), 16).ok()?;
    let g = u8::from_str_radix(parts[1].trim(), 16).ok()?;
    let b = u8::from_str_radix(parts[2].trim(), 16).ok()?;
    Some((r, g, b))
}

/// Perceptual luminance (Rec. 709). A background darker than ~0.5 is "dark".
fn is_dark_rgb((r, g, b): (u8, u8, u8)) -> bool {
    let lum = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
    lum < 128.0
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
                Event::Key(k) if k.code == KeyCode::Char('q') => {
                    return Ok(());
                }
                // 'm' toggles the background music.
                Event::Key(k) if k.code == KeyCode::Char('m') => {
                    music.toggle();
                }
                Event::Resize(w, h) => {
                    *size = (w as usize, h as usize);
                    let (sw, sh) = scene_size(w as usize, h as usize);
                    scene.set_size(sw, sh);
                    cracktro.set_size(w as usize, h as usize);
                }
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
