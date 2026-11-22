# Architecture

How the FBR RAM Downloader cracktro is put together, and why it is built the
way it is. For usage see [README.md](README.md); for the plug-in interface see
[Plugging in the real hack](README.md#plugging-in-the-real-hack).

## 1. What this is

A terminal cracktro — the small, self-contained intro that a cracking group
bolts onto a release. It plays a warp starfield, a shimmering border title, a
spinning 3D logo, a progress "loader" with a shimmer, a scrolling greeting, and
a blow-away ending, all in the 4 colours of a Game Boy screen.

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
5. **A small, boring dependency set.** The only crates are `crossterm`
   (terminal I/O), `ratatui` (back-buffer diffing), `rodio` (in-process audio),
   and `libc` (the OSC 11 background query). The rasterizer, palette logic and
   layout are hand-rolled.

## 3. Module map

| File | Lines | Responsibility |
|------|-------|----------------|
| `src/main.rs` | ~447 | Entry point, argument parsing, the event loop, live resize, headless `--dump`, `--layout` probe, background-color detection, terminal setup/teardown |
| `src/tui.rs` | ~703 | The cracktro itself: layout, screen elements, the loader, the scroller, the ending, and the layout tests |
| `src/scene.rs` | ~592 | 3D pipeline (model transform, projection, culling, painter's algorithm, rasterization, dynamic lighting, model cycling) plus the warp starfield |
| `src/canvas.rs` | ~272 | The 4-colour palettes, quantization/shimmer helpers, the background-contrast remap, the cell grid, and ANSI serialization |
| `src/model_data.rs` | ~8800 | Generated mesh data for the three models (do not edit by hand) |
| `src/hack.rs` | ~87 | The `Hack` trait, `HackState`, and the simulated `StubHack` |
| `src/music.rs` | ~153 | Optional looping background music (in-process via `rodio`), auto-detected and toggled at runtime |
| `gen_models.py` | ~165 | Regenerates `model_data.rs` from the three sources in `models/` (weld, decimate, normalize) |
| `make_gif.py` | ~106 | Renders dumped ANSI frames to an animated GIF (for previews) |
| `demo.sh` | ~31 | One-command build + run |

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
                     ├─ 3D logo, dithered blocks     -> centered in its region
                     ├─ loader label + bar + status  -> rows h-5..h-3
                     └─ scroller                     -> row h-1
                     │
                     v
        Canvas::to_buffer() ──> ratatui back-buffer, diffed and flushed
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

Because the cracktro is foreground-only, a shade that matches the terminal's
background would be invisible. `Palette::remap_for(bg)` fixes this: it compares
each of the four shades' luminance to the background's and swaps any that are
too close for the nearest visible one. The background is the terminal's actual
color, read with an OSC 11 query at startup (`main.rs::detect_background`);
`--dark`/`--light` force the assumption, and a non-answering terminal falls
back to a set of shades that is visible on both a black and a white
background. This is what makes the cracktro read correctly on light, dark,
*and* mid-tone (grey) themes.

## 6. The 3D pipeline (`scene.rs`)

**Mesh.** Three embedded models, in rotation order: the FBR nano logo (56
verts, 120 tris, used as-is), the v100, and the P5000. The two big models are
decimated to ~2000 triangles (keeping the largest-area triangles, which
preserves the silhouette) and normalized so their largest extent is 1.0 and
they're centered on the origin. `gen_models.py` regenerates `model_data.rs`
from the sources in `models/`.

**Height matching.** The v100 and P5000 are flat slabs that are the same width
as the FBR logo but about half its height. Each carries a uniform `scale`
(computed at startup as `FBR_Y / y_extent(model)`) applied around its centroid
so its height matches the FBR logo's; the width extends proportionally and,
being centered, any overflow clips symmetrically at the frame edge.

**Model cycling.** The cracktro swaps to the next model every half-turn of the
*auto* rotation (`auto_yaw` accumulates only the steady auto spin, not the
user's drag/flick, so grabbing the model never triggers a swap). Each model
carries its own centroid, computed once at startup.

**Centering.** Every vertex is translated by `-centroid` before rotating.
Without this a mesh spins about a point that is not its middle and sits
off-centre by a couple of cells. With it, the model projects symmetrically
about the frame's center.

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

**Shading.** Three dynamic lights, all computed once per frame:

- a **key light** that slowly precesses around the model (biased toward the
  camera so the front stays lit, with a gentle vertical bob) for the base flat
  shading,
- a **rim light** orbiting the other way, from the side, which catches edges as
  it sweeps around, and
- a **breathing ambient** that subtly pulses the whole model.

Each of the two directional lights also drives a Blinn-Phong specular highlight
(a broad soft sheen from the key, a tight glint from the rim). The interplay of
the model's spin with the two moving lights keeps the shading in constant,
non-repeating motion — this is what reveals the detail in the simplified
meshes.

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
loader       rows h-5 ..= h-3      (label, bar, status)
breathing    row  h-2
scroller     row  h-1
region       rows region_top ..= region_bottom
               region_top    = 1
               region_bottom = h - 1 - BREATH_ROWS - LOADER_ROWS
model        largest breakpoint (mw, mh) with
               mw <= w - 4  and  mh <= region_h
             placed at x = (w - mw) / 2                      (exact centering)
                       y = region_top + (region_h - mh) * MODEL_V_BIAS
```

**Model size** comes from a fixed breakpoint table (`MODEL_BREAKPOINTS`), largest
first: `56x28, 48x24, 40x20, 32x16, 24x12, 18x9, 14x7, 10x5`. Cells are ~1:2, so a
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
  cycled per character. The live terminal size is drawn top-left.
- **Key hints** — `M MUTE • Q QUIT`, right-aligned on the top border in the mid
  shade (deliberately understated), drawn only when it cannot collide with the
  title.
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
audio file in `music/` alphabetically) and decodes + plays it **in-process**
via `rodio`, looping. No external player (mpv/ffplay) is needed, so the binary
is fully self-contained. If there is no audio device, or the file can't be
opened/decoded, it degrades to silence rather than failing (the cracktro still
runs). `toggle()` pauses or resumes the sink, which is what the `m` key does.
The stream and sink are dropped on exit, so music never outlives the program.

## 12. The main loop

- Fixed ~30 fps tick (33 ms).
- Each iteration: re-read the terminal size (a cheap ioctl, so resizes are
  picked up even when the terminal doesn't deliver a `Resize` event); drain
  pending events (`q` quit, `m` mute, mouse down/drag/up); compute `dt`
  (clamped to 100 ms so a stall cannot fling the animation); advance the scene;
  poll the hack; render; write the frame.
- The frame is rendered into a `Canvas`, then handed to ratatui's `draw`, which
  auto-resizes its back-buffer to the real terminal size, diffs it against the
  previous frame, and writes only the changed cells. So a resize reflows
  cleanly with no blank flash and no full-screen reprint.
- Terminal state (raw mode, alternate screen, mouse capture, cursor) is
  restored on every exit path, including errors — and the teardown is
  best-effort, so a failure there can't mask the real result or skip the
  music/hack cleanup.

## 13. Headless modes and tooling

- `--dump N [W H]` renders N frames at a fixed size to stdout, separated by
  blank lines, with the stub hack driven by frame count so output is
  deterministic. This is how the visuals are inspected and tested without a
  terminal.
- `--layout [W H]` prints the computed layout for a size (or the current
  terminal) and exits — the probe used to verify framing across sizes.
- `make_gif.py` parses the dumped ANSI into a grid and rasterizes it with
  M+ Nerd Font Mono into an animated GIF, used for the repo preview.
- `gen_models.py` regenerates `model_data.rs` from the three sources in
  `models/` (weld, decimate, normalize).

## 14. Testing and verification

Eight unit tests run with `cargo test`, in `tui.rs`, `scene.rs` and `canvas.rs`:

- the model never touches the border;
- the model always stays inside its region;
- the model is horizontally centred and sits below centre by exactly
  `MODEL_V_BIAS` (never above centre);
- the loader block is ordered and inside the border;
- the model clears the loader;
- the drawn model stays within 3 cells of its frame centre over a full spin
  across all three models (the flat badges drift more as they turn edge-on);
- the background-contrast remap keeps every palette shade visible against a
  sweep of backgrounds (both palettes), including the mid-tones a naive
  dark/light split gets wrong;
- a non-detected background falls back to shades visible on both black and
  white.

Beyond the unit tests, two sweep scripts were used during development (kept out
of the repo): a 224-size layout sweep asserting containment and placement, and a
frame-level edge scan asserting that nothing but frame characters ever appears
on the outer ring. Both are driven by `--layout` and `--dump`, which is exactly
why those flags exist.

## 15. Key decisions and trade-offs

- **Hand-rolled rasterizer over a 3D crate.** The whole point is a tiny,
  dependency-light binary whose output is byte-deterministic and testable. The
  models are at most ~2000 triangles; a software rasterizer is a few dozen lines
  and gives exact control over quantization.
- **One brightness sample per cell, shaded by glyph density.** Two earlier
  approaches were tried and rejected: ASCII ramp characters (`. : - = + * # @`)
  read as a sparse sketch rather than a solid object, and flat solid blocks
  (`█` with a background fill) looked flat *and* required painting a background.
  Density-dithered blocks (`░▒▓█`) give a solid silhouette, real shading, and no
  background at all.
- **No background painting.** The program paints nothing behind its glyphs.
  This is the only way the cracktro can claim to work on any terminal, any
  theme and any font. The cost — a foreground shade can match the background —
  is handled by the startup contrast remap (`Palette::remap_for`), which swaps
  any invisible shade for the nearest visible one, so the lightest entry no
  longer washes out on a light theme.
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
- **Change the model's spin** — `SPIN_SPEED` / `SPIN_DIR` in `src/scene.rs`.
- **Change the models** — replace a source in `models/` and run
  `gen_models.py`, or hand-edit `model_data.rs`. The rotation order is the
  `MODELS` array in `scene.rs`.
- **Adjust vertical placement** — `MODEL_V_BIAS` in `src/tui.rs`.
- **Add a palette** — add a `const fn` beside `Palette::green` / `Palette::grey`
  and select it in `main.rs`.
- **Tune contrast** — `Palette::CONTRAST` in `src/canvas.rs` (the minimum
  luminance distance a shade must keep from the background).
- **Add model sizes** — extend `MODEL_BREAKPOINTS`.
- **Wire in the real hack** — implement `Hack` and swap the two construction
  sites in `main.rs`, as described in the README.

After any layout change, run `cargo test` and sweep with `--layout` before
committing.
