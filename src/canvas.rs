//! FBR brand color system + a full-screen colored canvas.
//!
//! Per the FBR brand rules (`FBR-DOCS/04 brand and design/Colour.md`), the
//! design language is dictated by the Game Boy DMG/MGB hardware: **exactly 4
//! colors, no more, no less.** Two quarttrochrome palettes are provided
//! (Green and Grey). All color in the cracktro is one of the 4 exact palette
//! colors — no interpolation — and the "color cycling" is the classic Game
//! Boy 4-shade shimmer (hard-stepping through the palette over time).

/// A 4-color Game Boy palette. Index 0 = lightest, 3 = darkest.
#[derive(Clone, Copy)]
pub struct Palette {
    pub colors: [(u8, u8, u8); 4],
}

impl Palette {
    /// Palette 1: Green Quarttrochrome (#F1FFF8, #6AB68A, #034F42, #011923).
    pub const fn green() -> Self {
        Palette {
            colors: [
                (241, 255, 248), // 0 lightest
                (106, 182, 138), // 1
                (3, 79, 66),     // 2
                (1, 25, 35),     // 3 darkest
            ],
        }
    }

    /// Palette 2: Grey Quarttrochrome (#F5F5F7, #B2AFB8, #4A4B51, #000000).
    pub const fn grey() -> Self {
        Palette {
            colors: [
                (245, 245, 247), // 0 lightest
                (178, 175, 184), // 1
                (74, 75, 81),    // 2
                (0, 0, 0),       // 3 darkest
            ],
        }
    }

    /// Map a brightness (0..=1) to one of the 4 exact shades (quantized, no
    /// interpolation). 1.0 = lightest, 0.0 = darkest.
    pub fn shade(&self, brightness: f32) -> (u8, u8, u8) {
        let idx = ((1.0 - brightness.clamp(0.0, 1.0)) * 3.0).round() as usize;
        self.colors[idx.min(3)]
    }

    /// 4-shade shimmer: a brightness mapped to a shade, then shifted through
    /// the palette over time. This is the Game Boy "color cycle" effect.
    pub fn cycle_shade(&self, brightness: f32, t: f32, phase: f32) -> (u8, u8, u8) {
        let base = ((1.0 - brightness.clamp(0.0, 1.0)) * 3.0).round() as usize;
        let shift = (((t * 0.5 + phase).rem_euclid(1.0)) * 4.0) as usize;
        self.colors[(base + shift) % 4]
    }
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: (u8, u8, u8),
    /// Explicit background color. Used by the model, which is painted as
    /// background-filled spaces: the terminal fills the whole cell rectangle
    /// regardless of font metrics, so there are never seams between rows.
    pub bg: Option<(u8, u8, u8)>,
    pub bold: bool,
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub cells: Vec<Cell>,
}

impl Canvas {
    /// Create a canvas whose every cell carries the palette's darkest shade as
    /// its background. The cracktro therefore paints its own backdrop and looks
    /// identical on any terminal theme (light or dark) — it never relies on the
    /// terminal's default colors, so it is fully self-contained.
    pub fn new(w: usize, h: usize, bg: (u8, u8, u8)) -> Self {
        Canvas {
            w,
            h,
            cells: vec![Cell { ch: ' ', fg: bg, bg: Some(bg), bold: false }; w * h],
        }
    }

    /// Draw a glyph, keeping the cell's backdrop so text never punches a hole
    /// in the painted background.
    pub fn set(&mut self, x: usize, y: usize, ch: char, fg: (u8, u8, u8), bold: bool) {
        if x < self.w && y < self.h {
            let bg = self.cells[y * self.w + x].bg;
            self.cells[y * self.w + x] = Cell { ch, fg, bg, bold };
        }
    }

    /// Paint a solid, gapless block: the block glyph *and* the cell
    /// background are both set to `color`. Terminals differ in how they fill a
    /// cell — some paint the background across the whole cell rectangle, some
    /// draw the `█` glyph edge-to-edge — so setting both maximises coverage and
    /// avoids seams between adjacent blocks. (If a terminal applies cell
    /// padding/line spacing that neither fills, a hairline seam is unavoidable
    /// from ANSI alone.)
    pub fn set_block(&mut self, x: usize, y: usize, color: (u8, u8, u8)) {
        if x < self.w && y < self.h {
            self.cells[y * self.w + x] = Cell { ch: '█', fg: color, bg: Some(color), bold: false };
        }
    }

    /// Draw `s` starting at column `x`, row `y`.
    pub fn text(&mut self, x: usize, y: usize, s: &str, fg: (u8, u8, u8), bold: bool) {
        for (i, c) in s.chars().enumerate() {
            self.set(x + i, y, c, fg, bold);
        }
    }

    /// Draw `s` centered on row `y` within the inner area (columns 1..w-1),
    /// so it never touches the side borders. Text wider than the inner area
    /// is truncated rather than allowed to spill over the border.
    pub fn text_center_inner(&mut self, y: usize, s: &str, fg: (u8, u8, u8), bold: bool) {
        let inner_w = self.w.saturating_sub(2);
        let clipped: String = s.chars().take(inner_w).collect();
        let tw = clipped.chars().count();
        let start = 1 + inner_w.saturating_sub(tw).saturating_div(2);
        self.text(start, y, &clipped, fg, bold);
    }

    /// Starting column to center a `width`-wide block within the inner area
    /// (columns 1..w-1).
    pub fn inner_center(&self, width: usize) -> usize {
        let inner_w = self.w.saturating_sub(2);
        1 + inner_w.saturating_sub(width).saturating_div(2)
    }

    /// Render to a truecolor ANSI string. Emits a color change only when the
    /// color actually differs from the previous cell (keeps output small).
    pub fn to_ansi(&self) -> String {
        let mut out = String::with_capacity(self.w * self.h * 3);
        let mut cur_fg: (u8, u8, u8) = (0, 0, 0);
        let mut cur_bg: Option<(u8, u8, u8)> = None;
        let mut cur_bold = false;
        for y in 0..self.h {
            for x in 0..self.w {
                let c = &self.cells[y * self.w + x];
                if c.fg != cur_fg {
                    out.push_str(&format!("\x1b[38;2;{};{};{}m", c.fg.0, c.fg.1, c.fg.2));
                    cur_fg = c.fg;
                }
                if c.bg != cur_bg {
                    match c.bg {
                        Some((r, g, b)) => out.push_str(&format!("\x1b[48;2;{};{};{}m", r, g, b)),
                        None => out.push_str("\x1b[49m"),
                    }
                    cur_bg = c.bg;
                }
                if c.bold != cur_bold {
                    out.push_str(if c.bold { "\x1b[1m" } else { "\x1b[22m" });
                    cur_bold = c.bold;
                }
                out.push(c.ch);
            }
            if y + 1 < self.h {
                out.push('\n');
            }
        }
        out.push_str("\x1b[0m");
        out
    }
}
