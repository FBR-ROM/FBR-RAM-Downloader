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

    /// Quantize a brightness (0..=1) to a palette index: 1.0 = lightest,
    /// 0.0 = darkest. No interpolation — the Game Boy way.
    fn shade_index(brightness: f32) -> usize {
        ((1.0 - brightness.clamp(0.0, 1.0)) * 3.0).round() as usize
    }

    /// Map a brightness (0..=1) to one of the 4 exact shades.
    pub fn shade(&self, brightness: f32) -> (u8, u8, u8) {
        self.colors[Self::shade_index(brightness).min(3)]
    }

    /// 4-shade shimmer: a brightness mapped to a shade, then shifted through
    /// the palette over time. This is the Game Boy "color cycle" effect.
    pub fn cycle_shade(&self, brightness: f32, t: f32, phase: f32) -> (u8, u8, u8) {
        let base = Self::shade_index(brightness);
        let shift = (((t * 0.5 + phase).rem_euclid(1.0)) * 4.0) as usize;
        self.colors[(base + shift) % 4]
    }

    /// Minimum luminance distance (0..255) a foreground shade must keep from
    /// the terminal background to stay visible.
    const CONTRAST: f32 = 32.0;

    /// Perceptual luminance (Rec. 709), 0..255.
    fn luminance((r, g, b): (u8, u8, u8)) -> f32 {
        0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
    }

    /// Return a copy of the palette with every shade that would be invisible
    /// against the terminal background swapped for the nearest visible one.
    ///
    /// The cracktro is foreground-only, so a glyph drawn in a color that
    /// matches the background vanishes. We compare each shade's luminance to
    /// the background's and, for any that are too close, pick the palette
    /// entry that is both visible and closest in brightness — the smallest
    /// change that keeps the shade readable. This handles mid-tone
    /// backgrounds (e.g. a grey theme) that a simple dark/light split gets
    /// wrong.
    ///
    /// `bg` is the terminal's actual background color (OSC 11 query). When it
    /// is `None` — the terminal didn't answer, or we have no TTY — we can't
    /// know the theme, so we make every shade visible on *both* a black and a
    /// white background. That guarantees contrast no matter what the terminal
    /// is actually using, which is the safe default for a foreground-only
    /// cracktro.
    pub fn remap_for(&self, bg: Option<(u8, u8, u8)>) -> Self {
        // The background luminance we must clear. `None` = unknown theme, in
        // which case a shade must survive both black (lum 0) and white
        // (lum 255).
        let bg_lum: Option<f32> = bg.map(Self::luminance);
        let visible = |c: (u8, u8, u8)| match bg_lum {
            Some(bl) => (Self::luminance(c) - bl).abs() >= Self::CONTRAST,
            None => {
                let l = Self::luminance(c);
                l >= Self::CONTRAST && l <= 255.0 - Self::CONTRAST
            }
        };
        let mut colors = self.colors;
        for i in 0..4 {
            if !visible(colors[i]) {
                let target = Self::luminance(colors[i]);
                let best = self
                    .colors
                    .iter()
                    .copied()
                    .filter(|c| visible(*c))
                    .min_by(|a, b| {
                        let da = (Self::luminance(*a) - target).abs();
                        let db = (Self::luminance(*b) - target).abs();
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    });
                if let Some(c) = best {
                    colors[i] = c;
                }
            }
        }
        Palette { colors }
    }
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: (u8, u8, u8),
    pub bold: bool,
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub cells: Vec<Cell>,
}

impl Canvas {
    /// A canvas of blank cells. Nothing paints a background: the cracktro
    /// draws foreground glyphs only, so it renders correctly on any terminal,
    /// any theme, and any font — it never assumes or overrides the
    /// terminal's own background.
    pub fn new(w: usize, h: usize) -> Self {
        Canvas {
            w,
            h,
            cells: vec![Cell { ch: ' ', fg: (0, 0, 0), bold: false }; w * h],
        }
    }

    pub fn set(&mut self, x: usize, y: usize, ch: char, fg: (u8, u8, u8), bold: bool) {
        if x < self.w && y < self.h {
            self.cells[y * self.w + x] = Cell { ch, fg, bold };
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
    /// color actually differs from the previous cell (keeps output small);
    /// blank cells never change color, so runs of them stay cheap.
    pub fn to_ansi(&self) -> String {
        let mut out = String::with_capacity(self.w * self.h * 3);
        let mut cur_fg: (u8, u8, u8) = (0, 0, 0);
        let mut cur_bold = false;
        for y in 0..self.h {
            for x in 0..self.w {
                let c = &self.cells[y * self.w + x];
                if c.ch != ' ' {
                    if c.fg != cur_fg {
                        out.push_str(&format!("\x1b[38;2;{};{};{}m", c.fg.0, c.fg.1, c.fg.2));
                        cur_fg = c.fg;
                    }
                    if c.bold != cur_bold {
                        out.push_str(if c.bold { "\x1b[1m" } else { "\x1b[22m" });
                        cur_bold = c.bold;
                    }
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

    /// Write the canvas into a ratatui [`Buffer`], which the `Terminal` diffs
    /// against the previous frame and flushes to the terminal. Only non-blank
    /// cells are written: the buffer is already blank, and skipping them keeps
    /// the diff small. The buffer must be at least `w` x `h`; out-of-range
    /// cells are dropped (the terminal is never smaller than the canvas).
    pub fn to_buffer(&self, buf: &mut ratatui::buffer::Buffer) {
        use ratatui::style::{Color, Modifier};
        for y in 0..self.h.min(buf.area.height as usize) {
            for x in 0..self.w.min(buf.area.width as usize) {
                let c = &self.cells[y * self.w + x];
                if c.ch == ' ' {
                    continue;
                }
                if let Some(cell) = buf.cell_mut((x as u16, y as u16)) {
                    cell.set_char(c.ch);
                    cell.set_fg(Color::Rgb(c.fg.0, c.fg.1, c.fg.2));
                    cell.modifier = if c.bold {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Palette;

    /// The remap's contract: after remapping against a background, no shade
    /// may be invisible against it. This is the whole point of the remap —
    /// the cracktro is foreground-only, so a shade that matches the background
    /// vanishes. Check both palettes across a sweep of backgrounds, including
    /// the mid-tones a naive dark/light split gets wrong.
    #[test]
    fn remap_keeps_every_shade_visible() {
        let bgs = [
            (0, 0, 0),
            (30, 30, 30),
            (40, 44, 52),
            (64, 64, 64),
            (85, 85, 85),
            (128, 128, 128),
            (200, 200, 200),
            (245, 245, 245),
            (255, 255, 255),
        ];
        for pal in [Palette::green(), Palette::grey()] {
            for bg in bgs {
                let remapped = pal.remap_for(Some(bg));
                let bl = Palette::luminance(bg);
                for c in remapped.colors {
                    let d = (Palette::luminance(c) - bl).abs();
                    assert!(
                        d >= Palette::CONTRAST,
                        "shade {:?} invisible against bg {:?} (dist {})",
                        c,
                        bg,
                        d
                    );
                }
            }
        }
    }

    /// A background we couldn't detect (`None`) must fall back to a palette
    /// that is visible on *both* a black and a white background — so contrast
    /// is guaranteed no matter what the terminal is actually using.
    #[test]
    fn remap_none_visible_on_both_themes() {
        for pal in [Palette::green(), Palette::grey()] {
            let remapped = pal.remap_for(None);
            for c in remapped.colors {
                let l = Palette::luminance(c);
                assert!(
                    l >= Palette::CONTRAST && l <= 255.0 - Palette::CONTRAST,
                    "shade {:?} not visible on both black and white (lum {})",
                    c,
                    l
                );
            }
        }
    }

    #[test]
    fn shade_and_cycle_shade_always_produce_exact_palette_colors() {
        for pal in [Palette::green(), Palette::grey()] {
            for b in [-0.5, 0.0, 0.25, 0.5, 0.75, 1.0, 1.5] {
                assert!(pal.colors.contains(&pal.shade(b)));
                for t in [0.0, 0.33, 1.25, 4.7] {
                    assert!(pal.colors.contains(&pal.cycle_shade(b, t, 0.17)));
                }
            }
            assert_eq!(pal.shade(1.0), pal.colors[0]);
            assert_eq!(pal.shade(0.0), pal.colors[3]);
        }
    }

    #[test]
    fn canvas_clipping_ansi_and_buffer_invariants() {
        use super::Canvas;
        let pal = Palette::green();
        let mut cv = Canvas::new(10, 3);
        // Out-of-bounds writes must be safe no-ops.
        cv.set(10, 0, 'X', pal.colors[0], true);
        cv.set(0, 3, 'X', pal.colors[0], true);
        // Centered text wider than inner area clips without touching col 0 or 9.
        cv.text_center_inner(1, "ABCDEFGHIJKLMNOP", pal.colors[1], true);
        assert_eq!(cv.cells[10].ch, ' ');
        assert_eq!(cv.cells[19].ch, ' ');
        assert_eq!(cv.cells[11].ch, 'A');
        assert_eq!(cv.cells[18].ch, 'H');

        let ansi = cv.to_ansi();
        assert!(!ansi.contains("\x1b[48;"), "must never emit background colors");
        assert!(ansi.ends_with("\x1b[0m"));
        // Adjacent cells with identical fg/bold emit the color sequence only once.
        let fg_seq = format!(
            "\x1b[38;2;{};{};{}m",
            pal.colors[1].0, pal.colors[1].1, pal.colors[1].2
        );
        assert_eq!(ansi.matches(&fg_seq).count(), 1);

        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 10, 3));
        cv.to_buffer(&mut buf);
        assert_eq!(buf.cell((1, 1)).unwrap().symbol(), "A");
        assert!(buf
            .cell((1, 1))
            .unwrap()
            .modifier
            .contains(ratatui::style::Modifier::BOLD));
    }
}
