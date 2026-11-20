//! 3D scene: renders the FBR nano logo (embedded OBJ model) with a
//! painter's-algorithm software rasterizer, plus a warp starfield.
//!
//! The model rasterizes one brightness sample per cell. The TUI turns each
//! sample into a shaded block (see `tui::dither_char`): every covered cell gets
//! at least a `░`, so the silhouette is always solid, and the density of the
//! block character carries the shading. Foreground glyphs only — no background
//! is ever painted, so it renders on any terminal and any theme.
//!
//! Cells are about twice as tall as wide, so the projection pre-compensates in
//! y to keep the model square.
//!
//! The model auto-spins clockwise. The user can grab it with the mouse and
//! flick it (drag to rotate, release to fling with momentum). A rare shimmer
//! flashes the model to the lightest shade while it spins.

use crate::model_data::{FACES, VERTS};

/// Auto-spin speed (rad/s) and direction (-1 = clockwise, viewed from above).
const SPIN_SPEED: f32 = 1.6;
const SPIN_DIR: f32 = -1.0;
/// Camera tilt around X (view slightly from above).
const TILT: f32 = 0.42;
/// Flick momentum decay (per second).
const DAMP: f32 = 1.7;
/// Below this |velocity|, auto-spin resumes.
const RESUME: f32 = 0.12;
/// Pitch clamp (rad) so the model can't be flipped over.
const PITCH_MAX: f32 = 1.1;

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3 { x: self.x - o.x, y: self.y - o.y, z: self.z - o.z }
    }
    fn cross(self, o: Vec3) -> Vec3 {
        Vec3 {
            x: self.y * o.z - self.z * o.y,
            y: self.z * o.x - self.x * o.z,
            z: self.x * o.y - self.y * o.x,
        }
    }
    fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn add(self, o: Vec3) -> Vec3 {
        Vec3 { x: self.x + o.x, y: self.y + o.y, z: self.z + o.z }
    }
    fn norm(self) -> Vec3 {
        let l = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt().max(1e-6);
        Vec3 { x: self.x / l, y: self.y / l, z: self.z / l }
    }
}

/// One rendered starfield frame: a grid of cells, each with a glyph and a
/// brightness.
#[derive(Clone)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    /// Row-major, `h * w` cells.
    pub cells: Vec<Cell>,
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    /// 0.0 (dark) ..= 1.0 (bright).
    pub bright: f32,
}

impl Frame {
    fn new(w: usize, h: usize) -> Self {
        Frame {
            w,
            h,
            cells: vec![Cell { ch: ' ', bright: 0.0 }; w * h],
        }
    }
    fn set(&mut self, x: usize, y: usize, ch: char, bright: f32) {
        if x < self.w && y < self.h {
            let i = y * self.w + x;
            // Keep the brighter of overlapping cells (front faces win).
            if bright >= self.cells[i].bright {
                self.cells[i] = Cell { ch, bright };
            }
        }
    }
}

/// The rendered model: one brightness sample per cell. `w` by `h` cells,
/// row-major; 0.0 = uncovered (leave the cell blank).
#[derive(Clone)]
pub struct ModelFrame {
    pub w: usize,
    pub h: usize,
    pub pix: Vec<f32>,
}

impl ModelFrame {
    fn new(w: usize, h: usize) -> Self {
        ModelFrame { w, h, pix: vec![0.0; w * h] }
    }
    fn set(&mut self, x: usize, y: usize, bright: f32) {
        if x < self.w && y < self.h {
            let i = y * self.w + x;
            // Keep the brighter of overlapping pixels (front faces win).
            if bright >= self.pix[i] {
                self.pix[i] = bright;
            }
        }
    }
}

pub struct Scene {
    w: usize,
    h: usize,
    /// Centroid of the model's vertices. The mesh is translated by `-center`
    /// before rotating, so the logo spins about its true center and projects
    /// symmetrically about the middle of its frame.
    center: Vec3,
    yaw: f32,
    pitch: f32,
    vyaw: f32,
    vpitch: f32,
    dragging: bool,
    time: f32,
    // Shimmer.
    shimmer_t: f32,
    next_shimmer: f32,
}

impl Scene {
    pub fn new(w: usize, h: usize) -> Self {
        Scene {
            w: w.max(8),
            h: h.max(6),
            center: model_centroid(),
            yaw: 0.0,
            pitch: 0.0,
            vyaw: 0.0,
            vpitch: 0.0,
            dragging: false,
            time: 0.0,
            shimmer_t: 0.0,
            next_shimmer: 3.0,
        }
    }

    pub fn set_size(&mut self, w: usize, h: usize) {
        self.w = w.max(8);
        self.h = h.max(6);
    }

    /// Mouse pressed on the model: start dragging.
    pub fn mouse_down(&mut self) {
        self.dragging = true;
        self.vyaw = 0.0;
        self.vpitch = 0.0;
    }

    /// Mouse released: fling with the current drag velocity.
    pub fn mouse_up(&mut self) {
        self.dragging = false;
    }

    /// Mouse dragged by (dx, dy) cells. Converts to angular velocity.
    pub fn mouse_drag(&mut self, dx: i32, dy: i32) {
        if !self.dragging {
            return;
        }
        let k = 0.05;
        self.yaw += dx as f32 * k;
        self.pitch += dy as f32 * k;
        self.pitch = self.pitch.clamp(-PITCH_MAX, PITCH_MAX);
        // Track velocity for the flick.
        self.vyaw = dx as f32 * k * 30.0;
        self.vpitch = dy as f32 * k * 30.0;
    }

    /// Advance the spin / flick / shimmer by `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        self.time += dt;
        if !self.dragging {
            self.yaw += self.vyaw * dt;
            self.pitch += self.vpitch * dt;
            self.pitch = self.pitch.clamp(-PITCH_MAX, PITCH_MAX);
            // Damp the flick.
            let damp = (DAMP * dt).min(1.0);
            self.vyaw -= self.vyaw * damp;
            self.vpitch -= self.vpitch * damp;
            // Resume auto-spin once the flick dies down.
            if self.vyaw.abs() < RESUME {
                self.vyaw = SPIN_SPEED * SPIN_DIR;
            }
        }
        // Rare shimmer.
        if self.shimmer_t > 0.0 {
            self.shimmer_t -= dt;
        } else if self.next_shimmer > 0.0 {
            self.next_shimmer -= dt;
            if self.next_shimmer <= 0.0 {
                self.shimmer_t = 0.4;
                self.next_shimmer = 3.0 + rand01() * 4.0;
            }
        }
    }

    /// Shimmer intensity 0..=1 (a short pulse).
    fn shimmer(&self) -> f32 {
        if self.shimmer_t <= 0.0 {
            return 0.0;
        }
        let x = 1.0 - self.shimmer_t / 0.4; // 0..=1 over the pulse
        (x * (1.0 - x) * 4.0).min(1.0) // triangle pulse
    }

    /// Render the model to a shaded sample frame (one sample per cell).
    pub fn render(&self) -> ModelFrame {
        // One sample per cell. Cells are ~1:2 (w:h), so the projection halves
        // y to keep the model square.
        let pw = self.w;
        let ph = self.h;
        let mut frame = ModelFrame::new(pw, ph);
        let dist = 60.0;
        let f = 60.0;
        // Worst-case projected extents in model units (width varies with yaw;
        // height includes the fixed camera tilt revealing the top face).
        // Physical screen: `pw` units wide, `2 * ph` units tall. Scale so the
        // model always fits, filling ~94% of the tighter axis.
        let scale = (pw as f32 / 1.12).min((2 * ph) as f32 / 1.40) * 0.94;
        let sh = self.shimmer();

        let ca = self.yaw.cos();
        let sa = self.yaw.sin();
        let ct = TILT.cos();
        let st = TILT.sin();
        let cp = self.pitch.cos();
        let sp = self.pitch.sin();

        // Transform + project all vertices.
        let mut proj: Vec<(f32, f32)> = Vec::with_capacity(VERTS.len());
        let mut wz: Vec<f32> = Vec::with_capacity(VERTS.len());
        let mut world: Vec<Vec3> = Vec::with_capacity(VERTS.len());
        for &(vx, vy, vz) in VERTS {
            // Translate the mesh so its centroid sits on the origin: the model
            // then projects symmetrically about the frame's center.
            let vx = vx - self.center.x;
            let vy = vy - self.center.y;
            let vz = vz - self.center.z;
            // Yaw (around Y).
            let x1 = vx * ca + vz * sa;
            let z1 = -vx * sa + vz * ca;
            let y1 = vy;
            // Pitch (around X).
            let y2 = y1 * cp - z1 * sp;
            let z2 = y1 * sp + z1 * cp;
            let x2 = x1;
            // Tilt (around X, camera).
            let y3 = y2 * ct - z2 * st;
            let z3 = y2 * st + z2 * ct;
            let x3 = x2;
            world.push(Vec3 { x: x3, y: y3, z: z3 });
            // Perspective.
            let negz = (dist - z3).max(1.0);
            let px = x3 * f / negz;
            let py = y3 * f / negz;
            // y maps at half rate: each cell row is twice a cell column.
            proj.push((px * scale + pw as f32 / 2.0, -py * scale * 0.5 + ph as f32 / 2.0));
            wz.push(z3);
        }

        struct PTri {
            depth: f32,
            bright: f32,
            pts: [(f32, f32); 3],
        }
        let mut ptris: Vec<PTri> = Vec::with_capacity(FACES.len());
        // Key light (fixed) + a slowly orbiting light that drives the sheen:
        // a Blinn-Phong highlight that travels over the faces as it spins.
        let light = Vec3 { x: -0.4, y: 0.5, z: 0.77 }.norm();
        let la = self.time * 0.9;
        let sheen_light = Vec3 { x: la.cos() * 0.8, y: 0.55, z: la.sin() * 0.8 }.norm();
        let view = Vec3 { x: 0.0, y: 0.0, z: 1.0 };
        let sheen_h = sheen_light.add(view).norm();

        for &(ia, ib, ic) in FACES {
            let a = world[ia];
            let b = world[ib];
            let c = world[ic];
            // Normal in transformed (camera) space.
            let n = b.sub(a).cross(c.sub(a)).norm();
            // Backface cull: keep faces whose normal points toward the
            // camera (camera is at +z, so normal.z > 0 faces us).
            if n.z <= 0.0 {
                continue;
            }
            let diff = n.dot(light).max(0.0);
            let mut bright = 0.18 + 0.82 * diff;
            // Specular sheen: a tight highlight that sweeps the model.
            let spec = n.dot(sheen_h).max(0.0).powi(24);
            bright = (bright + 0.7 * spec).min(1.0);
            // Shimmer: flash toward the lightest shade.
            if sh > 0.0 {
                bright = bright + (1.0 - bright) * sh;
            }
            let depth = (wz[ia] + wz[ib] + wz[ic]) / 3.0;
            ptris.push(PTri {
                depth,
                bright,
                pts: [proj[ia], proj[ib], proj[ic]],
            });
        }

        // Painter's algorithm: far (smaller z) first.
        ptris.sort_by(|x, y| x.depth.partial_cmp(&y.depth).unwrap_or(std::cmp::Ordering::Equal));

        for pt in &ptris {
            rasterize_tri(&mut frame, pt.pts, pt.bright);
        }

        frame
    }
}

/// The bounding-box center of the drawn (non-blank) samples, in cells.
#[cfg(test)]
fn drawn_center(frame: &ModelFrame) -> Option<(f32, f32)> {
    let (mut minx, mut maxx, mut miny, mut maxy) = (usize::MAX, 0usize, usize::MAX, 0usize);
    let mut any = false;
    for y in 0..frame.h {
        for x in 0..frame.w {
            if frame.pix[y * frame.w + x] > 0.04 {
                any = true;
                minx = minx.min(x);
                maxx = maxx.max(x);
                miny = miny.min(y);
                maxy = maxy.max(y);
            }
        }
    }
    any.then(|| {
        (
            (minx + maxx) as f32 / 2.0,
            (miny + maxy) as f32 / 2.0,
        )
    })
}

/// The centroid of the model's vertices, computed once. Used to translate the
/// mesh onto the origin so the logo is centered as it spins.
fn model_centroid() -> Vec3 {
    let mut sum = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    for &(x, y, z) in VERTS {
        sum = sum.add(Vec3 { x, y, z });
    }
    let n = VERTS.len().max(1) as f32;
    Vec3 { x: sum.x / n, y: sum.y / n, z: sum.z / n }
}

fn edge(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    (px - b.0) * (a.1 - b.1) - (a.0 - b.0) * (py - b.1)
}

fn rasterize_tri(frame: &mut ModelFrame, pts: [(f32, f32); 3], bright: f32) {
    let [a, b, c] = pts;
    let minx = a.0.min(b.0).min(c.0).floor() as i32;
    let maxx = a.0.max(b.0).max(c.0).ceil() as i32;
    let miny = a.1.min(b.1).min(c.1).floor() as i32;
    let maxy = a.1.max(b.1).max(c.1).ceil() as i32;
    let h = frame.h as i32;
    let w = frame.w as i32;
    for y in miny.max(0)..=maxy.min(h - 1) {
        for x in minx.max(0)..=maxx.min(w - 1) {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d1 = edge(px, py, a, b);
            let d2 = edge(px, py, b, c);
            let d3 = edge(px, py, c, a);
            let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            if !(has_neg && has_pos) {
                frame.set(x as usize, y as usize, bright);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The logo must stay centered in its own frame all the way around the
    /// spin: the drawn silhouette's bounding-box center tracks the frame
    /// center (the mesh is translated onto its centroid for this reason).
    #[test]
    fn logo_stays_centered_while_spinning() {
        let (w, h) = (48usize, 24usize);
        let mut scene = Scene::new(w, h);
        let cx = (w - 1) as f32 / 2.0;
        let cy = (h - 1) as f32 / 2.0;
        let mut worst_x = 0.0f32;
        let mut worst_y = 0.0f32;
        for _ in 0..64 {
            for _ in 0..4 {
                scene.advance(0.05);
            }
            if let Some((bx, by)) = drawn_center(&scene.render()) {
                worst_x = worst_x.max((bx - cx).abs());
                worst_y = worst_y.max((by - cy).abs());
            }
        }
        // A flat box's silhouette shifts as it turns edge-on: measured drift is
        // 0.50 cells across, so anything above 0.75 means the mesh is
        // off-center horizontally. Vertically the fixed camera tilt reveals the
        // top face, which lifts the silhouette by up to a cell (measured 1.00)
        // — that is the 3D pose, not a placement error, so it gets a looser
        // bound.
        assert!(worst_x <= 0.75, "logo drifts {worst_x:.2} cells horizontally");
        assert!(worst_y <= 1.25, "logo drifts {worst_y:.2} cells vertically");
    }
}

/// Cheap deterministic-ish random in [0,1).
static RNG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x2545F4914F6CDD1D);
fn rand01() -> f32 {
    use std::sync::atomic::Ordering;
    let mut s = RNG.fetch_add(0x9E3779B97F4A7C15, Ordering::Relaxed);
    s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((s >> 33) as f32) / (1u64 << 31) as f32
}

/// Warp starfield: stars fly toward the viewer from the center.
/// Rendered into a `Frame` (glyph + brightness); the TUI tints it.
pub struct Starfield {
    stars: Vec<[f32; 3]>,
    w: usize,
    h: usize,
}

impl Starfield {
    pub fn new(w: usize, h: usize, count: usize) -> Self {
        let mut stars = Vec::with_capacity(count);
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut rng = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f32) / (1u64 << 31) as f32
        };
        for _ in 0..count {
            stars.push([rng() * 2.0 - 1.0, rng() * 2.0 - 1.0, rng() * 0.9 + 0.1]);
        }
        Starfield { stars, w, h }
    }

    pub fn set_size(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
    }

    /// Advance and render one frame. `speed` scales the warp.
    pub fn render(&mut self, dt: f32, speed: f32) -> Frame {
        let mut frame = Frame::new(self.w, self.h);
        let cx = self.w as f32 / 2.0;
        let cy = self.h as f32 / 2.0;
        let aspect = (self.w as f32 / self.h.max(1) as f32).max(0.1);
        for s in &mut self.stars {
            s[2] -= dt * speed;
            if s[2] <= 0.05 {
                s[0] = (s[0] * 2.0 - 1.0).clamp(-1.0, 1.0);
                s[1] = (s[1] * 2.0 - 1.0).clamp(-1.0, 1.0);
                s[2] = 0.9 + (s[0] * s[0] + s[1] * s[1]) * 0.1;
            }
            let px = s[0] / s[2] * aspect * cx * 0.9;
            let py = s[1] / s[2] * cy * 0.9;
            let x = (cx + px).round() as i32;
            let y = (cy + py).round() as i32;
            if x >= 0 && y >= 0 && x < self.w as i32 && y < self.h as i32 {
                let bright = (1.0 - s[2]).clamp(0.0, 1.0);
                let ch = if bright > 0.85 { '*' } else if bright > 0.5 { '+' } else { '.' };
                frame.set(x as usize, y as usize, ch, bright);
            }
        }
        frame
    }
}
