//! FBR cracktro: composes the warp starfield, a big 3D "FBR" title, the
//! color-cycled 3D logo (half-block pixels), the "DOWNLOADING RAM" bar, a
//! straight scroller, and a freeze-and-blow-away ending — all in the 4-color
//! FBR Game Boy palette.

use crate::canvas::{Canvas, Cell, Palette};
use crate::hack::HackState;
use crate::scene::{ModelFrame, Starfield};

const BAR_WIDTH: usize = 44;

/// Rows of blocky glyphs in the big title.
pub const TITLE_ROWS: usize = 4;

/// Big 4-row blocky font for the title. Each glyph is 4 wide.
/// ('#' = ink, '.' = empty.) Add glyphs here to change the title text.
const BIG_FONT: &[(&str, [&str; 4])] = &[
    ("F", ["####", "#...", "###.", "#..."]),
    ("B", ["###.", "#..#", "#..#", "###."]),
    ("R", ["###.", "#..#", "###.", "#.."]),
];

/// Greetings / credits shown on the bottom scroller.
///
/// EDIT THIS to add your own text. It scrolls left on a loop; separate
/// entries with "  \u{2022}  " (or anything you like). Keep it one line.
pub const GREETINGS: &str = "  MUSIC: \"ARCANE MAGIC\"  \u{2022}  GREETINGS TO:  \u{2022}  THE FBR CREW  \u{2022}  ALL THE RAM HUNTERS  \u{2022}  STAY UNLOCKED  \u{2022}  ";

/// Model size breakpoints in cells, largest first: `(width, rows)`. Pixels are
/// one per cell and cells are ~1:2, so a square model spans twice as many
/// columns as rows. The layout picks the largest breakpoint that fits both the
/// available width and the available rows, so the model is always fully in
/// view and never bleeds over the border or into the loader.
pub const MODEL_BREAKPOINTS: &[(usize, usize)] = &[
    (48, 24),
    (40, 20),
    (32, 16),
    (24, 12),
    (18, 9),
    (14, 7),
    (10, 5),
];

/// The complete responsive layout for a terminal of `w` x `h` cells.
///
/// Everything the renderer draws derives from this one struct, so the
/// composition is guaranteed to fit: the model is centered in the region
/// between the title and the loader, and every row/column is clamped inside
/// the border.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// First row of the model's region (just below the title and its shadow).
    pub region_top: usize,
    /// Last row of the model's region (just above the loader).
    pub region_bottom: usize,
    /// Model size in cells and its top-left corner.
    pub model_w: usize,
    pub model_h: usize,
    pub model_x: usize,
    pub model_y: usize,
    pub label_row: usize,
    pub bar_row: usize,
    pub status_row: usize,
    pub scroller_row: usize,
    /// True when the frame is wide enough for the title + key hints.
    pub show_hint: bool,
    /// True when the terminal is too small for the full composition.
    pub too_small: bool,
}

/// Compute the responsive layout for a `w` x `h` terminal. Pure function of
/// the size, so it can be unit-checked and printed with `--layout`.
pub fn layout(w: usize, h: usize) -> Layout {
    let w = w.max(3);
    let h = h.max(3);
    let inner_w = w.saturating_sub(2);

    // The loader block is anchored to the bottom: label, bar, status, then a
    // blank breathing row, then the scroller on the bottom border.
    let label_row = h.saturating_sub(5).max(2);
    let bar_row = label_row + 1;
    let status_row = bar_row + 1;
    let scroller_row = h - 1;

    // The model lives between the title (+ shadow) and the loader.
    let region_top = (1 + TITLE_ROWS + 2).min(h.saturating_sub(2)).max(1);
    let region_bottom = label_row.saturating_sub(1).max(region_top);
    let region_h = region_bottom - region_top + 1;

    // Largest breakpoint that fits both ways. If even the smallest does not
    // fit, the model is omitted (0x0) rather than drawn as a sliver — the rest
    // of the composition still renders, and nothing ever bleeds.
    let avail_w = inner_w.saturating_sub(2);
    let mut model_w = 0usize;
    let mut model_h = 0usize;
    for &(bw, bh) in MODEL_BREAKPOINTS {
        if bw <= avail_w && bh <= region_h {
            model_w = bw;
            model_h = bh;
            break;
        }
    }

    let model_x = 1 + inner_w.saturating_sub(model_w) / 2;
    let model_y = region_top + (region_h.saturating_sub(model_h)) / 2;

    // Title (18 chars) + hints (16) + margins need roughly 40 columns.
    let show_hint = w >= 46;
    let too_small = w < 80 || h < 24;

    Layout {
        region_top,
        region_bottom,
        model_w,
        model_h,
        model_x,
        model_y,
        label_row,
        bar_row,
        status_row,
        scroller_row,
        show_hint,
        too_small,
    }
}

/// Model size in cells, for the scene (which rasterizes at this size).
pub fn logo_cells(w: usize, h: usize) -> (usize, usize) {
    let l = layout(w, h);
    (l.model_w, l.model_h)
}

/// Human-readable layout dump, used by `--layout` to verify framing at any
/// terminal size without running the TUI.
pub fn describe_layout(w: usize, h: usize) -> String {
    let l = layout(w, h);
    format!(
        "terminal {w}x{h}\n\
         model {}x{} at col {} row {} (rows {}..{})\n\
         title row 1..{}\n\
         label {}  bar {}  status {}  scroller {}\n\
         hint {}  too_small {}\n",
        l.model_w,
        l.model_h,
        l.model_x,
        l.model_y,
        l.region_top,
        l.region_bottom,
        TITLE_ROWS,
        l.label_row,
        l.bar_row,
        l.status_row,
        l.scroller_row,
        l.show_hint,
        l.too_small,
    )
}

/// The completion celebration: the finished screen freezes in place, then
/// blows away — a gust sweeps left to right and every character falls off the
/// bottom under gravity (with a little wind), leaving the final message.
struct Celebration {
    cells: Vec<Cell>, // snapshot of the last complete frame
    sw: usize,
    sh: usize,
    t: f32,
}

impl Celebration {
    /// Seconds for the gust to sweep across the screen width.
    const STAGGER: f32 = 0.6;
    /// Downward acceleration (cells/s^2).
    const GRAVITY: f32 = 45.0;
    /// Rightward acceleration (cells/s^2) — the "blow away".
    const WIND: f32 = 6.0;

    fn snapshot(cv: &Canvas) -> Self {
        Celebration { cells: cv.cells.clone(), sw: cv.w, sh: cv.h, t: 0.0 }
    }

    /// Seconds until even the last column has left the screen.
    fn clear_time(&self) -> f32 {
        let fall = (2.0 * (self.sh as f32 + 4.0) / Self::GRAVITY).sqrt();
        Self::STAGGER + fall + 0.2
    }

    fn render(&mut self, dt: f32, p: Palette, w: usize, h: usize) -> String {
        self.t += dt;
        let t = self.t;
        let mut cv = Canvas::new(w, h, p.colors[3]);
        // Center the frozen screen (handles a resize mid-celebration).
        let ox = w.saturating_sub(self.sw) / 2;
        let oy = h.saturating_sub(self.sh) / 2;
        for sy in 0..self.sh {
            for sx in 0..self.sw {
                let cell = self.cells[sy * self.sw + sx];
                if cell.ch == ' ' {
                    continue;
                }
                // The gust sweeps left -> right: each column holds still
                // until its delay passes, then drops away, accelerating.
                let delay = (sx as f32 / self.sw.max(1) as f32) * Self::STAGGER;
                let u = t - delay;
                if u < 0.0 {
                    cv.set(ox + sx, oy + sy, cell.ch, cell.fg, cell.bold);
                    continue;
                }
                let fx = sx as f32 + 0.5 * Self::WIND * u * u;
                let fy = sy as f32 + 0.5 * Self::GRAVITY * u * u;
                let px = (ox as f32 + fx).round() as usize;
                let py = (oy as f32 + fy).round() as usize;
                cv.set(px, py, cell.ch, cell.fg, cell.bold);
            }
        }
        // Once the screen has blown away, leave the message blinking.
        if t > self.clear_time() {
            let col = p.cycle_shade(1.0, t, 0.0);
            cv.text_center_inner(h / 2, "RAM UNLOCKED \u{2022} DONE", col, true);
        }
        cv.to_ansi()
    }
}

pub struct Cracktro {
    starfield: Starfield,
    time: f32,
    palette: Palette,
    celebration: Option<Celebration>,
}

impl Cracktro {
    pub fn new(w: usize, h: usize, palette: Palette) -> Self {
        Cracktro {
            starfield: Starfield::new(w, h, 140),
            time: 0.0,
            palette,
            celebration: None,
        }
    }

    pub fn set_size(&mut self, w: usize, h: usize) {
        self.starfield.set_size(w, h);
    }

    /// Render one frame. `logo` is the 3D model's pixel frame; `state` the
    /// hack state.
    pub fn render(
        &mut self,
        dt: f32,
        logo: &ModelFrame,
        state: &HackState,
        w: usize,
        h: usize,
    ) -> String {
        self.time += dt;
        let t = self.time;
        let p = self.palette;

        // Celebration active: the frozen screen blows away; nothing animates
        // behind it.
        if let Some(celeb) = &mut self.celebration {
            return celeb.render(dt, p, w, h);
        }

        let mut cv = Canvas::new(w, h, p.colors[3]);

        // Guard: need at least a 3x3 box.
        if w < 3 || h < 3 {
            return cv.to_ansi();
        }

        // 1. Warp starfield background, clipped to the inner area so it never
        //    touches the border. The brightest stars pop in the lightest shade.
        let sf = self.starfield.render(dt, 0.55);
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                if y < sf.h && x < sf.w {
                    let c = &sf.cells[y * sf.w + x];
                    if c.ch != ' ' {
                        let col = if c.bright > 0.85 {
                            p.colors[0]
                        } else {
                            p.shade(0.3 + c.bright * 0.5)
                        };
                        cv.set(x, y, c.ch, col, false);
                    }
                }
            }
        }

        // 2. Closed box frame: top, bottom, and both side borders.
        self.box_frame(&mut cv, w, h, t);

        // 2b. "CRACKED BY FBR ROM" title in the top border (shimmering).
        let title = "CRACKED BY FBR ROM";
        let tw = title.chars().count();
        let start = cv.inner_center(tw);
        for (i, c) in title.chars().enumerate() {
            let ccol = p.cycle_shade(1.0, t, i as f32 * 0.05);
            cv.set(start + i, 0, c, ccol, true);
        }

        // 2c. Key hints, right-aligned in the top border. Only drawn when
        //     the frame is wide enough to keep them clear of the title.
        let lay = layout(w, h);
        let hint = "M MUTE \u{2022} Q QUIT";
        let hw = hint.chars().count();
        let hint_start = w.saturating_sub(2).saturating_sub(hw);
        if lay.show_hint && hint_start > start + tw + 2 {
            for (i, c) in hint.chars().enumerate() {
                cv.set(hint_start + i, 0, c, p.colors[2], false);
            }
        }

        // 3. Big 3D "FBR" title, anchored under the top border.
        self.big_title(&mut cv, "FBR", 1, t);

        // 4. 3D logo — centered in the region between the title and the loader
        //    (see `layout`). Each pixel is a solid block (glyph + background
        //    both set to the shade), flat-shaded to the exact palette colors,
        //    so adjacent pixels join with no seams. Uncovered cells stay
        //    transparent so the starfield shows around the model. The model is
        //    always fully inside the region: the breakpoint chosen by `layout`
        //    guarantees it fits and never bleeds.
        for cy in 0..lay.model_h {
            let row = lay.model_y + cy;
            if row <= lay.region_top.saturating_sub(1) || row >= h - 1 {
                continue; // never paint over the border rows
            }
            for cx in 0..lay.model_w {
                let idx = cy * logo.w + cx;
                if idx >= logo.pix.len() {
                    continue;
                }
                let bright = logo.pix[idx];
                if bright < 0.04 {
                    continue;
                }
                cv.set_block(lay.model_x + cx, row, p.shade(bright));
            }
        }

        // 5. Loader label + bar + status.
        let label_col = p.colors[2];
        cv.text_center_inner(lay.label_row, "DOWNLOADING RAM", label_col, true);
        self.bar(&mut cv, lay.bar_row, state, t);
        // Small terminals: prompt to resize instead of the status line.
        // Pick a message that actually fits the inner width.
        let inner_w = w.saturating_sub(2);
        let (status, status_col, status_bold) = if state.done {
            ("RAM UNLOCKED \u{2022} DONE", p.cycle_shade(1.0, t, 0.0), true)
        } else if lay.too_small {
            let msg = if inner_w >= 20 { "RESIZE FOR BEST FIT" } else { "RESIZE" };
            (msg, p.colors[0], true)
        } else {
            (state.status.as_str(), p.colors[0], false)
        };
        if lay.status_row < h - 1 {
            cv.text_center_inner(lay.status_row, status, status_col, status_bold);
        }

        // 7. Bottom border marquee (straight, single color).
        self.marquee(&mut cv, w, h, t);

        // 8. On completion: freeze this complete frame, then blow it away.
        if state.done {
            self.celebration = Some(Celebration::snapshot(&cv));
            return self.celebration.as_mut().unwrap().render(0.0, p, w, h);
        }

        cv.to_ansi()
    }

    /// Draw the full closed box: top/bottom borders (with the title / marquee
    /// drawn over them by their own functions) and the left/right `│` sides.
    fn box_frame(&self, cv: &mut Canvas, w: usize, h: usize, _t: f32) {
        let col = self.palette.colors[1];
        // Top border.
        for x in 0..w {
            cv.set(x, 0, '─', col, false);
        }
        cv.set(0, 0, '┌', col, false);
        cv.set(w - 1, 0, '┐', col, false);
        // Bottom border.
        for x in 0..w {
            cv.set(x, h - 1, '─', col, false);
        }
        cv.set(0, h - 1, '└', col, false);
        cv.set(w - 1, h - 1, '┘', col, false);
        // Side borders.
        for y in 1..h - 1 {
            cv.set(0, y, '│', col, false);
            cv.set(w - 1, y, '│', col, false);
        }
    }

    fn big_title(&self, cv: &mut Canvas, text: &str, top: usize, t: f32) {
        let p = self.palette;
        // Lay out the glyph grid first (TITLE_ROWS rows tall, 4-wide glyphs).
        let gw = 4 * text.chars().count() + (text.chars().count() - 1);
        let mut grid: Vec<Vec<char>> = vec![vec![' '; gw]; TITLE_ROWS];
        for (li, ch) in text.chars().enumerate() {
            if let Some((_, g)) = BIG_FONT.iter().find(|(n, _)| *n == ch.to_string()) {
                for r in 0..TITLE_ROWS {
                    for c in 0..4 {
                        let col = li * 5 + c;
                        if col < gw {
                            grid[r][col] = g[r].chars().nth(c).unwrap_or(' ');
                        }
                    }
                }
            }
        }
        let start = cv.inner_center(gw);
        // Drop shadow: the glyph shape shifted down-right, skipping any cell
        // the face itself covers (so counters and gaps stay open). Mid shade,
        // so the extrusion is actually visible against the dark background.
        for (dx, dy) in [(1usize, 1usize), (2, 2)] {
            for r in 0..TITLE_ROWS {
                for c in 0..gw {
                    if grid[r][c] != '#' {
                        continue;
                    }
                    let (sr, sc) = (top + r + dy, start + c + dx);
                    let on_face = sr >= top
                        && sr < top + TITLE_ROWS
                        && sc >= start
                        && sc < start + gw
                        && grid[sr - top][sc - start] == '#';
                    if !on_face {
                        cv.set(sc, sr, '#', p.colors[2], false);
                    }
                }
            }
        }
        // Shimmering face on top.
        for r in 0..TITLE_ROWS {
            for c in 0..gw {
                if grid[r][c] == '#' {
                    let col = p.cycle_shade(1.0, t, c as f32 * 0.04);
                    cv.set(start + c, top + r, '#', col, true);
                }
            }
        }
    }

    /// The loader: a progress bar in palette shade 2 with a shade-1 dithered
    /// shimmer that sweeps the whole bar in complete passes (Windows-7 style),
    /// independent of how far the download has got.
    fn bar(&self, cv: &mut Canvas, row: usize, state: &HackState, t: f32) {
        let p = self.palette;
        let base = p.colors[2];
        let shine = p.colors[1];
        // Bar width adapts to the inner area so it never touches the border.
        let bar_w = (BAR_WIDTH as i32).min((cv.w.saturating_sub(12)) as i32).max(8) as usize;
        let filled = (state.progress * bar_w as f32).round() as usize;
        let pct = (state.progress * 100.0).round() as usize;
        let pct_s = format!("  {}%", pct);
        let total = 2 + bar_w + pct_s.chars().count();
        let start = cv.inner_center(total);

        // The shimmer: a soft band of dithered shades that travels the whole
        // bar, loops, and does complete passes.
        let band_w = (bar_w / 3).max(4);
        let travel = bar_w + band_w; // full pass incl. entering/leaving
        let head = (t * 16.0) as usize % travel;
        let third = (band_w / 3).max(1);

        let mut x = start;
        cv.set(x, row, '[', base, true);
        x += 1;
        for i in 0..bar_w {
            // Base: solid fill up to progress, light track beyond it.
            let (mut ch, mut col, mut bold) = if i < filled {
                ('█', base, false)
            } else {
                ('░', base, false)
            };
            // Distance behind the shimmer's leading edge (0 = head).
            if i < head && head - i <= band_w {
                let d = head - i;
                // Dithered density: bright core fading to a sparse tail.
                let (dch, dcol) = if d <= third {
                    ('█', shine)
                } else if d <= 2 * third {
                    ('▓', shine)
                } else {
                    ('▒', shine)
                };
                ch = dch;
                col = dcol;
                bold = true;
            } else if i == filled {
                // The growth frontier, just ahead of the fill.
                ch = '▓';
                col = shine;
                bold = true;
            }
            cv.set(x, row, ch, col, bold);
            x += 1;
        }
        cv.set(x, row, ']', base, true);
        x += 1;
        for c in pct_s.chars() {
            cv.set(x, row, c, base, c != ' ');
            x += 1;
        }
    }

    fn marquee(&self, cv: &mut Canvas, w: usize, h: usize, t: f32) {
        let p = self.palette;
        let row = h - 1;
        // Bottom border line.
        for x in 0..w {
            cv.set(x, row, '─', p.colors[1], false);
        }
        cv.set(0, row, '└', p.colors[1], false);
        cv.set(w - 1, row, '┘', p.colors[1], false);
        // Straight scroller along the border, chars color-cycled through the
        // palette shades — the 4-color take on the classic C64 scroller.
        let speed = 10.0; // chars/sec
        let len = GREETINGS.chars().count();
        let offset = ((t * speed) as usize) % len;
        let chars: Vec<char> = GREETINGS.chars().collect();
        for x in 1..w.saturating_sub(1) {
            let c = chars[(x + offset) % len];
            let col = p.cycle_shade(1.0, t, x as f32 * 0.08);
            cv.set(x, row, c, col, false);
        }
    }
}
