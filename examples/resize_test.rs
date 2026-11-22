//! Minimal live-resize test for the ratatui backend.
//!
//! Draws a full-screen box border plus the current terminal size (WxH) in the
//! top-left. Drag the terminal edges: the size readout updates and the border
//! reflows to the new edges on every frame. Press `q` to quit.
//!
//! Build & run:
//!   cargo run --release --example resize_test

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders},
    Terminal,
};

const TICK: Duration = Duration::from_millis(33); // ~30 fps

fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::cursor::Hide
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal);

    disable_raw_mode()?;
    execute!(
        io::stdout(),
        crossterm::cursor::Show,
        LeaveAlternateScreen
    )?;
    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<(), Box<dyn std::error::Error>> {
    let mut last = Instant::now();

    loop {
        // Drain input; quit on 'q'.
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(k) = event::read()? {
                if k.code == KeyCode::Char('q') {
                    return Ok(());
                }
            }
        }

        let now = Instant::now();
        let _dt = now.duration_since(last).as_secs_f32().min(0.1);
        last = now;

        // `draw` auto-resizes the back-buffer to the real terminal size and
        // diffs against the previous frame, so the border + readout track the
        // terminal on every resize with no manual handling.
        terminal.draw(|f| {
            let area = f.area();
            let (w, h) = (area.width, area.height);

            let title = format!(" resize test  \u{2022}  {w} x {h}  \u{2022}  q to quit ");
            let block = Block::default()
                .borders(Borders::ALL)
                .title(Line::from(title))
                .style(Style::default().fg(Color::Green));

            // A centered readout so the size change is obvious.
            let readout = format!("{w} x {h}");
            let cy = (h.saturating_sub(1)) / 2;
            let cx = (w.saturating_sub(1)) / 2;
            if cy > 0 && cx > 0 {
                // write the readout centered
                let rw = readout.chars().count() as u16;
                let start = cx.saturating_sub(rw / 2);
                for (i, c) in readout.chars().enumerate() {
                    if let Some(cell) = f.buffer_mut().cell_mut((start + i as u16, cy)) {
                        cell.set_char(c);
                        cell.set_fg(Color::White);
                        cell.modifier = Modifier::BOLD;
                    }
                }
            }

            f.render_widget(block, area);
        })?;

        let elapsed = now.elapsed();
        if elapsed < TICK {
            std::thread::sleep(TICK - elapsed);
        }
    }
}
