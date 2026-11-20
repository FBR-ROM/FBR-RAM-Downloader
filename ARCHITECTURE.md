# Architecture

How the FBR RAM Downloader cracktro is put together, and why it is built the
way it is. For usage see [README.md](README.md); for the plug-in interface see
[Plugging in the real hack](README.md#plugging-in-the-real-hack).

## 1. What this is

A terminal cracktro — the small, self-contained intro that a cracking group
bolts onto a release. It plays a warp starfield, a big blocky title, a spinning
3D logo, a progress "loader" with a shimmer, a scrolling greeting, and a
blow-away ending, all in the 4 colours of a Game Boy screen.

It is deliberately a *presentation layer*. The real RAM-unlock work sits behind
one small trait, so the visuals can be developed, demoed and reviewed on their
own with a simulated stub.

## 2. The constraints that shaped everything

These are not preferences; they are the rules the whole design falls out of.

1. **Exactly four colours, no interpolation.** Per the FBR brand rules, every
   pixel on screen is one of four exact palette entries. There is no blending,
   no gradients, no anti-aliasing. All "shading" is quantization plus dithering.
2. **Foreground glyphs only.** The program never paints a background and never
   assumes the terminal's colours. It must look right on a dark theme, a light
   theme, any font, and any terminal or terminal-based UI. This rules out
   background-colour fills, which is why shading is done with glyph density.
3. **Any terminal size.** The composition must fit, stay centered, and never
   bleed over the border — from a 20x8 window to a 240x120 one.
4. **Deterministic and testable.** Rendering is a pure function of state, so it
   can be driven headlessly (`--dump`) and unit-tested without a terminal.
5. **No dependencies beyond the terminal library.** The only crate is
   `crossterm`; the rasterizer, palette logic and layout are hand-rolled.

## 3. Module map

| File | Lines | Responsibility |
|------|-------|----------------|
| `src/main.rs` | ~320 | Entry point, argument parsing, the event loop, live resize, headless `--dump`, `--layout` probe, terminal setup/teardown |
| `src/tui.rs` | ~730 | The cracktro itself: layout, screen elements, the loader, the scroller, the ending, and the layout tests |
| `src/scene.rs` | ~490 | 3D pipeline (model transform, projection, culling, painter's algorithm, rasterization, shading) plus the warp starfield |
| `src/canvas.rs` | ~140 | The 4-colour palettes, quantization/shimmer helpers, the cell grid, and ANSI serialization |
| `src/model_data.rs` | ~184 | Generated mesh data: 56 vertices, 120 triangles (do not edit by hand) |
| `src/hack.rs` | ~133 | The `Hack` trait, `HackState`, the simulated `StubHack`, and a worked C-binding example |
| `src/music.rs` | ~150 | Optional looping background music, auto-detected and toggled at runtime |
| `gen_model.py` | ~45 | Rasterizes `FBR nano.svg` into `model_data.rs` |
| `make_gif.py` | ~106 | Renders dumped ANSI frames to an animated GIF (for previews) |
| `demo.sh` | ~41 | One-command build + run, including a `nix-shell` fallback for `mpv` |

Dependency direction is one-way and shallow:

```
main.rs ──> tui.rs ──> canvas.rs
   │          │
   │          └─────> scene.rs ──> model_data.rs
   ├──> hack.rs
   └──> music.rs
```

`scene.rs` knows nothing about the terminal; it produces brightness samples.
`tui.rs` owns every pixel decision. `canvas.rs` owns colour and ANSI.

## 4. Per-frame data flow

One frame is a pure function of `(dt, scene state, hack state, w, h)`:

```
Hack::poll() ──> HackState{progress, status, done, error}
                     │
Scene::advance(dt) ──┤
Scene::render() ─────┴─> ModelFrame (one brightness sample per cell)
Starfield::render(dt) ──> Frame (glyph + brightness per cell)
                     │
                     v
        Cracktro::render(dt, logo, state, w, h)
                     │
                     ├─ layout(w, h)                -> where everything goes
                     ├─ starfield cells             -> inner area
                     ├─ box frame + border title    -> row 0 / h-1 / col 0 / w-1
                     ├─ key hints                    -> row 0, right-aligned
                     ├─ big FBR title                -> rows 1..6
                     ├─ 3D logo, dithered blocks     -> centered in its region
                     ├─ loader label + bar + status  -> rows h-5..h-3
                     └─ scroller                     -> row h-1
                     │
                     v
              Canvas::to_ansi() ──> one string, written in a single Print
```

Drawing order matters: later writes win, which is how text lands on top of the
model on small terminals. The starfield is drawn first so everything else
covers it.

## 5. Colour: quantization, shimmer, dither

`canvas::Palette` holds four RGB triples, index 0 lightest → 3 darkest:

- **Green** (default): `#F1FFF8, #6AB68A, #034F42, #011923`
- **Grey**: `#F5F5F7, #B2AFB8, #4A4B51, #000000`

Two mappings turn a continuous brightness into the 4-colour world:

- `Palette::shade(b)` — quantize: `index = round((1 - b) * 3)`. Stable shading.
- `Palette::cycle_shade(b, t, phase)` — quantize, then *rotate* through the
  palette over time. This is the Game Boy "colour cycling" shimmer, used by the
  title, the border title and the scroller.

There is a third mapping for the logo: `tui::dither_char`. Quantizing straight
to four levels looked flat, because a box only has a handful of face
brightnesses. So the ramp is spread over the range the model actually produces
(`MODEL_MIN_BRIGHT..MODEL_MAX_BRIGHT`) and then **ordered-dithered** with a 4x4
Bayer matrix (`tui::BAYER`), picking between the two nearest density levels by
cell position. That yields smooth intermediate tones out of four characters
(`░▒▓█`) and four palette entries, with no blending.

`canvas::Canvas` is a `w * h` grid of `Cell { ch, fg, bold }`. `to_ansi` emits
an escape sequence only when the colour or weight *changes* from the previous
cell, which keeps frames small (~1.5 KB for a 100x30 frame). It never emits
background codes.

## 6. The 3D pipeline (`scene.rs`)

**Mesh.** 56 vertices, 120 triangles, generated from `FBR nano.svg` by
`gen_model.py` (which rasterizes the SVG into an extruded mark and emits
triangulated faces). The result is a flat slab roughly 1.0 x 1.0 x 0.28 model
units.

**Centering.** On construction the mesh's **centroid** is computed
(`model_centroid`) and every vertex is translated by `-centroid` before
rotating. Without this the slab (whose x range is `-0.534..0.466`) spins about
a point that is not its middle, and the visible logo sits off-centre by a
couple of cells. With it, measured drift over a full spin is 0.5 cells.

**Transform.** Per vertex: yaw around Y (auto-spin, `SPIN_SPEED = 1.6 rad/s`,
clockwise), pitch around X (mouse-driven, clamped to `PITCH_MAX = 1.1`), then a
fixed camera tilt `TILT = 0.42 rad` so you look slightly down at the model and
see its top face.

**Projection.** Perspective with `dist = 60`, `f = 60`. Because a terminal cell
is about twice as tall as it is wide, the projected `y` is scaled by **0.5**
relative to `x`, which makes the model physically square on screen. The scale
is chosen from the frame size so the model always fits: worst-case projected
extents are 1.12 units wide and 1.40 tall (the tilt reveals the top face), so
`scale = min(w / 1.12, 2h / 1.40) * 0.94`.

**Visibility and ordering.** Faces whose normal points away from the camera are
culled (`n.z <= 0`), and the survivors are sorted far-to-near and drawn with
the painter's algorithm.

**Rasterization.** Each triangle is filled with three edge functions
(`rasterize_tri`); a sample is inside when all three edges agree in sign.
`ModelFrame::set` keeps the *brighter* sample when triangles overlap, which is
what makes front faces win.

**Shading.** Two lights:

- a fixed key light `(-0.4, 0.5, 0.77)` for the base flat shading
  (`brightness = 0.18 + 0.82 * max(n · L, 0)`), and
- a light that orbits at `0.9 rad/s`, driving a tight Blinn-Phong specular
  term (`max(n · H, 0)^24 * 0.7`) — the sheen that sweeps the faces.

A rare shimmer (a 0.4 s pulse every 3–7 s) flashes the model toward the lightest
shade. Mouse input can grab the model: dragging rotates it, releasing flings it
with momentum (`DAMP = 1.7`), and auto-spin resumes below `RESUME = 0.12`.

**Starfield.** 140 stars with a depth `z`; `z` decreases each frame and stars
wrap when they pass the camera, so they appear to fly outward from the centre.
Aspect-corrected, with `*`/`+`/`.` glyphs by brightness.

## 7. The responsive layout (`tui::layout`)

All geometry is a closed-form function of `(w, h)` in cell coordinates, with
row 0 / col 0 at the top-left. The returned `Layout` is the single source of
truth for the renderer.

```
borders      row 0, row h-1, col 0, col w-1
title        rows 1 ..= 1 + TITLE_ROWS + SHADOW_ROWS
loader       rows h-5 ..= h-3      (label, bar, status)
breathing    row  h-2
scroller     row  h-1
region       rows region_top ..= region_bottom
               region_top    = 1 + TITLE_ROWS + SHADOW_ROWS   (= 7)
               region_bottom = h - 1 - BREATH_ROWS - LOADER_ROWS
model        largest breakpoint (mw, mh) with
               mw <= w - 4  and  mh <= region_h
             placed at x = (w - mw) / 2                      (exact centering)
                       y = region_top + (region_h - mh) * MODEL_V_BIAS
```

**Model size** comes from a fixed breakpoint table (`MODEL_BREAKPOINTS`), largest
first: `48x24, 40x20, 32x16, 24x12, 18x9, 14x7, 10x5`. Cells are ~1:2, so a
square model is twice as wide as it is tall. The largest entry that fits both
the available width and the region's rows is used; below the smallest the model
is omitted rather than drawn as a sliver. Discrete tiers (rather than a
continuous scale) keep the model's pixel size stable and the composition
predictable.

**Horizontal placement** is exact: `x = (w - mw) / 2` splits the leftover space
into a left margin of `(w - mw) / 2` and a right margin of
`w - mw - (w - mw) / 2`, which differ by at most one cell. Because
`mw <= w - 4`, a centred model can never touch a side border.

**Vertical placement** is deliberately *below* centre by `MODEL_V_BIAS = 0.60`
(a fraction of the leftover space; 0.5 is dead centre, 1.0 is flush above the
loader). Geometrically centred looked too high: the model's lit top face draws
the eye upward and there is more empty space below it than above. The value is
clamped so the model can never reach the title or the loader. It is the one knob
to tune for vertical feel.

**Resize robustness.** The terminal size is re-read every frame with a cheap
ioctl rather than trusted only from `Resize` events, which some terminals and
multiplexers do not deliver. A stale size would mis-frame the whole composition
— it previously made the logo appear far too high after a window grew.

## 8. Screen elements

- **Box frame** — `┌ ┐ └ ┘ ─ │` on the outer ring. Every other element is
  clamped inside it; only the frame characters ever occupy row 0 / row h-1 /
  col 0 / col w-1.
- **Border title** — `CRACKED BY FBR ROM` centred on the top border, colour
  cycled per character.
- **Key hints** — `M MUTE • Q QUIT`, right-aligned on the top border in the mid
  shade (deliberately understated), drawn only when it cannot collide with the
  title.
- **Big title** — `FBR` in a 4-row blocky font (`BIG_FONT`), with an extruded
  drop shadow in the mid shade that is *masked* by the glyph face so counters
  and gaps stay open. The face shimmers through the palette.
- **Loader** — `DOWNLOADING RAM`, a progress bar, and a status line, anchored to
  the bottom just above the scroller with one blank row of breathing space. The
  bar is shade 2 only, with a shade-1 dithered shimmer (`█▓▒`) that sweeps the
  whole bar in complete passes, Windows-7 style, independent of progress. A
  `▓` marks the growth frontier.
- **Scroller** — a straight C64-style scroller on the bottom border, scrolled at
  10 chars/s with per-character colour cycling. Edit the `GREETINGS` constant to
  change the text.
- **Small terminals** — below 80x24 the status line asks you to resize, using a
  message short enough to fit.

## 9. The ending

On `done` the current frame is **snapshotted** and the cracktro switches to
`Celebration`, which renders nothing else. The frozen screen holds, then blows
away: a gust sweeps left to right (each column waits
`(sx / sw) * STAGGER` seconds, `STAGGER = 0.6`), after which every character
falls with `GRAVITY = 45` cells/s² and a little `WIND = 6` cells/s² to the
right. The snapshot is re-centred if the terminal resizes mid-celebration.
Once everything has left the screen, a blinking `RAM UNLOCKED • DONE` remains
until the program exits.

## 10. The pluggable hack

```rust
pub trait Hack {
    fn start(&mut self) -> Result<(), String>;  // once, before the loop
    fn poll(&mut self) -> HackState;            // once per frame, non-blocking
    fn finish(&mut self) -> Result<(), String>; // once, on any exit
}
```

`HackState` carries `progress` (0..=1), a `status` string, `done`, and an
optional `error`. Nothing else in the UI knows what the hack does: `progress`
drives the bar, `done` triggers the celebration and the exit, and `error` is
shown before exiting. `StubHack` simulates a 12-second download so the visuals
can be developed standalone. The README has a worked example of binding a C
implementation.

## 11. Music

`music::Music` resolves a track (an explicit `--music FILE`, else the first
audio file in `music/` alphabetically) and spawns the first available player
from `mpv → ffplay → paplay → aplay`, looping. If no player exists it degrades
to silence rather than failing. `toggle()` stops or restarts playback, which is
what the `m` key does. The child process is killed on drop, so music never
outlives the program. `demo.sh` wraps the run in `nix-shell -p mpv` when no
player is installed, so the track still plays on NixOS.

## 12. The main loop

- Fixed ~30 fps tick (33 ms).
- Each iteration: re-read the terminal size; drain pending events (`q` quit,
  `m` mute, `Resize`, mouse down/drag/up); compute `dt` (clamped to 100 ms so a
  stall cannot fling the animation); advance the scene; poll the hack; render;
  write the frame.
- Frames are written with a single `Print` after `MoveTo(0, 0)`. After a resize
  the reflowed content is cleared and redrawn **in the same batch**, so there is
  no blank flash between them.
- Terminal state (raw mode, alternate screen, mouse capture, cursor) is restored
  on every exit path, including errors.

## 13. Headless modes and tooling

- `--dump N [W H]` renders N frames at a fixed size to stdout, separated by
  blank lines, with the stub hack driven by frame count so output is
  deterministic. This is how the visuals are inspected and tested without a
  terminal.
- `--layout [W H]` prints the computed layout for a size (or the current
  terminal) and exits — the probe used to verify framing across sizes.
- `make_gif.py` parses the dumped ANSI into a grid and rasterizes it with
  M+ Nerd Font Mono into an animated GIF, used for the repo preview.
- `gen_model.py` regenerates `model_data.rs` from the SVG source.

## 14. Testing and verification

Six unit tests run with `cargo test`, all in `tui.rs` and `scene.rs`:

- the model never touches the border;
- the model always stays inside its region;
- the model is horizontally centred and sits below centre by exactly
  `MODEL_V_BIAS` (never above centre);
- the loader block is ordered and inside the border;
- the model clears the loader;
- the drawn logo stays within 0.75 cells of its frame centre over a full spin
  (measured 0.50).

Beyond the unit tests, two sweep scripts were used during development (kept out
of the repo): a 224-size layout sweep asserting containment and placement, and a
frame-level edge scan asserting that nothing but frame characters ever appears
on the outer ring. Both are driven by `--layout` and `--dump`, which is exactly
why those flags exist.

## 15. Key decisions and trade-offs

- **Hand-rolled rasterizer over a 3D crate.** The whole point is a tiny,
  dependency-light binary whose output is byte-deterministic and testable. The
  model is 120 triangles; a software rasterizer is a few dozen lines and gives
  exact control over quantization.
- **One brightness sample per cell, shaded by glyph density.** Two earlier
  approaches were tried and rejected: ASCII ramp characters (`. : - = + * # @`)
  read as a sparse sketch rather than a solid object, and flat solid blocks
  (`█` with a background fill) looked flat *and* required painting a background.
  Density-dithered blocks (`░▒▓█`) give a solid silhouette, real shading, and no
  background at all.
- **No background painting.** The program paints nothing behind its glyphs. This
  costs the ability to control contrast, but it is the only way the cracktro can
  claim to work on any terminal, any theme and any font. It also means the
  lightest palette entry can be low-contrast on a light theme — accepted, since
  portability was the explicit requirement.
- **Discrete size breakpoints over continuous scaling.** Fewer, predictable
  sizes keep the model crisp and the composition stable, and make the layout
  provably correct rather than approximately correct.
- **Deliberate vertical bias.** Geometric centring and perceived centring are
  not the same thing for a top-lit object; the bias is a documented constant
  rather than a magic number buried in the drawing code.
- **Frame-polled terminal size.** Cheap, and it removes a whole class of
  mis-framing bugs on terminals that do not report resizes.
- **The ending is a snapshot, not a re-render.** Freezing the actual last frame
  and animating its cells gives a true "the screen falls apart" effect, and
  keeps the celebration independent of the hack and the scene.

## 16. Extending it

- **Change the scroller text** — edit `GREETINGS` in `src/tui.rs`.
- **Change the logo's spin** — `SPIN_SPEED` / `SPIN_DIR` in `src/scene.rs`.
- **Change the model** — replace the SVG and run `gen_model.py`, or hand-edit
  `model_data.rs`.
- **Adjust vertical placement** — `MODEL_V_BIAS` in `src/tui.rs`.
- **Add a palette** — add a `const fn` beside `Palette::green` / `Palette::grey`
  and select it in `main.rs`.
- **Add model sizes** — extend `MODEL_BREAKPOINTS`.
- **Wire in the real hack** — implement `Hack` and swap the two construction
  sites in `main.rs`, as described in the README.

After any layout change, run `cargo test` and sweep with `--layout` before
committing.
