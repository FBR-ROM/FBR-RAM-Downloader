# FBR RAM Downloader — Game Boy Cracktro

A Game Boy DMG/MGB-styled cracktro for the FBR RAM-unlock hack: a warp
starfield, a big 3D "FBR" title, a dither-shaded 3D spinning logo (the
centerpiece), a "DOWNLOADING RAM" loader with a Windows-7-style shimmer, a
straight color-cycled scroller, and a freeze-then-blow-away ending. The hack
is pluggable — today it's a simulated stub so the UI can be developed and
demoed standalone.

## Color

Per the FBR brand rules (`FBR-DOCS/04 brand and design/Colour.md`), the design
is dictated by the Game Boy DMG/MGB hardware: **exactly 4 colors, no more, no
less.** Two quarttrochrome palettes:

- **Green** (default): `#F1FFF8, #6AB68A, #034F42, #011923`
- **Grey**: `#F5F5F7, #B2AFB8, #4A4B51, #000000`

Every pixel on screen is one of the 4 exact colors (no interpolation). The
"color cycling" is the classic Game Boy 4-shade shimmer — hard-stepping
through the palette over time.

The cracktro draws **foreground glyphs only** — it never paints a background,
so it works on any terminal, any theme, and any font without assuming or
overriding the terminal's own colors.

## Run

```sh
./demo.sh                 # interactive (press q to quit)
./demo.sh --grey          # Grey Quarttrochrome palette
./demo.sh --dump 360 80 24  # headless: print frames to stdout
```

The UI resizes live with the terminal (btop-style) — drag the window and it
reflows, keeping the 3D model centered between the title and the loader. The
bar fills over ~12 s (stub), then the screen freezes and blows away.

Music plays by default; press `m` to mute/unmute. `./demo.sh` runs inside a
`nix-shell -p mpv` when no audio player is installed, so the track still
plays on NixOS.

### Flags

| Flag | Effect |
|------|--------|
| `--grey` | Use the Grey Quarttrochrome palette (default: Green) |
| `--music FILE` | Play a specific track (else auto-detect `music/`) |
| `--no-music` | Start with music off |
| `--dump N [W H]` | Headless: print N frames to stdout |
| `--layout [W H]` | Print the computed layout for a size and exit (debug aid) |

### Keys

| Key | Effect |
|-----|--------|
| `q` | Quit |
| `m` | Mute / unmute the music (music is on by default) |
| drag | Grab the model and flick it to spin |

## Music

Drop a track in `music/` (mp3/ogg/flac/wav/m4a/opus). It auto-plays, looping,
when you run the program — only inside the program, stopped cleanly on exit.
The first audio file in `music/` (alphabetically) is used; `--music FILE`
picks a specific one. Playback uses the first available player: `mpv` →
`ffplay` → `paplay` → `aplay`. If none is installed it silently no-ops.

## Preview GIF

```sh
./demo.sh --dump 360 80 24 > /tmp/frames.txt
nix-shell -p python3Packages.pillow --run "python3 make_gif.py /tmp/frames.txt 2 preview.gif"
```

Renders with **M+ Nerd Font Mono** (the "60" weight) and truecolor.

## Tests

```sh
nix-shell -p rustc cargo --run "cargo test"
```

The layout and the logo's centering are unit-tested: the model never touches
the border, always stays inside its region, clears the loader, is horizontally
centered and sits below centre by exactly `MODEL_V_BIAS`, and the drawn logo
stays within 0.75 cells of its frame center over a full spin (measured 0.50).

## Layout

```
src/
  main.rs    entry point, event loop, live resize, flags, --dump mode
  scene.rs   3D scene: extruded logo + warp starfield (painter's algorithm)
  tui.rs     the cracktro: starfield, 3D title, logo, bar, marquee, blow-away ending
  canvas.rs  4-color Game Boy palettes, shimmer, truecolor ANSI canvas
  music.rs   optional looping background music (auto-detect music/)
  hack.rs    the pluggable Hack trait + StubHack (simulated)
```

## Plugging in the real hack

The cracktro is only the *presentation layer*. Everything it displays comes
from one small interface, [`Hack`](src/hack.rs), so the whole UI is decoupled
from the real RAM-unlock work. Today it is driven by `StubHack`, which fakes a
12-second download so the visuals can be developed standalone.

### The contract

```rust
pub trait Hack {
    fn start(&mut self) -> Result<(), String>;  // called once, before the loop
    fn poll(&mut self) -> HackState;            // once per frame; MUST be non-blocking
    fn finish(&mut self) -> Result<(), String>; // called once on exit (success, quit or error)
}

pub struct HackState {
    pub progress: f32,          // 0.0 ..= 1.0  -> the loader bar and percentage
    pub status: String,         // status line, e.g. "mapping pages"
    pub done: bool,             // true once finished -> freeze + blow-away ending
    pub error: Option<String>,  // Some(..) -> shown, then the program exits with it
}
```

Rules of the road:

- `poll` is called about 30x/second and **must not block**. Kick off work in
  `start` (or a background thread) and have `poll` only read the latest
  snapshot. The frame loop drives the animation; a blocking `poll` stutters it.
- `progress` is the only thing that moves the bar; `done` is the only thing
  that triggers the celebration and the exit. Nothing else in the UI changes.
- `finish` is always called, even on `q`/quit or error, so it is the place to
  stop threads, free buffers, or unlock anything you locked.

### Worked example: a C-backed hack

1. **Put the C source in the repo**, e.g. `src/hack.c`, exposing a tiny C ABI:

    ```c
    #include <string.h>
    static float g_progress; static int g_done;
    int  hack_start(void) { g_progress = 0.0f; g_done = 0; /* ... */ return 0; }
    float hack_progress(void) { return g_progress; }   /* 0..1 */
    int  hack_done(void) { return g_done; }
    int  hack_finish(void) { /* tear down */ return 0; }
    void hack_status(char *buf, size_t len) { snprintf(buf, len, "mapping pages"); }
    ```

    For a long-running job, have `hack_start` spawn a thread that updates
    `g_progress`/`g_done` and returns immediately.

2. **Compile and link it** with a `build.rs` (add `cc` as a build-dependency in
   `Cargo.toml`):

    ```rust
    // build.rs
    fn main() { cc::Build::new().file("src/hack.c").compile("hack"); }
    ```

    ```toml
    [build-dependencies]
    cc = "1"
    ```

3. **Declare the ABI and implement `Hack`** in `src/hack.rs` (there is a
   commented-out example at the bottom of that file you can uncomment):

    ```rust
    #[link(name = "hack")]
    extern "C" {
        fn hack_start() -> i32;
        fn hack_progress() -> f32;
        fn hack_status(buf: *mut u8, len: usize);
        fn hack_done() -> i32;
        fn hack_finish() -> i32;
    }

    pub struct CHack;

    impl Hack for CHack {
        fn start(&mut self) -> Result<(), String> {
            if unsafe { hack_start() } == 0 { Ok(()) } else { Err("start failed".into()) }
        }
        fn poll(&mut self) -> HackState {
            let mut buf = [0u8; 128];
            unsafe { hack_status(buf.as_mut_ptr(), buf.len()) };
            let status = String::from_utf8_lossy(&buf)
                .split('\0').next().unwrap_or("").to_string();
            HackState {
                progress: unsafe { hack_progress() }.clamp(0.0, 1.0),
                status,
                done: unsafe { hack_done() } != 0,
                error: None,
            }
        }
        fn finish(&mut self) -> Result<(), String> {
            unsafe { hack_finish() };
            Ok(())
        }
    }
    ```

4. **Swap it in** at the two places the stub is constructed (the interactive
   loop and `--dump`), in `src/main.rs`:

    ```rust
    let mut hack: Box<dyn hack::Hack> = Box::new(hack::CHack);
    ```

That is the entire integration. The starfield, 3D model, loader, scroller and
blow-away ending are driven purely by `HackState`, so they need no changes.
Use `--dump` to eyeball frames headlessly while wiring things up.

## Cracktro notes

- **Starfield**: warp effect — stars fly toward the viewer from center
  (`scene.rs::Starfield`). The brightest stars pop in the lightest shade.
- **3D logo**: the FBR nano logo is an embedded OBJ model
  (`src/model_data.rs`, generated from `fbrnanologo.obj` by `gen_model.py`),
  rotated + perspective-projected, backface-culled, depth-sorted, flat-shaded.
  It rasterizes to one brightness sample per cell, which the TUI draws as a
  **dithered block** (`░▒▓█`): every covered cell gets at least `░`, so the
  silhouette is always solid, and the character density carries the shading —
  the four levels are spread over the model's real brightness range and
  ordered-dithered (4x4 Bayer) so intermediate tones appear. Color is the mid
  palette shade, brightening to the lightest for highlights. The projection
  pre-compensates for the ~1:2 cell aspect, so the model stays square. It
  auto-spins clockwise, a rare shimmer flashes it bright, and a slowly orbiting
  light drives a specular **sheen** (Blinn-Phong in `scene.rs`). Spin:
  `SPIN_SPEED` / `SPIN_DIR` in `scene.rs`.
- **Centering**: the mesh is translated onto its own centroid before rotating,
  so the logo spins about its middle and projects symmetrically. Horizontally
  the layout centers it exactly; vertically it is placed deliberately **below**
  centre by `MODEL_V_BIAS` in `tui.rs` (0.5 = dead centre, 1.0 = flush above the
  loader), because the lit top face draws the eye upward — raise the constant to
  move the logo further down. Measured horizontal drift over a full spin is 0.5
  cells, asserted by a unit test.
- **Mouse grab + flick**: click and drag the logo to rotate it; release to
  fling it with momentum. It damps back into the auto-spin. (crossterm mouse
  capture; pitch is clamped so it can't be flipped over.)
- **Title**: chunky 4-row block glyphs (`BIG_FONT` in `tui.rs`) with a masked
  drop shadow, shimmering through the palette.
- **Loader**: the progress bar sits just above the scroller with a blank
  breathing row between them. It is shade 2 only, with a shade-1 **dithered
  shimmer** (`█▓▒`) that sweeps the whole bar in complete passes, Windows-7
  style, independent of progress.
- **Scroller**: a straight C64-style scroller along the bottom border,
  chars color-cycled through the palette shades.
- **Hints**: a dim `M MUTE • Q QUIT` sits right-aligned in the top border when
  there is room clear of the title. On terminals below 80x24 the status line
  asks you to `RESIZE FOR BEST FIT`.
- **Responsive layout**: `tui::layout(w, h)` is a closed-form function of the
  terminal size and the single source of truth for every position (see its doc
  comment for the formulas). The model picks the largest size from a fixed
  breakpoint table — `48x24, 40x20, 32x16, 24x12, 18x9, 14x7, 10x5` cells —
  that fits both the available width and the rows between the title and the
  loader, then is centered with `x = (w - mw) / 2`, `y = (h - mh) / 2` clamped
  into the region. Below the smallest breakpoint the model is omitted rather
  than drawn as a sliver. `--layout W H` prints the result.
- **Resize robustness**: the terminal size is re-read every frame (a cheap
  ioctl) rather than trusted only from `Resize` events, which some terminals
  and multiplexers do not deliver reliably — a stale size would mis-frame the
  whole composition.
- **Containment**: every element is clamped to the inner area; only the frame
  characters ever touch row 0 / row h-1 / column 0 / column w-1.
- **Blow-away ending**: on `done`, the whole screen freezes, then blows
  away — a gust sweeps left to right and every character falls off the
  bottom (`tui.rs::Celebration`), leaving a blinking "RAM UNLOCKED • DONE".
  The celebration runs ~3.5 s, then the program exits.
- **Live resize**: btop-style — on `Resize`, the reflowed content is cleared
  and the new frame is redrawn in the same write batch (no blank flash). The
  layout is derived from the terminal size each frame, so it reflows.
- **Greetings**: edit `GREETINGS` in `src/tui.rs` to change the bottom
  scroller text.
